//! Typed errors crossing the JSON-RPC protocol boundary.

mod identity;

pub use identity::{FieldPath, OpId, TargetId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// A requested resource does not exist.
    NotFound,
    /// A resource or operation is unavailable.
    Unavailable,
    /// An unexpected protocol or service failure occurred.
    Internal,
}

impl ErrorKind {
    /// Returns the wire token clients branch on.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::ParseError => "parse_error",
            Self::InvalidRequest => "invalid_request",
            Self::MethodNotFound => "method_not_found",
            Self::InvalidParams => "invalid_params",
            Self::InvalidResponse => "invalid_response",
            Self::DuplicateRequestId => "duplicate_request_id",
            Self::NotFound => "not_found",
            Self::Unavailable => "unavailable",
            Self::Internal => "internal",
        }
    }

    /// Returns the recovery guidance published for this kind.
    #[must_use]
    pub const fn hint(self) -> &'static str {
        match self {
            Self::ParseError => "send one valid JSON record",
            Self::InvalidRequest => "send a JSON-RPC 2.0 request with a string id",
            Self::MethodNotFound => "consult rpc.discover for supported methods",
            Self::InvalidParams => "correct the method parameters",
            Self::InvalidResponse => "return exactly one of result or error",
            Self::DuplicateRequestId => "wait for the existing request with this id to finish",
            Self::NotFound => "list the resource family to find a live id",
            Self::Unavailable => "retry after the reported condition clears",
            Self::Internal => "inspect the runtime log for the operation id",
        }
    }

    const fn json_rpc_code(self) -> JsonRpcCode {
        match self {
            Self::ParseError => JsonRpcCode::ParseError,
            Self::InvalidRequest | Self::DuplicateRequestId | Self::InvalidResponse => {
                JsonRpcCode::InvalidRequest
            }
            Self::MethodNotFound => JsonRpcCode::MethodNotFound,
            Self::InvalidParams => JsonRpcCode::InvalidParams,
            Self::Internal => JsonRpcCode::InternalError,
            Self::NotFound | Self::Unavailable => JsonRpcCode::ServerError,
        }
    }
}

/// Machine-readable context attached to a boundary failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ErrorData {
    /// Stable error category.
    pub kind: ErrorKind,
    /// Recovery guidance for a client.
    pub hint: String,
    /// Whether retrying the same request may succeed.
    pub retryable: bool,
    /// Path to the invalid input field, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_path: Option<FieldPath>,
    /// Stable target identity, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<TargetId>,
    /// Operation identity when effects may have begun.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op_id: Option<OpId>,
}

impl ErrorData {
    fn new(kind: ErrorKind, retryable: bool) -> Self {
        Self {
            kind,
            hint: kind.hint().to_owned(),
            retryable,
            field_path: None,
            target: None,
            op_id: None,
        }
    }

    fn with_field_path(mut self, field_path: Option<&str>) -> Self {
        self.field_path = field_path.map(|path| FieldPath::new(path.to_owned()));
        self
    }
}

/// Error raised while parsing, validating or correlating protocol records.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum BoundaryError {
    /// The incoming bytes are not valid JSON.
    #[error("input is not a JSON document")]
    ParseError {
        /// Machine-readable error context.
        data: ErrorData,
    },
    /// The incoming request envelope is invalid.
    #[error("request envelope violates JSON-RPC 2.0")]
    InvalidRequest {
        /// Machine-readable error context.
        data: ErrorData,
    },
    /// The requested method is not registered.
    #[error("no such JSON-RPC method")]
    MethodNotFound {
        /// Machine-readable error context.
        data: ErrorData,
    },
    /// The method parameters do not match its closed schema.
    #[error("method parameters violate the closed schema")]
    InvalidParams {
        /// Machine-readable error context.
        data: ErrorData,
    },
    /// The same request ID is already active on the connection.
    #[error("request id is already in flight on this connection")]
    DuplicateRequestId {
        /// Machine-readable error context.
        data: ErrorData,
    },
    /// The incoming response has neither or both terminal branches.
    #[error("response envelope violates JSON-RPC 2.0")]
    InvalidResponse {
        /// Machine-readable error context.
        data: ErrorData,
    },
    /// A typed domain failure is ready to cross the protocol boundary.
    #[error("operation failed")]
    Domain {
        /// Machine-readable error context.
        data: ErrorData,
    },
}

