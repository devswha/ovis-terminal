//! Launching a holder process detached from its launcher's session.

use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

/// Spawns `command` as the leader of a new session.
///
/// The process has no controlling terminal and is outside the launcher's
/// session, so a launcher exit, SSH logout or terminal hangup does not end it.
/// The caller chooses its stdio; the returned handle may be dropped once the
/// launcher no longer needs to observe the holder.
///
/// # Errors
///
/// Returns the spawn failure, including a failed `setsid(2)` in the child.
pub fn spawn_detached(command: &mut Command) -> io::Result<Child> {
    let new_session = || {
        // SAFETY: setsid(2) takes no arguments, touches no parent memory and is
        // async-signal-safe, so it may run between fork and exec.
        if unsafe { libc::setsid() } == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    };
    // SAFETY: the hook only calls the async-signal-safe setsid(2) and allocates nothing.
    unsafe { command.pre_exec(new_session) };
    command.spawn()
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::process::{Command, Stdio};

    use super::spawn_detached;

    fn session_of(pid: u32) -> libc::pid_t {
        let pid = libc::pid_t::try_from(pid).expect("pid fits pid_t");
        // SAFETY: getsid(2) takes a PID by value and touches no memory.
        unsafe { libc::getsid(pid) }
    }

    #[test]
    fn detached_holder_leads_its_own_session() {
        // Given / When: a holder stand-in spawned detached.
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "read line"]).stdin(Stdio::piped());
        let mut child = spawn_detached(&mut command).expect("spawn detached");

        // Then: it leads a new session, apart from the launcher's.
        let pid = libc::pid_t::try_from(child.id()).expect("pid fits pid_t");
        assert_eq!(session_of(child.id()), pid);
        assert_ne!(session_of(child.id()), session_of(std::process::id()));
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(b"done\n")
            .expect("release");
        assert!(child.wait().expect("exit").success());
    }
}
