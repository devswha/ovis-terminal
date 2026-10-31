//! Running planned cases under one overall bound and turning them into case reports.

use std::time::Duration;

use tokio::time::Instant;

use super::session::{Context, Session};
use crate::error::RunnerError;
use crate::evidence::{CaseReport, Failure, Outcome};
use crate::ids::Decimal;
use crate::scenario::load::PlannedCase;
use crate::scenario::matcher::Assertion;
use crate::scenario::model::Platform;

/// Bound on one case, including fixture start and every step.
const CASE_BOUND: Duration = Duration::from_secs(300);

fn failure(error: &RunnerError, step: Option<usize>) -> Failure {
    Failure {
        kind: error.kind(),
        hint: error.hint(),
        detail: error.to_string(),
        step: step.map(Decimal::from_usize),
    }
}

fn report(planned: &PlannedCase, outcome: Outcome, failure: Option<Failure>) -> CaseReport {
    CaseReport {
        scenario: planned.scenario.clone(),
        case: planned.case,
        summary: planned.summary.clone(),
        declared: planned.declared.clone(),
        outcome,
        failure,
        assertions: Vec::new(),
        descriptors: Vec::new(),
        connections: Vec::new(),
        processes: Vec::new(),
        fixture: None,
        elapsed_ms: Decimal(0),
    }
}

/// A case passes only with no failure and every assertion held.
fn case_outcome(failure: Option<&Failure>, assertions: &[Assertion]) -> Outcome {
    if failure.is_some() || assertions.iter().any(|assertion| !assertion.passed) {
        Outcome::Failed
    } else {
        Outcome::Passed
    }
}

/// Runs one case; a case passes only with at least one assertion and no failure.
pub(crate) async fn run_case(ctx: &Context, planned: &PlannedCase) -> CaseReport {
    if !planned.platforms.contains(&Platform::current()) {
        let mut skipped = report(planned, Outcome::Skipped, None);
        skipped.failure = Some(Failure {
            kind: "platform_not_declared",
            hint: "run this case on a platform the scenario declares",
            detail: std::env::consts::OS.to_owned(),
            step: None,
        });
        return skipped;
    }
    let started = Instant::now();
    let mut session = match Session::new(ctx) {
        Ok(session) => session,
        Err(error) => return report(planned, Outcome::Failed, Some(failure(&error, None))),
    };
    let steps = async {
        for (index, step) in planned.steps.iter().enumerate() {
            session
                .execute(step)
                .await
                .map_err(|error| (index, error))?;
        }
        if ctx.inject.is_some() && !session.injected {
            return Err((planned.steps.len(), RunnerError::InjectionUnplanned));
        }
        Ok::<(), (usize, RunnerError)>(())
    };
    let outcome = tokio::time::timeout(CASE_BOUND, steps)
        .await
        .unwrap_or_else(|_| {
            Err((
                planned.steps.len(),
                RunnerError::Timeout {
                    waiting_for: "the case to finish",
                    bound_ms: u64::try_from(CASE_BOUND.as_millis()).unwrap_or(u64::MAX),
                },
            ))
        });
    let cleanup = session.close().await;
    let mut case = report(planned, Outcome::Passed, None);
    let first = match outcome {
        Err((index, error)) => Some(failure(&error, Some(index))),
        Ok(()) => cleanup.first().map(|error| failure(error, None)),
    };
    let none = (first.is_none() && session.assertions.is_empty())
        .then(|| failure(&RunnerError::NoAssertions, None));
    case.failure = first.or(none);
    case.outcome = case_outcome(case.failure.as_ref(), &session.assertions);
    case.assertions = std::mem::take(&mut session.assertions);
    case.descriptors = std::mem::take(&mut session.descriptors);
    case.connections = std::mem::take(&mut session.connections);
    case.processes = std::mem::take(&mut session.processes);
    case.fixture = session.fixture_report.take();
    case.elapsed_ms = Decimal(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    case
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{case_outcome, failure};
    use crate::error::RunnerError;
    use crate::evidence::Outcome;
    use crate::scenario::matcher::Assertion;

    #[test]
    fn a_failed_assertion_or_a_failure_fails_the_case() {
        let held = Assertion::equal("held".to_owned(), json!(1), json!(1));
        let broken = Assertion::equal("broken".to_owned(), json!(1), json!(2));
        assert_eq!(
            case_outcome(None, std::slice::from_ref(&held)),
            Outcome::Passed
        );
        assert_eq!(
            case_outcome(None, &[held.clone(), broken]),
            Outcome::Failed,
            "a failed assertion fails the case"
        );
        let timeout = failure(
            &RunnerError::Timeout {
                waiting_for: "a record",
                bound_ms: 1,
            },
            Some(0),
        );
        assert_eq!(
            case_outcome(Some(&timeout), &[held]),
            Outcome::Failed,
            "a failure fails the case"
        );
    }
}
