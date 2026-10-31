//! The HTTP/1.1 upgrade that opens a WebSocket (RFC 6455 section 4.2).
//!
//! The request head is read within a byte bound and checked before any protocol byte
//! is read: method and version, the upgrade headers, the key, the WebSocket version and
//! the browser origin. A refusal is an HTTP response with a typed JSON body that echoes
//! nothing from the request; the connection then closes.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha1::{Digest as _, Sha1};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use super::error::UpgradeDefect;
use super::{AllowedOrigins, WsError};

/// The largest request head read before the upgrade is refused.
pub(super) const MAX_HEAD: usize = 8 * 1024;
const MAX_HEADERS: usize = 32;
const ACCEPT_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// `Sec-WebSocket-Accept` for a client's `Sec-WebSocket-Key`.
pub(super) fn accept_key(key: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key);
    hasher.update(ACCEPT_GUID);
    STANDARD.encode(hasher.finalize())
}

/// Answers the upgrade request on `stream`; returns the bytes the client sent after its
/// request head, which already belong to the WebSocket.
///
/// # Errors
///
/// The [`WsError`] of a refused, malformed or interrupted upgrade; a refusal has been
/// answered with an HTTP error response.
pub(super) async fn answer<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    origins: &AllowedOrigins,
) -> Result<Vec<u8>, WsError> {
    let refused = match read_head(stream).await {
        Ok((head, rest)) => match accept(&head, origins) {
            Ok(key) => {
                let response = format!(
                    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\
                     Connection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                    accept_key(&key)
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .map_err(WsError::Transport)?;
                return Ok(rest);
            }
            Err(refused) => refused,
        },
        Err(gone @ (WsError::Transport(_) | WsError::InvalidUpgrade(UpgradeDefect::Truncated))) => {
            return Err(gone);
        }
        Err(refused) => refused,
    };
    // The peer may already be gone; the refusal is reported either way.
    match stream.write_all(&refusal(&refused)).await {
        Ok(()) | Err(_) => Err(refused),
    }
}

async fn read_head<S: AsyncRead + Unpin>(stream: &mut S) -> Result<(Vec<u8>, Vec<u8>), WsError> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await.map_err(WsError::Transport)?;
        let bytes = chunk
            .get(..read)
            .ok_or(WsError::Transport(std::io::ErrorKind::InvalidData.into()))?;
        if bytes.is_empty() {
            return Err(WsError::InvalidUpgrade(UpgradeDefect::Truncated));
        }
        buffer.extend_from_slice(bytes);
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let parsed = httparse::Request::new(&mut headers).parse(&buffer);
        match parsed {
            Ok(httparse::Status::Complete(length)) => {
                let rest = buffer.split_off(length);
                return Ok((buffer, rest));
            }
            Ok(httparse::Status::Partial) if buffer.len() < MAX_HEAD => {}
            Ok(httparse::Status::Partial) | Err(httparse::Error::TooManyHeaders) => {
                return Err(WsError::UpgradeTooLarge);
            }
            Err(_) => return Err(WsError::InvalidUpgrade(UpgradeDefect::Malformed)),
        }
    }
}

/// Checks a complete request head; returns the client's key.
fn accept(head: &[u8], origins: &AllowedOrigins) -> Result<Vec<u8>, WsError> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut request = httparse::Request::new(&mut headers);
    let malformed = WsError::InvalidUpgrade(UpgradeDefect::Malformed);
    if !matches!(request.parse(head), Ok(httparse::Status::Complete(_))) {
        return Err(malformed);
    }
    if request.method != Some("GET") || request.version != Some(1) {
        return Err(WsError::InvalidUpgrade(UpgradeDefect::NotGet));
    }
    let values = |name: &'static str| {
        request
            .headers
            .iter()
            .filter(move |header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value)
    };
    let has_token = |name: &'static str, token: &str| {
        values(name).any(|value| {
            value
                .split(|byte| *byte == b',')
                .any(|item| item.trim_ascii().eq_ignore_ascii_case(token.as_bytes()))
        })
    };
    if !has_token("upgrade", "websocket") || !has_token("connection", "upgrade") {
        return Err(WsError::InvalidUpgrade(UpgradeDefect::NotUpgrade));
    }
    if !values("sec-websocket-version").eq([b"13".as_slice()]) {
        return Err(WsError::UnsupportedVersion);
    }
    let mut keys = values("sec-websocket-key");
    let key = match (keys.next(), keys.next()) {
        (Some(key), None) if STANDARD.decode(key).is_ok_and(|raw| raw.len() == 16) => key,
        _ => return Err(WsError::InvalidUpgrade(UpgradeDefect::Key)),
    };
    origins.admit(values("origin"))?;
    Ok(key.to_vec())
}

fn refusal(error: &WsError) -> Vec<u8> {
    let status = match error {
        WsError::OriginRejected => "403 Forbidden",
        WsError::UnsupportedVersion => "426 Upgrade Required\r\nSec-WebSocket-Version: 13",
        WsError::UpgradeTooLarge => "431 Request Header Fields Too Large",
        _ => "400 Bad Request",
    };
    let body = serde_json::json!({"kind": error.kind(), "hint": error.hint()}).to_string();
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

#[cfg(test)]
mod tests;
