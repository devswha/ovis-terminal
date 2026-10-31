use serde_json::json;

use super::*;

#[tokio::test(start_paused = true)]
async fn ready_result_at_or_after_expiry_is_not_success() {
    // Given a result and deadline that will both be ready before the waiter is polled.
    for elapsed in [10_000, 10_001] {
        let start = Instant::now();
        let op_id = OpId::new("01800000-0000-7000-8000-000000000001".into());
        // When the result arrives at or after the caller's deadline.
        tokio::time::advance(Duration::from_millis(elapsed)).await;
        let result = wait(
            std::future::ready(Ok(json!({"late":true}))),
            op_id.clone(),
            start,
        )
        .await;
        // Then settlement is a typed expiry with the exact same-clock elapsed counter.
        let error = result.expect_err("expired settlement cannot be successful");
        assert_eq!(error.kind(), "deadline_exceeded");
        assert_eq!(error.data().op_id, Some(op_id));
        assert_eq!(error.data().deadline_ms, Some(DurationMs::new(10_000)));
        assert_eq!(error.data().elapsed_ms, Some(DurationMs::new(elapsed)));
    }
}

#[tokio::test(start_paused = true)]
async fn ready_result_before_expiry_is_success() {
    // Given a request whose deadline has not elapsed.
    let start = Instant::now();
    tokio::time::advance(Duration::from_millis(9_999)).await;
    // When the ready result settles before the boundary.
    let result = wait(
        std::future::ready(Ok(json!({"on_time":true}))),
        OpId::new("01800000-0000-7000-8000-000000000001".into()),
        start,
    )
    .await;
    // Then it is returned without manufacturing a timeout.
    assert_eq!(result.expect("within deadline"), json!({"on_time":true}));
}

#[tokio::test(start_paused = true)]
async fn shared_outcome_keeps_each_callers_own_deadline() {
    // Given two callers subscribed to one operation at different instants.
    let first_start = Instant::now();
    let (result, first) = watch::channel(None);
    tokio::time::advance(Duration::from_millis(5_000)).await;
    let second_start = Instant::now();
    let second = result.subscribe();
    let op = OpId::new("01800000-0000-7000-8000-000000000001".into());
    // When one shared result arrives after the first deadline but before the second.
    tokio::time::advance(Duration::from_millis(5_001)).await;
    result.send_replace(Some(Ok(json!({"completed":true}))));
    let first = wait(receive(first), op.clone(), first_start).await;
    let second = wait(receive(second), op, second_start).await;
    // Then expiry is per RPC and does not consume the result or cancel other waiters.
    assert_eq!(
        first.expect_err("first expired").kind(),
        "deadline_exceeded"
    );
    assert_eq!(second.expect("second has time"), json!({"completed":true}));
}
