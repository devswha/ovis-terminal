//! `retention.floor_advanced`: the durable announcement of an eviction.

use ovis_terminal_protocol::domain::documents_retention::{
    RetentionClass, RetentionPosition, RetentionQuantity,
};
use serde::{Deserialize, Serialize};

use crate::events::EventData;

/// A class's floor advanced; positions below `floor` now read as expired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct FloorAdvanced {
    /// Class whose floor moved.
    pub class: RetentionClass,
    /// New oldest retained position.
    pub floor: RetentionPosition,
    /// Records tombstoned by this sweep.
    pub evicted: RetentionQuantity,
}

impl FloorAdvanced {
    /// The advance of `class` to `floor`, tombstoning `evicted` records.
    pub const fn new(class: RetentionClass, floor: RetentionPosition, evicted: u64) -> Self {
        Self {
            class,
            floor,
            evicted: RetentionQuantity::new(evicted),
        }
    }
}

impl EventData for FloorAdvanced {
    fn event_type(&self) -> &'static str {
        "retention.floor_advanced"
    }
}
