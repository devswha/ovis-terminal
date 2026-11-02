//! `install-probe` (plan todo 68): runs the literal `sh install.sh` commands of every
//! installer journey against local fixture releases and reports what each did.
//!
//! The good release is the binary under test staged and sealed by `release-stage`
//! (todo 67), published with a release manifest in the GitHub layout and served over a
//! loopback HTTP server; the adversarial releases are derived from it (one flipped byte, a
//! `..` member, an unshipped platform, a version the binary does not report). Each case
//! runs in a private home, XDG tree and attempt directory and checks the install
//! directory, the binary hash and everything outside it before and after.
//!
//! Exit status: 0 every case passed, 1 a case failed, 2 usage, 3 the probe could not run.

mod adversarial;
mod archive;
mod cases;
mod cli;
mod error;
mod fixture;
mod probe;
mod report;
mod run;
mod serve;
mod world;

pub use cases::CASES;
pub use cli::{InstallRun, execute, main};
pub use error::InstallProbeError;
