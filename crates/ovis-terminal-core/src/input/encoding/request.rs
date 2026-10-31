//! One input request turned into one queue entry against one mode snapshot.

use ovis_terminal_protocol::schema::methods::input::{BracketedPolicy, Submit};

use super::encoded::{Encoded, Entry, InputKind};
use super::error::EncodingError;
use super::literal::{self, Framing, Intent};
use super::modes::ModeSnapshot;
use super::raw::RawGrant;
use super::{keys, mouse};

/// An input request as the `input.*` methods admit it.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum InputRequest<'a> {
    /// Literal text typed as data, then its submission in the same entry (`input.send`).
    Text {
        /// Literal text.
        text: &'a str,
        /// What follows the text.
        submit: &'a Submit,
    },
    /// Pasted text under an explicit bracketing policy (`input.paste`).
    Paste {
        /// Literal text.
        text: &'a str,
        /// Bracketing policy.
        policy: BracketedPolicy,
    },
    /// Named keys (`input.keys`).
    Keys(&'a [String]),
    /// Pre-encoded mouse reports.
    Mouse(&'a [u8]),
    /// Raw bytes under the raw-input grant (`input.raw`).
    Raw {
        /// Bytes to write exactly.
        bytes: &'a [u8],
        /// The authority's grant.
        grant: &'a RawGrant,
    },
}

fn submission(submit: &Submit, snapshot: &ModeSnapshot) -> Result<Vec<u8>, EncodingError> {
    match submit {
        Submit::None => Ok(Vec::new()),
        Submit::Enter => keys::encode(&[String::from("enter")], snapshot),
        Submit::Keys(names) => keys::encode(names, snapshot),
        _ => Err(EncodingError::UnsupportedOption),
    }
}

fn literal_entry(
    text: &str,
    intent: Intent,
    snapshot: &ModeSnapshot,
) -> Result<(Entry, Vec<u8>), EncodingError> {
    let framing = literal::frame(text, intent, snapshot)?;
    let mut bytes = Vec::new();
    literal::write(text, framing, &mut bytes);
    let kind = match intent {
        Intent::Type => InputKind::Text,
        Intent::Paste(_) => InputKind::Paste,
    };
    let entry = Entry {
        kind,
        mode_revision: snapshot.revision(),
        bracketed: framing == Framing::Bracketed,
    };
    Ok((entry, bytes))
}

const fn unframed(kind: InputKind, snapshot: &ModeSnapshot) -> Entry {
    Entry {
        kind,
        mode_revision: snapshot.revision(),
        bracketed: false,
    }
}

/// Encodes `request` against `snapshot`; every check runs before any byte is produced.
///
/// # Errors
///
/// `paste_refused` for line breaks or tabs without bracketed paste, `invalid_encoding` for
/// control characters, paste markers, unencodable chords or malformed mouse reports,
/// `unknown_key`, and `invalid_params` for an empty request.
pub fn encode(
    snapshot: &ModeSnapshot,
    request: InputRequest<'_>,
) -> Result<Encoded, EncodingError> {
    match request {
        InputRequest::Text { text, submit } => {
            let (entry, literal) = literal_entry(text, Intent::Type, snapshot)?;
            let submit = submission(submit, snapshot)?;
            entry.finish(literal, &submit)
        }
        InputRequest::Paste { text, policy } => {
            let (entry, literal) = literal_entry(text, Intent::Paste(policy), snapshot)?;
            entry.finish(literal, &[])
        }
        InputRequest::Keys(names) => {
            unframed(InputKind::Keys, snapshot).finish(keys::encode(names, snapshot)?, &[])
        }
        InputRequest::Mouse(report) => {
            mouse::check(report, snapshot)?;
            unframed(InputKind::Mouse, snapshot).finish(report.to_vec(), &[])
        }
        InputRequest::Raw { bytes, grant: _ } => {
            unframed(InputKind::Raw, snapshot).finish(bytes.to_vec(), &[])
        }
    }
}
