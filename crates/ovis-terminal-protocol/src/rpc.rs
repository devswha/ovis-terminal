//! Correlated JSON-RPC request and response envelopes.

mod counter;

use std::collections::HashSet;

pub use counter::DecimalU64;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::errors::{BoundaryError, ErrorData, JsonRpcCode};

/// String correlation key shared by a request and its terminal response.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(String);

impl RequestId {
    /// Creates a request ID from its wire string.
    #[must_use]
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Borrows the request ID string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Closed JSON-RPC request envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RpcRequest {
    /// JSON-RPC protocol version, which must be `"2.0"`.
    jsonrpc: String,
    /// String ID echoed by exactly one terminal response.
    id: RequestId,
    /// Method name selected by the dispatcher.
    method: String,
    /// Method-specific parameter object.
    params: Value,
}

impl RpcRequest {
    /// Returns this request's correlation ID.
    #[must_use]
    pub const fn id(&self) -> &RequestId {
        &self.id
    }

    /// Returns the validated method name for dispatch.
    #[must_use]
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Returns the method parameter object for validation and decoding.
    #[must_use]
    pub const fn params(&self) -> &Value {
        &self.params
    }

    /// Checks the top-level field set of a method's closed params object.
    ///
    /// # Errors
    ///
    /// Returns [`BoundaryError::InvalidParams`] with the first unknown field path.
    pub fn validate_params(&self, allowed_fields: &[&str]) -> Result<(), BoundaryError> {
        let Some(params) = self.params.as_object() else {
            return Err(BoundaryError::invalid_params(Some("params")));
        };
        if let Some(field) = params
            .keys()
            .find(|field| !allowed_fields.contains(&field.as_str()))
        {
            let path = format!("params.{field}");
            return Err(BoundaryError::invalid_params(Some(&path)));
        }
        if let Some(target) = params.get("target") {
            let Some(target) = target.as_object() else {
                return Err(BoundaryError::invalid_params(Some("params.target")));
            };
            if target.len() != 1 {
                return Err(BoundaryError::invalid_params(Some("params.target")));
            }
            if let Some(field) = target
                .keys()
                .find(|field| !["terminal_id", "address", "name"].contains(&field.as_str()))
            {
                let path = format!("params.target.{field}");
                return Err(BoundaryError::invalid_params(Some(&path)));
            }
            if let Some((field, value)) = target.iter().next()
                && value.as_str().is_none()
            {
                let path = format!("params.target.{field}");
                return Err(BoundaryError::invalid_params(Some(&path)));
            }
        }
        Ok(())
    }
}

/// Parses and validates the fixed JSON-RPC request envelope.
///
/// # Errors
///
/// Returns a typed parse or invalid-request error without forwarding serde prose.
pub fn parse_request(input: &str) -> Result<RpcRequest, BoundaryError> {
    let value: Value = serde_json::from_str(input).map_err(BoundaryError::parse_error)?;
    let Some(object) = value.as_object() else {
        return Err(BoundaryError::invalid_request(Some("$")));
    };
    if let Some(field) = object
        .keys()
        .find(|field| !["jsonrpc", "id", "method", "params"].contains(&field.as_str()))
    {
        let path = field.as_str();
        return Err(BoundaryError::invalid_request(Some(path)));
    }

    let jsonrpc = string_field(object, "jsonrpc")
        .filter(|version| *version == "2.0")
        .ok_or_else(|| BoundaryError::invalid_request(Some("jsonrpc")))?;
    let id =
        string_field(object, "id").ok_or_else(|| BoundaryError::invalid_request(Some("id")))?;
    let method = string_field(object, "method")
        .filter(|method| !method.is_empty())
        .ok_or_else(|| BoundaryError::invalid_request(Some("method")))?;
    let params = object
        .get("params")
        .filter(|params| params.is_object())
        .cloned()
        .ok_or_else(|| BoundaryError::invalid_request(Some("params")))?;

    Ok(RpcRequest {
        jsonrpc: jsonrpc.to_owned(),
        id: RequestId::new(id.to_owned()),
        method: method.to_owned(),
        params,
    })
}

/// Closed successful JSON-RPC response envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RpcSuccess {
    /// JSON-RPC protocol version.
    pub jsonrpc: String,
    /// Correlation key copied from the accepted request.
    pub id: RequestId,
    /// Method result.
    pub result: Value,
}

