//! Owner-only named pipes.
//!
//! A security descriptor built from the account's own SID, pipe instances that refuse
//! remote clients, the connected client's account, and the descriptor the operating
//! system actually applied.
//!
//! There is no fallback to default security: a descriptor that cannot be built fails
//! the bind, so no pipe is ever created with the default or a null DACL.

use std::ffi::c_void;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle as _, OwnedHandle};

use ovis_terminal_protocol::domain::windows_service::PipeName;
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SEM_TIMEOUT, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, PSID, RevertToSelf, SECURITY_ATTRIBUTES,
    TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Pipes::{ImpersonateNamedPipeClient, WaitNamedPipeW};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken,
};

use super::PipePresence;

mod applied;

pub use applied::{AceEntry, InstanceSecurity, instance_security};

/// A NUL-terminated UTF-16 copy of `text`.
pub(super) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn check(succeeded: i32) -> io::Result<()> {
    if succeeded == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Reads a NUL-terminated UTF-16 string the system allocated, then frees it.
///
/// # Safety
///
/// `text` must be a live `LocalAlloc` allocation holding a NUL-terminated string.
pub(super) unsafe fn take_local_string(text: *mut u16) -> String {
    // SAFETY: the caller guarantees a NUL-terminated string.
    let owned = unsafe { read_wide(text) };
    // SAFETY: `text` was allocated by the system with LocalAlloc and is freed once here.
    unsafe { LocalFree(text.cast()) };
    owned
}

/// Copies a NUL-terminated UTF-16 string.
///
/// # Safety
///
/// `text` must point at a live NUL-terminated UTF-16 string.
pub(super) unsafe fn read_wide(text: *const u16) -> String {
    let mut length = 0_usize;
    loop {
        // SAFETY: every index up to and including the terminator is inside the string.
        let unit = unsafe { text.add(length) };
        // SAFETY: `unit` is inside the string, as above.
        if unsafe { unit.read() } == 0 {
            break;
        }
        length = length.saturating_add(1);
    }
    // SAFETY: `length` units starting at `text` were just read as initialized.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, length) })
}

/// The string form of `sid`.
///
/// # Safety
///
/// `sid` must point at a valid SID for the duration of the call.
pub(super) unsafe fn sid_string(sid: PSID) -> io::Result<String> {
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: the caller guarantees `sid`; `text` is a live local out-pointer.
    check(unsafe { ConvertSidToStringSidW(sid, &raw mut text) })?;
    // SAFETY: on success the call returned a LocalAlloc'd NUL-terminated string.
    Ok(unsafe { take_local_string(text) })
}

fn token_user(token: HANDLE) -> io::Result<String> {
    let mut needed = 0_u32;
    // SAFETY: a null buffer of length zero only asks for the required size.
    unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &raw mut needed) };
    let words = usize::try_from(needed)
        .map_err(io::Error::other)?
        .div_ceil(size_of::<usize>());
    let mut buffer = vec![0_usize; words.max(1)];
    // SAFETY: the buffer is live, aligned for TOKEN_USER and at least `needed` bytes.
    check(unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &raw mut needed,
        )
    })?;
    let user = buffer.as_ptr().cast::<TOKEN_USER>();
    // SAFETY: the call filled the buffer with a TOKEN_USER whose SID points into it.
    let sid = unsafe { (*user).User.Sid };
    // SAFETY: `sid` points into `buffer`, which outlives the call.
    unsafe { sid_string(sid) }
}

/// The SID of the account this process runs as.
///
/// # Errors
///
/// The token query failure.
pub fn current_user_sid() -> io::Result<String> {
    let mut raw: HANDLE = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess has no preconditions; its pseudo-handle needs no closing.
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: `process` names this process and `raw` is a live out-pointer.
    check(unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut raw) })?;
    // SAFETY: the call returned a new token handle that nothing else owns.
    let token = unsafe { OwnedHandle::from_raw_handle(raw) };
    token_user(token.as_raw_handle())
}

struct LocalDescriptor(PSECURITY_DESCRIPTOR);

// SAFETY: the descriptor is written once at creation and only read afterwards, by the
// kernel during pipe creation; no thread mutates it.
unsafe impl Send for LocalDescriptor {}
// SAFETY: as above, shared references only read it.
unsafe impl Sync for LocalDescriptor {}

