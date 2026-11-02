//! Transport-neutral replay of the Ovis consumer conformance fixtures.
//!
//! A fixture set (`conformance/fixtures/ovis`) pins the schema, ledgers and its own
//! files by SHA-256, maps every IDEAL predicate and exclusion to the lane scenarios
//! that prove it, and carries replay cases: requests with symbolic identities bound
//! at run time, expected results checked by JSON pointer, awaited events and the
//! normalized receipts every client must reproduce. A consumer supplies a
//! [`Transport`] (the Rust conformance runner speaks the binary's `connect` bridge,
//! the Bun client its own socket) and [`replay()`] does the rest.

pub mod bind;
pub mod case;
pub mod error;
pub mod expect;
pub mod hash;
pub mod legacy;
pub mod manifest;
pub mod predicates;
pub mod receipt;
pub mod refresh;
pub mod replay;
pub mod set;

pub use bind::Bindings;
pub use case::{Adversarial, Call, Case, CaseId, CaseKind, ConnLabel, EventStep, Server, Step};
pub use error::ReplayError;
pub use expect::{Check, Expect, Matcher};
pub use legacy::{Field, LegacyMap, Translation};
pub use manifest::{Manifest, Pin, ServerProfile};
pub use predicates::{Predicate, Predicates, ScenarioState};
pub use receipt::{Outcome, Receipt, ReceiptLine};
pub use refresh::refresh;
pub use replay::{CaseRun, Options, Transport, replay};
pub use set::FixtureSet;
