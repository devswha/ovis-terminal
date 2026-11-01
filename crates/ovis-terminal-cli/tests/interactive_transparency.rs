//! Plan todo 62: the attach client draws nothing of its own. A probe writes C0 and C1
//! controls, SGR, DECSET/DECRST and OSC sequences; what the client puts on the outer
//! terminal is, byte for byte, what the server sent a reference viewer of the same screen,
//! followed only by the documented attribute reset on detach, and the server sent the
//! program's raw bytes unchanged.
#![cfg(unix)]

#[path = "attach/support.rs"]
pub mod support;

#[path = "interactive_transparency/viewer.rs"]
pub mod viewer;

use base64::Engine as _;
use serde_json::{Value, json};
use support::{Outer, Served, TestResult, WAIT};
use viewer::Viewer;

/// Prints a banner, waits for one line typed through the attach, prints the payload, then
/// waits on its terminal so it stays silent.
const PROBE: &str = r#"printf 'T62-READY'; read -r go; printf "$OVT_PAYLOAD"; read -r go"#;

/// The payload as a printf format: C0 (BS, HT, BEL, CR, LF), 8-bit C1 (CSI, IND, NEL),
/// SGR, DECSET/DECRST (alternate screen, bracketed paste, cursor) and OSC (title and a
/// hyperlink ended by ST), every mode switched back off.
const PAYLOAD: &str = r"T62-BEGIN\010\011\007\r\n\033[1;31mred\033[0m\233?25l\233?25h\204\205\033[?1049h\033[?2004h\033[?2004l\033[?1049l\033]0;t62-title\007\033]8;;https://example.invalid/t62\033\\link\033]8;;\033\\T62-END";

/// Byte classes the probe must have written, as the program's raw output shows them.
const CLASSES: [(&str, &[u8]); 8] = [
    ("C0 BS HT BEL", b"\x08\x09\x07"),
    ("C1 CSI", b"\x9b?25l"),
    ("C1 IND NEL", b"\x84\x85"),
    ("SGR", b"\x1b[1;31m"),
    ("DECSET", b"\x1b[?1049h"),
    ("DECRST", b"\x1b[?2004l"),
    ("OSC title", b"\x1b]0;t62-title\x07"),
    (
        "OSC hyperlink",
        b"\x1b]8;;https://example.invalid/t62\x1b\\",
    ),
];

/// Launches the probe on the outer terminal's 100x30 grid, so attaching never resizes it.
fn probe(served: &Served) -> TestResult<String> {
    let launch = json!({
        "op_id": uuid::Uuid::now_v7().to_string(),
        "program": {"argv": ["/bin/sh", "-c", PROBE]},
        "geometry": {"cols": 100, "rows": 30}, "cwd": "/tmp",
        "env": {"PATH": "/usr/bin:/bin", "OVT_PAYLOAD": PAYLOAD},
        "name": null, "labels": null, "parent": null, "child_policy": null, "keep": null,
        "first_message": null, "machine": null,
    });
    let run = served.call("terminal.run", &launch)?;
    let address = run.pointer("/terminal/address").and_then(Value::as_str);
    Ok(address
        .ok_or_else(|| format!("no address in {run}"))?
        .to_owned())
}

/// Waits, on the server, until the program's raw output holds `text`; the probe then blocks
/// on its terminal, so every viewer attached afterwards is painted from the same screen.
fn printed(served: &Served, (address, text): (&str, &str)) -> TestResult {
    let waited = served.call(
        "terminal.wait_output",
        &json!({
            "target": {"address": address}, "cursor": {"offset": "0"},
            "pattern": {"text": text}, "timeout_ms": WAIT.as_millis().to_string(),
        }),
    )?;
    match waited.get("outcome").and_then(Value::as_str) {
        Some("matched") => Ok(()),
        _ => Err(format!("{text} was never printed: {waited}").into()),
    }
}

/// The program's raw output from its first byte.
fn raw_output(served: &Served, address: &str) -> TestResult<Vec<u8>> {
    let read = served.call(
        "terminal.output",
        &json!({"target": {"address": address}, "from_offset": "0", "limit_bytes": "65536"}),
    )?;
    let encoded = read.get("data_b64").and_then(Value::as_str);
    let encoded = encoded.ok_or_else(|| format!("no data in {read}"))?;
    Ok(base64::engine::general_purpose::STANDARD.decode(encoded)?)
}

#[test]
fn the_attach_client_draws_exactly_the_bytes_the_server_sent_and_nothing_else() -> TestResult {
    let served = Served::start()?;
    let address = probe(&served)?;
    printed(&served, (&address, "T62-READY"))?;
    let mut outer = Outer::attach(&served, &["terminal", "attach", "--target", &address])?;
    assert!(outer.shows("T62-READY", WAIT), "the attach never painted");
    let mut reference = Viewer::observe(&served, (&address, "reference"))?;
    assert!(
        reference.until(b"T62-READY", WAIT),
        "the reference never painted"
    );
    outer.type_bytes(b"go\r")?;
    assert!(
        reference.until(b"T62-END", WAIT),
        "the payload never reached the reference"
    );
    assert!(
        outer.shows("T62-END", WAIT),
        "the payload never reached the outer terminal"
    );
    let sent = std::mem::take(&mut reference.received);
    reference.close()?;
    outer.type_bytes(b"\x1c")?;
    let drawn = outer.drawn_until_closed(WAIT)?;
    let (code, record, restored) = outer.exited()?;
    assert_eq!(code, Some(0), "{record}");
    assert!(restored, "the outer terminal settings changed");
    let raw = raw_output(&served, &address)?;
    let live = raw
        .strip_prefix(b"T62-READY".as_slice())
        .ok_or("no banner")?;
    for (class, bytes) in CLASSES {
        let written = live.windows(bytes.len()).any(|window| window == bytes);
        assert!(
            written,
            "the probe never wrote {class}: {}",
            live.escape_ascii()
        );
    }
    assert!(
        sent.ends_with(live),
        "the server changed the program's bytes:\n sent {}\n raw  {}",
        sent.escape_ascii(),
        live.escape_ascii()
    );
    assert_eq!(
        drawn.strip_prefix(sent.as_slice()),
        Some(b"\x1b[0m".as_slice()),
        "the client drew bytes the server never sent:\n drawn {}\n sent  {}",
        drawn.escape_ascii(),
        sent.escape_ascii()
    );
    served.stop()
}