impl BoundaryError {
    /// Creates a malformed-JSON failure.
    #[must_use]
    pub fn parse_error() -> Self {
        Self::ParseError {
            data: ErrorData::new(ErrorKind::ParseError, false),
        }
    }

    /// Creates an invalid request failure with an optional field path.
    #[must_use]
    pub fn invalid_request(field_path: Option<&str>) -> Self {
        Self::InvalidRequest {
            data: ErrorData::new(ErrorKind::InvalidRequest, false).with_field_path(field_path),
        }
    }

    /// Creates an invalid-parameters failure with an optional field path.
    #[must_use]
    pub fn invalid_params(field_path: Option<&str>) -> Self {
        Self::InvalidParams {
            data: ErrorData::new(ErrorKind::InvalidParams, false).with_field_path(field_path),
        }
    }

    /// Creates an unknown-method failure.
    #[must_use]
    pub fn method_not_found() -> Self {
        Self::MethodNotFound {
            data: ErrorData::new(ErrorKind::MethodNotFound, false),
        }
    }

    /// Creates a duplicate in-flight request ID failure.
    #[must_use]
    pub fn duplicate_request_id() -> Self {
        Self::DuplicateRequestId {
            data: ErrorData::new(ErrorKind::DuplicateRequestId, false).with_field_path(Some("id")),
        }
    }

    /// Creates a response-envelope validation failure.
    #[must_use]
    pub fn invalid_response() -> Self {
        Self::InvalidResponse {
            data: ErrorData::new(ErrorKind::InvalidResponse, false),
        }
    }

    /// Creates a typed domain error carrying its kind's published hint.
    #[must_use]
    pub fn domain(kind: ErrorKind, retryable: bool) -> Self {
        Self::Domain {
            data: ErrorData::new(kind, retryable),
        }
    }

    /// Adds a stable target identity.
    #[must_use]
    pub fn with_target(mut self, target: TargetId) -> Self {
        self.data_mut().target = Some(target);
        self
    }

    /// Adds an operation identity when effects may have begun.
    #[must_use]
    pub fn with_op_id(mut self, op_id: OpId) -> Self {
        self.data_mut().op_id = Some(op_id);
        self
    }

    /// Returns the stable machine token clients branch on.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        self.data().kind.token()
    }

    /// Returns the typed error category.
    #[must_use]
    pub const fn error_kind(&self) -> ErrorKind {
        self.data().kind
    }

    /// Returns the recovery guidance for this failure.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        self.data().kind.hint()
    }

    /// Returns the JSON-RPC code selected by the typed kind.
    #[must_use]
    pub const fn json_rpc_code(&self) -> JsonRpcCode {
        self.data().kind.json_rpc_code()
    }

    /// Returns the offending field path, when known.
    #[must_use]
    pub fn field_path(&self) -> Option<&str> {
        self.data().field_path.as_ref().map(FieldPath::as_str)
    }

    /// Returns the structured error data for a JSON-RPC failure object.
    #[must_use]
    pub const fn data(&self) -> &ErrorData {
        match self {
            Self::ParseError { data }
            | Self::InvalidRequest { data }
            | Self::MethodNotFound { data }
            | Self::InvalidParams { data }
            | Self::DuplicateRequestId { data }
            | Self::InvalidResponse { data }
            | Self::Domain { data } => data,
        }
    }

    const fn data_mut(&mut self) -> &mut ErrorData {
        match self {
            Self::ParseError { data }
            | Self::InvalidRequest { data }
            | Self::MethodNotFound { data }
            | Self::InvalidParams { data }
            | Self::DuplicateRequestId { data }
            | Self::InvalidResponse { data }
            | Self::Domain { data } => data,
        }
    }
}

impl From<&BoundaryError> for JsonRpcCode {
    fn from(error: &BoundaryError) -> Self {
        error.json_rpc_code()
    }
}
