use std::io::Write as _;
use std::sync::mpsc;

use ovis_terminal_vt::OutputOffset;

use super::super::{
    BlockingGate, DbWriter, OutputEvent, PumpEnd, PumpThread, SchedulerError, spawn_terminal,
};
use super::{WAIT, await_event, setup, vt};

#[test]
fn the_db_writer_refuses_past_its_queue_and_joins_with_every_accepted_job() {
    let writer = DbWriter::spawn("t56-db", Vec::<u32>::new(), 1).expect("thread starts");
    let (started_tx, started) = mpsc::channel();
    let (release, gate) = mpsc::channel::<()>();
    let first = writer
        .try_submit(move |rows: &mut Vec<u32>| {
            started_tx.send(()).expect("test listens");
            gate.recv().expect("test releases");
            rows.push(1);
        })
        .expect("queued");
    started.recv_timeout(WAIT).expect("the first job runs");
    let second = writer
        .try_submit(|rows: &mut Vec<u32>| rows.push(2))
        .expect("one queue slot");
    let refused = writer
        .try_submit(|rows: &mut Vec<u32>| rows.push(3))
        .expect_err("the queue is full");
    assert!(matches!(
        refused,
        SchedulerError::WriterBusy { capacity: 1 }
    ));
    assert_eq!(refused.kind(), "backpressure");

    release.send(()).expect("writer listens");
    first.blocking_recv().expect("first ran");
    second.blocking_recv().expect("second ran");
    assert_eq!(writer.shutdown().expect("joined"), vec![1, 2]);
}

#[test]
fn a_panicking_db_job_is_reported_at_shutdown() {
    let writer = DbWriter::spawn("t56-db-panic", 0_u8, 4).expect("thread starts");
    let result = writer
        .try_submit(|_: &mut u8| -> u8 { panic!("job fails") })
        .expect("queued");
    assert!(
        result.blocking_recv().is_err(),
        "a panicked job sends nothing"
    );
    let ended = writer.shutdown().expect_err("the thread panicked");
    assert_eq!(ended.kind(), "internal");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_blocking_gate_refuses_one_past_its_slots() {
    let gate = BlockingGate::new(2);
    let (started_tx, started) = mpsc::channel();
    let mut releases = Vec::new();
    let mut jobs = Vec::new();
    for index in 0..2_u8 {
        let (release, wait) = mpsc::channel::<()>();
        let started_tx = started_tx.clone();
        jobs.push(
            gate.submit(move || {
                started_tx.send(()).expect("test listens");
                wait.recv().expect("test releases");
                index
            })
            .expect("a slot is free"),
        );
        releases.push(release);
    }
    for _ in 0..2 {
        started.recv_timeout(WAIT).expect("both jobs run");
    }
    let refused = gate.submit(|| 9_u8).expect_err("every slot is busy");
    assert!(matches!(
        refused,
        SchedulerError::BlockingSaturated { limit: 2 }
    ));
    assert_eq!(gate.available(), 0);
    for release in releases {
        release.send(()).expect("job listens");
    }
    let mut done = Vec::new();
    for job in jobs {
        done.push(job.await.expect("joined"));
    }
    assert_eq!(done, vec![0, 1]);
    assert_eq!(gate.available(), 2);
    assert_eq!(
        gate.submit(|| 9_u8).expect("free again").await.ok(),
        Some(9)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_owned_pump_carries_output_until_eof_and_is_joined() {
    let (handle, feed, task) = spawn_terminal(vt(), tokio::io::sink(), &setup());
    let mut events = handle.subscribe_output();
    let (reader, mut writer) = std::io::pipe().expect("pipe");
    let pump = PumpThread::spawn("t56-pump", reader, feed).expect("thread starts");
    let line = b"hello from the program\r\n";
    for _ in 0..1000 {
        writer.write_all(line).expect("pipe write");
    }
    drop(writer);
    let report = tokio::task::spawn_blocking(move || pump.join())
        .await
        .expect("join task")
        .expect("pump joined");
    let total = u64::try_from(line.len() * 1000).expect("fits");
    assert_eq!((report.bytes, report.end), (total, PumpEnd::Eof));
    await_event(&mut events, |event| match event {
        OutputEvent::Parsed { offset } if *offset == OutputOffset::new(total) => Some(()),
        _ => None,
    })
    .await;

    handle.stop().expect("stop queued");
    task.join().await.expect("actor joined");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pump_stops_when_its_actor_has_stopped() {
    let (handle, feed, task) = spawn_terminal(vt(), tokio::io::sink(), &setup());
    handle.stop().expect("stop queued");
    task.join().await.expect("actor joined");
    let (reader, mut writer) = std::io::pipe().expect("pipe");
    writer.write_all(b"late output").expect("pipe write");
    let stopped = tokio::task::spawn_blocking(move || {
        PumpThread::spawn("t56-pump-orphan", reader, feed)?.join()
    });
    let report = stopped.await.expect("join task").expect("pump joined");
    assert_eq!((report.bytes, report.end), (0, PumpEnd::ActorStopped));
    drop(writer);
}
