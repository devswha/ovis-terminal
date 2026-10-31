//! Literal text, key and paste encoding without hidden execution (plan todo 32).
//!
//! [`encode`] turns one input request into the exact bytes of one queue entry, decided
//! against one frozen [`ModeSnapshot`] whose revision travels with the bytes:
//!
//! - Literal text is data. Line breaks and tabs keep their literal meaning only inside a
//!   bracketed paste, so without bracketed paste they are refused (`paste_refused`) whatever
//!   the submission, `none` included. Every other control character and any embedded
//!   bracketed-paste marker is `invalid_encoding`.
//! - The submission is encoded separately and appended after the literal in the same entry,
//!   outside any paste frame.
//! - Named keys follow the negotiated cursor and kitty disambiguation modes; an unknown name
//!   is `unknown_key` and a chord the modes cannot carry is `invalid_encoding`, both before
//!   any byte exists.
//! - Mouse reports must match the negotiated tracking and encoding and pass unchanged.
//! - Raw bytes need a [`RawGrant`] from the authority and pass unchanged, unknown sequences
//!   included; nothing is swallowed.
//!
//! [`Encoded::ensure_current`] fences the write: bytes encoded at one mode revision are
//! `mode_mismatch` once the terminal moved to another.

mod encoded;
mod error;
mod keys;
mod literal;
mod modes;
mod mouse;
mod raw;
mod request;

pub use encoded::{Encoded, EncodingEvidence, InputKind};
pub use error::{EncodingError, PayloadIndex};
pub use modes::ModeSnapshot;
pub use raw::{RawGrant, RawRequest};
pub use request::{InputRequest, encode};
