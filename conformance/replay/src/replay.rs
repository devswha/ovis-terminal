//! Running one case over any transport.

use std::future::Future;

use serde_json::{Value, json};

use crate::bind::Bindings;
use crate::case::{Call, Case, ConnLabel, Step};
use crate::error::ReplayError;
use crate::expect::Check;
use crate::legacy::LegacyMap;
use crate::receipt::{Receipt, ReceiptLine};

/// A consumer's connection to the runtime.
///
/// Correlated calls and awaited notifications on named persistent connections; each
/// label maps to one connection the transport opens (and greets with `hello`) on first use.
pub trait Transport {
    /// The transport's own failure.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Sends one JSON-RPC request on `conn` and returns its reply, `{"result"}` or
    /// `{"error"}`; notifications that arrive meanwhile stay available to `event`.
    fn call(
        &mut self,
        conn: &ConnLabel,
        request: Value,
    ) -> impl Future<Output = Result<Value, Self::Error>> + Send;

    /// Returns the next `{"method", "params"}` notification named `method` on `conn`.
    fn event(
        &mut self,
        conn: &ConnLabel,
        method: &str,
    ) -> impl Future<Output = Result<Value, Self::Error>> + Send;
}

/// How a case is replayed.
#[derive(Debug)]
#[non_exhaustive]
pub struct Options<'a> {
    /// Translation of legacy steps.
    pub legacy: &'a LegacyMap,
    /// Server bindings for `$bind:` symbols.
    pub bindings: Bindings,
    /// The `--inject` request: only inject steps planning it run.
    pub inject: Option<&'a Value>,
}

impl<'a> Options<'a> {
    /// Options replaying with `legacy`, `bindings` and, for a failure case, `inject`.
    #[must_use]
    pub const fn new(legacy: &'a LegacyMap, bindings: Bindings, inject: Option<&'a Value>) -> Self {
        Self {
            legacy,
            bindings,
            inject,
        }
    }
}

/// What one case run observed.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct CaseRun {
    /// Every check, in order; the run stops after the first step with a failed check.
    pub checks: Vec<Check>,
    /// The normalized receipt of the executed steps.
    pub receipt: Receipt,
    /// Every request as sent, after translation and binding.
    pub sent: Vec<Value>,
}

impl CaseRun {
    /// Whether every check held.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|check| check.passed)
    }
}

/// Replays `case` over `transport`.
///
/// # Errors
///
/// Returns an unbound symbol, an untranslatable legacy step or a transport failure;
/// failed expectations are reported in [`CaseRun::checks`] instead.
pub async fn replay<T: Transport + Send>(
    case: &Case,
    transport: &mut T,
    options: Options<'_>,
) -> Result<CaseRun, ReplayError> {
    let Options {
        legacy,
        mut bindings,
        inject,
    } = options;
    let mut run = CaseRun::default();
    for step in &case.steps {
        if inject.is_some_and(|request| step.is_inject() && !step.injects(request)) {
            continue;
        }
        let checks = match step {
            Step::Call(call) => {
                let sent = (call.method.clone(), call.params.clone());
                exchange((call, sent), (transport, &mut bindings), &mut run).await?
            }
            Step::Legacy(call) => {
                let sent = legacy.translate(&call.id, (&call.method, &call.params))?;
                exchange((call, sent), (transport, &mut bindings), &mut run).await?
            }
            Step::Event(event) => {
                let record =
                    transport
                        .event(&event.conn, &event.method)
                        .await
                        .map_err(|source| ReplayError::Transport {
                            step: event.method.clone(),
                            source: Box::new(source),
                        })?;
                run.receipt.push(ReceiptLine::event(&event.method, &record));
                event
                    .expect
                    .check((&format!("event:{}", event.method), &record), &mut bindings)?
            }
        };
        let failed = checks.iter().any(|check| !check.passed);
        run.checks.extend(
            checks
                .into_iter()
                .map(|check| check.scoped(case.id.as_str())),
        );
        if failed {
            break;
        }
    }
    Ok(run)
}

async fn exchange<T: Transport + Send>(
    (call, (method, params)): (&Call, (String, Value)),
    (transport, bindings): (&mut T, &mut Bindings),
    run: &mut CaseRun,
) -> Result<Vec<Check>, ReplayError> {
    let params = bindings.resolve(&params)?;
    let request = json!({"jsonrpc": "2.0", "id": call.id, "method": method, "params": params});
    run.sent.push(request.clone());
    let reply = transport
        .call(&call.conn, request)
        .await
        .map_err(|source| ReplayError::Transport {
            step: call.id.clone(),
            source: Box::new(source),
        })?;
    run.receipt
        .push(ReceiptLine::reply(&call.id, &method, &reply));
    call.expect.check((&call.id, &reply), bindings)
}
