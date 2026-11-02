//! The `migration` step of `client/migration`: the Ovis adapter migration guide
//! (`docs/ovis-adapter.md`, or `--guide`) checked against the binary.
//!
//! The happy kind proves section-6 coverage (every Ovis path the step lists has one guide
//! row with its fate), that every name a row or example uses exists in the published
//! schema and the schema the packed Bun client ships, that rows list only served methods
//! (unserved ones as gaps), that the examples keep the Ovis policies, and replays every
//! example against a fresh server. The failure kind runs the planned injections, or only
//! the one `--inject` names: a guide example with one call rewritten, which must be
//! refused with its typed kind while the follow-up reads show nothing changed.

use std::collections::BTreeMap;

use ovis_terminal_replay::{Case, CaseKind, FixtureSet};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::engine::replay::{assertion, replay_failure};
use crate::engine::session::Session;
use crate::error::RunnerError;
use crate::scenario::matcher::Assertion;

mod contract;
mod guide;
mod policy;
mod served;
#[cfg(test)]
mod tests;

use contract::Contract;
use guide::{Fate, Guide};

/// One Ovis path of architect section 6 and the fate the guide must give it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PathFate {
    path: String,
    fate: Fate,
}

/// A guide example with one call rewritten, and what the binary must answer.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Injection {
    /// Compared exactly with `--inject`.
    request: Value,
    example: String,
    /// Id of the rewritten call; the steps after it are replaced by `then`.
    step: String,
    /// Params laid over the call's own, key by key.
    params: Map<String, Value>,
    expect: Value,
    /// Steps that prove the refusal changed nothing.
    #[serde(default)]
    then: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Migration {
    kind: CaseKind,
    /// Where `paths` was copied from (advisory path, section, lines, sha256); recorded in
    /// the evidence, never asserted, since the advisory is not in the repository.
    source: crate::evidence::Descriptors,
    paths: Vec<PathFate>,
    #[serde(default)]
    injections: Vec<Injection>,
}

fn inconsistent(scenario: &str, detail: &'static str) -> RunnerError {
    RunnerError::ScenarioInconsistent {
        scenario: scenario.to_owned(),
        detail,
    }
}

/// `example` with `injection` applied: the steps before its call, the rewritten call,
/// then the proof steps.
fn injected(example: &Value, injection: &Injection) -> Result<Value, RunnerError> {
    let steps = example
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| inconsistent(&injection.example, "a guide example has no steps"))?;
    let at = steps
        .iter()
        .position(|step| step.pointer("/call/id") == Some(&json!(injection.step)))
        .ok_or_else(|| {
            inconsistent(&injection.step, "an injection names no call of its example")
        })?;
    let mut call = steps
        .get(at)
        .and_then(|step| step.get("call"))
        .cloned()
        .unwrap_or_default();
    if let Some(params) = call.get_mut("params").and_then(Value::as_object_mut) {
        params.extend(injection.params.clone());
    }
    if let Some(object) = call.as_object_mut() {
        object.insert("expect".to_owned(), injection.expect.clone());
        object.remove("inject");
    }
    let mut rewritten: Vec<Value> = steps.iter().take(at).cloned().collect();
    rewritten.push(json!({ "call": call }));
    rewritten.extend(injection.then.iter().cloned());
    let mut case = example.clone();
    if let Some(object) = case.as_object_mut() {
        object.insert("steps".to_owned(), Value::Array(rewritten));
        object.insert("kind".to_owned(), json!("failure"));
    }
    Ok(case)
}

