//! A client connection holding one attach stream, reassembling what it receives.

use base64::Engine as _;
use ovis_terminal_vt::Vt;
use serde_json::{Value, json};

use super::TestResult;
use super::fixture::{TERMINAL, vt_config};
use super::socket::{Client, at, request};

#[derive(Debug)]
pub(crate) struct Viewer {
    client: Client,
    pub stream: String,
    pub result: Value,
    /// Every DATA byte in stream order; offsets are checked to be contiguous.
    pub received: Vec<u8>,
    pub controls: Vec<Value>,
    pub failures: Vec<Value>,
    pub credit: bool,
    sent: u64,
}

/// `attach.open` params for `TERMINAL` as client `client_id` in `role`.
pub(crate) fn open_params(op: &str, client_id: &str, role: &str) -> Value {
    json!({
        "op_id": op, "target": {"terminal_id": TERMINAL}, "role": role,
        "client_id": client_id, "geometry": null, "stream_mode": "plain", "takeover": null,
    })
}

impl Viewer {
    /// Sends `attach.open` and reads until its reply; earlier notifications are kept.
    /// With `credit`, every received byte is acknowledged as it arrives.
    pub(crate) async fn open(client: Client, params: &Value, credit: bool) -> TestResult<Self> {
        let mut viewer = Self {
            client,
            stream: String::new(),
            result: Value::Null,
            received: Vec::new(),
            controls: Vec::new(),
            failures: Vec::new(),
            credit,
            sent: 0,
        };
        viewer
            .client
            .send(&request("open", "attach.open", params))
            .await?;
        loop {
            let message = viewer.client.expect_next().await?;
            if message.get("id") == Some(&json!("open")) {
                viewer.result = message;
                break;
            }
            viewer.apply(&message).await?;
        }
        if let Some(stream) = at(&viewer.result, "/result/stream_id").as_str() {
            stream.clone_into(&mut viewer.stream);
        }
        Ok(viewer)
    }

    /// Sends one request on this connection and returns its reply, recording any
    /// stream messages that arrive first.
    pub(crate) async fn call(&mut self, method: &str, params: &Value) -> TestResult<Value> {
        self.sent = self.sent.saturating_add(1);
        let id = format!("call-{}", self.sent);
        self.client.send(&request(&id, method, params)).await?;
        loop {
            let message = self.client.expect_next().await?;
            if message.get("id") == Some(&json!(id)) {
                return Ok(message);
            }
            self.apply(&message).await?;
        }
    }

    async fn apply(&mut self, message: &Value) -> TestResult {
        match message.get("method").and_then(Value::as_str) {
            Some("attach.data") => {
                if self.stream.is_empty() {
                    // DATA may precede the open reply; it names its own stream.
                    at(message, "/params/stream_id")
                        .as_str()
                        .unwrap_or_default()
                        .clone_into(&mut self.stream);
                }
                let offset: u64 = at(message, "/params/offset")
                    .as_str()
                    .unwrap_or("x")
                    .parse()?;
                if offset != u64::try_from(self.received.len())? {
                    return Err(format!("non-contiguous DATA at {offset}").into());
                }
                let data = at(message, "/params/data_b64").as_str().unwrap_or_default();
                self.received
                    .extend(base64::engine::general_purpose::STANDARD.decode(data)?);
                if self.credit {
                    self.acknowledge().await?;
                }
            }
            Some("attach.control") => self.controls.push(message.clone()),
            _ if message.get("error").is_some() => self.failures.push(message.clone()),
            _ => {}
        }
        Ok(())
    }

    /// Acknowledges every byte received so far, without waiting for the reply.
    pub(crate) async fn acknowledge(&mut self) -> TestResult {
        self.sent = self.sent.saturating_add(1);
        let params =
            json!({"stream_id": self.stream, "acknowledged": self.received.len().to_string()});
        let id = format!("credit-{}", self.sent);
        self.client
            .send(&request(&id, "attach.credit", &params))
            .await
    }

    /// Reads stream messages until `done` holds.
    pub(crate) async fn until(&mut self, done: impl Fn(&Self) -> bool) -> TestResult {
        while !done(self) {
            let message = self.client.expect_next().await?;
            self.apply(&message).await?;
        }
        Ok(())
    }

    /// Control records of `kind`, in arrival order.
    pub(crate) fn kinds(&self, kind: &str) -> Vec<&Value> {
        self.controls
            .iter()
            .filter(|record| at(record, "/params/kind") == kind)
            .collect()
    }

    /// The first control record of `kind`.
    pub(crate) fn first(&self, kind: &str) -> TestResult<Value> {
        let record = self.kinds(kind).first().copied().cloned();
        record.ok_or_else(|| format!("no {kind} record").into())
    }

    /// The VT a client builds from the received bytes alone.
    pub(crate) fn screen(&self) -> TestResult<Vt> {
        let mut vt = Vt::new(vt_config()?);
        let outcome = vt.feed(&self.received);
        drop(outcome);
        Ok(vt)
    }
}
