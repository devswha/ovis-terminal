//! Settling a finished, failed or cancelled reply task into exactly one reply.

use ovis_terminal_protocol::errors::{BoundaryError, ErrorKind};
use ovis_terminal_protocol::rpc::RpcReply;
use tokio::task::{self, JoinError};

use super::{Requests, failure};
use crate::conn::{ConnectionError, RequestHandler};

impl<H: RequestHandler> Requests<H> {
    /// Answers the request a joined task served; a task cancelled by `$/cancelRequest` is
    /// answered `cancelled`, a task that failed otherwise `internal`.
    pub(super) async fn settle(
        &mut self,
        joined: Result<(task::Id, RpcReply), JoinError>,
    ) -> Result<(), ConnectionError> {
        let (task, reply, cancelled) = match joined {
            Ok((task, reply)) => (task, Some(reply), false),
            Err(error) => (error.id(), None, error.is_cancelled()),
        };
        let id = self.ids.remove(&task).ok_or_else(|| {
            ConnectionError::Correlation(BoundaryError::unknown_or_settled_request_id())
        })?;
        self.reply_tasks.remove(&id);
        let reply = reply.unwrap_or_else(|| {
            let kind = if cancelled {
                ErrorKind::from("cancelled".to_owned())
            } else {
                ErrorKind::Internal
            };
            failure(id.clone(), &BoundaryError::domain(kind, false))
        });
        let settled = self
            .in_flight
            .complete(&id, reply)
            .map_err(ConnectionError::Correlation)?;
        self.reply(&settled).await
    }
}
