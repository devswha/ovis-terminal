//! The runner's replay transport: each fixture connection label is one connection
//! through the binary's `connect` bridge, greeted with the owner credential.

use std::collections::{BTreeMap, VecDeque};

use ovis_terminal_replay::{ConnLabel, Transport};
use serde_json::{Value, json};

use super::super::conn::Wait;
use super::super::exchange::RECORD_BOUND_MS;
use super::super::session::Session;
use crate::error::RunnerError;
use crate::ids::ConnName;
use crate::scenario::matcher::Expect;
use crate::scenario::model::{Connect, Hello, Principal, Split, Via};

/// Connections of one replay case, by fixture label.
pub(super) struct Wire<'s, 'a> {
    session: &'s mut Session<'a>,
    case: String,
    /// Notifications that arrived while a reply was awaited, per connection.
    held: BTreeMap<ConnName, VecDeque<Value>>,
}

impl<'s, 'a> Wire<'s, 'a> {
    pub(super) fn new(session: &'s mut Session<'a>, case: &str) -> Self {
        Self {
            session,
            case: case.to_owned(),
            held: BTreeMap::new(),
        }
    }

    /// The connections this case opened, for teardown.
    pub(super) fn opened(self) -> Vec<ConnName> {
        self.held.into_keys().collect()
    }

    async fn conn(&mut self, label: &ConnLabel) -> Result<ConnName, RunnerError> {
        let name: ConnName =
            serde_json::from_value(json!(format!("{}.{}", self.case, label.as_str()))).map_err(
                |source| RunnerError::ScenarioInvalid {
                    path: std::path::PathBuf::from(label.as_str()),
                    field: "conn".to_owned(),
                    source,
                },
            )?;
        if !self.held.contains_key(&name) {
            let connect = Connect {
                conn: name.clone(),
                via: Via::Bridge,
                principal: Principal::Owner,
                bearer: None,
            };
            self.session.connect(&connect).await?;
            let hello = Hello {
                conn: name.clone(),
                framed: false,
                expect: Expect::default(),
            };
            self.session.hello(&hello).await?;
            self.held.insert(name.clone(), VecDeque::new());
        }
        Ok(name)
    }

    async fn next(
        &mut self,
        name: &ConnName,
        waiting_for: &'static str,
    ) -> Result<Value, RunnerError> {
        self.session
            .conn(name)?
            .next(Wait::within(RECORD_BOUND_MS, waiting_for))
            .await?
            .ok_or(RunnerError::Closed { waiting_for })
    }

    fn hold(&mut self, name: &ConnName, record: Value) {
        self.held.entry(name.clone()).or_default().push_back(record);
    }
}

impl Transport for Wire<'_, '_> {
    type Error = RunnerError;

    async fn call(&mut self, conn: &ConnLabel, request: Value) -> Result<Value, RunnerError> {
        let name = self.conn(conn).await?;
        let id = request.get("id").cloned();
        let connection = self.session.conn(&name)?;
        let bytes = connection.encode(&request)?;
        connection.write(&bytes, &Split::Whole).await?;
        loop {
            let record = self.next(&name, "a replay reply").await?;
            if record.get("id").is_some() && record.get("id") == id.as_ref() {
                return Ok(record);
            }
            if record.get("id").is_some() {
                return Err(RunnerError::Unexpected {
                    waiting_for: "the reply of the replayed request",
                });
            }
            self.hold(&name, record);
        }
    }

    async fn event(&mut self, conn: &ConnLabel, method: &str) -> Result<Value, RunnerError> {
        let name = self.conn(conn).await?;
        let named = |record: &Value| record.get("method").and_then(Value::as_str) == Some(method);
        if let Some(queue) = self.held.get_mut(&name)
            && let Some(at) = queue.iter().position(named)
        {
            queue.drain(..at);
            return queue.pop_front().ok_or(RunnerError::Closed {
                waiting_for: "a held notification",
            });
        }
        self.held.entry(name.clone()).or_default().clear();
        loop {
            let record = self.next(&name, "a replay notification").await?;
            if record.get("id").is_none() && named(&record) {
                return Ok(record);
            }
        }
    }
}
