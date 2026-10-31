//! Open, follow the stream, forward input, follow resizes, detach.

use base64::Engine as _;
use ovis_terminal_protocol::schema::records::{AttachControl, AttachData, ControlKind};
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt as _;
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::sync::mpsc;

use super::client::{Client, Notice};
use super::open::{geometry_lease, open};
use super::tty::{self, RawMode, keyboard, typed_or_never};
use super::{AttachError, AttachPlan};

/// The detach key: Ctrl-\ (FS, 0x1c), read as a byte while the terminal is raw.
const DETACH_KEY: u8 = 0x1c;
/// Bound on waiting for the stream's CLOSE after asking to detach.
const CLOSE_BOUND: std::time::Duration = std::time::Duration::from_secs(5);
/// Bytes typed before the stream is input-ready that are held for it; more is refused.
const HELD_INPUT_BYTES: usize = 64 << 10;

fn op_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn unreadable(detail: &'static str) -> impl FnOnce(serde_json::Error) -> AttachError {
    move |_| AttachError::Protocol { detail }
}

fn listen(kind: SignalKind) -> Result<Signal, AttachError> {
    signal(kind).map_err(|source| AttachError::Io {
        operation: "watch signals",
        source,
    })
}

/// One attached stream and what acting on it needs.
struct Attached {
    client: Client,
    plan: AttachPlan,
    stream: String,
    lease: Option<String>,
    geometry_lease: Option<String>,
    window: u64,
    acknowledged: u64,
    input_ready: bool,
    held: Vec<u8>,
}

impl Attached {
    /// Closes the stream and waits, bounded, for its CLOSE: the server revokes and commits
    /// this client's leases before CLOSE is sent, so none is held once this returns true.
    async fn detach(
        &mut self,
        notices: &mut mpsc::UnboundedReceiver<Notice>,
        reason: &str,
    ) -> Value {
        let requested = self
            .client
            .call("attach.close", json!({"stream_id": self.stream}))
            .await
            .is_ok();
        let closed = requested
            && tokio::time::timeout(CLOSE_BOUND, async {
                while let Some(notice) = notices.recv().await {
                    if matches!(self.notice(notice).await, Ok(Some(_)) | Err(_)) {
                        return true;
                    }
                }
                false
            })
            .await
            .unwrap_or(false);
        json!({"detached": {"stream_id": self.stream, "reason": reason, "stream_closed": closed}})
    }

