//! Literal text, keys, paste, mouse and raw bytes encoded against one frozen mode revision.

use std::error::Error;

use fixture::{Fixture, OPERATOR, PLAIN_INTERACTOR, vt};
use ovis_terminal_core::input::encoding::{
    EncodingError, InputKind, InputRequest, ModeSnapshot, encode,
};
use ovis_terminal_protocol::schema::methods::input::{BracketedPolicy, Submit};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

#[path = "../src/input/encoding/qa/evidence.rs"]
mod evidence;
#[path = "../src/input/encoding/qa/fixture.rs"]
pub mod fixture;
#[path = "../src/input/encoding/qa/keys.rs"]
mod keys;
#[path = "../src/input/encoding/qa/literal.rs"]
mod literal;
#[path = "../src/input/encoding/qa/mouse.rs"]
mod mouse;

const START: &[u8] = b"\x1b[200~";
const END: &[u8] = b"\x1b[201~";

fn rejection(result: Result<impl std::fmt::Debug, EncodingError>) -> TestResult<EncodingError> {
    match result {
        Ok(encoded) => Err(format!("expected a typed rejection, encoded {encoded:?}").into()),
        Err(error) => Ok(error),
    }
}

#[test]
fn happy() -> TestResult {
    // Given a program that enabled bracketed paste and SGR mouse reports.
    let mut terminal = vt(b"\x1b[?2004h\x1b[?1000h\x1b[?1006h")?;
    let modes = ModeSnapshot::capture(&terminal);
    let fixture = Fixture::new()?;
    let operator = fixture.raw_grant(&fixture.session(OPERATOR)?)?;

    // When multiline text is sent with Enter, a mouse report and unknown raw bytes are admitted.
    let text = "first\nsecond; rm -rf ~\tdone";
    let sent = encode(
        &modes,
        InputRequest::Text {
            text,
            submit: &Submit::Enter,
        },
    )?;
    let report = b"\x1b[<0;10;5M\x1b[<0;10;5m";
    let mouse = encode(&modes, InputRequest::Mouse(report))?;
    let raw_bytes = b"\x1b[999;unknown\x07\xff\x00";
    let raw = encode(
        &modes,
        InputRequest::Raw {
            bytes: raw_bytes,
            grant: &operator,
        },
    )?;

    // Then the text is one bracketed literal followed by Enter in the same entry.
    let bytes = sent.bytes();
    let (submit, literal) = bytes.split_last().ok_or("send produced no bytes")?;
    assert_eq!(*submit, b'\r');
    let payload = literal
        .strip_prefix(START)
        .and_then(|rest| rest.strip_suffix(END))
        .ok_or("send is not framed by bracketed-paste markers")?;
    assert_eq!(payload, text.as_bytes());
    let proof = sent.evidence();
    assert_eq!(proof.kind, InputKind::Text);
    assert!(proof.bracketed);
    assert_eq!(proof.mode_revision, terminal.mode_revision());
    assert_eq!(proof.literal_bytes.get(), u64::try_from(literal.len())?);
    assert_eq!(proof.submit_bytes.get(), 1);
    // And mouse and raw bytes are delivered byte-identical.
    assert_eq!(mouse.bytes(), report);
    assert_eq!(raw.bytes(), raw_bytes);
    assert_eq!(raw.evidence().kind, InputKind::Raw);
    sent.ensure_current(terminal.mode_revision())?;
    println!("send evidence: {}", serde_json::to_string(proof)?);
    println!("send bytes: {:?}", String::from_utf8_lossy(bytes));

    // And the evidence fences the write: a later mode change makes it stale.
    terminal.feed(b"\x1b[?2004l");
    assert_eq!(
        rejection(sent.ensure_current(terminal.mode_revision()))?.kind(),
        "mode_mismatch"
    );
    Ok(())
}

#[test]
fn failure() -> TestResult {
    // Given a program without bracketed paste, and one with it.
    let plain = ModeSnapshot::capture(&vt(b"")?);
    let bracketed = ModeSnapshot::capture(&vt(b"\x1b[?2004h")?);
    let fixture = Fixture::new()?;

    // When multiline text is sent with submit none in plain mode.
    let refused = rejection(encode(
        &plain,
        InputRequest::Text {
            text: "first\nsecond",
            submit: &Submit::None,
        },
    ))?;
    // Then it is refused before any byte exists, naming the mode revision it was checked at.
    assert_eq!(refused.kind(), "paste_refused");
    assert!(matches!(
        refused,
        EncodingError::NeedsBracketedPaste { revision } if revision == plain.revision()
    ));
    println!("multiline in plain mode: {} ({refused})", refused.kind());

    // When a payload smuggles a paste terminator or control bytes, even with bracketed paste.
    for text in [
        "ok\x1b[201~\rrm -rf ~",
        "ok\u{9b}201~x",
        "bell\x07",
        "esc\x1b[A",
        "nul\0",
    ] {
        let error = rejection(encode(
            &bracketed,
            InputRequest::Paste {
                text,
                policy: BracketedPolicy::Auto,
            },
        ))?;
        // Then it is invalid_encoding, never a partial write.
        assert_eq!(error.kind(), "invalid_encoding", "{text:?}");
        println!("{text:?}: {} ({error})", error.kind());
    }
    let marker = rejection(encode(
        &bracketed,
        InputRequest::Text {
            text: "a\x1b[201~b",
            submit: &Submit::Enter,
        },
    ))?;
    assert!(matches!(marker, EncodingError::PasteMarker { at } if at.get() == 1));

    // When raw bytes come from a principal without the raw grant.
    let denied = rejection(fixture.raw_grant(&fixture.session(PLAIN_INTERACTOR)?))?;
    // Then the grant is refused as raw_not_authorized.
    assert_eq!(denied.kind(), "raw_not_authorized");
    Ok(())
}
