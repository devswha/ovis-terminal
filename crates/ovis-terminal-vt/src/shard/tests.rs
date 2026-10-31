use std::num::NonZeroUsize;
use std::sync::Arc;

use proptest::prelude::*;

use super::{ParseBudget, PendingCap, Shard, ShardError};
use crate::{Geometry, HistoryLimit, OutputOffset, Vt, VtConfig};

fn engine() -> Vt {
    let geometry = Geometry::new(20, 6).expect("valid geometry");
    Vt::new(VtConfig::new(geometry, HistoryLimit::new(50)))
}

fn budget(bytes: usize) -> ParseBudget {
    ParseBudget::new(NonZeroUsize::new(bytes).expect("nonzero budget"))
}

fn token() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        "[a-z ]{1,12}".prop_map(String::into_bytes),
        Just("\u{4e2d}\u{1f600}e\u{301}".as_bytes().to_vec()),
        Just(b"\r\n".to_vec()),
        (1_u8..6, 1_u8..20).prop_map(|(row, column)| format!("\x1b[{row};{column}H").into_bytes()),
        (0_u8..108).prop_map(|code| format!("\x1b[{code}m").into_bytes()),
        Just(b"\x1b[?1049h".to_vec()),
        Just(b"\x1b[?1049l".to_vec()),
        Just(b"\x1b[6n".to_vec()),
        Just(b"\x1b]7501;state=idle\x1b\\".to_vec()),
    ]
}

fn chunks() -> impl Strategy<Value = Vec<Vec<u8>>> {
    prop::collection::vec(prop::collection::vec(token(), 0..12), 1..8)
        .prop_map(|chunks| chunks.into_iter().map(|tokens| tokens.concat()).collect())
}

proptest! {
    /// Budgeted turns over any chunking reach the state of one whole feed, report the
    /// same queries at the same offsets, and never feed more than the budget at once.
    #[test]
    fn budgeted_turns_equal_one_whole_feed(chunks in chunks(), per_turn in 1_usize..64) {
        let whole: Vec<u8> = chunks.concat();
        let mut reference = engine();
        let expected = reference.feed(&whole);

        let mut shard = Shard::new(engine(), budget(per_turn), PendingCap::new(whole.len()));
        for chunk in chunks {
            shard.push(Arc::from(chunk)).expect("the cap holds every chunk");
        }
        let mut queries = Vec::new();
        let mut osc = Vec::new();
        let mut turns = 0_usize;
        while shard.pending_bytes() > 0 {
            let turn = shard.turn();
            prop_assert!(turn.fed <= per_turn);
            prop_assert!(turn.fed > 0);
            prop_assert_eq!(turn.remaining, shard.pending_bytes());
            queries.extend(turn.outcome.queries);
            osc.extend(turn.outcome.osc);
            turns = turns.saturating_add(1);
        }
        prop_assert_eq!(turns, whole.len().div_ceil(per_turn));
        prop_assert_eq!(shard.vt().offset(), reference.offset());
        prop_assert_eq!(shard.vt().screen(), reference.screen());
        prop_assert_eq!(shard.vt().cursor(), reference.cursor());
        prop_assert_eq!(shard.vt().modes(), reference.modes());
        prop_assert_eq!(queries, expected.queries);
        prop_assert_eq!(osc, expected.osc);
    }
}

#[test]
fn a_chunk_past_the_cap_is_refused_without_queuing() {
    let mut shard = Shard::new(engine(), budget(4), PendingCap::new(10));
    shard.push(Arc::from(&b"abcdef"[..])).expect("fits");
    let refused = shard.push(Arc::from(&b"ghijk"[..]));
    assert_eq!(
        refused,
        Err(ShardError::Full {
            offered: 5,
            pending: 6,
            cap: 10
        })
    );
    assert_eq!(refused.map_err(|error| error.kind()), Err("backpressure"));
    assert_eq!(shard.pending_bytes(), 6, "a refusal queues nothing");

    let turn = shard.turn();
    assert_eq!((turn.fed, turn.remaining), (4, 2));
    shard
        .push(Arc::from(&b"ghijk"[..]))
        .expect("a parse turn made room");
    assert_eq!(shard.pending_bytes(), 7);
    while shard.pending_bytes() > 0 {
        shard.turn();
    }
    assert_eq!(shard.vt().offset(), OutputOffset::new(11));
}

#[test]
fn an_idle_turn_feeds_nothing() {
    let mut shard = Shard::new(engine(), ParseBudget::DEFAULT, PendingCap::DEFAULT);
    shard.push(Arc::from(&b""[..])).expect("empty chunk");
    let turn = shard.turn();
    assert_eq!((turn.fed, turn.remaining), (0, 0));
    assert_eq!(shard.vt().offset(), OutputOffset::ZERO);
}
