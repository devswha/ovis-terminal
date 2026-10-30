//! Behavior tests for exactly-once settlement of interleaved requests.

use std::collections::HashSet;

use ovis_terminal_protocol::rpc::{InFlightRequests, RequestId, RpcReply, RpcSuccess};
use proptest::prelude::*;
use serde_json::Value;

#[test]
fn settles_interleaved_requests_once_and_keeps_ids_independent() {
    let first = RequestId::new("first".to_owned());
    let second = RequestId::new("second".to_owned());
    let mut in_flight = InFlightRequests::default();

    in_flight
        .begin(first.clone())
        .expect("first request should begin");
    in_flight
        .begin(second.clone())
        .expect("second request should begin");

    let second_reply = RpcReply::Success(RpcSuccess::new(second.clone(), Value::Null));
    let settled = in_flight
        .complete(&second, second_reply)
        .expect("second request should settle first");
    assert_eq!(settled.id(), &second);

    let second_again = RpcReply::Success(RpcSuccess::new(second.clone(), Value::Null));
    let error = in_flight
        .complete(&second, second_again)
        .expect_err("a second settlement must be rejected");
    assert_eq!(error.kind(), "unknown_or_settled_request_id");

    let first_reply = RpcReply::Success(RpcSuccess::new(first.clone(), Value::Null));
    let settled = in_flight
        .complete(&first, first_reply)
        .expect("unsettled first request remains active");
    assert_eq!(settled.id(), &first);
}

#[test]
fn refuses_a_reply_for_a_different_in_flight_id() {
    let requested = RequestId::new("requested".to_owned());
    let unrelated = RequestId::new("unrelated".to_owned());
    let mut in_flight = InFlightRequests::default();
    in_flight
        .begin(requested.clone())
        .expect("request should begin");

    let reply = RpcReply::Success(RpcSuccess::new(unrelated, Value::Null));
    let error = in_flight
        .complete(&requested, reply)
        .expect_err("mismatched response ID must not settle the request");
    assert_eq!(error.kind(), "response_id_mismatch");

    let matching = RpcReply::Success(RpcSuccess::new(requested.clone(), Value::Null));
    assert!(in_flight.complete(&requested, matching).is_ok());
}

proptest! {
    #[test]
    fn settlement_matches_a_reference_set(
        operations in prop::collection::vec((any::<u8>(), any::<bool>()), 0..64)
    ) {
        let mut in_flight = InFlightRequests::default();
        let mut active = HashSet::new();

        for (raw_id, settle) in operations {
            let id = RequestId::new(format!("r{raw_id}"));
            if settle {
                let reply = RpcReply::Success(RpcSuccess::new(id.clone(), Value::Null));
                let expected = active.remove(&id);
                match in_flight.complete(&id, reply) {
                    Ok(settled) => {
                        prop_assert!(expected);
                        prop_assert_eq!(settled.id(), &id);
                    }
                    Err(error) => {
                        prop_assert!(!expected);
                        prop_assert_eq!(error.kind(), "unknown_or_settled_request_id");
                    }
                }
            } else {
                let expected = active.insert(id.clone());
                match in_flight.begin(id) {
                    Ok(()) => prop_assert!(expected),
                    Err(error) => {
                        prop_assert!(!expected);
                        prop_assert_eq!(error.kind(), "duplicate_request_id");
                    }
                }
            }
        }
    }
}
