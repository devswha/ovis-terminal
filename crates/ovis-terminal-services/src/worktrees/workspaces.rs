//! The workspace side of a checkout: opening and closing workspaces linked to it.

use std::fmt::Debug;
use std::path::Path;

use ovis_terminal_core::registry::WorkspaceId;
use ovis_terminal_protocol::schema::ids::CheckoutId;

/// Why the workspace service could not open or close a workspace.
pub type WorkspaceFailure = Box<dyn std::error::Error + Send + Sync>;

/// Opens workspaces on checkouts and closes them; assembly supplies the layout owner.
pub trait Workspaces: Send + Sync + Debug {
    /// Opens one workspace rooted at `path` for `checkout`.
    ///
    /// # Errors
    ///
    /// Returns the owner's failure; nothing was opened.
    fn open(&self, checkout: &CheckoutId, path: &Path) -> Result<WorkspaceId, WorkspaceFailure>;

    /// Closes exactly these workspaces.
    ///
    /// # Errors
    ///
    /// Returns the owner's failure; nothing was closed.
    fn close(&self, workspaces: &[WorkspaceId]) -> Result<(), WorkspaceFailure>;
}
