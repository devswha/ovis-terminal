//! The framed-channel layout, read from the wire codec rather than restated.

use serde_json::{Value, json};

use crate::wire::{Channel, FRAME_HEADER_LEN, FrameKind, MAX_FRAME_PAYLOAD};

const fn kind_contract(kind: FrameKind) -> (&'static str, &'static str) {
    match kind {
        FrameKind::Data => (
            "DATA",
            "Channel 0: one JSON-RPC message. Stream channels: downstream terminal bytes only.",
        ),
        FrameKind::Credit => (
            "CREDIT",
            "Eight-byte little-endian cumulative acknowledged filtered-stream offset.",
        ),
        FrameKind::Close => ("CLOSE", "Typed close control for one channel."),
    }
}

/// Describes the framed mode: header layout, bounds and every kind the decoder accepts.
pub(super) fn stream_layout() -> Value {
    let kinds: Vec<Value> = (0..=u8::MAX)
        .filter_map(|value| {
            let kind = FrameKind::try_from(value).ok()?;
            let (name, contract) = kind_contract(kind);
            Some(json!({ "name": name, "value": value, "contract": contract }))
        })
        .collect();
    json!({
        "header_bytes": FRAME_HEADER_LEN,
        "header": [
            { "name": "channel", "offset": 0, "encoding": "u32le" },
            { "name": "kind", "offset": 4, "encoding": "u8" },
            { "name": "length", "offset": 5, "encoding": "u32le" },
        ],
        "max_payload_bytes": MAX_FRAME_PAYLOAD,
        "control_channel": Channel::CONTROL.value(),
        "kinds": kinds,
        "upgrade": "hello with framed: true; the reply is the last NDJSON line",
    })
}
