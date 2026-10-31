//! Admission: every check runs before the entry takes a place in the sequence.

use ovis_terminal_protocol::schema::scalar::ByteCount;

use super::entry::Entry;
use super::{Payload, Request, Sequencer, SequencerError, View, Writer};
use crate::input::encoding::{Encoded, InputRequest, ModeSnapshot, RawGrant, RawRequest, encode};
use crate::input::receipts::{Delivery, InputSeq, Receipt, Reporter, Stage};
use crate::leases::Write;

impl Sequencer {
    /// Admits one entry and answers with its `queued` receipt without waiting for any write.
    ///
    /// # Errors
    ///
    /// `not_ready`, `recovering` or `finished` before the stream is live; `stale_target` or
    /// `process_replaced` for identities that moved; the lease and grant refusals
    /// (`no_lease`, `forbidden`, credential kinds); `raw_not_authorized` and the encoding
    /// kinds; `input_overflow` when the queue is full.
    pub fn admit(
        &mut self,
        view: &View<'_>,
        request: Request<'_>,
    ) -> Result<Receipt, SequencerError> {
        let process = self.live()?;
        self.binding.check(&request.submission.expected, process)?;
        let Request { writer, submission } = request;
        // The raw grant is decided before the lease so a missing grant reads as such.
        let (raw, encoded) = self.encode(view.modes, (&writer, submission.payload))?;
        let authorization = view
            .leases
            .authorize_write(
                writer.caller(),
                Write::new(submission.payload.write_kind(), writer.lease),
            )
            .map_err(SequencerError::Lease)?;
        self.queue.room(encoded.bytes().len(), self.limits)?;
        let next = self
            .last_input
            .checked_add(1)
            .ok_or(SequencerError::Exhausted)?;
        let seq = InputSeq::new(next);
        let stage = match submission.reporter {
            Reporter::Undeclared => Stage::Unknown,
            Reporter::Declared => Stage::Pending,
        };
        let evidence = encoded.evidence();
        let admitted = evidence
            .literal_bytes
            .get()
            .saturating_add(evidence.submit_bytes.get());
        let receipt = Receipt {
            op_id: submission.op_id,
            queued_seq: seq,
            kind: evidence.kind,
            admitted_bytes: ByteCount::new(admitted),
            mode_revision: encoded.receipt_revision(),
            output_cursor: view.moment.output,
            delivery: Delivery::Queued,
            agent_accepted: stage,
            turn_started: stage,
            turn_ended: stage,
        };
        self.last_input = next;
        self.receipts.insert(receipt.clone());
        self.queue.push(Entry {
            seq,
            process,
            authorization,
            raw,
            encoded,
        });
        Ok(receipt)
    }

    fn encode(
        &self,
        modes: &ModeSnapshot,
        (writer, payload): (&Writer<'_>, Payload<'_>),
    ) -> Result<(Option<RawGrant>, Encoded), SequencerError> {
        let encoded = match payload {
            Payload::Raw(bytes) => {
                // The grant is asked for this sequencer's own terminal, so it cannot come
                // from another target.
                let request = RawRequest::new(self.binding.target().clone(), writer.at);
                let grant = RawGrant::decide(self.binding.authority(), writer.session, request)
                    .map_err(SequencerError::Encoding)?;
                let encoded = encode(
                    modes,
                    InputRequest::Raw {
                        bytes,
                        grant: &grant,
                    },
                );
                return encoded
                    .map(|encoded| (Some(grant), encoded))
                    .map_err(SequencerError::Encoding);
            }
            Payload::Text { text, submit } => encode(modes, InputRequest::Text { text, submit }),
            Payload::Paste { text, policy } => encode(modes, InputRequest::Paste { text, policy }),
            Payload::Keys(keys) => encode(modes, InputRequest::Keys(keys)),
            Payload::Mouse(report) => encode(modes, InputRequest::Mouse(report)),
        };
        encoded
            .map(|encoded| (None, encoded))
            .map_err(SequencerError::Encoding)
    }
}
