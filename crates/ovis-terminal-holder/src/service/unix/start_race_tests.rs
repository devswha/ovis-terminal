//! Two starts racing to replace a crashed holder's stale endpoint: exactly one serves.

use std::os::unix::net::UnixListener;
use std::sync::{Arc, Barrier, mpsc};
use std::time::Duration;

use ovis_terminal_protocol::domain::unix_service::{BootProof, DiscoveryRecord, Observed};
use ovis_terminal_protocol::schema::ids::{ProcessPid, StartToken};

use super::test_root::TestRoot;
use super::{ServiceError, ServicePaths, observe, record, run, stop};

fn reaped_pid() -> u32 {
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

/// What a crashed holder leaves: its socket file and a record naming a dead process.
fn crash_leftovers(paths: &ServicePaths) {
    record::create_directory(paths.directory()).unwrap();
    drop(UnixListener::bind(paths.socket()).unwrap());
    let dead = BootProof::new(
        ProcessPid::new(reaped_pid()),
        StartToken::new("gone".to_owned()),
    );
    let address = paths.socket().display().to_string();
    record::write(
        &paths.record(),
        &DiscoveryRecord::new(paths.session().label(), address, dead),
    )
    .unwrap();
    assert!(matches!(
        observe(paths).unwrap(),
        Observed::StaleEndpoint { .. }
    ));
}

#[test]
fn concurrent_starts_over_a_stale_endpoint_leave_exactly_one_holder_and_type_the_loser() {
    let root = TestRoot::new("race");
    let paths = root.paths("s");
    for iteration in 0..50 {
        crash_leftovers(&paths);
        let (ready, published) = mpsc::channel::<DiscoveryRecord>();
        let (finished, outcomes) = mpsc::channel::<Result<(), ServiceError>>();
        let gate = Arc::new(Barrier::new(2));
        let starters: Vec<_> = (0..2)
            .map(|_| {
                let (paths, ready, finished) = (paths.clone(), ready.clone(), finished.clone());
                let gate = Arc::clone(&gate);
                std::thread::spawn(move || {
                    gate.wait();
                    let outcome = run(&paths, None, |record| {
                        ready
                            .send(record.clone())
                            .map_err(|_| ServiceError::NotRunning)
                    });
                    finished.send(outcome).unwrap();
                })
            })
            .collect();
        drop((ready, finished));
        let winner = published.recv_timeout(Duration::from_secs(30)).unwrap();
        let loser = outcomes.recv_timeout(Duration::from_secs(30)).unwrap();
        let Err(ServiceError::AlreadyRunning {
            holder: Some(holder),
        }) = loser
        else {
            panic!("iteration {iteration}: the loser was not a typed already_running: {loser:?}");
        };
        assert_eq!(*holder, winner.boot_proof, "iteration {iteration}");
        assert_eq!(
            published.try_recv().ok(),
            None,
            "iteration {iteration}: two holders announced ready"
        );
        assert_eq!(
            observe(&paths).unwrap(),
            Observed::Running {
                record: winner.clone()
            }
        );
        let receipt = stop(&paths).unwrap();
        assert_eq!(receipt.ended_holder, winner.boot_proof);
        outcomes
            .recv_timeout(Duration::from_secs(30))
            .unwrap()
            .unwrap();
        for starter in starters {
            starter.join().unwrap();
        }
        assert_eq!(
            observe(&paths).unwrap(),
            Observed::Stopped,
            "iteration {iteration}"
        );
    }
}
