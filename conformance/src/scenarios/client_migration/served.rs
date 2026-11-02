//! The methods `ovis-terminal serve` lists in `hello`, read over its `connect` bridge in a
//! fresh isolated root.

use std::collections::BTreeSet;

use ovis_terminal_protocol::schema::PROTOCOL_GENERATION;
use serde_json::{Value, json};

use crate::engine::conn::Wait;
use crate::engine::exchange::RECORD_BOUND_MS;
use crate::engine::root::IsolatedRoot;
use crate::engine::session::Session;
use crate::error::RunnerError;
use crate::ids::ConnName;
use crate::scenario::model::{Connect, FixtureKind, Principal, Serve, Split, Via};

impl Session<'_> {
    pub(super) async fn served_methods(&mut self) -> Result<BTreeSet<String>, RunnerError> {
        let previous = std::mem::replace(&mut self.root, IsolatedRoot::create()?);
        let name: ConnName = serde_json::from_value(json!("migration.served"))?;
        let hello = async {
            self.serve(&Serve {
                fixture: FixtureKind::Assembled,
                qa_graphics_sequence_max_bytes: None,
                qa_output_ring_bytes: None,
            })
            .await?;
            self.connect(&Connect {
                conn: name.clone(),
                via: Via::Bridge,
                principal: Principal::Owner,
                bearer: None,
            })
            .await?;
            let conn = self.conn(&name)?;
            let request = json!({
                "jsonrpc": "2.0", "id": "hello", "method": "hello",
                "params": {
                    "generation": PROTOCOL_GENERATION,
                    "client": {"name": "ovis-conformance", "version": env!("CARGO_PKG_VERSION")},
                    "credential": conn.secret(),
                    "framed": false,
                },
            });
            let bytes = conn.encode(&request)?;
            conn.write(&bytes, &Split::Whole).await?;
            let reply = conn
                .next(Wait::within(RECORD_BOUND_MS, "the hello reply"))
                .await?
                .ok_or(RunnerError::Closed {
                    waiting_for: "the hello reply",
                })?;
            conn.hangup().await?;
            Ok::<Value, RunnerError>(reply)
        }
        .await;
        let finished = if self.conns.contains_key(&name) {
            self.finish_conn(&name).await
        } else {
            Ok(())
        };
        let stopped = if self.fixture.is_some() {
            self.stop_fixture().await.map(drop)
        } else {
            Ok(())
        };
        drop(std::mem::replace(&mut self.root, previous));
        let reply = hello?;
        finished?;
        stopped?;
        Ok(reply
            .pointer("/result/methods")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect())
    }
}
