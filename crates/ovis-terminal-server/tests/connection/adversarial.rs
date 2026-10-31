//! Malformed input, refused credentials and cancellation on a live connection.

use std::sync::Arc;

use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde_json::json;

use super::handler::Recorder;
use super::support::{Fixture, TestResult, at, error_kind, request};

#[tokio::test]
async fn credentials_are_checked_before_hello_exposes_identity() -> TestResult {
    let expired = Fixture::expiring(TimestampMs::new(1))?;
    let server = expired.serve(1, Arc::new(Recorder::default())).await?;
    let forged = format!("{}x", expired.bearer);
    let cases = [
        ("", "unauthenticated"),
        (forged.as_str(), "invalid_credential"),
        (expired.bearer.as_str(), "credential_expired"),
    ];
    for (bearer, kind) in cases {
        let mut client = expired.connect().await?;
        let reply = client.hello("1", bearer).await?;
        assert_eq!(error_kind(&reply), kind, "bearer {bearer:?}");
        assert!(
            reply.get("result").is_none(),
            "no identity before authentication"
        );
    }
    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn requests_beyond_the_in_flight_bound_get_backpressure_and_cancel_on_shutdown() -> TestResult
{
    let fixture = Fixture::new()?;
    let recorder = Arc::new(Recorder::default());
    let server = fixture.serve(1, Arc::clone(&recorder)).await?;
    let mut client = fixture.connect().await?;
    let hello = client.hello_framed("1", &fixture.bearer).await?;
    let bound = at(&hello, "/result/limits/max_in_flight_requests")
        .as_u64()
        .ok_or("bound")?;
    for index in 0..bound {
        let id = format!("hold-{index}");
        client
            .send(&request(&id, "doc.get", &json!({ "name": "hold" })))
            .await?;
    }
    recorder.wait_held(bound).await?;
    client
        .send(&request("over", "doc.get", &json!({ "name": "b" })))
        .await?;
    let busy = client.expect_next().await?;
    assert_eq!(at(&busy, "/id"), "over");
    assert_eq!(error_kind(&busy), "backpressure");
    assert_eq!(at(&busy, "/error/data/retryable"), true);

    let report = server.shutdown().await?;
    assert_eq!(
        recorder.cancelled(),
        bound,
        "every held request was cancelled and joined"
    );
    let closing = client.expect_next().await?;
    assert_eq!(at(&closing, "/method"), "$/closing");
    assert_eq!(at(&closing, "/params/reason"), "server_shutdown");
    assert!(client.next().await?.is_none());
    assert_eq!(report.panicked(), 0);
    Ok(())
}

#[tokio::test]
async fn unknown_frame_kind_after_upgrade_closes_with_its_kind() -> TestResult {
    let fixture = Fixture::new()?;
    let server = fixture.serve(1, Arc::new(Recorder::default())).await?;
    let mut client = fixture.connect().await?;
    client.hello_framed("1", &fixture.bearer).await?;
    client.send_raw(&[0, 0, 0, 0, 7, 0, 0, 0, 0]).await?;
    let closing = client.expect_next().await?;
    assert_eq!(at(&closing, "/params/reason"), "unknown_kind");
    assert!(client.next().await?.is_none());
    let report = server.shutdown().await?;
    assert_eq!(report.failures().get("unknown_kind"), Some(&1));
    Ok(())
}

#[tokio::test]
async fn correlatable_bad_requests_are_answered_and_the_rest_close() -> TestResult {
    let fixture = Fixture::new()?;
    let server = fixture.serve(1, Arc::new(Recorder::default())).await?;
    let mut client = fixture.connect().await?;
    client.hello("1", &fixture.bearer).await?;
    client
        .send_raw(b"{\"jsonrpc\":\"2.0\",\"id\":\"x\",\"method\":5,\"params\":{}}\n")
        .await?;
    let invalid = client.expect_next().await?;
    assert_eq!(at(&invalid, "/id"), "x");
    assert_eq!(error_kind(&invalid), "invalid_request");
    client
        .send(&request("still-open", "doc.get", &json!({ "name": "b" })))
        .await?;
    assert_eq!(at(&client.expect_next().await?, "/result/name"), "b");
    client.send_raw(b"not json\n").await?;
    let closing = client.expect_next().await?;
    assert_eq!(at(&closing, "/params/reason"), "parse_error");
    assert!(client.next().await?.is_none());
    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn a_second_hello_and_a_duplicate_in_flight_id_are_refused() -> TestResult {
    let fixture = Fixture::new()?;
    let recorder = Arc::new(Recorder::default());
    let server = fixture.serve(1, Arc::clone(&recorder)).await?;
    let mut client = fixture.connect().await?;
    client.hello("1", &fixture.bearer).await?;
    let again = client.hello("1", &fixture.bearer).await?;
    assert_eq!(error_kind(&again), "invalid_request");
    client
        .send(&request("dup", "doc.get", &json!({ "name": "hold" })))
        .await?;
    recorder.wait_held(1).await?;
    client
        .send(&request("dup", "doc.get", &json!({ "name": "b" })))
        .await?;
    let duplicate = client.expect_next().await?;
    assert_eq!(error_kind(&duplicate), "duplicate_request_id");
    drop(client);
    server.shutdown().await?;
    assert_eq!(recorder.cancelled(), 1);
    Ok(())
}
