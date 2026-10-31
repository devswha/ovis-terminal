//! Schema forms of protocol scalars: decimal-string counters and open output enums.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::rpc::DecimalU64;

/// Canonical decimal pattern shared by every 64-bit counter on the wire.
const DECIMAL_U64_PATTERN: &str = "^(0|[1-9][0-9]{0,19})$";

fn decimal_schema(description: &str) -> Schema {
    json_schema!({
        "type": "string",
        "pattern": DECIMAL_U64_PATTERN,
        "description": description,
    })
}

impl JsonSchema for DecimalU64 {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("DecimalCounter")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        decimal_schema("Unsigned 64-bit integer carried as a canonical decimal string.")
    }
}

/// Declares a named 64-bit counter carried as a canonical decimal string.
macro_rules! decimal_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(DecimalU64);

        impl $name {
            /// Wraps a native counter value.
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(DecimalU64::new(value))
            }

            /// Returns the native counter value.
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0.get()
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed(stringify!($name))
            }

            fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
                decimal_schema(concat!(stringify!($name), " carried as a canonical decimal string."))
            }
        }
    };
}

decimal_newtype!(
    /// Runtime-lifetime event sequence number.
    EventSeq
);
decimal_newtype!(
    /// Monotonic revision of a resource, screen, layout or snapshot.
    Revision
);
decimal_newtype!(
    /// Cumulative byte offset in raw output, a filtered stream or history.
    ByteOffset
);
decimal_newtype!(
    /// Byte quantity such as a limit, a credit grant or a backlog size.
    ByteCount
);
decimal_newtype!(
    /// Number of items such as retained events, history lines or in-flight requests.
    Count
);
decimal_newtype!(
    /// Epoch of an interact or geometry lease; bumps on every ownership change.
    LeaseEpoch
);
decimal_newtype!(
    /// Incarnation counter of a terminal or a spawned program.
    Incarnation
);
decimal_newtype!(
    /// Protocol generation negotiated by `hello`; additive releases keep it.
    ProtocolGeneration
);
decimal_newtype!(
    /// Milliseconds since the Unix epoch.
    TimestampMs
);
decimal_newtype!(
    /// Version of a document, prompt or other compare-and-swap record.
    RecordVersion
);
decimal_newtype!(
    /// Line number in retained terminal history.
    LineNumber
);
decimal_newtype!(
    /// Turn number of a terminal; increments when the root status leaves working.
    TurnNumber
);
decimal_newtype!(
    /// Duration in milliseconds.
    DurationMs
);

/// An output enum value that a client tolerates even when it postdates the client.
///
/// Values added by a later same-generation release decode as [`OpenEnum::Unknown`]
/// with the original string, instead of failing the whole reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
#[expect(
    clippy::exhaustive_enums,
    reason = "known-or-unknown is the complete open-enum contract"
)]
pub enum OpenEnum<T> {
    /// A value this build of the protocol knows.
    Known(T),
    /// A value added after this build; the string is kept verbatim.
    Unknown(String),
}

/// JSON Schema extension key marking an output enum as open to additions.
pub const OPEN_ENUM_EXTENSION: &str = "x-ovis-open-enum";

impl<T: JsonSchema> JsonSchema for OpenEnum<T> {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        Cow::Owned(format!("Open{}", T::schema_name()))
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut schema = generator.subschema_for::<T>();
        schema.insert(OPEN_ENUM_EXTENSION.to_owned(), true.into());
        schema
    }
}
