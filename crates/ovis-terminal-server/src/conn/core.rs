//! The sans-IO connection state machine: bytes in, ordered steps out.
//!
//! Hello is answered here, before any request reaches a handler. A framed upgrade is
//! applied to the decoder right after the hello result is queued, so bytes the client
//! sent behind its hello line are parsed as frames however the transport split them.

use std::sync::Arc;

use ovis_terminal_authz::{AuthzError, PeerAdmission, Session};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::rpc::{
    RequestId, RpcFailure, RpcReply, RpcRequest, RpcSuccess, parse_request,
};
use ovis_terminal_protocol::wire::{
    Channel, ConnectionDecoder, FeedOutcome, FrameError, FrameKind, WireMessage,
};
use serde_json::Value;

use super::hello::{Shared, negotiate};
use super::{ConnectionError, HelloError};

/// One action the driver performs, in order.
#[derive(Debug)]
pub(super) enum Step {
    /// Write this reply.
    Reply(RpcReply),
    /// Switch outbound encoding to frames after everything queued before it.
    Upgrade,
    /// Run this request under the authenticated session.
    Dispatch {
        request: RpcRequest,
        session: Arc<Session>,
    },
    /// Stop waiting for this request's reply: a `$/cancelRequest` notification.
    Cancel(RequestId),
    /// Send `$/closing` for this failure and end the connection.
    Close(ConnectionError),
}

/// The request a `{"method": "$/cancelRequest", "params": {"id": ...}}` notification names.
fn cancellation(text: &str) -> Option<RequestId> {
    let record: Value = serde_json::from_str(text).ok()?;
    let named = record.get("id").is_none()
        && record.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
        && record.get("method").and_then(Value::as_str) == Some("$/cancelRequest");
    let id = record.pointer("/params/id").and_then(Value::as_str)?;
    named.then(|| RequestId::new(id.to_owned()))
}

#[derive(Debug)]
enum Phase {
    AwaitingHello(PeerAdmission),
    Refused(AuthzError),
    Established(Arc<Session>),
    Closed,
}

/// Connection state from the first byte to the close.
#[derive(Debug)]
pub(super) struct ConnectionCore {
    decoder: ConnectionDecoder,
    phase: Phase,
    shared: Arc<Shared>,
}

impl ConnectionCore {
    pub(super) fn new(shared: Arc<Shared>, admission: Result<PeerAdmission, AuthzError>) -> Self {
        let phase = match admission {
            Ok(admission) => Phase::AwaitingHello(admission),
            Err(error) => Phase::Refused(error),
        };
        Self {
            decoder: ConnectionDecoder::new(),
            phase,
            shared,
        }
    }

    /// Consumes one transport read; a `Close` step is always the last step.
    pub(super) fn feed(&mut self, mut bytes: &[u8]) -> Vec<Step> {
        let mut steps = Vec::new();
        loop {
            let outcome = match self.decoder.feed(bytes) {
                Ok(outcome) => outcome,
                Err(error) => return self.close(steps, error.into()),
            };
            if let Err(error) = self.drain(&mut steps) {
                return self.close(steps, error);
            }
            if matches!(self.phase, Phase::Closed) {
                return steps;
            }
            match outcome {
                FeedOutcome::NeedDrain { consumed } => match bytes.get(consumed..) {
                    Some(rest) => bytes = rest,
                    None => return self.close(steps, FrameError::QueueAccountingOverflow.into()),
                },
                _ => return steps,
            }
        }
    }

    fn close(&mut self, mut steps: Vec<Step>, error: ConnectionError) -> Vec<Step> {
        self.phase = Phase::Closed;
        steps.push(Step::Close(error));
        steps
    }

    fn drain(&mut self, steps: &mut Vec<Step>) -> Result<(), ConnectionError> {
        while let Some(message) = self.decoder.next_message()? {
            let record = match message {
                WireMessage::ControlLine(line) => line,
                WireMessage::Frame(frame)
                    if frame.channel == Channel::CONTROL && frame.kind == FrameKind::Data =>
                {
                    frame.payload
                }
                WireMessage::Frame(frame) => {
                    return Err(ConnectionError::UnownedChannel {
                        channel: frame.channel.value(),
                    });
                }
                _ => return Err(ConnectionError::UnsupportedRecord),
            };
            self.on_record(&record, steps)?;
            if matches!(self.phase, Phase::Closed) {
                return Ok(());
            }
        }
        Ok(())
    }

    fn on_record(&mut self, record: &[u8], steps: &mut Vec<Step>) -> Result<(), ConnectionError> {
        let text = std::str::from_utf8(record).map_err(ConnectionError::InvalidUtf8)?;
        if matches!(self.phase, Phase::Established(_))
            && let Some(id) = cancellation(text)
        {
            steps.push(Step::Cancel(id));
            return Ok(());
        }
        let request = match parse_request(text) {
            Ok(request) => request,
            Err(error) => match recover_id(text) {
                Some(id) => {
                    steps.push(failure(id, &error));
                    return Ok(());
                }
                None => return Err(ConnectionError::Uncorrelatable(error)),
            },
        };
        let is_hello = request.method() == "hello";
        match std::mem::replace(&mut self.phase, Phase::Closed) {
            Phase::AwaitingHello(admission) if is_hello => {
                let (phase, answered) = self.hello(&request, admission)?;
                self.phase = phase;
                steps.extend(answered);
            }
            Phase::AwaitingHello(admission) => {
                self.phase = Phase::AwaitingHello(admission);
                steps.push(refusal(&request, &HelloError::HelloRequired));
            }
            Phase::Refused(error) => {
                let error = HelloError::Admission(error);
                steps.push(refusal(&request, &error));
                return Err(ConnectionError::Refused(error));
            }
            Phase::Established(session) if is_hello => {
                self.phase = Phase::Established(session);
                steps.push(refusal(&request, &HelloError::AlreadyEstablished));
            }
            Phase::Established(session) => {
                self.phase = Phase::Established(Arc::clone(&session));
                steps.push(Step::Dispatch { request, session });
            }
            Phase::Closed => {}
        }
        Ok(())
    }

    fn hello(
        &mut self,
        request: &RpcRequest,
        admission: PeerAdmission,
    ) -> Result<(Phase, Vec<Step>), ConnectionError> {
        let negotiated = match negotiate(request, &self.shared, admission) {
            Ok(negotiated) => negotiated,
            Err(error) => {
                let refused = vec![refusal(request, &error)];
                return Ok((Phase::AwaitingHello(admission), refused));
            }
        };
        let result = serde_json::to_value(&negotiated.result)?;
        let reply = RpcSuccess::new(request.id().clone(), result);
        let mut steps = vec![Step::Reply(RpcReply::Success(reply))];
        if negotiated.framed {
            steps.push(Step::Upgrade);
            self.decoder.upgrade_to_framed()?;
        }
        Ok((Phase::Established(Arc::new(negotiated.session)), steps))
    }
}

fn refusal(request: &RpcRequest, error: &HelloError) -> Step {
    failure(request.id().clone(), &error.to_boundary())
}

fn failure(id: RequestId, error: &BoundaryError) -> Step {
    Step::Reply(RpcReply::Failure(RpcFailure::new(id, error)))
}

/// The string id of a record that is JSON but not a valid request.
fn recover_id(text: &str) -> Option<RequestId> {
    let value: Value = serde_json::from_str(text).ok()?;
    let id = value.get("id")?.as_str()?;
    Some(RequestId::new(id.to_owned()))
}

#[cfg(test)]
mod tests;