    async fn data(&mut self, data: AttachData) -> Result<(), AttachError> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data.data_b64.as_bytes())
            .map_err(|_| AttachError::Protocol {
                detail: "attach.data is not base64",
            })?;
        let mut stdout = tokio::io::stdout();
        let written = stdout.write_all(&bytes).await.and(stdout.flush().await);
        written.map_err(|source| AttachError::Io {
            operation: "write the screen to stdout",
            source,
        })?;
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let end = data.offset.get().saturating_add(length);
        if end.saturating_sub(self.acknowledged) >= self.window / 2 {
            let credit = json!({"stream_id": self.stream, "acknowledged": end.to_string()});
            self.client.call("attach.credit", credit).await?;
            self.acknowledged = end;
        }
        Ok(())
    }

    /// Applies one notification; `Some` once the server closed the stream.
    async fn notice(&mut self, (method, params): Notice) -> Result<Option<Value>, AttachError> {
        if params.get("stream_id").and_then(Value::as_str) != Some(self.stream.as_str()) {
            return Ok(None);
        }
        match method.as_str() {
            "attach.data" => {
                let data = serde_json::from_value(params).map_err(unreadable("attach.data"))?;
                self.data(data).await?;
            }
            "attach.control" => {
                let control: AttachControl =
                    serde_json::from_value(params).map_err(unreadable("attach.control"))?;
                match control.kind {
                    OpenEnum::Known(ControlKind::InputReady) => {
                        self.input_ready = true;
                        let held = std::mem::take(&mut self.held);
                        self.forward(&held).await?;
                    }
                    OpenEnum::Known(ControlKind::InputRevoked) => self.input_ready = false,
                    OpenEnum::Known(ControlKind::Close) => {
                        self.input_ready = false;
                        let close = serde_json::to_value(&control.close).unwrap_or(Value::Null);
                        return Ok(Some(
                            json!({"closed": {"stream_id": self.stream, "close": close}}),
                        ));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Ok(None)
    }

    /// Writes `bytes` under the interact lease.
    async fn forward(&self, bytes: &[u8]) -> Result<(), AttachError> {
        let Some(lease) = self.lease.as_ref().filter(|_| !bytes.is_empty()) else {
            return Ok(());
        };
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        let input = json!({
            "op_id": op_id(), "target": self.plan.target, "expect": null,
            "data_b64": data, "lease_id": lease,
        });
        self.client.call("input.raw", input).await.map(drop)
    }

    /// Forwards typed bytes, holding them until the stream is input-ready; `true` once
    /// the detach key was typed.
    async fn typed(&mut self, bytes: &[u8]) -> Result<bool, AttachError> {
        let (typed, detach) = bytes
            .iter()
            .position(|byte| *byte == DETACH_KEY)
            .map_or((bytes, false), |at| {
                (bytes.get(..at).unwrap_or_default(), true)
            });
        if self.input_ready {
            self.forward(typed).await?;
        } else if self.held.len().saturating_add(typed.len()) <= HELD_INPUT_BYTES {
            self.held.extend_from_slice(typed);
        } else {
            return Err(AttachError::InputNotReady);
        }
        Ok(detach)
    }

    async fn resize(&self) -> Result<(), AttachError> {
        let Some(lease) = &self.geometry_lease else {
            return Ok(());
        };
        let (cols, rows) = tty::grid()?;
        let resize = json!({
            "op_id": op_id(), "target": self.plan.target, "lease_id": lease,
            "geometry": {"cols": cols, "rows": rows},
        });
        self.client.call("attach.resize", resize).await.map(drop)
    }

    async fn follow(
        &mut self,
        notices: &mut mpsc::UnboundedReceiver<Notice>,
    ) -> Result<Value, AttachError> {
        let mut keyboard = (!self.plan.observe).then(keyboard);
        let mut winch = listen(SignalKind::window_change())?;
        let mut terminate = listen(SignalKind::terminate())?;
        let mut hangup = listen(SignalKind::hangup())?;
        let mut interrupt = listen(SignalKind::interrupt())?;
        loop {
            tokio::select! {
                notice = notices.recv() => match notice {
                    None => return Err(AttachError::ConnectionLost),
                    Some(notice) => if let Some(end) = self.notice(notice).await? {
                        return Ok(end);
                    },
                },
                typed = typed_or_never(keyboard.as_mut()) => match typed {
                    None => return Ok(self.detach(notices, "stdin_closed").await),
                    Some(bytes) => if self.typed(&bytes).await? {
                        return Ok(self.detach(notices, "detach_key").await);
                    },
                },
                _ = winch.recv() => self.resize().await?,
                _ = terminate.recv() => return Ok(self.detach(notices, "terminated").await),
                _ = hangup.recv() => return Ok(self.detach(notices, "hangup").await),
                _ = interrupt.recv() => return Ok(self.detach(notices, "interrupted").await),
            }
        }
    }
}

/// Attaches with `client` and runs until detach or close; raw mode, when used, is
/// restored before this returns, whatever the outcome.
pub(super) async fn run(
    plan: &AttachPlan,
    (client, mut notices): (Client, mpsc::UnboundedReceiver<Notice>),
) -> Result<Value, AttachError> {
    if !plan.observe && !tty::interactive() {
        return Err(AttachError::NotATty);
    }
    let opened = open(&client, plan).await?;
    let stream = opened
        .stream_id
        .as_ref()
        .map(|stream| stream.as_str().to_owned())
        .ok_or(AttachError::Protocol {
            detail: "attach.open returned no stream",
        })?;
    let geometry_lease = if plan.observe {
        None
    } else {
        geometry_lease(&client, plan).await?
    };
    let mut attached = Attached {
        lease: opened
            .lease
            .as_ref()
            .map(|lease| lease.lease_id.as_str().to_owned()),
        window: opened.credit_initial.get(),
        client,
        plan: plan.clone(),
        stream,
        geometry_lease,
        acknowledged: 0,
        input_ready: false,
        held: Vec::new(),
    };
    let raw = if plan.observe {
        None
    } else {
        Some(RawMode::enter()?)
    };
    let outcome = attached.follow(&mut notices).await;
    drop(raw);
    outcome
}
