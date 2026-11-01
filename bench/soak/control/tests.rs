//! The hub on real sockets: a viewer that never reads is cut at its byte bound with an
//! explicit record, while a reading viewer gets every event in order and nobody waits.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use ovis_terminal_protocol::schema::scalar::{ByteOffset, Count};

use super::super::wire::{Event, Hello, Line};
use super::Inbound;
use super::hub::Hub;

const BOUND: Duration = Duration::from_secs(10);
const LIMIT: u64 = 64 * 1024;

fn socket(name: &str) -> PathBuf {
    let base = std::env::var_os("OVIS_FIXTURE_ROOT").map_or_else(std::env::temp_dir, PathBuf::from);
    let dir = base.join(format!("soak-hub-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("c.sock")
}

fn join(path: &PathBuf, (viewer, reads): (&str, bool)) -> UnixStream {
    let mut stream = UnixStream::connect(path).unwrap();
    let hello = Hello {
        viewer: viewer.to_owned(),
        reads,
        from_seq: None,
    };
    let line = format!("{}\n", serde_json::to_string(&hello).unwrap());
    stream.write_all(line.as_bytes()).unwrap();
    stream
}

fn pump(hub: &mut Hub, events: &Receiver<Inbound>) {
    match events.recv_timeout(BOUND).unwrap() {
        Inbound::Viewer(message) => hub.handle(message, &[]).unwrap(),
        other => panic!("unexpected inbound {other:?}"),
    }
}

fn lines(stream: &UnixStream) -> Receiver<Line> {
    let reading = stream.try_clone().unwrap();
    let (sender, received) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(reading).lines() {
            let Ok(text) = line else { return };
            if sender
                .send(serde_json::from_str::<Line>(&text).unwrap())
                .is_err()
            {
                return;
            }
        }
    });
    received
}

fn progress() -> Event {
    Event::Progress {
        offsets: (0..100)
            .map(|offset| Some(ByteOffset::new(u64::MAX.saturating_sub(offset))))
            .collect(),
        checkpoints: Count::new(0),
    }
}

#[test]
fn a_stalled_viewer_is_cut_at_its_bound_while_a_reader_misses_nothing() {
    let path = socket("stall");
    let (inbound, events) = std::sync::mpsc::sync_channel(64);
    let mut hub = Hub::bind(&path, (inbound, LIMIT)).unwrap();
    let stalled = join(&path, ("stalled", false));
    pump(&mut hub, &events);
    let reader = join(&path, ("reader", true));
    pump(&mut hub, &events);
    let received = lines(&reader);
    let mut seen = Vec::new();
    let mut cut = None;
    // Each publish waits for the reader's copy, so only the stalled viewer accumulates.
    for _ in 0..20_000 {
        hub.publish(progress()).unwrap();
        loop {
            let Line::Event { seq, event } = received.recv_timeout(BOUND).unwrap() else {
                panic!("the reader got a non-event line");
            };
            seen.push(seq.get());
            match event {
                Event::ViewerCut {
                    viewer,
                    queued_bytes,
                    ..
                } => cut = Some((viewer, queued_bytes.get())),
                Event::Progress { .. } => break,
                other => panic!("unexpected event {other:?}"),
            }
        }
        if cut.is_some() {
            break;
        }
    }
    let (viewer, queued) = cut.expect("the stalled viewer was never cut");
    assert_eq!(viewer, "stalled");
    assert!(queued <= LIMIT, "{queued} queued over the {LIMIT} bound");
    assert_eq!(
        seen,
        (0..u64::try_from(seen.len()).unwrap()).collect::<Vec<_>>()
    );
    let health = hub.health(&[]);
    let by_name = |name: &str| {
        health
            .viewers
            .iter()
            .find(|v| v.viewer == name)
            .unwrap()
            .clone()
    };
    assert!(by_name("stalled").cut);
    assert!(!by_name("reader").cut);
    assert!(by_name("stalled").max_queued_bytes.get() <= LIMIT);
    drop((stalled, reader));
    hub.close().unwrap();
    assert!(!path.exists(), "close removes the socket");
}

#[test]
fn a_resumed_viewer_gets_an_explicit_gap_for_events_no_longer_retained() {
    let path = socket("gap");
    let (inbound, events) = std::sync::mpsc::sync_channel(64);
    let mut hub = Hub::bind(&path, (inbound, LIMIT * 64)).unwrap();
    for _ in 0..600 {
        hub.publish(Event::LinkLost {
            group: Count::new(0),
        })
        .unwrap();
    }
    let mut stream = UnixStream::connect(&path).unwrap();
    let hello = Hello {
        viewer: "late".to_owned(),
        reads: true,
        from_seq: Some(ovis_terminal_protocol::schema::scalar::EventSeq::new(3)),
    };
    stream
        .write_all(format!("{}\n", serde_json::to_string(&hello).unwrap()).as_bytes())
        .unwrap();
    pump(&mut hub, &events);
    let received = lines(&stream);
    let Line::Gap { from, to } = received.recv_timeout(BOUND).unwrap() else {
        panic!("the first line is not a gap record");
    };
    assert_eq!((from.get(), to.get()), (3, 88));
    let Line::Event { seq, .. } = received.recv_timeout(BOUND).unwrap() else {
        panic!("the replay does not continue after the gap");
    };
    assert_eq!(seq.get(), 88);
    drop(stream);
    hub.close().unwrap();
}
