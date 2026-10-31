//! The encoding boundary error, its stable kinds and recovery hints.

use ovis_terminal_authz::AuthzError;
use ovis_terminal_vt::{ModeRevision, MouseEncoding};

/// Byte position inside the payload a request submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PayloadIndex(usize);

impl PayloadIndex {
    pub(super) const fn new(index: usize) -> Self {
        Self(index)
    }

    /// Zero-based byte index.
    pub const fn get(self) -> usize {
        self.0
    }
}

/// Why a request was not encoded; no byte exists for the terminal when this is returned.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EncodingError {
    /// Nothing would be written.
    #[error("the request carries nothing to write")]
    Empty,
    /// The encoded entry does not fit a byte count.
    #[error("the encoded entry is larger than a byte count can describe")]
    Oversized,
    /// Literal text contains a bracketed-paste start or end marker.
    #[error("byte {} opens a bracketed-paste marker inside literal text", at.get())]
    PasteMarker {
        /// Where the marker starts.
        at: PayloadIndex,
    },
    /// Literal text contains a control character that would act instead of being typed.
    #[error("byte {} is the control character U+{code:04X}", at.get())]
    ControlCharacter {
        /// Where the character starts.
        at: PayloadIndex,
        /// Its code point.
        code: u32,
    },
    /// Line breaks or tabs cannot stay literal without bracketed paste.
    #[error(
        "line breaks and tabs stay literal only under bracketed paste, absent at mode revision {revision}"
    )]
    NeedsBracketedPaste {
        /// Mode revision the request was checked against.
        revision: ModeRevision,
    },
    /// The key name is not known.
    #[error("{} is not a key name", key.escape_debug())]
    UnknownKey {
        /// The name as sent.
        key: String,
    },
    /// The chord has no encoding in the negotiated keyboard modes.
    #[error("{} has no encoding at mode revision {revision}", key.escape_debug())]
    KeyUnencodable {
        /// The name as sent.
        key: String,
        /// Mode revision the request was checked against.
        revision: ModeRevision,
    },
    /// The program negotiated a keyboard protocol this runtime does not encode.
    #[error("mode revision {revision} negotiated a keyboard protocol without an encoder here")]
    KeyboardProtocol {
        /// Mode revision the request was checked against.
        revision: ModeRevision,
    },
    /// The program did not ask for mouse reports.
    #[error("mouse reporting is off at mode revision {revision}")]
    MouseOff {
        /// Mode revision the request was checked against.
        revision: ModeRevision,
    },
    /// The bytes are not mouse reports in the negotiated encoding.
    #[error("byte {} does not start a well-formed report in the negotiated mouse encoding", at.get())]
    MalformedMouse {
        /// Start of the first malformed report.
        at: PayloadIndex,
        /// Encoding the program negotiated.
        expected: MouseEncoding,
    },
    /// The authority refused the raw-input grant or withdrew it before the write.
    #[error("the raw-input grant was refused")]
    RawNotAuthorized(#[source] AuthzError),
    /// The request names an option this runtime has no encoder for.
    #[error("the request names an option this runtime does not encode")]
    UnsupportedOption,
    /// The terminal modes moved past the revision the bytes were encoded against.
    #[error("encoded at mode revision {encoded}, the terminal is at {current}")]
    ModeChanged {
        /// Revision the bytes were encoded against.
        encoded: ModeRevision,
        /// Revision the terminal has now.
        current: ModeRevision,
    },
}

impl EncodingError {
    /// The stable machine-readable kind clients branch on.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Empty | Self::UnsupportedOption => "invalid_params",
            Self::Oversized => "input_overflow",
            Self::PasteMarker { .. }
            | Self::ControlCharacter { .. }
            | Self::KeyUnencodable { .. }
            | Self::KeyboardProtocol { .. }
            | Self::MouseOff { .. }
            | Self::MalformedMouse { .. } => "invalid_encoding",
            Self::NeedsBracketedPaste { .. } => "paste_refused",
            Self::UnknownKey { .. } => "unknown_key",
            Self::RawNotAuthorized(_) => "raw_not_authorized",
            Self::ModeChanged { .. } => "mode_mismatch",
        }
    }

    /// Recovery guidance published with the kind.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Empty => "send text, keys or bytes",
            Self::Oversized => "split the input into smaller requests",
            Self::PasteMarker { .. } | Self::ControlCharacter { .. } => {
                "send keys with input.keys or authorized bytes with input.raw"
            }
            Self::NeedsBracketedPaste { .. } => {
                "send one line at a time or wait until the program enables bracketed paste"
            }
            Self::UnknownKey { .. } => "use a name such as enter, esc, up, ctrl+c, alt+x or f5",
            Self::KeyUnencodable { .. } | Self::KeyboardProtocol { .. } => {
                "send the bytes the program expects with input.raw"
            }
            Self::MouseOff { .. } | Self::MalformedMouse { .. } => {
                "read the terminal modes and send reports in the negotiated encoding"
            }
            Self::RawNotAuthorized(_) => "obtain the raw-input grant for this terminal",
            Self::UnsupportedOption => "use an option this schema version publishes",
            Self::ModeChanged { .. } => "encode again against the current terminal modes",
        }
    }
}
