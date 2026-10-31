//! Model check: under any interleaving of admission, writes, evidence and interruptions,
//! entries reach the holder in admission order, at most once, never after uncertainty.

use std::collections::BTreeMap;

use ovis_terminal_core::input::receipts::Delivery;
use ovis_terminal_core::input::sequencer::{Payload, QueueLimits, Sequencer};
use ovis_terminal_holder::protocol::Input;
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::input::Submit;
use ovis_terminal_protocol::schema::methods::server::WriteSequence;
use proptest::prelude::{Just, Strategy, prop_assert, prop_assert_eq, prop_oneof, proptest};
use proptest::test_runner::TestCaseError;

use super::fixture::{Fixture, WRITER, ack, admit, modes, process, submission, view};

#[derive(Debug, Clone, Copy)]
enum Step {
    Admit,
    Next,
    Ack(u8),
    LinkLost,
    Ready,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        Just(Step::Admit),
        Just(Step::Next),
        (0_u8..3).prop_map(Step::Ack),
        Just(Step::LinkLost),
        Just(Step::Ready),
    ]
}

fn fail(error: impl std::fmt::Display) -> TestCaseError {
    TestCaseError::fail(error.to_string())
}

/// Index of the admitted entry a user write carries; replies are not modelled here.
fn entry_index(input: &Input) -> Result<u64, TestCaseError> {
    let text = std::str::from_utf8(&input.bytes).map_err(fail)?;
    text.trim_start_matches('e').parse().map_err(fail)
}

proptest! {
    #[test]
    fn writes_follow_admission_order_once(steps in proptest::collection::vec(step(), 1..60)) {
        let fixture = Fixture::new().map_err(fail)?;
        let leases = fixture.leases().map_err(fail)?;
        let modes = modes(b"").map_err(fail)?;
        let view = view(&leases, &modes);
        let alice = fixture.client("alice", WRITER).map_err(fail)?;
        let mut sequencer: Sequencer = fixture.sequencer(QueueLimits::DEFAULT).map_err(fail)?;
        let (mut admitted, mut last_index, mut last_write) = (0_u64, 0_u64, 0_u64);
        let mut sent: Option<Input> = None;
        let mut seen: BTreeMap<String, Delivery> = BTreeMap::new();
        for step in steps {
            match step {
                Step::Admit => {
                    let text = format!("e{}", admitted.saturating_add(1));
                    let payload = Payload::Text { text: &text, submit: &Submit::None };
                    let request = (alice.writer(), submission(&text, fixture.expected().map_err(fail)?, payload));
                    if admit(&mut sequencer, &view, request).is_ok() {
                        admitted = admitted.saturating_add(1);
                    }
                }
                Step::Next => {
                    if let Some(input) = sequencer.next(&view).map_err(fail)?.dispatch {
                        let index = entry_index(&input)?;
                        prop_assert!(index > last_index, "entry {index} after {last_index}");
                        prop_assert!(input.sequence > last_write);
                        (last_index, last_write) = (index, input.sequence);
                        sent = Some(input);
                    }
                }
                Step::Ack(fraction) => {
                    if let Some(input) = sent.take() {
                        let length = u64::try_from(input.bytes.len()).map_err(fail)?;
                        let written = match fraction { 0 => 0, 1 => length.div_ceil(2).saturating_sub(1), _ => length };
                        sequencer.acknowledge(&ack(&input, written)).map_err(fail)?;
                    }
                }
                Step::LinkLost => {
                    sequencer.link_lost();
                }
                Step::Ready => {
                    sequencer.stream_ready(process(1).map_err(fail)?, WriteSequence::new(last_write));
                }
            }
            for index in 1..=admitted {
                let op = format!("e{index}");
                let Some(receipt) = sequencer.receipt(&OpId::new(op.clone())) else { continue };
                if let Some(before) = seen.insert(op, receipt.delivery) {
                    let regressed = before != Delivery::Queued && receipt.delivery == Delivery::Queued;
                    prop_assert!(!regressed, "receipt {index} went back to queued");
                }
            }
        }
        prop_assert_eq!(seen.len(), usize::try_from(admitted).map_err(fail)?);
    }
}
