//! Exactness: a checkpoint persisted to disk plus the retained tail rebuilds the engine
//! that saw the whole stream, without repeating answered queries or committed OSC.

use ovis_terminal_holder::protocol::{Output, TerminalId, TerminalIncarnation, TerminalTarget};
use ovis_terminal_protocol::domain::recovery::OutputGap;
use ovis_terminal_protocol::schema::scalar::ByteOffset;
use ovis_terminal_vt::{Geometry, HistoryLimit, OscRecord, TerminalQuery, Vt, VtConfig};
use proptest::prelude::*;

use crate::recovery::{CheckpointStore, Fence, RecoveryError, Retained, Splice, restore};

const FRAGMENTS: [&[u8]; 18] = [
    b"hello ",
    b"\xc3\xa9",
    b"\xe2\x94\x80",
    b"\r\n",
    b"\x1b[1;31m",
    b"\x1b[0m",
    b"\x1b[2J",
    b"\x1b[H",
    b"\x1b[5;9H",
    b"\x1b[?1049h",
    b"\x1b[?1049l",
    b"\x1b[6n",
    b"\x1b[c",
    b"\x1b]7501;state=working\x07",
    b"\x1b]2;title\x1b\\",
    b"\x1b[?2004h",
    b"\x1b7",
    b"\x1b8",
];

fn config() -> Result<VtConfig, TestCaseError> {
    let geometry = Geometry::new(20, 6).map_err(|e| TestCaseError::fail(e.to_string()))?;
    Ok(VtConfig::new(geometry, HistoryLimit::new(50)))
}

fn target() -> TerminalTarget {
    TerminalTarget {
        terminal_id: TerminalId::new("terminal:49".to_owned()),
        terminal_incarnation: TerminalIncarnation::new(1),
        process_incarnation: None,
    }
}

/// Feeds `bytes` in `chunk`-sized writes, collecting every query and OSC record.
fn feed(vt: &mut Vt, (bytes, chunk): (&[u8], usize)) -> (Vec<TerminalQuery>, Vec<OscRecord>) {
    let (mut queries, mut osc) = (Vec::new(), Vec::new());
    for piece in bytes.chunks(chunk) {
        let fed = vt.feed(piece);
        queries.extend(fed.queries);
        osc.extend(fed.osc);
    }
    (queries, osc)
}

fn stream() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(prop::sample::select(FRAGMENTS.to_vec()), 1..48)
        .prop_map(|parts| parts.concat())
}

