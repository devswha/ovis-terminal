//! Holder output connected to revisioned screen and history (plan todo 17).
//!
//! [`Screen`] feeds one terminal's ordered holder bytes into the VT engine in
//! bounded slices, keeps a bounded raw ring with a visible floor, takes
//! checkpoint boundaries and queues terminal-query replies on an internal lane.
//! Screen, history, search and selection reads answer from immutable snapshots
//! pinned by content revision, so pages stay coherent while output continues;
//! reads never write to the program, scroll a viewport or touch viewers.

mod config;
mod error;
mod frozen;
mod pins;
mod pipeline;
mod reads;
mod replies;
mod requests;
mod ring;
mod views;

pub use config::{ScreenBounds, ScreenConfig};
pub use error::ScreenError;
pub use pipeline::Screen;
pub use replies::{QueryReply, ReplyBatch};
pub use requests::{
    HistoryRequest, Needle, OutputRequest, PageCursor, SearchRequest, SelectionRequest,
    decode_request,
};
pub use views::{
    CheckpointMark, HistoryPage, HistoryState, Ingested, LiveState, OutputSlice, ReadMeta,
    Recovery, RetainedRange, ScreenTransition, ScreenView, SearchHit, SearchResult, SelectionText,
    Unavailability, UnavailableReason,
};
