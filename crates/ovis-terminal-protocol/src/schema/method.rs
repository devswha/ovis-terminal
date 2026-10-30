//! Method and event descriptors that tie wire names to their schema types.

use schemars::{Schema, SchemaGenerator};

use super::error_kinds::DomainErrorKind;
use crate::errors::ErrorKind;

/// Builds the schema of one params, result or event data type.
pub type SchemaFn = fn(&mut SchemaGenerator) -> Schema;

/// Whether a method only reads state or may change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Effect {
    /// Reads state; never injects input, starts a daemon or changes a viewport.
    Query,
    /// Changes state; takes a client-minted `op_id` for idempotent retries.
    Mutation,
    /// Changes only this connection's streams or subscriptions; nothing durable.
    Control,
}

/// How a method names the resource it acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Targeting {
    /// The method names no single resource.
    Untargeted,
    /// The method looks a resource up by its stable identity.
    Lookup,
    /// The method resolves an explicit terminal target.
    Terminal,
}

/// One JSON-RPC method of the full v1 catalog.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct MethodSpec {
    /// Wire method name.
    pub name: &'static str,
    /// One-line description published in the schema.
    pub summary: &'static str,
    /// Read or write.
    pub effect: Effect,
    /// How the method names its resource.
    pub targeting: Targeting,
    /// Method-specific error kinds beyond the effect and targeting sets.
    pub errors: &'static [DomainErrorKind],
    /// Reference to the closed params object.
    pub params: SchemaFn,
    /// Reference to the result object.
    pub result: SchemaFn,
}

/// One event type delivered in the event envelope.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct EventSpec {
    /// Wire event type.
    pub name: &'static str,
    /// One-line description published in the schema.
    pub summary: &'static str,
    /// Event data object.
    pub data: SchemaFn,
}

/// A kind a method may return, from either the envelope or the domain set.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MethodErrorKind {
    /// Kind produced by request parsing and dispatch.
    Envelope(ErrorKind),
    /// Kind produced by the method's domain.
    Domain(DomainErrorKind),
}

const COMMON_DOMAIN: &[DomainErrorKind] = &[
    DomainErrorKind::Unauthenticated,
    DomainErrorKind::Forbidden,
    DomainErrorKind::DeadlineExceeded,
    DomainErrorKind::Cancelled,
    DomainErrorKind::Backpressure,
];

const MUTATION_DOMAIN: &[DomainErrorKind] = &[
    DomainErrorKind::OpExpired,
    DomainErrorKind::OpReused,
    DomainErrorKind::PersistenceFailed,
    DomainErrorKind::OutcomeUnknown,
];

const TARGET_DOMAIN: &[DomainErrorKind] = &[
    DomainErrorKind::AmbiguousTarget,
    DomainErrorKind::StaleTarget,
    DomainErrorKind::MachineUnreachable,
];

impl MethodSpec {
    /// Lists every error kind this method may return, without duplicates.
    #[must_use]
    pub fn error_kinds(&self) -> Vec<MethodErrorKind> {
        let mut kinds = vec![
            MethodErrorKind::Envelope(ErrorKind::InvalidParams),
            MethodErrorKind::Envelope(ErrorKind::Unavailable),
            MethodErrorKind::Envelope(ErrorKind::Internal),
        ];
        let lookup = match self.targeting {
            Targeting::Untargeted => &[][..],
            Targeting::Lookup => &[MethodErrorKind::Envelope(ErrorKind::NotFound)][..],
            Targeting::Terminal => {
                kinds.extend(TARGET_DOMAIN.iter().copied().map(MethodErrorKind::Domain));
                &[MethodErrorKind::Envelope(ErrorKind::NotFound)][..]
            }
        };
        kinds.extend_from_slice(lookup);
        kinds.extend(COMMON_DOMAIN.iter().copied().map(MethodErrorKind::Domain));
        let mutation = match self.effect {
            Effect::Query | Effect::Control => &[][..],
            Effect::Mutation => MUTATION_DOMAIN,
        };
        let domain = mutation.iter().chain(self.errors).copied();
        for kind in domain.map(MethodErrorKind::Domain) {
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        kinds
    }
}

/// Declares a [`MethodSpec`] whose params and result are named schema types.
macro_rules! method {
    ($name:literal, $effect:ident, $targeting:ident, $params:ty => $result:ty,
     $summary:literal, [$($kind:ident),* $(,)?]) => {
        $crate::schema::method::MethodSpec {
            name: $name,
            summary: $summary,
            effect: $crate::schema::method::Effect::$effect,
            targeting: $crate::schema::method::Targeting::$targeting,
            errors: &[$($crate::schema::error_kinds::DomainErrorKind::$kind),*],
            params: ::schemars::SchemaGenerator::subschema_for::<$params>,
            result: ::schemars::SchemaGenerator::subschema_for::<$result>,
        }
    };
}

/// Declares an [`EventSpec`] whose data is a named schema type.
macro_rules! event {
    ($name:literal, $summary:literal, $data:ty) => {
        $crate::schema::method::EventSpec {
            name: $name,
            summary: $summary,
            data: ::schemars::SchemaGenerator::subschema_for::<$data>,
        }
    };
}

pub(crate) use event;
pub(crate) use method;
