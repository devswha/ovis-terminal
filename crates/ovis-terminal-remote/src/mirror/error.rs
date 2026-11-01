//! Failures of mirroring one remote record.

/// Why a remote record could not be mirrored; the cursor is left unchanged.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MirrorError {
    /// The remote record does not decode.
    #[error("the remote record does not decode")]
    Decode(#[source] serde_json::Error),
    /// The machine or lifetime is not a runtime identity of this protocol.
    #[error("the remote machine or runtime lifetime is not a valid identity")]
    Identity(#[source] ovis_terminal_core::registry::IdentityError),
}

impl MirrorError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Decode(_) => "invalid_params",
            Self::Identity(_) => "remote_incompatible",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Decode(_) => "send event and heartbeat records of this protocol generation",
            Self::Identity(_) => "update the remote runtime to this protocol generation",
        }
    }
}
