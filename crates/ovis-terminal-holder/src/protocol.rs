//! Independently versioned holder protocol and adoption identity contracts.
//!
//! Shared-per-session/build holders and any internal dedicated-holder placement
//! use these same messages. Holder placement is not a public v1 launch field.

mod adoption;
#[cfg(test)]
mod adoption_sequence_tests;
#[cfg(test)]
mod counter_json_tests;
mod decimal;
mod decimal_u16;
mod decimal_u32;
mod frame;
mod identity;
mod messages;
mod optional_decimal;
mod optional_decimal_u32;
#[cfg(test)]
mod property_message_examples;
#[cfg(test)]
mod property_tests;

pub use adoption::{AdoptionError, AdoptionTable, HolderDescriptor, negotiate};
pub use frame::{
    FrameDecoder, FrameError, FrameHeader, decode_message, encode_message, parse_header,
};
pub use identity::{
    CoreBuildId, HolderBuildId, HolderInstanceId, ProcessIncarnation, ProcessStartToken,
    TerminalId, TerminalIncarnation,
};
pub use messages::{
    Drain, Exit, ExitUnknown, ExitUnknownReason, Forget, Goodbye, GoodbyeReason, HeldTerminal,
    Hello, Input, InputRejected, InputRejection, InputWritten, Message, MessageKind, Output,
    PreviousCoreState, PtyIoFailure, PtyOperation, Resize, Signal, SignalKind, Spawn, SpawnFailed,
    SpawnFailureKind, SpawnFailureReason, Spawned, Tail, TailRequest, TerminalTarget, Welcome,
    WriteGrant,
};

/// Independently versioned generation of the holder-to-core contract.
pub const HOLDER_PROTOCOL_GENERATION: u16 = 1;

/// Maximum encoded holder payload, in bytes.
pub const MAX_FRAME_PAYLOAD: usize = 16 * 1024 * 1024;