impl Session<'_> {
    pub(crate) async fn migration(&mut self, step: &Migration) -> Result<(), RunnerError> {
        let path = Guide::path(self.ctx.guide.as_ref(), self.ctx.repo.root());
        let guide = Guide::read(&path)?;
        let expected: BTreeMap<String, Fate> = step
            .paths
            .iter()
            .map(|row| (row.path.clone(), row.fate))
            .collect();
        let mut checks = vec![
            Assertion::equal(
                "migration/coverage".to_owned(),
                guide::complete(),
                guide::coverage(&expected, &guide),
            ),
            Assertion::equal(
                "migration/coverage/paths".to_owned(),
                json!(step.paths.len().to_string()),
                json!(guide.rows.len().to_string()),
            ),
            Assertion::equal(
                "migration/policy".to_owned(),
                policy::kept(),
                policy::report(&guide.examples),
            ),
        ];
        self.descriptors.push(crate::evidence::DescriptorRecord {
            source: "scenarios/client/migration.json#source".to_owned(),
            descriptors: step.source.clone(),
        });
        let set = FixtureSet::load(&self.fixture_dir()).map_err(replay_failure)?;
        if step.kind == CaseKind::Failure {
            self.assert(checks)?;
            return self.injections(&guide, (&set, &step.injections)).await;
        }
        let contract = self.contract(&set, &guide).await?;
        checks.extend(self.packed_client()?);
        checks.push(Assertion::equal(
            "migration/contract".to_owned(),
            contract::consistent(),
            contract.report(&guide),
        ));
        self.assert(checks)?;
        for (id, example) in &guide.examples {
            self.replay_example((id, example), &set).await?;
        }
        Ok(())
    }

    async fn replay_example(
        &mut self,
        (id, example): (&str, &Value),
        set: &FixtureSet,
    ) -> Result<(), RunnerError> {
        let case: Case = serde_json::from_value(example.clone())
            .map_err(|_| inconsistent(id, "a guide example is not an ovis-replay/1 case"))?;
        let run = self.run_replay(set, (&case, None)).await?;
        let mut checks: Vec<Assertion> = run.checks.into_iter().map(assertion).collect();
        checks.push(Assertion::equal(
            format!("migration/example/{id}/sent"),
            json!(true),
            json!(!run.sent.is_empty()),
        ));
        self.assert(checks)
    }

    async fn injections(
        &mut self,
        guide: &Guide,
        (set, planned): (&FixtureSet, &[Injection]),
    ) -> Result<(), RunnerError> {
        let selected: Vec<&Injection> = match self.ctx.inject.clone() {
            None => planned.iter().collect(),
            Some(requested) => {
                let found = planned.iter().find(|entry| entry.request == requested);
                self.assert(vec![Assertion::equal(
                    "migration/inject/planned".to_owned(),
                    requested,
                    found.map_or(Value::Null, |entry| entry.request.clone()),
                )])?;
                self.injected = true;
                found.into_iter().collect()
            }
        };
        for injection in selected {
            let example = guide
                .examples
                .iter()
                .find(|(id, _)| *id == injection.example)
                .map(|(_, case)| case)
                .ok_or_else(|| inconsistent(&injection.example, "an injection names no example"))?;
            let id = format!("{}.injected", injection.example);
            let mut rewritten = injected(example, injection)?;
            if let Some(object) = rewritten.as_object_mut() {
                object.insert("id".to_owned(), json!(id));
            }
            self.replay_example((&id, &rewritten), set).await?;
        }
        Ok(())
    }

    /// The schema the packed client ships is the published one.
    fn packed_client(&self) -> Result<Vec<Assertion>, RunnerError> {
        let root = self.ctx.repo.root();
        let read = |relative: &str| crate::scenario::load::read_json(&root.join(relative));
        let published = read("schema/ovis-terminal.openrpc.json")?;
        let shipped = read("packages/client-ts/src/generated/openrpc.json")?;
        Ok(vec![Assertion::equal(
            "migration/packed-client/schema".to_owned(),
            json!(true),
            json!(published == shipped),
        )])
    }

    async fn contract(&mut self, set: &FixtureSet, guide: &Guide) -> Result<Contract, RunnerError> {
        let schema = crate::scenario::load::read_json(
            &self
                .ctx
                .repo
                .root()
                .join("schema/ovis-terminal.openrpc.json"),
        )?;
        let mut contract = Contract::of_schema(&schema);
        contract.served = self.served_methods().await?;
        contract.legacy = set.legacy.methods.keys().cloned().collect();
        let repo = &self.ctx.repo;
        let executable = |id: &str| {
            crate::ids::ScenarioId::parse(id)
                .is_ok_and(|id| crate::scenario::load::load(repo, &id).is_ok())
        };
        let cited_scenarios = guide
            .rows
            .iter()
            .flat_map(|row| row.fixtures.iter())
            .filter(|cited| cited.strip_prefix("scenario:").is_some_and(executable))
            .cloned();
        contract.fixtures = set
            .cases
            .iter()
            .map(|case| format!("fixture:{}", case.id.as_str()))
            .chain(cited_scenarios)
            .collect();
        Ok(contract)
    }
}
