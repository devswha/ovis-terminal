//! Real child processes stand in for transports; each one is this test's own child.

use std::time::Duration;

use ovis_terminal_protocol::schema::methods::machine::MachineTransport;
use proptest::prelude::*;
use serde_json::json;

use super::Handshake;
use crate::identity::{Credential, fresh_challenge};
use crate::transport::{Link, TransportError, TransportSpec};

const BOUND: Duration = Duration::from_secs(10);

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn link(script: &str, reply: &str) -> TestResult<Link> {
    let argv = ["sh", "-c", script, "sh", reply]
        .map(str::to_owned)
        .to_vec();
    let spec = TransportSpec::parse(&MachineTransport::Argv(argv))?;
    Ok(Link::spawn(&spec)?)
}

async fn hello(link: &mut Link, bound: Duration) -> Result<(), TransportError> {
    let credential = Credential::new("ovt1.1.bearer".to_owned());
    link.handshake(&credential, Handshake::new(fresh_challenge(), bound))
        .await
        .map(drop)
}

fn hello_result() -> String {
    json!({"jsonrpc": "2.0", "id": "hello", "result": {
        "machine_id": "machine:0191f1a2-0000-7000-8000-000000000043",
        "runtime_lifetime": "runtime:1", "server_boot_id": "boot:1", "generation": "1",
        "version": "0.1.0", "methods": [],
        "capabilities": {"transports": [], "stream_modes": [], "framed": true},
        "limits": ovis_terminal_protocol::schema::limits::Limits::v1(),
        "server_time_ms": "0", "endpoint_kind": "stdio", "boot_proof": null,
    }})
    .to_string()
}

#[tokio::test]
async fn a_banner_before_the_reply_is_not_protocol() -> TestResult {
    let mut link = link("echo 'Welcome to box'; exec cat >/dev/null", "")?;

    let error = hello(&mut link, BOUND).await.expect_err("banner");

    assert_eq!(error.kind(), "transport_failed");
    let TransportError::NotProtocol { first_bytes } = error else {
        panic!("{error:?}")
    };
    assert_eq!(first_bytes.bytes(), b"Welcome to box");
    Ok(())
}

#[tokio::test]
async fn a_credential_prompt_times_out_typed_with_the_prompt_as_evidence() -> TestResult {
    let mut link = link("printf 'Password: '; exec cat >/dev/null", "")?;

    let error = hello(&mut link, Duration::from_millis(300))
        .await
        .expect_err("prompt");

    assert_eq!(error.kind(), "handshake_timeout");
    let TransportError::Timeout { received, .. } = error else {
        panic!("{error:?}")
    };
    assert_eq!(received.bytes(), b"Password: ");
    Ok(())
}

#[tokio::test]
async fn a_transport_that_exits_reports_its_status_and_stderr_only_as_diagnostics() -> TestResult {
    let mut link = link(
        "read line; echo 'Permission denied (publickey)' >&2; exit 255",
        "",
    )?;

    let error = hello(&mut link, BOUND).await.expect_err("closed");

    assert_eq!(error.kind(), "transport_failed");
    let TransportError::Closed {
        status,
        diagnostics,
    } = error
    else {
        panic!("{error:?}")
    };
    assert_eq!(status, Some(255));
    assert_eq!(diagnostics.bytes(), b"Permission denied (publickey)\n");
    Ok(())
}

#[tokio::test]
async fn a_typed_credential_refusal_is_an_authentication_failure() -> TestResult {
    let refusal = json!({"jsonrpc": "2.0", "id": "hello", "error": {"code": -32000,
        "message": "refused", "data": {"kind": "invalid_credential", "hint": "h", "retryable": false}}});
    let mut link = link("read line; printf '%s\\n' \"$1\"", &refusal.to_string())?;

    let error = hello(&mut link, BOUND).await.expect_err("refused");

    assert_eq!(error.kind(), "auth_failed");
    Ok(())
}

#[tokio::test]
async fn stderr_chatter_never_reaches_the_protocol_stream() -> TestResult {
    let script = "echo 'warning: chatter' >&2; read line; printf '%s\\n' \"$1\"";
    let mut link = link(script, &hello_result())?;

    hello(&mut link, BOUND)
        .await
        .expect("the reply is read past stderr chatter");
    let end = link.receive().await.expect_err("the transport then ends");

    let TransportError::Closed {
        status,
        diagnostics,
    } = end
    else {
        panic!("{end:?}")
    };
    assert_eq!(
        (status, diagnostics.bytes()),
        (Some(0), &b"warning: chatter\n"[..])
    );
    Ok(())
}

#[tokio::test]
async fn a_missing_program_is_a_typed_spawn_failure() -> TestResult {
    let argv = vec!["/nonexistent/ovis-terminal-transport".to_owned()];
    let spec = TransportSpec::parse(&MachineTransport::Argv(argv))?;

    let error = Link::spawn(&spec).expect_err("no such program");

    assert_eq!(error.kind(), "transport_failed");
    Ok(())
}

fn round_trip(chunks: &[Vec<u8>]) -> TestResult<Vec<u8>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let spec = TransportSpec::parse(&MachineTransport::Argv(vec!["cat".to_owned()]))?;
        let mut link = Link::spawn(&spec)?;
        for chunk in chunks {
            link.send(chunk).await?;
        }
        link.send(b"\n").await?;
        Ok(link.receive().await?)
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn every_byte_except_the_record_end_passes_unchanged_under_any_split(
        chunks in prop::collection::vec(
            prop::collection::vec(any::<u8>().prop_filter("record end", |b| *b != b'\n'), 0..64),
            0..16,
        )
    ) {
        let expected: Vec<u8> = chunks.concat();
        let echoed = round_trip(&chunks).map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(echoed, expected);
    }
}
