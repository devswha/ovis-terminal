//! The state of one running case: isolated root, fixture, connections and assertions.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use super::conn::Connection;
use super::root::IsolatedRoot;
use super::transport::Endpoint;
use crate::error::RunnerError;
use crate::evidence::{ConnectionReport, DescriptorRecord, ProcessReport};
use crate::fixture::launch::FixtureProcess;
use crate::ids::ConnName;
use crate::scenario::labels::Labels;
use crate::scenario::load::Repo;
use crate::scenario::matcher::Assertion;

/// What every case of one run shares.
#[derive(Debug, Clone)]
pub(crate) struct Context {
    pub(crate) repo: Repo,
    pub(crate) binary: PathBuf,
    /// This runner's own executable, re-run in fixture mode.
    pub(crate) runner: PathBuf,
    /// The `--inject` request, which a failure case's inject step must plan.
    pub(crate) inject: Option<Value>,
}

pub(crate) struct Session<'a> {
    pub(crate) ctx: &'a Context,
    pub(crate) root: IsolatedRoot,
    pub(crate) fixture: Option<FixtureProcess>,
    pub(crate) conns: BTreeMap<ConnName, Connection>,
    pub(crate) labels: Labels,
    pub(crate) assertions: Vec<Assertion>,
    pub(crate) descriptors: Vec<DescriptorRecord>,
    pub(crate) connections: Vec<ConnectionReport>,
    pub(crate) processes: Vec<ProcessReport>,
    pub(crate) fixture_report: Option<Value>,
    /// Whether an inject step ran the `--inject` request.
    pub(crate) injected: bool,
    /// Outer PTYs of attach clients, by step name.
    #[cfg(unix)]
    pub(crate) ptys: BTreeMap<String, super::pty::outer::OuterPty>,
    /// Bearers replies handed out, by the name a `keep_bearer` gave them.
    pub(crate) kept: BTreeMap<String, String>,
}

impl<'a> Session<'a> {
    pub(crate) fn new(ctx: &'a Context) -> Result<Self, RunnerError> {
        Ok(Self {
            ctx,
            root: IsolatedRoot::create()?,
            fixture: None,
            conns: BTreeMap::new(),
            labels: Labels::default(),
            assertions: Vec::new(),
            descriptors: Vec::new(),
            connections: Vec::new(),
            processes: Vec::new(),
            fixture_report: None,
            injected: false,
            #[cfg(unix)]
            ptys: BTreeMap::new(),
            kept: BTreeMap::new(),
        })
    }

    /// The binary and the isolated root, plus the fixture socket once one is serving.
    pub(crate) fn endpoint(&self) -> Endpoint {
        Endpoint {
            binary: self.ctx.binary.clone(),
            root: self.root.path().to_path_buf(),
            socket: self.fixture.as_ref().map_or_else(
                || self.root.path().join("run").join("s.sock"),
                |fixture| fixture.ready.socket.clone(),
            ),
        }
    }

    pub(crate) fn bearer(&self) -> Option<&str> {
        self.fixture
            .as_ref()
            .map(|fixture| fixture.ready.bearer.as_str())
    }

    pub(crate) fn conn(&mut self, name: &ConnName) -> Result<&mut Connection, RunnerError> {
        self.conns
            .get_mut(name)
            .ok_or_else(|| RunnerError::UnknownConnection {
                conn: name.as_str().to_owned(),
            })
    }

    /// Keeps the bearer a reply handed out under `name`; the assertion records only whether
    /// one was there, never its text.
    pub(crate) fn keep_bearer(&mut self, (scope, record): (&str, &Value), name: &str) -> Assertion {
        let bearer = record
            .pointer("/result/issued/bearer")
            .and_then(Value::as_str);
        if let Some(bearer) = bearer {
            self.kept.insert(name.to_owned(), bearer.to_owned());
        }
        Assertion::equal(
            format!("{scope}/keep_bearer[{name}]"),
            Value::Bool(true),
            Value::Bool(bearer.is_some()),
        )
    }

    /// Records every assertion; the first one that failed ends the step.
    pub(crate) fn assert(&mut self, checked: Vec<Assertion>) -> Result<(), RunnerError> {
        let failed = checked
            .iter()
            .find(|assertion| !assertion.passed)
            .map(|assertion| assertion.check.clone());
        self.assertions.extend(checked);
        failed.map_or(Ok(()), |check| Err(RunnerError::AssertionFailed { check }))
    }

    /// Asserts that every received byte of `name` was decoded, then closes and reaps it.
    pub(crate) async fn finish_conn(&mut self, name: &ConnName) -> Result<(), RunnerError> {
        let conn = self
            .conns
            .remove(name)
            .ok_or_else(|| RunnerError::UnknownConnection {
                conn: name.as_str().to_owned(),
            })?;
        let (received, decoded) = conn.accounted();
        let (report, process) = conn.finish().await;
        self.connections.push(report);
        self.processes.extend(process);
        self.assert(vec![Assertion::equal(
            format!("connection[{}]/bytes_received=bytes_decoded", name.as_str()),
            Value::String(received.to_string()),
            Value::String(decoded.to_string()),
        )])
    }

    /// Stops the fixture and keeps its serve report and process record.
    pub(crate) async fn stop_fixture(&mut self) -> Result<Value, RunnerError> {
        let fixture = self.fixture.take().ok_or(RunnerError::FixtureState)?;
        let (report, process) = fixture.stop().await;
        self.processes.push(process);
        let report = report?;
        self.fixture_report = Some(report.clone());
        Ok(report)
    }

    /// Ends the case: closes every remaining connection, then stops the fixture.
    pub(crate) async fn close(&mut self) -> Vec<RunnerError> {
        let mut errors = Vec::new();
        let names: Vec<ConnName> = self.conns.keys().cloned().collect();
        for name in names {
            if let Some(conn) = self.conns.remove(&name) {
                let (report, process) = conn.finish().await;
                self.connections.push(report);
                self.processes.extend(process);
            }
        }
        if self.fixture.is_some()
            && let Err(error) = self.stop_fixture().await
        {
            errors.push(error);
        }
        errors
    }
}
