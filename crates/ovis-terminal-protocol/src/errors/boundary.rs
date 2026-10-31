//! Boundary error variants, stable metadata, and JSON-RPC code conversion.

use thiserror::Error;

use super::{ErrorData, ErrorKind, FieldPath, JsonRpcCode, OpId, TargetId};

/// Error raised while parsing, validating or correlating protocol records.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BoundaryError {
    /// The incoming bytes are not valid JSON.
    #[error("input is not a JSON document")]
    ParseError {
        /// Machine-readable error context.
        data: Box<ErrorData>,
        /// Underlying parser diagnostic, excluded from the wire representation.
        #[source]
        source: serde_json::Error,
    },
    /// The incoming request envelope is invalid.
    #[error("request envelope violates JSON-RPC 2.0")]
    InvalidRequest {
        /// Machine-readable error context.
        data: Box<ErrorData>,
    },
    /// The requested method is not registered.
    #[error("no such JSON-RPC method")]
    MethodNotFound {
        /// Machine-readable error context.
        data: Box<ErrorData>,
    },
    /// The method parameters do not match its closed schema.
    #[error("method parameters violate the closed schema")]
    InvalidParams {
        /// Machine-readable error context.
        data: Box<ErrorData>,
    },
    /// The same request ID is already active on the connection.
    #[error("request id is already in flight on this connection")]
    DuplicateRequestId {
        /// Machine-readable error context.
        data: Box<ErrorData>,
    },
    /// The request is not active on this connection or already settled.
    #[error("request id is unknown or already settled")]
    UnknownOrSettledRequestId {
        /// Machine-readable error context.
        data: Box<ErrorData>,
    },
    /// The reply ID does not match the request being settled.
    #[error("response request id does not match settlement id")]
    ResponseIdMismatch {
        /// Machine-readable error context.
        data: Box<ErrorData>,
    },
    /// The incoming response has neither or both terminal branches.
    #[error("response envelope violates JSON-RPC 2.0")]
    InvalidResponse {
        /// Machine-readable error context.
        data: Box<ErrorData>,
    },
    /// A response JSON document could not be parsed.
    #[error("response is not a JSON document")]
    InvalidResponseJson {
        /// Machine-readable error context.
        data: Box<ErrorData>,
        /// Underlying parser diagnostic, excluded from the wire representation.
        #[source]
        source: serde_json::Error,
    },
    /// A typed domain failure is ready to cross the protocol boundary.
    #[error("operation failed")]
    Domain {
        /// Machine-readable error context.
        data: Box<ErrorData>,
    },
}

impl BoundaryError {
    /// Creates a malformed-JSON failure.
    #[must_use]
    pub fn parse_error(source: serde_json::Error) -> Self {
        Self::ParseError {
            data: Box::new(ErrorData::new(ErrorKind::ParseError, false)),
            source,
        }
    }

    /// Creates an invalid request failure with an optional field path.
    #[must_use]
    pub fn invalid_request(field_path: Option<&str>) -> Self {
        Self::InvalidRequest {
            data: Box::new(
                ErrorData::new(ErrorKind::InvalidRequest, false).with_field_path(field_path),
            ),
        }
    }

    /// Creates an invalid-parameters failure with an optional field path.
    #[must_use]
    pub fn invalid_params(field_path: Option<&str>) -> Self {
        Self::InvalidParams {
            data: Box::new(
                ErrorData::new(ErrorKind::InvalidParams, false).with_field_path(field_path),
            ),
        }
    }

    /// Creates an unknown-method failure identifying the method field.
    #[must_use]
    pub fn method_not_found() -> Self {
        Self::MethodNotFound {
            data: Box::new(
                ErrorData::new(ErrorKind::MethodNotFound, false).with_field_path(Some("method")),
            ),
        }
    }

    /// Creates a duplicate in-flight request ID failure.
    #[must_use]
    pub fn duplicate_request_id() -> Self {
        Self::DuplicateRequestId {
            data: Box::new(
                ErrorData::new(ErrorKind::DuplicateRequestId, false).with_field_path(Some("id")),
            ),
        }
    }

