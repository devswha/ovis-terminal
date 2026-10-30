//! Stable error kinds and their JSON-RPC code mapping.

use serde::{Deserialize, Serialize};

/// JSON-RPC response code selected from a typed boundary outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum JsonRpcCode {
    /// Invalid JSON text.
    ParseError,
    /// A request envelope violates JSON-RPC 2.0.
    InvalidRequest,
    /// The method name is not recognized.
    MethodNotFound,
    /// A method parameter violates its typed schema.
    InvalidParams,
    /// An unexpected internal failure occurred.
    InternalError,
    /// A protocol-defined application error.
    ServerError,
}

impl JsonRpcCode {
    /// Returns the stable numeric JSON-RPC code.
    #[must_use]
    pub const fn value(self) -> i32 {
        match self {
            Self::ParseError => -32700,
            Self::InvalidRequest => -32600,
            Self::MethodNotFound => -32601,
            Self::InvalidParams => -32602,
            Self::InternalError => -32603,
            Self::ServerError => -32000,
        }
    }
}

/// Stable machine-readable error classification.
///
/// The set is open: a kind this build does not know decodes as [`ErrorKind::Other`]
/// with its token, and encodes back to the same token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum ErrorKind {
    /// Malformed JSON text.
    ParseError,
    /// Malformed JSON-RPC request envelope.
    InvalidRequest,
    /// Unknown RPC method.
    MethodNotFound,
    /// Invalid method parameters.
    InvalidParams,
    /// Invalid JSON-RPC response envelope.
    InvalidResponse,
    /// Duplicate ID already active on this connection.
    DuplicateRequestId,
    /// A request is unknown or already has a terminal response.
    UnknownOrSettledRequestId,
    /// A terminal response carries a different request ID.
    ResponseIdMismatch,
    /// A requested resource does not exist.
    NotFound,
    /// A resource or operation is unavailable.
    Unavailable,
    /// An unexpected protocol or service failure occurred.
    Internal,
    /// A kind this build does not know, kept verbatim.
    Other(String),
}

impl From<String> for ErrorKind {
    fn from(token: String) -> Self {
        match token.as_str() {
            "parse_error" => Self::ParseError,
            "invalid_request" => Self::InvalidRequest,
            "method_not_found" => Self::MethodNotFound,
            "invalid_params" => Self::InvalidParams,
            "invalid_response" => Self::InvalidResponse,
            "duplicate_request_id" => Self::DuplicateRequestId,
            "unknown_or_settled_request_id" => Self::UnknownOrSettledRequestId,
            "response_id_mismatch" => Self::ResponseIdMismatch,
            "not_found" => Self::NotFound,
            "unavailable" => Self::Unavailable,
            "internal" => Self::Internal,
            _ => Self::Other(token),
        }
    }
}

impl From<ErrorKind> for String {
    fn from(kind: ErrorKind) -> Self {
        match kind {
            ErrorKind::Other(token) => token,
            known => known.token().to_owned(),
        }
    }
}

impl ErrorKind {
    /// Returns the wire token clients branch on.
    #[must_use]
    pub fn token(&self) -> &str {
        match self {
            Self::ParseError => "parse_error",
            Self::InvalidRequest => "invalid_request",
            Self::MethodNotFound => "method_not_found",
            Self::InvalidParams => "invalid_params",
            Self::InvalidResponse => "invalid_response",
            Self::DuplicateRequestId => "duplicate_request_id",
            Self::UnknownOrSettledRequestId => "unknown_or_settled_request_id",
            Self::ResponseIdMismatch => "response_id_mismatch",
            Self::NotFound => "not_found",
            Self::Unavailable => "unavailable",
            Self::Internal => "internal",
            Self::Other(token) => token,
        }
    }

    /// Returns the recovery guidance published for this kind.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::ParseError => "send one valid JSON record",
            Self::InvalidRequest => "send a JSON-RPC 2.0 request with a string id",
            Self::MethodNotFound => "consult rpc.discover for supported methods",
            Self::InvalidParams => "correct the method parameters",
            Self::InvalidResponse => "return exactly one of result or error",
            Self::DuplicateRequestId => "wait for the existing request with this id to finish",
            Self::UnknownOrSettledRequestId => {
                "inspect connection state before settling the request"
            }
            Self::ResponseIdMismatch => "settle the response using its matching request id",
            Self::NotFound => "list the resource family to find a live id",
            Self::Unavailable => "retry after the reported condition clears",
            Self::Internal => "inspect the runtime log for the operation id",
            Self::Other(_) => "consult schema.errors for this kind",
        }
    }

    pub(super) const fn json_rpc_code(&self) -> JsonRpcCode {
        match self {
            Self::ParseError => JsonRpcCode::ParseError,
            Self::InvalidRequest
            | Self::DuplicateRequestId
            | Self::UnknownOrSettledRequestId
            | Self::ResponseIdMismatch
            | Self::InvalidResponse => JsonRpcCode::InvalidRequest,
            Self::MethodNotFound => JsonRpcCode::MethodNotFound,
            Self::InvalidParams => JsonRpcCode::InvalidParams,
            Self::Internal => JsonRpcCode::InternalError,
            Self::NotFound | Self::Unavailable | Self::Other(_) => JsonRpcCode::ServerError,
        }
    }
}
