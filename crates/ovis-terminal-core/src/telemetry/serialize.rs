use super::{FieldValue, TraceContext};

const MAX_FIELD_CHARS: usize = 256;

pub(super) fn serialize(context: &TraceContext, value: Option<FieldValue<'_>>) -> Vec<u8> {
    let mut fields = JsonFields {
        output: String::from("{"),
        first: true,
    };
    for (key, value) in [
        ("request", context.request.as_deref()),
        ("operation", context.operation.as_deref()),
        ("principal", context.principal.as_deref()),
        ("machine", context.machine.as_deref()),
        ("runtime", context.runtime.as_deref()),
        ("resource", context.resource.as_deref()),
    ] {
        if let Some(value) = value {
            fields.string(key, value);
        }
    }
    if let Some(sequence) = context.sequence {
        fields.number("sequence", sequence);
    }
    if let Some(outcome) = context.outcome {
        fields.string("outcome", outcome.field());
    }
    if let Some(value) = value {
        let rendered = match value {
            FieldValue::Text(text) => bounded(text),
            FieldValue::Credential(_) | FieldValue::SecretInput(_) => "[REDACTED]".to_owned(),
        };
        fields.string("value", &rendered);
    }
    fields.output.push('}');
    fields.output.into_bytes()
}

struct JsonFields {
    output: String,
    first: bool,
}

impl JsonFields {
    fn string(&mut self, key: &str, value: &str) {
        self.separator();
        self.output.push('"');
        self.output.push_str(key);
        self.output.push_str("\":\"");
        self.output.push_str(&escape(&bounded(value)));
        self.output.push('"');
    }

    fn number(&mut self, key: &str, value: u64) {
        self.separator();
        self.output.push('"');
        self.output.push_str(key);
        self.output.push_str("\":");
        self.output.push_str(&value.to_string());
    }

    fn separator(&mut self) {
        if !self.first {
            self.output.push(',');
        }
        self.first = false;
    }
}

fn bounded(value: &str) -> String {
    value.chars().take(MAX_FIELD_CHARS).collect()
}

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                let code = u32::from(character);
                escaped.push_str("\\u");
                escaped.push(hex_digit((code >> 12) & 0xf));
                escaped.push(hex_digit((code >> 8) & 0xf));
                escaped.push(hex_digit((code >> 4) & 0xf));
                escaped.push(hex_digit(code & 0xf));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

const fn hex_digit(value: u32) -> char {
    match value {
        0 => '0',
        1 => '1',
        2 => '2',
        3 => '3',
        4 => '4',
        5 => '5',
        6 => '6',
        7 => '7',
        8 => '8',
        9 => '9',
        10 => 'a',
        11 => 'b',
        12 => 'c',
        13 => 'd',
        14 => 'e',
        15 => 'f',
        _ => '?',
    }
}
