//! The text form of a structured result: one `key: value` line per scalar, nested by indent.
//! A string value is data: its control characters are escaped, so a program's text (a
//! prompt, a screen line, a session name) can never start a new line of its own or send a
//! sequence to the reader's terminal.

use serde_json::Value;

/// Renders `value` for a person; the JSON form is the contract.
pub(super) fn render(value: &Value) -> String {
    let mut out = String::new();
    match scalar(value) {
        Some(text) => {
            out.push_str(&text);
            out.push('\n');
        }
        None => nest(value, 0, &mut out),
    }
    out
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::Null => Some("null".to_owned()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) => Some(inert(text)),
        Value::Array(items) if items.is_empty() => Some("[]".to_owned()),
        Value::Object(fields) if fields.is_empty() => Some("{}".to_owned()),
        Value::Array(_) | Value::Object(_) => None,
    }
}

/// `text` with every control character written as its Rust escape (`\n`, `\u{1b}`).
fn inert(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() || character == '\\' {
                character.escape_default().to_string()
            } else {
                character.to_string()
            }
        })
        .collect()
}

fn nest(value: &Value, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    let entries: Vec<(String, &Value)> = match value {
        Value::Object(fields) => fields
            .iter()
            .map(|(key, item)| (format!("{key}:"), item))
            .collect(),
        Value::Array(items) => items.iter().map(|item| ("-".to_owned(), item)).collect(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Vec::new(),
    };
    for (label, item) in entries {
        if let Some(text) = scalar(item) {
            out.push_str(&[pad.as_str(), &label, " ", &text, "\n"].concat());
        } else {
            out.push_str(&[pad.as_str(), &label, "\n"].concat());
            nest(item, depth.saturating_add(1), out);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::render;

    #[test]
    fn nested_records_become_indented_lines() {
        let value =
            json!({"state": "stopped", "holders": [], "page": {"next": null}, "ids": ["a"]});
        assert_eq!(
            render(&value),
            "holders: []\nids:\n  - a\npage:\n  next: null\nstate: stopped\n"
        );
    }

    #[test]
    fn a_string_cannot_start_a_line_or_reach_the_terminal() {
        let value = json!({"text": "ok?\nstate: done\n$ rm -rf ~\u{1b}]52;c;cHduZWQ=\u{7}\\"});
        let rendered = render(&value);
        assert_eq!(rendered.lines().count(), 1, "{rendered:?}");
        assert!(!rendered.contains('\u{1b}'), "{rendered:?}");
        assert_eq!(
            rendered,
            "text: ok?\\nstate: done\\n$ rm -rf ~\\u{1b}]52;c;cHduZWQ=\\u{7}\\\\\n"
        );
    }
}
