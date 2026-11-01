//! The runtime identity one state directory serves under across server lifetimes: the
//! machine and runtime stay, and every start takes the next server boot identity, so a
//! later boot can tell records of an earlier one apart from its own.

use std::io::Write as _;
use std::path::Path;

use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId,
};
use serde::{Deserialize, Serialize};

use super::AssemblyError;

const FILE: &str = "identity.json";

/// The durable part of the identity; counters are canonical decimal strings.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    machine_id: String,
    server_boot_id: String,
}

fn minted() -> Result<Stored, AssemblyError> {
    let machine: MachineId = format!("machine:{}", uuid::Uuid::now_v7())
        .parse()
        .map_err(AssemblyError::start("mint the machine identity"))?;
    Ok(Stored {
        machine_id: machine.to_string(),
        server_boot_id: "0".to_owned(),
    })
}

fn read(state: &Path) -> Result<Stored, AssemblyError> {
    match std::fs::read(state.join(FILE)) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(AssemblyError::start("decode the identity"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => minted(),
        Err(error) => Err(AssemblyError::start("read the identity")(error)),
    }
}

/// Writes `stored` beside the old file, syncs it, then renames it over the old one.
fn publish(state: &Path, stored: &Stored) -> Result<(), AssemblyError> {
    let staged = state.join(format!("{FILE}.next"));
    let bytes = serde_json::to_vec(stored).map_err(AssemblyError::start("encode the identity"))?;
    let mut file =
        std::fs::File::create(&staged).map_err(AssemblyError::start("stage the identity"))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(AssemblyError::start("stage the identity"))?;
    std::fs::rename(&staged, state.join(FILE)).map_err(AssemblyError::start("publish the identity"))
}

/// Takes the next server boot identity under `state` and returns the identity to serve.
pub(super) fn next_boot(state: &Path) -> Result<RuntimeIdentity, AssemblyError> {
    let stored = read(state)?;
    let machine: MachineId = stored
        .machine_id
        .parse()
        .map_err(AssemblyError::start("decode the machine identity"))?;
    let previous: u64 = stored
        .server_boot_id
        .parse()
        .map_err(AssemblyError::start("decode the server boot identity"))?;
    let next = previous
        .checked_add(1)
        .ok_or_else(|| AssemblyError::start("take the next server boot")("boot counter spent"))?;
    let boot =
        ServerBootId::try_from(next).map_err(AssemblyError::start("server boot identity"))?;
    publish(
        state,
        &Stored {
            machine_id: stored.machine_id,
            server_boot_id: next.to_string(),
        },
    )?;
    let host = HostBootId::try_from(1).map_err(AssemblyError::start("host boot identity"))?;
    let runtime = RuntimeId::try_from(1).map_err(AssemblyError::start("runtime identity"))?;
    Ok(RuntimeIdentity::new(
        HostIdentity::new(machine, host),
        runtime,
        boot,
    ))
}

#[cfg(test)]
mod tests {
    use super::{FILE, next_boot};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            match std::fs::remove_dir_all(&self.0) {
                Ok(()) | Err(_) => {}
            }
        }
    }

    fn scratch() -> Result<Scratch, std::io::Error> {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("ovt31i-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path)?;
        Ok(Scratch(path))
    }

    #[test]
    fn each_start_takes_the_next_boot_and_keeps_the_machine() -> TestResult {
        let state = scratch()?;
        let first = next_boot(state.path())?;
        let second = next_boot(state.path())?;
        assert_eq!(first.server_boot_id().get(), 1);
        assert_eq!(second.server_boot_id().get(), 2);
        assert_eq!(first.machine_id(), second.machine_id());
        Ok(())
    }

    #[test]
    fn the_last_boot_round_trips_and_a_spent_counter_is_refused() -> TestResult {
        let state = scratch()?;
        let machine = next_boot(state.path())?.machine_id();
        let near_end = u64::MAX - 1;
        std::fs::write(
            state.path().join(FILE),
            format!(r#"{{"machine_id":"{machine}","server_boot_id":"{near_end}"}}"#),
        )?;
        assert_eq!(next_boot(state.path())?.server_boot_id().get(), u64::MAX);
        let spent = next_boot(state.path())
            .err()
            .ok_or("a spent counter was accepted")?;
        assert_eq!(spent.kind(), "serve_failed");
        Ok(())
    }

    #[test]
    fn a_numeric_boot_counter_is_refused() -> TestResult {
        let state = scratch()?;
        let machine = next_boot(state.path())?.machine_id();
        std::fs::write(
            state.path().join(FILE),
            format!(r#"{{"machine_id":"{machine}","server_boot_id":3}}"#),
        )?;
        assert!(next_boot(state.path()).is_err());
        Ok(())
    }
}
