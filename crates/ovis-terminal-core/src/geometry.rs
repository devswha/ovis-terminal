//! Geometry transitions and unchanged-grid repaint (plan todo 26).
//!
//! A resize is admitted only under the geometry lease ([`crate::leases::Authorization`]
//! from `authorize_resize`); there is no anonymous resize or focus frame. One transition
//! runs at a time and every step is ordered:
//!
//! 1. `geometry.changed` is committed before the holder hears of the new grid, so it
//!    precedes every byte the program writes at that grid ([`Dispatch::into_commit`]).
//! 2. Only the commit receipt releases the holder message; the VT applies the grid first
//!    ([`GeometryController::apply`]).
//! 3. When the holder acknowledges delivery, the runtime records the checkpoint at the
//!    new grid and revision and itself asserts `geometry.input_ready`
//!    ([`GeometryController::acknowledged`]). No program report and no output silence
//!    is involved.
//!
//! Requests arriving during a transition coalesce: the newest waits, every older waiting
//! one resolves `superseded`. Repaint returns a paint at the unchanged grid without any
//! holder message, so the program sees no resize and no injected keys.

mod controller;
mod dispatch;
mod epoch;
mod error;
mod event;
mod outcome;
mod settle;
mod view;

pub use controller::GeometryController;
pub use dispatch::{Committing, Dispatch};
pub use epoch::GeometryEpoch;
pub use error::GeometryError;
pub use event::GeometryEvent;
pub use outcome::{Fence, Outcome, Requested, Resolution, Resolved, Settled, Superseded};
pub use view::{Focused, Repaint};
