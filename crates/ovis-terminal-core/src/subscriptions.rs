//! Independent subscriptions, replay and recoverable loss (plan todo 22).
//!
//! A [`Subscriptions`] hub owns the only coordinator delivery and copies every committed
//! event into the matching subscriptions' own bounded queues. Each subscription has its
//! own identity, replaceable [`Selector`], scan cursor and heartbeat, so subscribing,
//! updating, stalling or closing one never touches another. Loss is committed as a
//! [`LossRecord`] in the durable event log before the consumer is told or closed.

mod consume;
mod error;
mod fanout;
mod feed;
mod hub;
mod limits;
mod loss;
mod pump;
mod selector;
mod shared;
mod start;
mod table;

pub use error::SubscriptionError;
pub use feed::{Batch, Closed, Feed, Heartbeat, Pulled, Recovery};
pub use hub::Subscriptions;
pub use limits::{MAX_HEARTBEAT, QueueLimits};
pub use loss::{LossCause, LossRecord};
pub use selector::Selector;
pub use start::{Request, Start, Started};
