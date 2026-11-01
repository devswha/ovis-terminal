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
use crate::fixture::launch;
use crate::fixture::wire::decode;
use crate::scenario::matcher::{Assertion, Subject};
use crate::scenario::model::{
    Connect, Effects, ExpectFailure, FixtureKind, Principal, Run, Send as SendStep, SendWire, Step,
    Via,
};

type StepFuture<'s> = Pin<Box<dyn Future<Output = Result<(), RunnerError>> + Send + 's>>;

impl Session<'_> {
    /// Runs one step; boxed so that `expect_failure` can run its inner step.
    pub(crate) fn execute<'s>(&'s mut self, step: &'s Step) -> StepFuture<'s> {
        Box::pin(async move {
            match step {
                Step::Serve(serve) => match serve.fixture {
                    FixtureKind::Protocol => self.serve().await,
                },
                Step::Connect(connect) => self.connect(connect).await,
                Step::Hello(hello) => self.hello(hello).await,
                Step::Send(send) => self.send(send).await,
                Step::SendWire(wire) => self.send_wire(wire).await,
                Step::Replies(replies) => self.replies(replies).await,
                Step::Notification(notification) => self.notification(notification).await,
                Step::Closed(closed) => self.closed(&closed.conn).await,
                Step::Hangup(hangup) => self.conn(&hangup.conn)?.hangup().await,
                Step::Rejections(rejections) => self.rejections(rejections).await,
                Step::Run(run) => self.run_binary(run).await,
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
                Step::Include(include) => Err(RunnerError::ScenarioInconsistent {
                    scenario: include.scenario.to_string(),
                    detail: "an include survived planning",
                }),
                Step::ExpectFailure(expected) => self.expect_failure(expected).await,
                Step::Inject(inject) => self.inject(inject).await,
            }
        })
    }

    async fn serve(&mut self) -> Result<(), RunnerError> {
        if self.fixture.is_some() {
            return Err(RunnerError::FixtureState);
        }
        if !cfg!(unix) {
            return Err(RunnerError::UnsupportedPlatform);
        }
        self.fixture =
            Some(launch::start(&self.ctx.runner, self.root.path(), &self.ctx.binary).await?);
        Ok(())
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

    async fn run_binary(&mut self, run: &Run) -> Result<(), RunnerError> {
        let bearer = self.bearer().map(str::to_owned);
        let ran = process::run(&self.endpoint(), run, bearer.as_deref()).await?;
        self.processes.push(ran.report);
        let repo = self.ctx.repo.root().to_path_buf();
        let mut checked = vec![Assertion::equal(
            "binary/exit".to_owned(),
            json!(run.exit),
            json!(ran.exit),
        )];
        let stdout = document(&ran.stdout);
        checked.extend(run.stdout.check(
            Subject {
                scope: "binary/stdout",
                value: &stdout,
            },
            &repo,
        ));
        let stderr = document(&ran.stderr);
        checked.extend(run.stderr.check(
            Subject {
                scope: "binary/stderr",
                value: &stderr,
            },
            &repo,
        ));
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
