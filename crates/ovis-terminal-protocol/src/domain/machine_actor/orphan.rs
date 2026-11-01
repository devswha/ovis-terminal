//! Orphan notices between machines: `$/orphaned` delivers one, `$/orphanedAck` settles it.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::agent_tree::OrphanNotice;
use crate::schema::ids::MachineId;
use crate::schema::scalar::OpenEnum;

/// Request a parent's machine sends to a machine holding its children.
pub const ORPHANED: &str = "$/orphaned";
/// Request that settles a pending orphan notice by its notification id.
pub const ORPHANED_ACK: &str = "$/orphanedAck";

const TOKEN_MAX: usize = 512;

/// Text that is not 1 to 512 visible ASCII characters.
#[derive(Debug, Error)]
#[error("expected 1 to 512 visible ASCII characters")]
#[non_exhaustive]
pub struct TokenError;

impl TokenError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        "invalid_params"
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        "echo the token exactly as the notice carried it"
    }
}

fn checked(text: String) -> Result<String, TokenError> {
    let visible = text.bytes().all(|byte| byte.is_ascii_graphic());
    if text.is_empty() || text.len() > TOKEN_MAX || !visible {
        return Err(TokenError);
    }
    Ok(text)
}

/// Declares an opaque token minted by the sender and echoed unchanged by the receiver.
macro_rules! token {
    ($(#[$meta:meta])* $name:ident, $description:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// The token text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = TokenError;

            fn try_from(text: String) -> Result<Self, Self::Error> {
                checked(text).map(Self)
            }
        }

        impl From<$name> for String {
            fn from(token: $name) -> Self {
                token.0
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed(stringify!($name))
            }

            fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "type": "string",
                    "pattern": "^[!-~]{1,512}$",
                    "description": $description,
                })
            }
        }
    };
}

token!(
    /// Opaque name of one finished parent program incarnation on its machine.
    ParentIncarnation,
    "Opaque token of one parent program incarnation, echoed unchanged."
);
token!(
    /// Opaque id of one orphan notice to one machine; replays carry the same id.
    NotificationId,
    "Opaque id of one orphan notice to one machine, echoed unchanged."
);

impl ParentIncarnation {
    /// The token of the parent incarnation `notice` reports finished.
    #[must_use]
    pub fn of(notice: &OrphanNotice) -> Self {
        Self(format!(
            "{}/{}/{}",
            notice.parent_machine.as_str(),
            notice.parent.terminal_id.as_str(),
            notice.parent.incarnation.get()
        ))
    }
}

impl NotificationId {
    /// The id of `notice` sent to `target`: the same notice to the same machine always
    /// has the same id, so its replay is recognized.
    #[must_use]
    pub fn of(target: &MachineId, notice: &OrphanNotice) -> Self {
        Self(format!(
            "{}>{}",
            ParentIncarnation::of(notice).0,
            target.as_str()
        ))
    }
}

/// Parameters of `$/orphaned`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct OrphanedParams {
    /// Id of this notice to the receiving machine.
    pub notification_id: NotificationId,
    /// The finished parent incarnation.
    pub parent_incarnation: ParentIncarnation,
    /// The parent's death.
    pub notice: OrphanNotice,
}

impl OrphanedParams {
    /// `notice` addressed to `target`.
    #[must_use]
    pub fn to(target: &MachineId, notice: OrphanNotice) -> Self {
        Self {
            notification_id: NotificationId::of(target, &notice),
            parent_incarnation: ParentIncarnation::of(&notice),
            notice,
        }
    }
}

/// How a receiving machine took an orphan notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Delivery {
    /// Applied now: children of that exact incarnation were ordered to end.
    Applied,
    /// Already applied; nothing was ended or started again.
    Duplicate,
}

/// Result of `$/orphaned`; it acknowledges the notice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct OrphanedResult {
    /// The notice acknowledged.
    pub notification_id: NotificationId,
    /// Its parent incarnation, echoed.
    pub parent_incarnation: ParentIncarnation,
    /// Whether this delivery applied it.
    pub delivery: OpenEnum<Delivery>,
}

impl OrphanedResult {
    /// The acknowledgement of `params`, taken as `delivery`.
    #[must_use]
    pub fn acknowledging(params: &OrphanedParams, delivery: Delivery) -> Self {
        Self {
            notification_id: params.notification_id.clone(),
            parent_incarnation: params.parent_incarnation.clone(),
            delivery: OpenEnum::Known(delivery),
        }
    }
}

/// Parameters of `$/orphanedAck`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct OrphanedAckParams {
    /// The notice acknowledged.
    pub notification_id: NotificationId,
    /// Its parent incarnation, as the notice carried it.
    pub parent_incarnation: ParentIncarnation,
}

impl From<OrphanedResult> for OrphanedAckParams {
    fn from(result: OrphanedResult) -> Self {
        Self {
            notification_id: result.notification_id,
            parent_incarnation: result.parent_incarnation,
        }
    }
}

/// What an acknowledgement settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AckOutcome {
    /// The pending notice was settled and leaves the outbox.
    Acknowledged,
    /// The notice was already settled; nothing changed.
    Duplicate,
}

/// Result of `$/orphanedAck`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct OrphanedAckResult {
    /// The notice.
    pub notification_id: NotificationId,
    /// What the acknowledgement settled.
    pub outcome: OpenEnum<AckOutcome>,
}

impl OrphanedAckResult {
    /// `notification_id` settled as `outcome`.
    #[must_use]
    pub const fn new(notification_id: NotificationId, outcome: AckOutcome) -> Self {
        Self {
            notification_id,
            outcome: OpenEnum::Known(outcome),
        }
    }
}
