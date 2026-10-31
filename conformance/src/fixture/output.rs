//! `terminal.output` over a deterministic 64 MiB retained output, paged under the frame bound.

use base64::Engine as _;
use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::rpc::RpcRequest;
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::methods::terminal_read::TerminalOutputParams;
use ovis_terminal_protocol::wire::MAX_FRAME_PAYLOAD;
use serde_json::{Value, json};

/// Length of the retained output of `terminal:1`.
pub(crate) const TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// The only terminal with retained output.
const TERMINAL: &str = "terminal:1";
/// Room left in a frame for the JSON-RPC envelope around `data_b64`.
const ENVELOPE_BYTES: u64 = 4096;

/// Byte `offset` of the retained output; position-dependent, so a dropped, duplicated or
/// reordered page changes the digest of the reassembled result.
pub(crate) const fn byte_at(offset: u64) -> u8 {
    let mixed = offset ^ (offset >> 8) ^ (offset >> 17);
    let [low, ..] = mixed.to_le_bytes();
    low
}

fn domain(kind: &str, hint: &'static str) -> BoundaryError {
    BoundaryError::domain(ErrorKind::from(kind.to_owned()), false).with_hint(hint)
}

/// The largest page whose base64 reply still fits one frame.
pub(crate) fn max_page_bytes() -> u64 {
    let frame = u64::try_from(MAX_FRAME_PAYLOAD).unwrap_or(u64::MAX);
    frame
        .saturating_sub(ENVELOPE_BYTES)
        .checked_div(4)
        .map_or(0, |quads| quads.saturating_mul(3))
}

/// Answers one page, or `frame_too_large` when the requested page could not fit a frame.
pub(crate) fn page(request: &RpcRequest) -> Result<Value, BoundaryError> {
    let params: TerminalOutputParams = decode_params(request).map_err(BoundaryError::from)?;
    match &params.target {
        Target::TerminalId(id) if id.as_str() == TERMINAL => {}
        _ => {
            return Err(domain(
                "not_found",
                "name a terminal that has retained output",
            ));
        }
    }
    let limit = params.limit_bytes.get();
    if limit > max_page_bytes() {
        return Err(domain(
            "frame_too_large",
            "request a smaller limit_bytes and follow next_offset",
        ));
    }
    let from = params.from_offset.get().min(TOTAL_BYTES);
    let end = from.saturating_add(limit).min(TOTAL_BYTES);
    let data: Vec<u8> = (from..end).map(byte_at).collect();
    Ok(json!({
        "data_b64": base64::engine::general_purpose::STANDARD.encode(data),
        "from_offset": from.to_string(),
        "next_offset": end.to_string(),
        "floor_offset": "0",
        "gap": false,
    }))
}

#[cfg(test)]
mod tests {
    use ovis_terminal_protocol::rpc::parse_request;
    use ovis_terminal_protocol::wire::MAX_FRAME_PAYLOAD;

    use super::{max_page_bytes, page};

    fn ask(params: &serde_json::Value) -> Result<serde_json::Value, String> {
        let text = serde_json::json!({"jsonrpc":"2.0","id":"p","method":"terminal.output","params":params});
        let request = parse_request(&text.to_string()).map_err(|error| error.kind().to_owned())?;
        page(&request).map_err(|error| error.kind().to_owned())
    }

    #[test]
    fn the_largest_allowed_page_fits_one_frame_and_one_more_byte_is_refused() {
        let target = serde_json::json!({"terminal_id": "terminal:1"});
        let largest = max_page_bytes().to_string();
        let reply =
            ask(&serde_json::json!({"target": target, "from_offset": "0", "limit_bytes": largest}));
        let size = reply.map(|value| value.to_string().len());
        assert!(
            size.as_ref().is_ok_and(|size| *size <= MAX_FRAME_PAYLOAD),
            "{size:?}"
        );
        let over = max_page_bytes().saturating_add(1).to_string();
        let refused =
            ask(&serde_json::json!({"target": target, "from_offset": "0", "limit_bytes": over}));
        assert_eq!(refused.err().as_deref(), Some("frame_too_large"));
        assert_eq!(
            ask(&serde_json::json!({"target": target, "from_offset": 0, "limit_bytes": "1"}))
                .err()
                .as_deref(),
            Some("invalid_params")
        );
    }
}
