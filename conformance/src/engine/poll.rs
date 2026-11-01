//! `poll_reply`: resend one read until its reply satisfies the expectation.
//!
//! For server state that settles after an event the runner cannot subscribe to, such as
//! a lease revoked once the server noticed a closed socket; the attempts are bounded.

use serde_json::Value;

use super::conn::Wait;
use super::exchange::RECORD_BOUND_MS;
use super::session::Session;
use crate::error::RunnerError;
use crate::scenario::matcher::{Assertion, Subject};
use crate::scenario::model::{PollReply, Split};

impl Session<'_> {
    /// Sends `poll.request` with a fresh id per attempt until its reply matches; only the
    /// last attempt's assertions are recorded, with whether the state settled.
    pub(crate) async fn poll_reply(&mut self, poll: &PollReply) -> Result<(), RunnerError> {
        let expect = poll.expect.resolved(&mut self.labels);
        let mut last = Vec::new();
        for attempt in 1..=poll.attempts {
            let mut request = poll.request.clone();
            let id = format!("{}-{attempt}", poll.id);
            if let Value::Object(fields) = &mut request {
                fields.insert("id".to_owned(), Value::String(id.clone()));
            }
            let conn = self.conn(&poll.conn)?;
            let bytes = conn.encode(&request)?;
            conn.write(&bytes, &Split::Whole).await?;
            let reply = self.reply_to(&poll.conn, &id).await?;
            let scope = format!("poll[{}]", poll.id);
            last = expect.check(
                Subject {
                    scope: &scope,
                    value: &reply,
                },
                self.ctx.repo.root(),
            );
            if last.iter().all(|assertion| assertion.passed) {
                break;
            }
        }
        last.push(Assertion::equal(
            format!("poll[{}]/settled", poll.id),
            Value::Bool(true),
            Value::Bool(last.iter().all(|assertion| assertion.passed)),
        ));
        self.assert(last)
    }

    /// The reply to `id`; notifications read meanwhile are kept for later steps.
    async fn reply_to(
        &mut self,
        conn: &crate::ids::ConnName,
        id: &str,
    ) -> Result<Value, RunnerError> {
        let wait = Wait::within(RECORD_BOUND_MS, "a polled reply");
        let mut notifications = Vec::new();
        let reply = loop {
            let record = self
                .conn(conn)?
                .next(wait)
                .await?
                .ok_or(RunnerError::Closed {
                    waiting_for: "a polled reply",
                })?;
            match record.get("id").and_then(Value::as_str) {
                Some(answered) if answered == id => break record,
                Some(_) => {
                    return Err(RunnerError::Unexpected {
                        waiting_for: "the polled reply",
                    });
                }
                None => notifications.push(record),
            }
        };
        let open = self.conn(conn)?;
        for record in notifications {
            open.stash(record);
        }
        Ok(reply)
    }
}
