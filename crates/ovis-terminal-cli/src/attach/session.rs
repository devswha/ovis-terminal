//! Open, follow the stream, forward input, follow resizes, detach and release.

use base64::Engine as _;
use ovis_terminal_protocol::schema::records::{AttachControl, AttachData, ControlKind};
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::client::{Client, Notice};
use super::input::split_detach;
use super::open::{geometry_lease, open, still_held};
use super::screen::Screen;
use super::signals::{Interrupt, Signals};
use super::tty::{self, Typed, keyboard, typed_or_never};
use super::{AttachError, AttachPlan};

/// Bound on waiting for the stream's CLOSE after asking to detach.
const CLOSE_BOUND: std::time::Duration = std::time::Duration::from_secs(5);
/// Bytes typed before the stream is input-ready that are held for it; more is refused.
const HELD_INPUT_BYTES: usize = 64 << 10;

type Notices = mpsc::UnboundedReceiver<Notice>;

fn op_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn unreadable(detail: &'static str) -> impl FnOnce(serde_json::Error) -> AttachError {
    move |_| AttachError::Protocol { detail }
}

/// One attached stream and what acting on it needs.
struct Attached {
    client: Client,
    plan: AttachPlan,
    stream: String,
    lease: Option<String>,
    geometry_lease: Option<String>,
    displaced: Value,
    window: u64,
    acknowledged: u64,
    input_ready: bool,
    held: Vec<u8>,
    screen: Screen,
}

impl Attached {
    /// Closes the stream, which releases every lease it holds, waits for its CLOSE, then
    /// reads the leases back so the record states whether any lease this client was granted
    /// is still held. The server sends CLOSE only after the revocation is committed, so a
    /// CLOSE received means the read sees it. Interruptions delivered meanwhile join this
    /// detach instead of cutting the wait short; only [`CLOSE_BOUND`] ends it early.
    async fn detach(&mut self, notices: &mut Notices, reason: &str) -> Value {
        let requested = self
            .client
            .call("attach.close", json!({"stream_id": self.stream}))
            .await
            .is_ok();
        let closed = requested && self.await_close(notices).await;
        let granted: Vec<String> = self
            .lease
            .iter()
            .chain(&self.geometry_lease)
            .cloned()
            .collect();
        let held = still_held(&self.client, &self.plan, &granted).await.ok();
        json!({"detached": {
            "stream_id": self.stream, "reason": reason, "stream_closed": closed,
            "leases_released": held.as_ref().map(Vec::is_empty), "leases_held": held,
            "displaced": self.displaced,
        }})
    }

    async fn await_close(&mut self, notices: &mut Notices) -> bool {
        // Only CLOSE ends the wait: the outcome is already detach, so a notice that cannot
        // be drawn or read (a hung-up terminal) changes nothing the record reports.
        let close = async {
            while let Some(notice) = notices.recv().await {
                if let Ok(Some(_)) = self.notice(notice).await {
                    return true;
                }
            }
            false
        };
        tokio::time::timeout(CLOSE_BOUND, close)
            .await
            .unwrap_or(false)
    }

    async fn data(&mut self, data: AttachData) -> Result<(), AttachError> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data.data_b64.as_bytes())
            .map_err(|_| AttachError::Protocol {
                detail: "attach.data is not base64",
            })?;
        self.screen.draw(&bytes).await?;
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

    /// Writes `bytes` unchanged under the interact lease.
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
        let (typed, detach) = split_detach(bytes);
        if self.input_ready {
            self.forward(typed).await?;
        } else if self.held.len().saturating_add(typed.len()) <= HELD_INPUT_BYTES {
            self.held.extend_from_slice(typed);
        } else {
            return Err(AttachError::InputNotReady);
        }
        Ok(detach)
    }

    /// Resizes through `attach.resize` under the geometry lease; without one (observing,
    /// or queued behind another owner) the outer grid never reaches the terminal.
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
        notices: &mut Notices,
        signals: &mut Signals,
    ) -> Result<Value, AttachError> {
        let mut keyboard = (!self.plan.observe).then(keyboard);
        loop {
            tokio::select! {
                notice = notices.recv() => match notice {
                    None => return Err(AttachError::ConnectionLost),
                    Some(notice) => match self.notice(notice).await {
                        Ok(None) => {}
                        Ok(Some(end)) => return Ok(end),
                        Err(AttachError::TerminalGone) => {
                            return Ok(self.detach(notices, "hangup").await);
                        }
                        Err(error) => return Err(error),
                    },
                },
                typed = typed_or_never(keyboard.as_mut()) => match typed {
                    Typed::Bytes(bytes) => if self.typed(&bytes).await? {
                        return Ok(self.detach(notices, "detach_key").await);
                    },
                    Typed::HungUp => return Ok(self.detach(notices, "hangup").await),
                    Typed::Failed(source) => {
                        return Err(AttachError::Io { operation: "read the keyboard", source });
                    }
                },
                delivered = signals.next() => match delivered {
                    Interrupt::Resize => self.resize().await?,
                    stop => return Ok(self.detach(notices, stop.reason()).await),
                },
            }
        }
    }
}

/// Attaches with `client` and runs until detach or close; the outer terminal's modes and
/// settings are restored before this returns, whatever the outcome.
pub(super) async fn run(
    plan: &AttachPlan,
    (client, mut notices): (Client, Notices),
    signals: &mut Signals,
) -> Result<Value, AttachError> {
    if !plan.observe && !tty::interactive() {
        return Err(AttachError::NotATty);
    }
    let opened = signals
        .during("opening the attach", open(&client, plan))
        .await?;
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
        let read = geometry_lease(&client, plan);
        signals.during("reading the geometry lease", read).await?
    };
    let mut attached = Attached {
        lease: opened
            .lease
            .as_ref()
            .map(|lease| lease.lease_id.as_str().to_owned()),
        displaced: serde_json::to_value(&opened.displaced).unwrap_or(Value::Null),
        window: opened.credit_initial.get(),
        client,
        plan: plan.clone(),
        stream,
        geometry_lease,
        acknowledged: 0,
        input_ready: false,
        held: Vec::new(),
        screen: Screen::open(!plan.observe)?,
    };
    let outcome = attached.follow(&mut notices, signals).await;
    drop(attached);
    outcome
}
