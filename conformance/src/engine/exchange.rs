//! Request/reply steps: hello, correlated replies, notifications, end of stream, rejections.

use std::collections::BTreeSet;

use ovis_terminal_protocol::schema::PROTOCOL_GENERATION;
use serde::Deserialize;
use serde_json::{Value, json};

use super::conn::Wait;
use super::session::Session;
use crate::error::RunnerError;
use crate::ids::ConnName;
use crate::scenario::matcher::{Assertion, Subject};
use crate::scenario::model::{Hello, Notification, Rejections, Replies, Split};

/// Default bound on any awaited record.
pub(crate) const RECORD_BOUND_MS: u64 = 20_000;

/// A protocol fixture of requests that must each be rejected with a typed kind.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RejectionFixture {
    format: String,
    #[expect(
        dead_code,
        reason = "human-readable fixture documentation, required by the closed schema"
    )]
    summary: String,
    cases: Vec<RejectionCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RejectionCase {
    name: String,
    method: String,
    params: Value,
    kind: String,
    #[serde(default)]
    field_path: Option<String>,
}

impl Session<'_> {
    pub(crate) async fn hello(&mut self, hello: &Hello) -> Result<(), RunnerError> {
        let bearer = self.bearer().ok_or(RunnerError::FixtureState)?.to_owned();
        let request = json!({
            "jsonrpc": "2.0", "id": "hello", "method": "hello",
            "params": {
                "generation": PROTOCOL_GENERATION,
                "client": {"name": "ovis-conformance", "version": env!("CARGO_PKG_VERSION")},
                "credential": bearer,
                "framed": hello.framed,
            },
        });
        let conn = self.conn(&hello.conn)?;
        let bytes = conn.encode(&request)?;
        conn.write(&bytes, &Split::Whole).await?;
        let reply = conn
            .next(Wait::within(RECORD_BOUND_MS, "the hello reply"))
            .await?
            .ok_or(RunnerError::Closed {
                waiting_for: "the hello reply",
            })?;
        if hello.framed && reply.get("result").is_some() {
            conn.upgrade()?;
        }
        let expect = hello.expect.resolved(&mut self.labels);
        let checked = expect.check(
            Subject {
                scope: "hello",
                value: &reply,
            },
            self.ctx.repo.root(),
        );
        self.assert(checked)
    }

    /// Reads until every listed id is answered once; a foreign or repeated id fails.
    pub(crate) async fn replies(&mut self, replies: &Replies) -> Result<(), RunnerError> {
        let bound = replies.within_ms.unwrap_or(RECORD_BOUND_MS);
        let wait = Wait::within(bound, "correlated replies");
        let mut pending: BTreeSet<&String> = replies.expect.keys().collect();
        let mut arrivals = Vec::new();
        let mut notifications = Vec::new();
        let mut checked = Vec::new();
        while !pending.is_empty() {
            let record =
                self.conn(&replies.conn)?
                    .next(wait)
                    .await?
                    .ok_or(RunnerError::Closed {
                        waiting_for: "correlated replies",
                    })?;
            let Some(id) = record.get("id").and_then(Value::as_str).map(str::to_owned) else {
                notifications.push(record);
                continue;
            };
            let Some(expect) = replies.expect.get(&id).filter(|_| pending.remove(&id)) else {
                return Err(RunnerError::Unexpected {
                    waiting_for: "a reply with a pending id",
                });
            };
            let expect = expect.resolved(&mut self.labels);
            let scope = format!("reply[{id}]");
            checked.extend(expect.check(
                Subject {
                    scope: &scope,
                    value: &record,
                },
                self.ctx.repo.root(),
            ));
            arrivals.push(Value::String(id));
        }
        let conn = self.conn(&replies.conn)?;
        for record in notifications {
            conn.stash(record);
        }
        let answered: Vec<Value> = replies.expect.keys().cloned().map(Value::String).collect();
        arrivals.sort_by_key(Value::to_string);
        checked.push(Assertion::equal(
            "replies/exactly_one_per_id".to_owned(),
            Value::Array(answered),
            Value::Array(arrivals),
        ));
        self.assert(checked)
    }

    /// Reads until the named notification; earlier records are kept, in order, for later steps.
    pub(crate) async fn notification(
        &mut self,
        notification: &Notification,
    ) -> Result<(), RunnerError> {
        let wait = Wait::within(RECORD_BOUND_MS, "a notification");
        let mut earlier = Vec::new();
        let found =
            loop {
                let record = self.conn(&notification.conn)?.next(wait).await?.ok_or(
                    RunnerError::Closed {
                        waiting_for: "a notification",
                    },
                )?;
                let named = record.get("method").and_then(Value::as_str)
                    == Some(notification.method.as_str());
                if named && record.get("id").is_none() {
                    break record;
                }
                earlier.push(record);
            };
        let conn = self.conn(&notification.conn)?;
        for record in earlier {
            conn.stash(record);
        }
        let expect = notification.expect.resolved(&mut self.labels);
        let scope = format!("notification[{}]", notification.method);
        let checked = expect.check(
            Subject {
                scope: &scope,
                value: &found,
            },
            self.ctx.repo.root(),
        );
        self.assert(checked)
    }

    /// The peer ends the stream with nothing further; then the byte accounting is asserted.
    pub(crate) async fn closed(&mut self, conn: &ConnName) -> Result<(), RunnerError> {
        let wait = Wait::within(RECORD_BOUND_MS, "end of stream");
        if self.conn(conn)?.next(wait).await?.is_some() {
            return Err(RunnerError::Unexpected {
                waiting_for: "end of stream",
            });
        }
        self.finish_conn(conn).await
    }

    /// Sends each fixture request in turn and checks its kind and offending field path.
    pub(crate) async fn rejections(&mut self, rejections: &Rejections) -> Result<(), RunnerError> {
        let relative = format!("fixtures/protocol/{}", rejections.fixture);
        let fixture: RejectionFixture = serde_json::from_value(self.ctx.repo.json(&relative)?)
            .map_err(|source| RunnerError::ScenarioInvalid {
                path: self.ctx.repo.conformance(&relative),
                field: ".".to_owned(),
                source,
            })?;
        if fixture.format != "ovis-protocol-fixture/1" {
            return Err(RunnerError::ScenarioNotExecutable { scenario: relative });
        }
        for case in fixture.cases {
            let params = self.labels.resolve(&case.params);
            let request =
                json!({"jsonrpc": "2.0", "id": case.name, "method": case.method, "params": params});
            let conn = self.conn(&rejections.conn)?;
            let bytes = conn.encode(&request)?;
            conn.write(&bytes, &Split::Whole).await?;
            let reply = conn
                .next(Wait::within(RECORD_BOUND_MS, "a rejection"))
                .await?
                .ok_or(RunnerError::Closed {
                    waiting_for: "a rejection",
                })?;
            let scope = format!("rejection[{}]", case.name);
            let mut checked = vec![
                Assertion::equal(
                    format!("{scope}/id"),
                    json!(case.name),
                    reply.get("id").cloned().unwrap_or(Value::Null),
                ),
                Assertion::equal(
                    format!("{scope}/error/data/kind"),
                    json!(case.kind),
                    reply
                        .pointer("/error/data/kind")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                Assertion::equal(
                    format!("{scope}/result"),
                    Value::Null,
                    reply.get("result").cloned().unwrap_or(Value::Null),
                ),
            ];
            if let Some(path) = &case.field_path {
                checked.push(Assertion::equal(
                    format!("{scope}/error/data/field_path"),
                    json!(path),
                    reply
                        .pointer("/error/data/field_path")
                        .cloned()
                        .unwrap_or(Value::Null),
                ));
            }
            self.assert(checked)?;
        }
        Ok(())
    }
}
