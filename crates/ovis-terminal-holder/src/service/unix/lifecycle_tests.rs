//! The holder service end to end in one process: run, observe, stop, observe.

use std::sync::mpsc;
use std::time::Duration;

use ovis_terminal_protocol::domain::unix_service::{DiscoveryRecord, Observed};

use super::test_root::TestRoot;
use super::{observe, run, stop};

#[test]
fn a_started_holder_is_running_until_stopped_and_stopping_enumerates_and_withdraws_it() {
    let root = TestRoot::new("life");
    let paths = root.paths("s");
    let (ready, published) = mpsc::channel::<DiscoveryRecord>();
    let serving = std::thread::spawn({
        let paths = paths.clone();
        move || {
            run(&paths, None, |record| {
                ready
                    .send(record.clone())
                    .map_err(|_| super::ServiceError::NotRunning)
            })
        }
    });
    let record = published.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(record.boot_proof.pid.get(), std::process::id());
    for _ in 0..2 {
        assert_eq!(
            observe(&paths).unwrap(),
            Observed::Running {
                record: record.clone()
            }
        );
    }
    let second = run(&paths, None, |_| Ok(())).map_err(|e| e.kind());
    assert_eq!(second, Err("already_running"), "a live holder was replaced");
    let receipt = stop(&paths).unwrap();
    assert_eq!(receipt.ended_holder, record.boot_proof);
    assert_eq!(receipt.ended_terminals, Vec::new());
    serving.join().unwrap().unwrap();
    assert_eq!(observe(&paths).unwrap(), Observed::Stopped);
    assert_eq!(
        stop(&paths).map(drop).map_err(|e| e.kind()),
        Err("already_stopped")
    );
    assert_eq!(
        std::fs::read_dir(paths.directory()).unwrap().count(),
        0,
        "socket or record left behind"
    );
}

#[test]
fn a_stale_endpoint_refuses_stop_and_is_replaced_only_by_an_explicit_start() {
    let root = TestRoot::new("replace");
    let paths = root.paths("s");
    super::record::create_directory(paths.directory()).unwrap();
    drop(std::os::unix::net::UnixListener::bind(paths.socket()).unwrap());
    assert_eq!(
        stop(&paths).map(drop).map_err(|e| e.kind()),
        Err("stale_endpoint")
    );
    assert!(
        paths.socket().exists(),
        "stop removed evidence it did not own"
    );
    let (ready, published) = mpsc::channel::<DiscoveryRecord>();
    let serving = std::thread::spawn({
        let paths = paths.clone();
        move || {
            run(&paths, None, |record| {
                ready
                    .send(record.clone())
                    .map_err(|_| super::ServiceError::NotRunning)
            })
        }
    });
    let record = published.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(observe(&paths).unwrap(), Observed::Running { record });
    stop(&paths).unwrap();
    serving.join().unwrap().unwrap();
}
