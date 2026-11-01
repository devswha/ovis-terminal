//! `attach.open`, `attach.credit`, `attach.close` and `attach.repaint` over a connection.
//!
//! [`AttachHandler`] serves the attach methods itself and passes every other method to
//! the wrapped handler. Streams are delivered on the connection that opened them.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ovis_terminal_core::leases::{Caller, Owner};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::decode_params;
use ovis_terminal_protocol::schema::ids::StreamId;
use ovis_terminal_protocol::schema::limits::StreamMode;
use ovis_terminal_protocol::schema::methods::attach::{
    AttachCreditParams, AttachOpenParams, AttachStreamParams, AttachStreamResult,
};
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_store::OpId as StoreOpId;
use serde_json::Value;

use super::error::StreamError;
use super::lane::AttachLane;
use super::offset::StreamOffset;
use super::open::OpenRequest;
use super::pump::Pump;
use crate::conn::{Call, RequestHandler};
use crate::dispatch::lanes::agent_status::TerminalDirectory;

const METHODS: [&str; 4] = [
    "attach.open",
    "attach.credit",
    "attach.close",
    "attach.repaint",
];

/// Serves attach methods and delegates the rest to `inner`.
#[derive(Debug)]
pub struct AttachHandler<H, T> {
    inner: Arc<H>,
    lane: Arc<AttachLane<T>>,
}

impl<H, T> AttachHandler<H, T> {
    /// Adds the attach methods of `lane` in front of `inner`.
    pub const fn new(inner: Arc<H>, lane: Arc<AttachLane<T>>) -> Self {
        Self { inner, lane }
    }
}

fn now() -> Result<TimestampMs, StreamError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StreamError::Clock)?;
    Ok(TimestampMs::new(
        u64::try_from(elapsed.as_millis()).map_err(|_| StreamError::Clock)?,
    ))
}

fn decode<P: serde::de::DeserializeOwned>(call: &Call) -> Result<P, BoundaryError> {
    decode_params(&call.request).map_err(BoundaryError::from)
}

impl<T: TerminalDirectory> AttachLane<T> {
    async fn open(self: &Arc<Self>, call: Call) -> Result<Value, BoundaryError> {
        let params: AttachOpenParams = decode(&call)?;
        let op_id = params.op_id.clone();
        self.open_stream(call, params)
            .await
            .map_err(|error| error.boundary().with_op_id(op_id))
    }

    async fn open_stream(
        self: &Arc<Self>,
        call: Call,
        params: AttachOpenParams,
    ) -> Result<Value, StreamError> {
        if params.stream_mode == StreamMode::Channel {
            return Err(StreamError::NeedsFramed);
        }
        let hub = self.hub(&self.terminals.resolve(&params.target)?)?;
        let principal = call.session.principal().clone();
        let key = (principal.clone(), params.op_id.clone());
        let canonical = serde_json::to_value(&params)?;
        if let Some(prior) = self.prior(&key, &canonical)? {
            return Ok(prior);
        }
        let stream = StreamId::new(format!("stream:{}", uuid::Uuid::now_v7()));
        let fresh = self.fresh_lease();
        let at = now()?;
        let session = Arc::clone(&call.session);
        let (worker, worker_stream) = (Arc::clone(&hub), stream.clone());
        let opened = tokio::task::spawn_blocking(move || {
            let owner = Owner::new(
                session.principal().clone(),
                params.client_id.clone(),
                session.connection(),
            );
            let request = OpenRequest {
                caller: Caller::new(&session, &params.client_id, at),
                owner: &owner,
                role: params.role,
                takeover: params.takeover.as_ref(),
                fresh,
                stream: &worker_stream,
            };
            worker.open(request, StoreOpId::new(params.op_id.as_str()))
        })
        .await??;
        if let Some(wake) = opened.wake {
            self.register(stream.clone(), (principal, Arc::clone(&hub)))?;
            let pump = Pump {
                hub,
                stream: stream.clone(),
                wake,
                notifier: call.notifier,
            };
            let lane = Arc::clone(self);
            self.tasks
                .lock()
                .map_err(|_| StreamError::Poisoned)?
                .spawn(async move {
                    let delivered = pump.run().await;
                    lane.retire(&stream)?;
                    delivered
                });
        }
        let result = serde_json::to_value(&opened.result)?;
        self.remember(key, (canonical, result.clone()))?;
        Ok(result)
    }

    fn credit(&self, call: &Call) -> Result<Value, BoundaryError> {
        let params: AttachCreditParams = decode(call)?;
        let at = StreamOffset::new(params.acknowledged.get());
        let principal = call.session.principal();
        self.acknowledge(principal, (&params.stream_id, at))
            .map_err(|error| error.boundary())?;
        let result = AttachStreamResult::new(params.stream_id, at.wire(), None);
        serde_json::to_value(result).map_err(|error| StreamError::from(error).boundary())
    }

    fn stream_call(&self, call: &Call) -> Result<Value, BoundaryError> {
        let params: AttachStreamParams = decode(call)?;
        let reply = || -> Result<Value, StreamError> {
            let hub = self.owned(call.session.principal(), &params.stream_id)?;
            let result = if call.request.method() == "attach.close" {
                let at = hub.close(&params.stream_id)?;
                AttachStreamResult::new(params.stream_id.clone(), at.wire(), None)
            } else {
                let checkpoint = hub.repaint(&params.stream_id)?;
                let at = checkpoint.stream_offset;
                AttachStreamResult::new(params.stream_id.clone(), at, Some(checkpoint))
            };
            Ok(serde_json::to_value(result)?)
        };
        reply().map_err(|error| error.boundary())
    }
}

impl<H: RequestHandler, T: TerminalDirectory> RequestHandler for AttachHandler<H, T> {
    fn methods(&self) -> Vec<&'static str> {
        let mut methods = self.inner.methods();
        methods.extend(METHODS);
        methods
    }

    async fn call(&self, call: Call) -> Result<Value, BoundaryError> {
        match call.request.method() {
            "attach.open" => self.lane.open(call).await,
            "attach.credit" => self.lane.credit(&call),
            "attach.close" | "attach.repaint" => self.lane.stream_call(&call),
            _ => self.inner.call(call).await,
        }
    }
}
