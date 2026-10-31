//! Optional generation-two evidence outside the frozen message payload.
//!
//! Generation-two readers reject unknown payload fields and frame tags, but
//! their adjacently tagged Message ignores unknown envelope fields. Hangup rides
//! an empty Output at the final raw offset; legacy readers observe no extra bytes.

use serde::{Deserialize, Serialize};

use super::{FrameError, Hangup, Message};

#[derive(Serialize)]
struct Outgoing<'a> {
    #[serde(flatten)]
    message: &'a Message,
    #[serde(skip_serializing_if = "Option::is_none")]
    hangup: Option<&'a Hangup>,
}

#[derive(Deserialize)]
struct Incoming {
    #[serde(flatten)]
    message: Message,
    hangup: Option<Hangup>,
}

pub(super) fn encode(message: &Message) -> Result<Vec<u8>, FrameError> {
    let hangup = match message {
        Message::Output(output) => output.hangup.as_ref(),
        Message::Hello(_)
        | Message::Welcome(_)
        | Message::Spawn(_)
        | Message::Spawned(_)
        | Message::SpawnFailed(_)
        | Message::Input(_)
        | Message::InputWritten(_)
        | Message::Resize(_)
        | Message::Signal(_)
        | Message::Exit(_)
        | Message::TailRequest(_)
        | Message::Tail(_)
        | Message::Forget(_)
        | Message::Drain(_)
        | Message::Goodbye(_)
        | Message::InputRejected(_) => None,
    };
    serde_json::to_vec(&Outgoing { message, hangup })
        .map_err(|source| FrameError::PayloadSerialization { source })
}

pub(super) fn decode(bytes: &[u8]) -> Result<Message, FrameError> {
    let Incoming {
        mut message,
        hangup,
    } = serde_json::from_slice(bytes).map_err(|source| FrameError::MalformedPayload { source })?;
    if let Some(hangup) = hangup {
        let Message::Output(output) = &mut message else {
            return Err(FrameError::MessageKindMismatch);
        };
        if !output.bytes.is_empty() || output.target.process_incarnation.is_none() {
            return Err(FrameError::MessageKindMismatch);
        }
        output.hangup = Some(hangup);
    }
    Ok(message)
}
