use proptest::prelude::*;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream, duplex};

use super::{AllowedOrigins, MAX_HEAD, accept_key, answer};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The RFC 6455 section 1.3 sample key.
const KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

fn request(extra: &str) -> String {
    format!(
        "GET /ovis HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\n\
         Connection: keep-alive, Upgrade\r\nSec-WebSocket-Key: {KEY}\r\n\
         Sec-WebSocket-Version: 13\r\n{extra}\r\n"
    )
}

/// Feeds `chunks` as separate writes and answers the upgrade; returns what the server
/// handed on, the kind of a refusal and the HTTP response text.
async fn upgrade(
    chunks: Vec<Vec<u8>>,
    origins: &AllowedOrigins,
) -> Result<(Result<Vec<u8>, &'static str>, String), Box<dyn std::error::Error>> {
    let (mut client, mut server): (DuplexStream, DuplexStream) = duplex(64 * 1024);
    let writer = async move {
        for chunk in chunks {
            client.write_all(&chunk).await?;
        }
        client.shutdown().await?;
        let mut response = String::new();
        client.read_to_string(&mut response).await?;
        Ok::<_, std::io::Error>(response)
    };
    let answering = async {
        let answered = answer(&mut server, origins).await;
        drop(server);
        answered.map_err(|error| error.kind())
    };
    let (response, answered) = tokio::join!(writer, answering);
    Ok((answered, response?))
}

#[test]
fn the_accept_key_matches_rfc_6455() {
    assert_eq!(accept_key(KEY.as_bytes()), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
}

#[tokio::test]
async fn an_upgrade_switches_protocols_and_hands_on_pipelined_bytes() -> TestResult {
    let pipelined = [request("").into_bytes(), vec![0x82, 0x80, 1, 2, 3, 4]].concat();
    let (answered, response) = upgrade(vec![pipelined], &AllowedOrigins::default()).await?;
    assert_eq!(answered, Ok(vec![0x82, 0x80, 1, 2, 3, 4]));
    assert!(
        response.starts_with("HTTP/1.1 101 Switching Protocols\r\n"),
        "{response}"
    );
    assert!(response.contains("\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n"));
    Ok(())
}

#[tokio::test]
async fn refusals_are_typed_http_errors_that_echo_nothing() -> TestResult {
    let secret = "Bearer ovt-secret-material";
    let foreign = format!("Origin: https://evil.example\r\nAuthorization: {secret}\r\n");
    let cases = [
        (request(&foreign), "origin_rejected", "403"),
        (
            request("").replace("Version: 13", "Version: 8"),
            "unsupported_ws_version",
            "426",
        ),
        (request("").replace("GET", "POST"), "invalid_upgrade", "400"),
        (
            request("").replace(KEY, "c2hvcnQ="),
            "invalid_upgrade",
            "400",
        ),
        (
            request("").replace("Upgrade: websocket\r\n", ""),
            "invalid_upgrade",
            "400",
        ),
        (
            "\x16\x03\x01 not http\r\n\r\n".to_owned(),
            "invalid_upgrade",
            "400",
        ),
    ];
    for (text, kind, status) in cases {
        let (answered, response) =
            upgrade(vec![text.into_bytes()], &AllowedOrigins::default()).await?;
        assert_eq!(answered, Err(kind), "{response}");
        assert!(
            response.starts_with(&format!("HTTP/1.1 {status} ")),
            "{response}"
        );
        let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
        let body: serde_json::Value = serde_json::from_str(body)?;
        assert_eq!(body.get("kind"), Some(&serde_json::json!(kind)));
        assert!(!response.contains("ovt-secret") && !response.contains("evil.example"));
    }
    Ok(())
}

#[tokio::test]
async fn an_oversized_or_truncated_head_is_refused() -> TestResult {
    let padding = format!("X-Pad: {}\r\n", "a".repeat(MAX_HEAD));
    let (answered, response) = upgrade(
        vec![request(&padding).into_bytes()],
        &AllowedOrigins::default(),
    )
    .await?;
    assert_eq!(answered, Err("upgrade_too_large"), "{response}");
    assert!(response.starts_with("HTTP/1.1 431 "), "{response}");
    let cut = request("").into_bytes();
    let (answered, _) = upgrade(
        vec![cut.get(..20).unwrap_or_default().to_vec()],
        &AllowedOrigins::default(),
    )
    .await?;
    assert_eq!(answered, Err("invalid_upgrade"));
    Ok(())
}

proptest! {
    #[test]
    fn the_head_is_found_however_the_transport_splits_it(
        cuts in proptest::collection::btree_set(1_usize..240, 0..6),
        tail in proptest::collection::vec(any::<u8>(), 0..32),
    ) {
        let bytes = [request("Origin: https://ovis.example\r\n").into_bytes(), tail.clone()].concat();
        let mut chunks = Vec::new();
        let mut start = 0;
        for cut in cuts.into_iter().filter(|cut| *cut < bytes.len()) {
            chunks.push(bytes.get(start..cut).unwrap_or_default().to_vec());
            start = cut;
        }
        chunks.push(bytes.get(start..).unwrap_or_default().to_vec());
        let origins = AllowedOrigins::new(["https://ovis.example".parse().map_err(|_| TestCaseError::fail("origin"))?]);
        let runtime = tokio::runtime::Builder::new_current_thread().build()
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let (answered, response) = runtime.block_on(upgrade(chunks, &origins))
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert!(response.starts_with("HTTP/1.1 101 "));
        prop_assert_eq!(answered, Ok(tail));
    }
}
