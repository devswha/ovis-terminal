//! Typed source of truth for every request, reply, event, error, channel,
//! limit and capability; deterministic OpenRPC generation and the
//! released-baseline compatibility gate (plan todo 4).

mod advertise;
mod amendments;
pub mod common;
pub mod compat;
mod decode;
mod document;
mod error;
mod error_catalog;
pub mod error_kinds;
mod event_catalog;
pub mod events;
pub mod ids;
pub mod limits;
pub mod method;
pub mod methods;
pub mod records;
pub mod scalar;
mod stream;
mod verify;

pub use advertise::advertised_methods;
pub use amendments::{ENTRY_KEYS, baseline_digest, check_amendments};
pub use compat::{Break, BreakRule, breaking_changes, check_compatibility};
pub use decode::{ParamsError, decode_params};
pub use document::{
    COMPONENTS_PREFIX, OPENRPC_VERSION, PROTOCOL_GENERATION, openrpc_document, render_openrpc,
};
pub use error::SchemaError;
pub use verify::verify_document;
