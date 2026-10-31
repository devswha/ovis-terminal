//! Model check of sequence acceptance, turn numbering and finish generations.

use ovis_terminal_protocol::schema::records::{StatusEntry, StatusOrigin, StatusReport};
use ovis_terminal_protocol::schema::scalar::{EventSeq, Incarnation, ReportSeq};
use proptest::prelude::*;

use crate::status::machine::{Accepted, Lifecycle};
use crate::status::row::Row;

const STATES: [Lifecycle; 6] = [
    Lifecycle::Working,
    Lifecycle::Idle,
    Lifecycle::Blocked,
    Lifecycle::Done,
    Lifecycle::Background,
    Lifecycle::Error,
];

fn accepted(lifecycle: Lifecycle, seq: u64) -> Accepted {
    let mut entry = StatusEntry::new(StatusReport::new(lifecycle.state()), StatusOrigin::Rpc);
    entry.report_seq = ReportSeq::new(seq);
    Accepted { entry, lifecycle }
}

proptest! {
    #[test]
    fn turns_count_entries_to_working_and_finishes_count_exits(
        steps in proptest::collection::vec(0usize..6, 1..40),
    ) {
        let mut row = Row::unreported(Some(Incarnation::new(1)));
        let (mut turns, mut finishes, mut working) = (0_u64, 0_u64, false);
        for (index, pick) in steps.into_iter().enumerate() {
            let lifecycle = STATES.get(pick).copied().unwrap_or(Lifecycle::Idle);
            let seq = u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1);
            let ended = row.accept(accepted(lifecycle, seq), EventSeq::new(seq));
            let now_working = lifecycle == Lifecycle::Working;
            if now_working && !working {
                turns = turns.saturating_add(1);
            }
            prop_assert_eq!(ended.is_some(), working && !now_working);
            if working && !now_working {
                finishes = finishes.saturating_add(1);
            }
            working = now_working;
            prop_assert_eq!(row.status.turn.get(), turns);
            prop_assert_eq!(row.status.finish_generation.get(), finishes);
            prop_assert_eq!(row.status.report_seq, Some(ReportSeq::new(seq)));
        }
        let root_working = row
            .status
            .root
            .as_ref()
            .map(|root| Lifecycle::of(&root.report.state) == Some(Lifecycle::Working));
        prop_assert_eq!(root_working, Some(working));
    }
}
