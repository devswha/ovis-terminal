//! An SSH loopback without sshd, so `machine.add` over SSH runs end to end.
//!
//! An `ssh` first on the fixture's search path accepts only the runtime's fixed SSH
//! transport argument vector for [`DESTINATION`] and carries its stdio to the peer through
//! the binary under test: the real argument vector, handshake and credential check.

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

/// The SSH destination that reaches the peer runtime.
pub(crate) const DESTINATION: &str = "peer.loopback";

/// The arguments `ssh` must receive, exactly as the runtime's SSH transport builds them.
const EXPECTED: &str = "-T|-e|none|-o|BatchMode=yes|-o|StrictHostKeyChecking=yes|--|peer.loopback|ovis-terminal|connect";

/// Writes `<bin>/ssh` reaching the peer listening on `socket` through `binary connect`.
pub(super) fn install(bin: &Path, (binary, socket): (&Path, &Path)) -> std::io::Result<()> {
    std::fs::create_dir_all(bin)?;
    let script = format!(
        "#!/bin/sh\nactual=$(printf '%s|' \"$@\")\nactual=${{actual%|}}\n\
         if [ \"$actual\" != '{EXPECTED}' ]; then\n  \
         printf 'ssh loopback: refused arguments %s\\n' \"$actual\" >&2\n  exit 255\nfi\n\
         OVIS_TERMINAL_SOCKET='{}'\nexport OVIS_TERMINAL_SOCKET\nexec '{}' connect\n",
        socket.display(),
        binary.display(),
    );
    let shim = bin.join("ssh");
    std::fs::write(&shim, script)?;
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755))
}