impl Drop for LocalDescriptor {
    fn drop(&mut self) {
        // SAFETY: the descriptor was LocalAlloc'd by the SDDL conversion and is freed once.
        unsafe { LocalFree(self.0) };
    }
}

/// A security descriptor owned by one account whose protected DACL allows only it.
pub struct OwnerSecurity {
    descriptor: LocalDescriptor,
    owner: String,
}

impl std::fmt::Debug for OwnerSecurity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnerSecurity")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

impl OwnerSecurity {
    /// `O:<sid>D:P(A;;GA;;;<sid>)`: owned by `sid`, inheritance blocked, only `sid` allowed.
    ///
    /// # Errors
    ///
    /// The conversion failure; no weaker descriptor is substituted.
    pub fn for_owner(sid: &str) -> io::Result<Self> {
        let sddl = wide(&format!("O:{sid}D:P(A;;GA;;;{sid})"));
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated and live; the out-pointers are live locals.
        check(unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &raw mut descriptor,
                std::ptr::null_mut(),
            )
        })?;
        Ok(Self {
            descriptor: LocalDescriptor(descriptor),
            owner: sid.to_owned(),
        })
    }

    /// The owning account's SID.
    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }
}

/// Creates one instance of `name` with `security`, refusing remote clients.
///
/// `first` sets `FILE_FLAG_FIRST_PIPE_INSTANCE`: creation then fails with access denied
/// while any other process holds an instance of the name, so a squatter is never joined.
///
/// # Errors
///
/// The creation failure; it must run inside a Tokio runtime.
pub fn create_instance(
    name: &PipeName,
    first: bool,
    security: &OwnerSecurity,
) -> io::Result<NamedPipeServer> {
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).map_err(io::Error::other)?,
        lpSecurityDescriptor: security.descriptor.0,
        bInheritHandle: 0,
    };
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true);
    // SAFETY: `attributes` is a live SECURITY_ATTRIBUTES whose descriptor `security`
    // keeps alive for the duration of the call.
    unsafe {
        options.create_with_security_attributes_raw(
            name.as_str(),
            (&raw mut attributes).cast::<c_void>(),
        )
    }
}

/// The account of the client connected to `pipe`, read by impersonating it.
///
/// The server must already have read from the pipe; impersonation is reverted before
/// returning, on success and on failure alike.
///
/// # Errors
///
/// The impersonation, token or revert failure.
pub fn client_sid(pipe: &impl AsRawHandle) -> io::Result<String> {
    // SAFETY: the pipe handle is open for the duration of the call.
    check(unsafe { ImpersonateNamedPipeClient(pipe.as_raw_handle()) })?;
    let mut raw: HANDLE = std::ptr::null_mut();
    // SAFETY: GetCurrentThread has no preconditions; its pseudo-handle needs no closing.
    let thread = unsafe { GetCurrentThread() };
    // SAFETY: `thread` names this thread and `raw` is a live out-pointer; OpenAsSelf
    // checks access against this process, not the impersonated client.
    let opened = check(unsafe { OpenThreadToken(thread, TOKEN_QUERY, 1, &raw mut raw) });
    // SAFETY: ends the impersonation this thread started above.
    let reverted = check(unsafe { RevertToSelf() });
    reverted?;
    opened?;
    // SAFETY: the call returned a new token handle that nothing else owns.
    let token = unsafe { OwnedHandle::from_raw_handle(raw) };
    token_user(token.as_raw_handle())
}

/// Whether `name` has an instance, without opening or disturbing it.
///
/// # Errors
///
/// A failure other than absence or every instance being busy.
pub fn presence(name: &PipeName) -> io::Result<PipePresence> {
    let wide_name = wide(name.as_str());
    // SAFETY: `wide_name` is NUL-terminated and live for the call.
    if unsafe { WaitNamedPipeW(wide_name.as_ptr(), 1) } != 0 {
        return Ok(PipePresence::Listening);
    }
    let error = io::Error::last_os_error();
    match error
        .raw_os_error()
        .and_then(|code| u32::try_from(code).ok())
    {
        Some(ERROR_FILE_NOT_FOUND) => Ok(PipePresence::Absent),
        Some(ERROR_SEM_TIMEOUT) => Ok(PipePresence::Listening),
        _ => Err(error),
    }
}