fn fail(error: &RecoveryError) -> TestCaseError {
    TestCaseError::fail(format!("{}: {error}", error.kind()))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(192))]

    #[test]
    fn a_disk_checkpoint_plus_the_retained_tail_equals_the_uninterrupted_engine(
        stream in stream(),
        cuts in (any::<prop::sample::Index>(), any::<prop::sample::Index>()),
        chunk in 1usize..17,
    ) {
        let config = config()?;
        let checkpointed = cuts.0.index(stream.len().saturating_add(1));
        let crash = checkpointed.saturating_add(cuts.1.index(stream.len().saturating_sub(checkpointed).saturating_add(1)));
        let mut whole = Vt::new(config);
        let (queries, osc) = feed(&mut whole, (&stream, chunk));
        let mut crashed = Vt::new(config);
        feed(&mut crashed, (stream.get(..checkpointed).unwrap_or_default(), chunk));
        let checkpoint = crashed.export_checkpoint();
        feed(&mut crashed, (stream.get(checkpointed..crash).unwrap_or_default(), chunk));
        let dir = std::env::temp_dir().join(format!("ovis-t49-exact-{}", uuid::Uuid::now_v7()));
        let store = CheckpointStore::new(&dir);
        store.save(&target(), &checkpoint).map_err(|e| fail(&e))?;
        let loaded = store.load(&target()).map_err(|e| fail(&e))?;
        std::fs::remove_dir_all(&dir).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let loaded = loaded.ok_or_else(|| TestCaseError::fail("checkpoint missing"))?;
        let held = u64::try_from(crash).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let fence = Fence::new(held, held);
        let restored = restore((Some(&loaded), config), &Retained::new(0, stream.clone()), fence)
            .map_err(|e| fail(&e))?;

        prop_assert_eq!(restored.screen.exact, loaded.fidelity().is_exact());
        prop_assert_eq!(restored.screen.gaps, Vec::<OutputGap>::new());
        prop_assert_eq!(restored.lag, 0);
        if restored.screen.exact {
            prop_assert_eq!(restored.vt.export_checkpoint(), whole.export_checkpoint());
        }
        let owed: Vec<_> = queries.into_iter().filter(|q| q.offset.get() > held).collect();
        prop_assert_eq!(restored.queries, owed);
        let fresh: Vec<_> = osc.into_iter().filter(|r| r.offset.get() > held).collect();
        prop_assert_eq!(restored.osc, fresh);
    }

    #[test]
    fn a_tail_that_overflowed_past_the_checkpoint_is_a_gap_and_never_exact(
        stream in stream(),
        cuts in (any::<prop::sample::Index>(), any::<prop::sample::Index>()),
    ) {
        let config = config()?;
        let crash = cuts.0.index(stream.len());
        let floor = crash.saturating_add(1).saturating_add(cuts.1.index(stream.len().saturating_sub(crash)));
        let mut crashed = Vt::new(config);
        crashed.feed(stream.get(..crash).unwrap_or_default());
        let checkpoint = crashed.export_checkpoint();
        let from = u64::try_from(floor).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let retained = Retained::new(from, stream.get(floor..).unwrap_or_default().to_vec());
        let restored = restore((Some(&checkpoint), config), &retained, Fence::default())
            .map_err(|e| fail(&e))?;

        let start = checkpoint.offset().get();
        prop_assert!(!restored.screen.exact);
        prop_assert_eq!(restored.screen.gaps, vec![OutputGap::new(ByteOffset::new(start), ByteOffset::new(from))]);
        prop_assert_eq!(restored.lag, from.saturating_sub(start));
        prop_assert_eq!(restored.screen.head.get(), retained.through);
        prop_assert!(restored.osc.iter().all(|r| r.offset.get() > from));
        prop_assert!(restored.queries.iter().all(|q| q.offset.get() > from));
    }

    #[test]
    fn live_output_after_a_replay_is_admitted_once_under_any_overlap(
        bytes in prop::collection::vec(any::<u8>(), 1..256),
        cuts in prop::collection::vec(any::<prop::sample::Index>(), 1..8),
        head in any::<prop::sample::Index>(),
    ) {
        let head = head.index(bytes.len().saturating_add(1));
        let mut starts: Vec<usize> = cuts.iter().map(|cut| cut.index(head.saturating_add(1))).collect();
        starts.sort_unstable();
        let mut splice = Splice::at(u64::try_from(head).map_err(|e| TestCaseError::fail(e.to_string()))?);
        let mut seen = bytes.get(..head).unwrap_or_default().to_vec();
        for start in starts.into_iter().chain([head]) {
            let output = Output {
                target: target(),
                offset: u64::try_from(start).map_err(|e| TestCaseError::fail(e.to_string()))?,
                bytes: bytes.get(start..).unwrap_or_default().to_vec(),
                hangup: None,
            };
            seen.extend_from_slice(splice.admit(&output).map_err(|e| fail(&e))?);
        }
        prop_assert_eq!(seen, bytes);
    }
}

#[test]
fn live_output_that_skips_bytes_is_refused_not_spliced() {
    let mut splice = Splice::at(4);
    let output = Output {
        target: target(),
        offset: 5,
        bytes: b"x".to_vec(),
        hangup: None,
    };
    let refused = splice
        .admit(&output)
        .map(<[u8]>::to_vec)
        .map_err(|e| e.kind());
    assert_eq!(refused, Err("holder_protocol"));
    assert_eq!(splice.next(), 4);
}
