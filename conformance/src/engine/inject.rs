//! The inject step: planned requests merged over base params, each reply checked.
//!
//! Without `--inject` every planned request runs. With it, only the planned request equal
//! to the injected JSON runs; a request the step does not plan fails the case.

use serde_json::{Map, Value, json};

use super::conn::Wait;
use super::exchange::RECORD_BOUND_MS;
use super::session::Session;
use crate::error::RunnerError;
use crate::scenario::labels::Labels;
use crate::scenario::matcher::{Assertion, Subject};
use crate::scenario::model::{Inject, Injection, Split};

/// `base` with the request's params laid over it, key by key.
fn merged(base: &Value, request: &Value) -> Value {
    let mut params: Map<String, Value> = base.as_object().cloned().unwrap_or_default();
    if let Some(overrides) = request.get("params").and_then(Value::as_object) {
        for (key, value) in overrides {
            params.insert(key.clone(), value.clone());
        }
    }
    Value::Object(params)
}

impl Session<'_> {
    pub(crate) async fn inject(&mut self, inject: &Inject) -> Result<(), RunnerError> {
        let selected: Vec<&Injection> = match &self.ctx.inject {
            None => inject.planned.iter().collect(),
            Some(requested) => {
                let found = inject
                    .planned
                    .iter()
                    .find(|planned| &planned.request == requested);
                self.assert(vec![Assertion::equal(
                    "inject/planned".to_owned(),
                    requested.clone(),
                    found.map_or(Value::Null, |planned| planned.request.clone()),
                )])?;
                self.injected = true;
                found.into_iter().collect()
            }
        };
        for (index, planned) in selected.into_iter().enumerate() {
            let base = Labels::default().resolve(&inject.base);
            let id = format!("inject-{index}");
            let request = json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": planned.request.get("method").cloned().unwrap_or(Value::Null),
                "params": merged(&base, &planned.request),
            });
            let conn = self.conn(&inject.conn)?;
            let bytes = conn.encode(&request)?;
            conn.write(&bytes, &Split::Whole).await?;
            let reply = conn
                .next(Wait::within(RECORD_BOUND_MS, "an injected reply"))
                .await?
                .ok_or(RunnerError::Closed {
                    waiting_for: "an injected reply",
                })?;
            let mut checked = vec![Assertion::equal(
                format!("inject[{index}]/id"),
                json!(id),
                reply.get("id").cloned().unwrap_or(Value::Null),
            )];
            let expect = planned.expect.resolved(&mut self.labels);
            checked.extend(expect.check(
                Subject {
                    scope: &format!("inject[{index}]"),
                    value: &reply,
                },
                self.ctx.repo.root(),
            ));
            self.assert(checked)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::merged;

    #[test]
    fn injected_params_replace_base_keys_and_keep_the_rest() {
        let base = json!({"op_id": "o", "seq": "9", "record": {"state": "idle"}});
        let request = json!({"method": "m", "params": {"seq": 3, "record": {"state": "done"}}});
        assert_eq!(
            merged(&base, &request),
            json!({"op_id": "o", "seq": 3, "record": {"state": "done"}})
        );
    }
}