impl RpcSuccess {
    /// Creates a successful response carrying the request ID.
    #[must_use]
    pub fn new(id: RequestId, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            id,
            result,
        }
    }
}

/// Error data attached to a JSON-RPC failure response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RpcErrorObject {
    /// Standard JSON-RPC code selected from a typed boundary error.
    pub code: i32,
    /// Human-readable summary; clients branch on `data.kind`.
    pub message: String,
    /// Stable machine-readable error metadata.
    pub data: ErrorData,
}

/// Closed failing JSON-RPC response envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RpcFailure {
    /// JSON-RPC protocol version.
    pub jsonrpc: String,
    /// Correlation key copied from the accepted request.
    pub id: RequestId,
    /// Structured terminal error.
    pub error: RpcErrorObject,
}

impl RpcFailure {
    /// Creates an error response from a typed boundary error.
    #[must_use]
    pub fn new(id: RequestId, error: &BoundaryError) -> Self {
        let code = JsonRpcCode::from(error).value();
        let data = error.data().clone();
        Self {
            jsonrpc: "2.0".to_owned(),
            id,
            error: RpcErrorObject {
                code,
                message: error.to_string(),
                data,
            },
        }
    }
}

/// Exactly one terminal result or structured error for an accepted request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum RpcReply {
    /// A successful terminal response.
    Success(RpcSuccess),
    /// A failing terminal response.
    Failure(RpcFailure),
}

impl RpcReply {
    /// Returns the request ID that this terminal response settles.
    #[must_use]
    pub const fn id(&self) -> &RequestId {
        match self {
            Self::Success(success) => &success.id,
            Self::Failure(failure) => &failure.id,
        }
    }
}

/// Parses and validates a response envelope, including branch exclusivity.
///
/// # Errors
///
/// Returns [`BoundaryError::InvalidResponse`] for an envelope with both or neither branch.
pub fn parse_reply(input: &str) -> Result<RpcReply, BoundaryError> {
    let reply: RpcReply =
        serde_json::from_str(input).map_err(BoundaryError::invalid_response_json)?;
    let jsonrpc = match &reply {
        RpcReply::Success(success) => success.jsonrpc.as_str(),
        RpcReply::Failure(failure) => failure.jsonrpc.as_str(),
    };
    if jsonrpc == "2.0" {
        Ok(reply)
    } else {
        Err(BoundaryError::invalid_response())
    }
}

/// Tracks request IDs which have not received a terminal response yet.
#[derive(Debug, Default)]
pub struct InFlightRequests {
    ids: HashSet<RequestId>,
}

impl InFlightRequests {
    /// Marks a request ID active, rejecting duplicate concurrent use.
    ///
    /// # Errors
    ///
    /// Returns [`BoundaryError::DuplicateRequestId`] when the ID is already active.
    pub fn begin(&mut self, id: RequestId) -> Result<(), BoundaryError> {
        if self.ids.insert(id) {
            Ok(())
        } else {
            Err(BoundaryError::duplicate_request_id())
        }
    }

    /// Settles one active request with its matching terminal response.
    ///
    /// # Errors
    ///
    /// Returns [`BoundaryError::ResponseIdMismatch`] if the reply has another ID.
    /// Returns [`BoundaryError::UnknownOrSettledRequestId`] if the ID is not active.
    pub fn complete(&mut self, id: &RequestId, reply: RpcReply) -> Result<RpcReply, BoundaryError> {
        if reply.id() != id {
            return Err(BoundaryError::response_id_mismatch());
        }
        if self.ids.remove(id) {
            Ok(reply)
        } else {
            Err(BoundaryError::unknown_or_settled_request_id())
        }
    }
}

fn string_field<'a>(object: &'a Map<String, Value>, field: &str) -> Option<&'a str> {
    object.get(field)?.as_str()
}
