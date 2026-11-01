//! Executing one step; every check becomes an assertion in the evidence.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use serde_json::{Value, json};

use super::conn::Connection;
use super::process::{self, document};
use super::session::Session;
use super::transport;
use crate::error::RunnerError;
use crate::evidence::{ConnectionReport, DescriptorRecord};
use crate::fixture::wire::decode;
use crate::scenario::matcher::{Assertion, Subject};
use crate::scenario::model::{
    Connect, Effects, ExpectFailure, Principal, Run, Send as SendStep, SendWire, Step, Via,
};

mod serve;

type StepFuture<'s> = Pin<Box<dyn Future<Output = Result<(), RunnerError>> + Send + 's>>;

impl Session<'_> {
    /// Runs one step; boxed so that `expect_failure` can run its inner step.
    pub(crate) fn execute<'s>(&'s mut self, step: &'s Step) -> StepFuture<'s> {
        Box::pin(async move {
            match step {
                Step::Serve(serve) => self.serve(serve).await,
                Step::Connect(connect) => self.connect(connect).await,
                Step::Hello(hello) => self.hello(hello).await,
                Step::Send(send) => self.send(send).await,
                Step::SendWire(wire) => self.send_wire(wire).await,
                Step::Replies(replies) => self.replies(replies).await,
                Step::PollReply(poll) => self.poll_reply(poll).await,
                Step::Notification(notification) => self.notification(notification).await,
                Step::Closed(closed) => self.closed(&closed.conn).await,
                Step::Hangup(hangup) => self.conn(&hangup.conn)?.hangup().await,
                Step::Rejections(rejections) => self.rejections(rejections).await,
                Step::Run(run) => self.run_binary(run).await,
                Step::Commands(commands) => self.commands(commands).await,
                Step::Effects(effects) => self.effects(effects),
                Step::Pages(pages) => self.pages(pages).await,
                Step::Stop(stopping) => {
                    let report = self.stop_fixture().await?;
                    let subject = Subject {
                        scope: "fixture",
                        value: &report,
                    };
                    let checked = stopping.expect.check(subject, self.ctx.repo.root());
                    self.assert(checked)
                }
                Step::Announced(announced) => self.announced(announced).await,
                Step::Include(include) => Err(RunnerError::ScenarioInconsistent {
                    scenario: include.scenario.to_string(),
                    detail: "an include survived planning",
                }),
                Step::ExpectFailure(expected) => self.expect_failure(expected).await,
                Step::Inject(inject) => self.inject(inject).await,
                Step::PtyOpen(_)
                | Step::PtyType(_)
                | Step::PtyExpect(_)
                | Step::PtyAbsent(_)
                | Step::PtyResize(_)
                | Step::PtyExit(_) => self.pty(step).await,
                Step::Gates(gates) => self.gates(gates).await,
                Step::GraphicsTranscript(step) => self.graphics_transcript(step).await,
            }
        })
    }

    async fn connect(&mut self, connect: &Connect) -> Result<(), RunnerError> {
        let ready = &self
            .fixture
            .as_ref()
            .ok_or(RunnerError::FixtureState)?
            .ready;
        let bearer =
            match (&connect.bearer, connect.principal) {
                (Some(name), _) => self.kept.get(name).cloned().ok_or_else(|| {
                    RunnerError::ScenarioInconsistent {
                        scenario: name.clone(),
                        detail: "a connection presents a bearer no reply kept",
                    }
                })?,
                (None, Principal::Owner) => ready.bearer.clone(),
                (None, Principal::Observe) => ready.observer.clone(),
                (None, Principal::Writer) => ready.writer.clone(),
            };
        if self.conns.contains_key(&connect.conn) {
            return Err(RunnerError::ScenarioInconsistent {
                scenario: connect.conn.as_str().to_owned(),
                detail: "a connection name is opened twice",
            });
        }
        let opened = transport::open(connect.via, &self.endpoint()).await?;
        let report = ConnectionReport {
            name: connect.conn.as_str().to_owned(),
            via: match connect.via {
                Via::Bridge => "bridge",
                Via::Socket => "socket",
            }
            .to_owned(),
            ..ConnectionReport::default()
        };
        self.conns.insert(
            connect.conn.clone(),
            Connection::new(opened, report, bearer),
        );
        Ok(())
    }

    async fn send(&mut self, send: &SendStep) -> Result<(), RunnerError> {
        let requests: Vec<Value> = send
            .requests
            .iter()
            .map(|request| self.labels.resolve(request))
            .collect();
        let conn = self.conn(&send.conn)?;
        let mut bytes = Vec::new();
        for request in &requests {
            bytes.extend(conn.encode(request)?);
        }
        conn.write(&bytes, &send.split).await
    }

    async fn send_wire(&mut self, wire: &SendWire) -> Result<(), RunnerError> {
        let frames = crate::fixture::wire::load(&self.ctx.repo)?;
        let entry = frames.named(&wire.frame)?;
        let bytes = entry.bytes()?;
        let declared = entry.declared()?;
        let bytewise: Vec<usize> = (1..bytes.len()).collect();
        let scope = format!("wire[{}]", entry.name);
        self.descriptors.push(DescriptorRecord {
            source: format!("fixtures/wire/frames.json#{}", entry.name),
            descriptors: entry.descriptors.clone(),
        });
        self.assert(vec![
            Assertion::equal(
                format!("{scope}/decode/whole"),
                declared.clone(),
                decode(&bytes, &[]),
            ),
            Assertion::equal(
                format!("{scope}/decode/bytewise"),
                declared,
                decode(&bytes, &bytewise),
            ),
        ])?;
        self.conn(&wire.conn)?
            .write(&bytes, &crate::scenario::model::Split::Whole)
            .await
    }

    /// Runs the binary once; `$op:`/`$value:` labels in its arguments and expectations
    /// resolve, and `$capture` matchers keep values from its stdout and stderr.
    pub(crate) async fn run_binary(&mut self, planned: &Run) -> Result<(), RunnerError> {
        let mut run = planned.clone();
        run.args = planned
            .args
            .iter()
            .map(
                |arg| match self.labels.resolve(&Value::String(arg.clone())) {
                    Value::String(text) => text,
                    other => other.to_string(),
                },
            )
            .collect();
        let bearer = self.bearer().map(str::to_owned);
        let ran = process::run(&self.endpoint(), &run, bearer.as_deref()).await?;
        self.processes.push(ran.report);
        let repo = self.ctx.repo.root().to_path_buf();
        let mut checked = vec![Assertion::equal(
            "binary/exit".to_owned(),
            json!(run.exit),
            json!(ran.exit),
        )];
        for (scope, expect, bytes) in [
            ("binary/stdout", &run.stdout, &ran.stdout),
            ("binary/stderr", &run.stderr, &ran.stderr),
        ] {
            let value = document(bytes);
            let expect = expect.resolved(&mut self.labels);
            checked.extend(expect.check(
                Subject {
                    scope,
                    value: &value,
                },
                &repo,
            ));
            for (name, captured) in expect.captures(&value) {
                self.labels.capture(name, captured);
            }
        }
        self.assert(checked)
    }

    fn effects(&mut self, effects: &Effects) -> Result<(), RunnerError> {
        let io = |source| RunnerError::Io {
            operation: "list the fixture effects",
            source,
        };
        let mut actual = BTreeMap::new();
        for entry in std::fs::read_dir(self.root.path().join("effects")).map_err(io)? {
            let path = entry.map_err(io)?.path();
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let content = std::fs::read(&path).map_err(io)?;
            actual.insert(name, document(&content));
        }
        let names =
            |map: &BTreeMap<String, Value>| Value::from(map.keys().cloned().collect::<Vec<_>>());
        let mut checked = vec![Assertion::equal(
            "effects/files".to_owned(),
            names(&effects.entries),
            names(&actual),
        )];
        for (name, expected) in &effects.entries {
            let actual = actual.get(name).cloned().unwrap_or(Value::Null);
            checked.push(Assertion::equal(
                format!("effects/{name}"),
                self.labels.resolve(expected),
                actual,
            ));
        }
        self.assert(checked)
    }

    async fn expect_failure(&mut self, expected: &ExpectFailure) -> Result<(), RunnerError> {
        let recorded = self.assertions.len();
        let outcome = self.execute(&expected.step).await;
        self.assertions.truncate(recorded);
        let actual = match &outcome {
            Ok(()) => json!({ "kind": null }),
            Err(error) => json!({ "kind": error.kind(), "detail": error.to_string() }),
        };
        let kind = actual.get("kind").cloned().unwrap_or(Value::Null);
        let passed = kind == json!(expected.kind);
        self.assert(vec![Assertion {
            check: "expect_failure/kind".to_owned(),
            expected: json!({ "kind": expected.kind }),
            actual,
            passed,
        }])
    }
}
