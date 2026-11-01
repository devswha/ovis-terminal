//! Discovery reads answered from this build's own OpenRPC document: `rpc.discover`,
//! `schema.errors`, `schema.events` and `schema.capabilities`. `hello` is answered by the
//! connection before any request reaches a handler; it is advertised because the server
//! serves it.

use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::common::NoParams;
use ovis_terminal_protocol::schema::limits::{Limits, TransportKind};
use ovis_terminal_protocol::schema::methods::discovery::{
    SchemaCapabilitiesResult, SchemaErrorsResult, SchemaEventsResult,
};
use ovis_terminal_protocol::schema::{PROTOCOL_GENERATION, decode_params, openrpc_document};
use ovis_terminal_server::conn::Call;
use serde_json::{Value, json};

use super::error::{checked, internal};

/// Answered by the connection layer on every transport.
pub(super) const HELLO: &str = "hello";

/// The reads this module answers.
pub(super) const DISCOVERY: [&str; 4] = [
    "rpc.discover",
    "schema.errors",
    "schema.events",
    "schema.capabilities",
];

/// The capabilities hello reports for a server listening on `transports`.
fn capabilities(transports: &[TransportKind]) -> Value {
    json!({"transports": transports, "stream_modes": [], "framed": true})
}

/// Answers one discovery read from the generated document.
pub(super) fn answer(call: &Call, transports: &[TransportKind]) -> Result<Value, BoundaryError> {
    let _: NoParams = decode_params(&call.request).map_err(BoundaryError::from)?;
    let document = openrpc_document().map_err(internal("render the OpenRPC document"))?;
    let section = |key: &str| document.get(key).cloned().unwrap_or(Value::Null);
    match call.request.method() {
        "schema.errors" => {
            checked::<SchemaErrorsResult>(json!({"kinds": section("x-error-kinds")}))
        }
        "schema.events" => checked::<SchemaEventsResult>(json!({"events": section("x-events")})),
        "schema.capabilities" => checked::<SchemaCapabilitiesResult>(json!({
            "generation": PROTOCOL_GENERATION,
            "capabilities": capabilities(transports),
            "limits": Limits::v1(),
        })),
        _ => Ok(document),
    }
}
