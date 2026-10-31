//! The owner and DACL the operating system applied to a pipe instance, read back from
//! the handle, so endpoint admission judges what Windows enforces rather than what the
//! holder intended.

use std::io;
use std::os::windows::io::AsRawHandle;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
    DACL_SECURITY_INFORMATION, GetAce, GetAclInformation, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID,
};

use super::{check, sid_string};

const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;

/// One entry of an applied DACL.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AceEntry {
    /// Whether the entry allows (rather than denies) access.
    pub allow: bool,
    /// The trustee's SID.
    pub trustee: String,
}

/// The owner and DACL of one pipe instance; a `None` DACL is a null DACL.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct InstanceSecurity {
    /// The owner's SID.
    pub owner: String,
    /// The DACL entries, `None` for a null DACL that allows everyone.
    pub dacl: Option<Vec<AceEntry>>,
}

struct Descriptor(PSECURITY_DESCRIPTOR);

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: GetSecurityInfo LocalAlloc'd the descriptor; it is freed once.
        unsafe { LocalFree(self.0) };
    }
}

fn entries(dacl: *const ACL) -> io::Result<Vec<AceEntry>> {
    let mut size = ACL_SIZE_INFORMATION::default();
    let length = u32::try_from(size_of::<ACL_SIZE_INFORMATION>()).map_err(io::Error::other)?;
    // SAFETY: `dacl` points into the live descriptor; the out-buffer is a live local of
    // the type the information class names.
    check(unsafe { GetAclInformation(dacl, (&raw mut size).cast(), length, AclSizeInformation) })?;
    let mut found = Vec::new();
    for index in 0..size.AceCount {
        let mut ace: *mut std::ffi::c_void = std::ptr::null_mut();
        // SAFETY: `index` is below the ACL's entry count and `ace` is a live out-pointer.
        check(unsafe { GetAce(dacl, index, &raw mut ace) })?;
        // SAFETY: every ACE starts with an ACE_HEADER.
        let kind = unsafe { (*ace.cast::<ACE_HEADER>()).AceType };
        let allow = match kind {
            ACCESS_ALLOWED_ACE_TYPE => true,
            ACCESS_DENIED_ACE_TYPE => false,
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("the pipe DACL holds an ACE of unexpected type {other}"),
                ));
            }
        };
        // Allowed and denied ACEs share one layout: header, mask, then the SID.
        // SAFETY: `ace` is an ACCESS_ALLOWED_ACE or ACCESS_DENIED_ACE; only the field
        // address is taken, nothing is read.
        let sid: PSID = unsafe { (&raw mut (*ace.cast::<ACCESS_ALLOWED_ACE>()).SidStart).cast() };
        // SAFETY: the SID lies inside the live descriptor for the call.
        let trustee = unsafe { sid_string(sid) }?;
        found.push(AceEntry { allow, trustee });
    }
    Ok(found)
}

/// Reads the owner and DACL Windows applied to `pipe`.
///
/// # Errors
///
/// The query failure, or an ACE type no pipe of this runtime carries.
pub fn instance_security(pipe: &impl AsRawHandle) -> io::Result<InstanceSecurity> {
    let mut owner: PSID = std::ptr::null_mut();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut raw: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: the pipe handle is open; the out-pointers are live locals; group and SACL
    // are not requested, so their pointers may be null.
    let status = unsafe {
        GetSecurityInfo(
            pipe.as_raw_handle(),
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &raw mut owner,
            std::ptr::null_mut(),
            &raw mut dacl,
            std::ptr::null_mut(),
            &raw mut raw,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(
            i32::try_from(status).map_err(io::Error::other)?,
        ));
    }
    let descriptor = Descriptor(raw);
    // SAFETY: `owner` points into the live descriptor.
    let owner = unsafe { sid_string(owner) }?;
    let dacl = if dacl.is_null() {
        None
    } else {
        Some(entries(dacl)?)
    };
    drop(descriptor);
    Ok(InstanceSecurity { owner, dacl })
}