    /// Creates an unknown or previously settled request failure.
    #[must_use]
    pub fn unknown_or_settled_request_id() -> Self {
        Self::UnknownOrSettledRequestId {
            data: Box::new(
                ErrorData::new(ErrorKind::UnknownOrSettledRequestId, false)
                    .with_field_path(Some("id")),
            ),
        }
    }

    /// Creates a mismatched response-ID failure.
    #[must_use]
    pub fn response_id_mismatch() -> Self {
        Self::ResponseIdMismatch {
            data: Box::new(
                ErrorData::new(ErrorKind::ResponseIdMismatch, false).with_field_path(Some("id")),
            ),
        }
    }

    /// Creates a response-envelope validation failure.
    #[must_use]
    pub fn invalid_response() -> Self {
        Self::InvalidResponse {
            data: Box::new(ErrorData::new(ErrorKind::InvalidResponse, false)),
        }
    }

    /// Creates a malformed-response failure while retaining the parser source.
    #[must_use]
    pub fn invalid_response_json(source: serde_json::Error) -> Self {
        Self::InvalidResponseJson {
            data: Box::new(ErrorData::new(ErrorKind::InvalidResponse, false)),
            source,
        }
    }

    /// Creates a typed domain error carrying its kind's published hint.
    #[must_use]
    pub fn domain(kind: ErrorKind, retryable: bool) -> Self {
        Self::Domain {
            data: Box::new(ErrorData::new(kind, retryable)),
        }
    }

    /// Adds a stable target identity.
    #[must_use]
    pub fn with_target(mut self, target: TargetId) -> Self {
        self.data_mut().target = Some(target);
        self
    }

    /// Replaces the kind's published hint with the guidance of the failing boundary.
    #[must_use]
    pub fn with_hint(mut self, hint: &'static str) -> Self {
        hint.clone_into(&mut self.data_mut().hint);
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
    pub fn kind(&self) -> &str {
        self.data().kind.token()
    }

    /// Returns the typed error category.
    #[must_use]
    pub fn error_kind(&self) -> &ErrorKind {
        &self.data().kind
    }

    /// Returns the recovery guidance for this failure.
    #[must_use]
    pub fn hint(&self) -> &'static str {
        self.data().kind.hint()
    }

    /// Returns the JSON-RPC code selected by the typed kind.
    #[must_use]
    pub fn json_rpc_code(&self) -> JsonRpcCode {
        self.data().kind.json_rpc_code()
    }

    /// Returns the offending field path, when known.
    #[must_use]
    pub fn field_path(&self) -> Option<&str> {
        self.data().field_path.as_ref().map(FieldPath::as_str)
    }

    /// Returns the structured error data for a JSON-RPC failure object.
    #[must_use]
    pub fn data(&self) -> &ErrorData {
        match self {
            Self::ParseError { data, .. }
            | Self::InvalidRequest { data }
            | Self::MethodNotFound { data }
            | Self::InvalidParams { data }
            | Self::DuplicateRequestId { data }
            | Self::UnknownOrSettledRequestId { data }
            | Self::ResponseIdMismatch { data }
            | Self::InvalidResponse { data }
            | Self::InvalidResponseJson { data, .. }
            | Self::Domain { data } => data,
        }
    }

    fn data_mut(&mut self) -> &mut ErrorData {
        match self {
            Self::ParseError { data, .. }
            | Self::InvalidRequest { data }
            | Self::MethodNotFound { data }
            | Self::InvalidParams { data }
            | Self::DuplicateRequestId { data }
            | Self::UnknownOrSettledRequestId { data }
            | Self::ResponseIdMismatch { data }
            | Self::InvalidResponse { data }
            | Self::InvalidResponseJson { data, .. }
            | Self::Domain { data } => data,
        }
    }
}

impl From<&BoundaryError> for JsonRpcCode {
    fn from(error: &BoundaryError) -> Self {
        error.json_rpc_code()
    }
}
