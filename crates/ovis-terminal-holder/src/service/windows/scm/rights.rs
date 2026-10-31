//! Granting the service logon right to the account a holder service runs as.
//!
//! `CreateServiceW` does not grant `SeServiceLogonRight`, and without it the manager
//! cannot start a service under a user account. Installing is an explicit administrative
//! act, so the grant happens there and nowhere else.

use std::io;

use ovis_terminal_protocol::domain::windows_service::ServiceAccount;
use windows_sys::Win32::Security::Authentication::Identity::{
    LSA_HANDLE, LSA_OBJECT_ATTRIBUTES, LSA_UNICODE_STRING, LsaAddAccountRights, LsaClose,
    LsaNtStatusToWinError, LsaOpenPolicy, POLICY_CREATE_ACCOUNT, POLICY_LOOKUP_NAMES,
};
use windows_sys::Win32::Security::{LookupAccountNameW, SID_NAME_USE};

use super::super::HostError;
use super::super::pipe::wide;
use super::oversized;

const SERVICE_LOGON_RIGHT: &str = "SeServiceLogonRight";

struct Policy(LSA_HANDLE);

impl Drop for Policy {
    fn drop(&mut self) {
        // SAFETY: the policy handle was opened by LsaOpenPolicy and is closed once.
        unsafe { LsaClose(self.0) };
    }
}

fn nt(status: i32, operation: &'static str) -> Result<(), HostError> {
    if status == 0 {
        return Ok(());
    }
    // SAFETY: the conversion only maps a status code.
    let code = unsafe { LsaNtStatusToWinError(status) };
    let code = i32::try_from(code).unwrap_or(i32::MAX);
    Err(HostError::os(operation, io::Error::from_raw_os_error(code)))
}

fn account_sid(account: &ServiceAccount) -> Result<Vec<u64>, HostError> {
    // LookupAccountNameW names local accounts without the `.\` that the manager takes.
    let local = account
        .as_str()
        .strip_prefix(".\\")
        .unwrap_or(account.as_str());
    let name = wide(local);
    let (mut sid_bytes, mut domain_units, mut use_) = (0_u32, 0_u32, SID_NAME_USE::default());
    // SAFETY: null buffers with zero sizes only ask for the required sizes.
    unsafe {
        LookupAccountNameW(
            std::ptr::null(),
            name.as_ptr(),
            std::ptr::null_mut(),
            &raw mut sid_bytes,
            std::ptr::null_mut(),
            &raw mut domain_units,
            &raw mut use_,
        )
    };
    let words = usize::try_from(sid_bytes).map_err(oversized)?.div_ceil(8);
    let mut sid = vec![0_u64; words.max(1)];
    let mut domain = vec![0_u16; usize::try_from(domain_units).map_err(oversized)?.max(1)];
    // SAFETY: both buffers are live and at least as large as the sizes passed.
    let found = unsafe {
        LookupAccountNameW(
            std::ptr::null(),
            name.as_ptr(),
            sid.as_mut_ptr().cast(),
            &raw mut sid_bytes,
            domain.as_mut_ptr(),
            &raw mut domain_units,
            &raw mut use_,
        )
    };
    if found == 0 {
        return Err(HostError::os(
            "look up the service account",
            io::Error::last_os_error(),
        ));
    }
    Ok(sid)
}

/// The SID of `account`, as text.
pub(in crate::service::windows) fn account_sid_string(
    account: &ServiceAccount,
) -> Result<String, HostError> {
    let mut sid = account_sid(account)?;
    // SAFETY: `sid` holds the SID LookupAccountNameW wrote and outlives the call.
    unsafe { super::super::pipe::sid_string(sid.as_mut_ptr().cast()) }
        .map_err(|source| HostError::os("format the account SID", source))
}

/// Grants `account` the right to log on as a service; granting it twice is harmless.
pub(super) fn grant_service_logon(account: &ServiceAccount) -> Result<(), HostError> {
    let mut sid = account_sid(account)?;
    let attributes = LSA_OBJECT_ATTRIBUTES {
        Length: u32::try_from(size_of::<LSA_OBJECT_ATTRIBUTES>()).map_err(oversized)?,
        ..LSA_OBJECT_ATTRIBUTES::default()
    };
    let mut handle: LSA_HANDLE = 0;
    let access = u32::try_from(POLICY_CREATE_ACCOUNT | POLICY_LOOKUP_NAMES).map_err(oversized)?;
    // SAFETY: a null system name selects this machine; the attributes and out-handle are
    // live locals.
    let opened = unsafe {
        LsaOpenPolicy(
            std::ptr::null(),
            &raw const attributes,
            access,
            &raw mut handle,
        )
    };
    nt(opened, "open the local security policy")?;
    let policy = Policy(handle);
    let mut right: Vec<u16> = SERVICE_LOGON_RIGHT.encode_utf16().collect();
    let bytes = u16::try_from(right.len().saturating_mul(2)).map_err(oversized)?;
    let name = LSA_UNICODE_STRING {
        Length: bytes,
        MaximumLength: bytes,
        Buffer: right.as_mut_ptr(),
    };
    // SAFETY: the policy is open, `sid` holds the looked-up SID and `name` points at the
    // live right name.
    let granted =
        unsafe { LsaAddAccountRights(policy.0, sid.as_mut_ptr().cast(), &raw const name, 1) };
    nt(granted, "grant the service logon right")
}
