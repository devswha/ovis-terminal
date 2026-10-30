//! `service.*`: explicit OS service registration, separate from inspection.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::OpId;
use crate::schema::common::NoParams;
use crate::schema::ids::{ServerBootId, ServiceLabel};
use crate::schema::limits::TransportKind;
use crate::schema::method::{MethodSpec, method};
use crate::schema::scalar::OpenEnum;

/// Service manager that owns the registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ServiceManager {
    /// A systemd user service.
    SystemdUser,
    /// A launchd agent.
    Launchd,
    /// A Windows service.
    WindowsService,
}

/// Endpoint descriptor; it names an address and boot proof but is not liveness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct EndpointDescriptor {
    /// Transport.
    pub transport: OpenEnum<TransportKind>,
    /// Address.
    pub address: String,
    /// Boot identity recorded when the endpoint was bound.
    pub server_boot_id: Option<ServerBootId>,
}

/// One service registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ServiceRecord {
    /// Registration label.
    pub label: ServiceLabel,
    /// Service manager.
    pub manager: OpenEnum<ServiceManager>,
    /// Endpoint the service binds.
    pub endpoint: EndpointDescriptor,
    /// Prerequisites still missing, such as user lingering.
    pub missing_prerequisites: Vec<String>,
}

/// Parameters of `service.install`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ServiceInstallParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Registration label.
    pub label: ServiceLabel,
}

/// Parameters of `service.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ServiceRemoveParams {
    /// Client-minted operation identity.
    pub op_id: OpId,
    /// Registration label.
    pub label: ServiceLabel,
}

/// Result of `service.describe`; describing never creates a service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ServiceDescribeResult {
    /// Registrations.
    pub services: Vec<ServiceRecord>,
}

/// Methods of this namespace.
pub(crate) const METHODS: &[MethodSpec] = &[
    method! {"service.install", Mutation, Untargeted, ServiceInstallParams => ServiceRecord,
    "Register the runtime with the OS service manager.", [ServiceUnsupported, ServiceExists]},
    method! {"service.remove", Mutation, Lookup, ServiceRemoveParams => ServiceRecord,
    "Remove a registration this runtime created.", [ServiceUnsupported]},
    method! {"service.describe", Query, Untargeted, NoParams => ServiceDescribeResult,
    "Describe registrations and endpoint descriptors without side effects.", []},
];
