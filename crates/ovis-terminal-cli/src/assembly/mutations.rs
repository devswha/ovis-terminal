//! `terminal.kill`, `rm`, `keep`, `rename`, `label`, `recover` and `process_info` over the
//! landed lifecycle service. Kill waits for exit evidence on the lifecycle's own receive
//! path, so afterwards the live screen and streams are reconciled from the committed
//! record: missing output is replayed from the holder's retained tail, then the exit.

use ovis_terminal_authz::Session;
use ovis_terminal_core::registry::{ResourceName, TerminalId};
use ovis_terminal_core::terminals::{Call, KillRequest, LabelRequest, Record, TerminalError};
use ovis_terminal_holder::protocol::{Output, TailRequest};
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::methods::terminal::{
    TerminalKeepParams, TerminalKillParams, TerminalLabelParams, TerminalMutateParams,
    TerminalRenameParams,
};
use serde_json::{Value, json};

use super::actor::Runtime;
use super::error::{internal, refusal};
use super::terminal::terminal_refusal;

fn refused(error: &TerminalError) -> BoundaryError {
    terminal_refusal(error)
}

impl Runtime {
    fn target_id(&self, target: &Target) -> Result<TerminalId, BoundaryError> {
        self.resolve_id(target)
    }

    fn mutated(&mut self, session: &Session, record: &Record) -> Result<Value, BoundaryError> {
        self.reconcile(session, record.id())?;
        let terminal = self.record_value(record)?;
        Ok(json!({"terminal": terminal, "revision": record.revision, "seq": record.revision}))
    }

    /// Brings the live screen and streams of `id` level with its committed record.
    pub(super) fn reconcile(
        &mut self,
        session: &Session,
        id: TerminalId,
    ) -> Result<(), BoundaryError> {
        let record = self
            .terminals
            .get(session, id)
            .map_err(|error| refused(&error))?;
        let (Some(exit), Some(target)) = (&record.exit, &record.holder_target) else {
            return Ok(());
        };
        let Some(live) = self.live.get_mut(&id).filter(|live| !live.ended) else {
            return Ok(());
        };
        let seen = live
            .hub
            .screen()
            .live()
            .map_err(internal("reconcile screen"))?
            .offset
            .get();
        if seen < exit.final_offset {
            let tail = self
                .holder
                .tail(&TailRequest {
                    target: target.clone(),
                    from_offset: seen,
                })
                .map_err(internal("read the retained tail"))?;
            let skip = usize::try_from(seen.saturating_sub(tail.from_offset)).unwrap_or(usize::MAX);
            let bytes = tail
                .bytes
                .get(skip..)
                .map(<[u8]>::to_vec)
                .unwrap_or_default();
            let output = Output {
                target: target.clone(),
                offset: seen,
                bytes,
                hangup: None,
            };
            live.hub
                .publish(&output)
                .map_err(|error| error.boundary())?;
        }
        live.hub.finish(exit).map_err(|error| error.boundary())?;
        live.sequencer.exited();
        live.ended = true;
        self.directory.exited(id);
        Ok(())
    }

    pub(super) fn kill(
        &mut self,
        session: &Session,
        params: &TerminalKillParams,
    ) -> Result<Value, BoundaryError> {
        let id = self.target_id(&params.target)?;
        let record = self
            .terminals
            .get(session, id)
            .map_err(|error| refused(&error))?;
        let expected = record
            .lifecycle
            .process()
            .cloned()
            .ok_or_else(|| refusal("finished", "the terminal's program already ended"))?;
        if let Some(expect) = &params.expect
            && expect
                .process_incarnation
                .is_some_and(|incarnation| incarnation.get() != expected.generation().get())
        {
            return Err(refusal(
                "stale_target",
                "the expected program incarnation was replaced",
            ));
        }
        let mut request = KillRequest::immediate(id, expected);
        request.hup_ms = params.escalation.hup_ms;
        request.term_ms = params.escalation.term_ms;
        let receipt = self
            .terminals
            .kill(Call::new(session, params.op_id.clone()), &request)
            .map_err(|error| refused(&error))?;
        self.reconcile(session, id)?;
        let observed = if receipt.exit.is_some() {
            "exited"
        } else {
            "unknown"
        };
        Ok(json!({
            "incarnation": receipt.process.generation().get().to_string(),
            "observed": observed,
        }))
    }

    pub(super) fn remove(
        &mut self,
        session: &Session,
        params: &TerminalMutateParams,
    ) -> Result<Value, BoundaryError> {
        let id = self.target_id(&params.target)?;
        let before = self
            .terminals
            .get(session, id)
            .map_err(|error| refused(&error))?;
        self.terminals
            .rm(Call::new(session, params.op_id.clone()), id)
            .map_err(|error| refused(&error))?;
        let terminal = self.record_value(&before)?;
        Ok(json!({"terminal": terminal, "revision": before.revision, "seq": before.revision}))
    }

    pub(super) fn keep(
        &mut self,
        session: &Session,
        params: &TerminalKeepParams,
    ) -> Result<Value, BoundaryError> {
        let id = self.target_id(&params.target)?;
        let call = Call::new(session, params.op_id.clone());
        let record = self
            .terminals
            .keep(call, (id, params.keep))
            .map_err(|error| refused(&error))?;
        self.mutated(session, &record)
    }

    pub(super) fn rename(
        &mut self,
        session: &Session,
        params: &TerminalRenameParams,
    ) -> Result<Value, BoundaryError> {
        let id = self.target_id(&params.target)?;
        let name = params
            .name
            .as_ref()
            .map(|name| ResourceName::new(name.as_str()))
            .transpose()
            .map_err(|_| refusal("invalid_params", "name a valid terminal name"))?;
        let call = Call::new(session, params.op_id.clone());
        let record = self
            .terminals
            .rename(call, (id, name))
            .map_err(|error| refused(&error))?;
        if let Some(live) = self.live.get(&id) {
            let generation = live.generation;
            self.directory.spawned(
                (id, generation),
                record
                    .launch
                    .name
                    .as_ref()
                    .map(|name| name.as_str().to_owned()),
            );
        }
        self.mutated(session, &record)
    }

    pub(super) fn label(
        &mut self,
        session: &Session,
        params: &TerminalLabelParams,
    ) -> Result<Value, BoundaryError> {
        let id = self.target_id(&params.target)?;
        let request = LabelRequest::new(
            id,
            params.set.clone().unwrap_or_default(),
            params.remove.clone().unwrap_or_default(),
        );
        let call = Call::new(session, params.op_id.clone());
        let record = self
            .terminals
            .label(call, request)
            .map_err(|error| refused(&error))?;
        self.mutated(session, &record)
    }

    pub(super) fn recover(
        &mut self,
        session: &Session,
        params: &TerminalMutateParams,
    ) -> Result<Value, BoundaryError> {
        let id = self.target_id(&params.target)?;
        let call = Call::new(session, params.op_id.clone());
        let record = self
            .terminals
            .recover(call, id)
            .map_err(|error| refused(&error))?;
        self.mutated(session, &record)
    }
}
