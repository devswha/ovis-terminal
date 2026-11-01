use proptest::prelude::*;
use serde_json::json;

use super::{
    Backlog, ConsumerLoss, ConsumerRef, LossRange, MethodLatency, RecoveryPhase, RecoveryState,
};
use crate::errors::OpId;
use crate::schema::ids::{ClientId, MethodName, StreamId, SubscriptionId, TerminalId};
use crate::schema::methods::server::{ConsumerQueue, ServerStatusResult};
use crate::schema::scalar::{ByteCount, ByteOffset, Count, DurationMs, EventSeq, OpenEnum};

fn events(first: u64, last: u64) -> LossRange {
    LossRange::Events {
        first_seq: EventSeq::new(first),
        last_seq: EventSeq::new(last),
    }
}

fn subscription() -> ConsumerRef {
    ConsumerRef::Subscription {
        subscription_id: SubscriptionId::new("sub-1".to_owned()),
    }
}

fn queue(losses: Vec<ConsumerLoss>) -> ConsumerQueue {
    let backlog = Backlog {
        queue_bytes: ByteCount::new(7),
        event_lag_ms: DurationMs::new(3),
        losses,
        losses_omitted: Count::new(0),
    };
    ConsumerQueue::new(ClientId::new("ovis".to_owned()), subscription(), backlog)
}

#[test]
fn only_resynchronized_event_ranges_count_as_resynced() {
    let resynced = ConsumerLoss {
        range: events(10, 14),
        resynced: true,
    };
    let bytes = ConsumerLoss {
        range: LossRange::Bytes {
            first_offset: ByteOffset::new(0),
            last_offset: ByteOffset::new(99),
        },
        resynced: true,
    };
    let row = queue(vec![resynced, ConsumerLoss::dropped(events(20, 29)), bytes]);
    assert_eq!(row.resynced_events, 5);
    assert_eq!(row.losses.len(), 3);
}

#[test]
fn resynced_count_saturates_instead_of_wrapping() {
    let all = ConsumerLoss {
        range: events(0, u64::MAX),
        resynced: true,
    };
    assert_eq!(queue(vec![all, all]).resynced_events, u32::MAX);
}

#[test]
fn damage_or_unsettled_survival_is_degraded_and_facts_are_recovered() {
    let phase = |state: RecoveryState| state.phase;
    let known = OpenEnum::Known;
    assert_eq!(
        phase(RecoveryState::derive(false, (0, 0), false)),
        known(RecoveryPhase::Clean)
    );
    assert_eq!(
        phase(RecoveryState::derive(false, (1, 0), false)),
        known(RecoveryPhase::Recovered)
    );
    assert_eq!(
        phase(RecoveryState::derive(false, (0, 0), true)),
        known(RecoveryPhase::Recovered)
    );
    assert_eq!(
        phase(RecoveryState::derive(true, (1, 0), true)),
        known(RecoveryPhase::Degraded)
    );
    assert_eq!(
        phase(RecoveryState::derive(false, (0, 2), true)),
        known(RecoveryPhase::Degraded)
    );
}

#[test]
fn counters_are_decimal_strings_up_to_u64_max() {
    let state = RecoveryState::derive(true, (u64::MAX, u64::MAX), true);
    let encoded = serde_json::to_value(&state).expect("encodes");
    assert_eq!(encoded.get("incidents"), Some(&json!(u64::MAX.to_string())));
    assert_eq!(
        encoded.get("unsettled"),
        Some(&json!("18446744073709551615"))
    );
    let decoded: RecoveryState = serde_json::from_value(encoded).expect("decodes");
    assert_eq!(decoded, state);
}

#[test]
fn numeric_counters_and_unknown_fields_are_rejected() {
    let numeric = json!({"phase": "clean", "corrupted": false, "incidents": 1, "unsettled": "0"});
    assert!(serde_json::from_value::<RecoveryState>(numeric).is_err());
    let extra = json!({"unit": "events", "first_seq": "1", "last_seq": "2", "reset": true});
    assert!(serde_json::from_value::<LossRange>(extra).is_err());
    let numeric_range = json!({"unit": "bytes", "first_offset": 1, "last_offset": "2"});
    assert!(serde_json::from_value::<LossRange>(numeric_range).is_err());
}

#[test]
fn a_status_written_before_recovery_state_existed_still_decodes() {
    let idle = json!({
        "state": "stale_endpoint", "runtime_lifetime": null, "server_boot_id": null,
        "uptime_ms": null, "holder_count": 0, "live_terminals": 0,
    });
    let status: ServerStatusResult = serde_json::from_value(idle).expect("decodes");
    assert_eq!(status.recovery_state, None);
}

fn consumer_ref() -> impl Strategy<Value = ConsumerRef> {
    prop_oneof![
        ("[a-z0-9-]{1,12}", "[a-z0-9-]{1,12}").prop_map(|(stream, terminal)| ConsumerRef::Stream {
            stream_id: StreamId::new(stream),
            terminal_id: TerminalId::new(terminal),
        }),
        "[a-z0-9-]{1,12}".prop_map(|id| ConsumerRef::Subscription {
            subscription_id: SubscriptionId::new(id),
        }),
    ]
}

fn loss() -> impl Strategy<Value = ConsumerLoss> {
    (any::<u64>(), any::<u64>(), any::<bool>(), any::<bool>()).prop_map(
        |(a, b, bytes, resynced)| {
            let range = if bytes {
                LossRange::Bytes {
                    first_offset: ByteOffset::new(a),
                    last_offset: ByteOffset::new(b),
                }
            } else {
                events(a, b)
            };
            ConsumerLoss { range, resynced }
        },
    )
}

proptest! {
    #[test]
    fn consumer_rows_round_trip(
        consumer in consumer_ref(),
        losses in proptest::collection::vec(loss(), 0..4),
        (queued, lag, omitted) in (any::<u64>(), any::<u64>(), any::<u64>()),
    ) {
        let backlog = Backlog {
            queue_bytes: ByteCount::new(queued),
            event_lag_ms: DurationMs::new(lag),
            losses,
            losses_omitted: Count::new(omitted),
        };
        let row = ConsumerQueue::new(ClientId::new("c".to_owned()), consumer, backlog);
        let text = serde_json::to_string(&row).expect("encodes");
        let back: ConsumerQueue = serde_json::from_str(&text).expect("decodes");
        prop_assert_eq!(back, row);
    }

    #[test]
    fn latency_rows_round_trip(
        (calls, failures, last, max) in (any::<u64>(), any::<u64>(), any::<u64>(), any::<u64>()),
        op in proptest::option::of("[a-f0-9-]{1,36}"),
    ) {
        let row = MethodLatency::new(
            MethodName::new("server.metrics".to_owned()),
            (Count::new(calls), Count::new(failures)),
            (DurationMs::new(last), DurationMs::new(max), op.map(OpId::new)),
        );
        let text = serde_json::to_string(&row).expect("encodes");
        let back: MethodLatency = serde_json::from_str(&text).expect("decodes");
        prop_assert_eq!(back, row);
    }
}
