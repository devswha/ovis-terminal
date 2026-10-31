//! Literal text stays data: framing follows the frozen paste mode, never the submit choice.

use ovis_terminal_core::input::encoding::{InputRequest, ModeSnapshot, encode};
use ovis_terminal_protocol::schema::methods::input::{BracketedPolicy, Submit};
use proptest::prelude::*;

use super::fixture::vt;
use super::{END, START, TestResult, rejection};

fn modes(output: &[u8]) -> TestResult<ModeSnapshot> {
    Ok(ModeSnapshot::capture(&vt(output)?))
}

fn case(error: impl std::fmt::Display) -> TestCaseError {
    TestCaseError::fail(error.to_string())
}

fn submits() -> [Submit; 3] {
    [
        Submit::None,
        Submit::Enter,
        Submit::Keys(vec!["enter".to_owned()]),
    ]
}

fn literal_char() -> impl Strategy<Value = char> {
    prop_oneof![
        8 => any::<char>().prop_filter("literal text has no control characters", |c| !c.is_control()),
        1 => prop::sample::select(vec!['\n', '\r', '\t']),
    ]
}

fn control_char() -> impl Strategy<Value = char> {
    any::<char>().prop_filter("only controls other than line breaks and tab", |c| {
        c.is_control() && !matches!(c, '\n' | '\r' | '\t')
    })
}

proptest! {
    #[test]
    fn bracketed_text_round_trips_with_submit_outside_the_frame(
        chars in prop::collection::vec(literal_char(), 1..64),
        enter in any::<bool>(),
    ) {
        let text: String = chars.into_iter().collect();
        let submit = if enter { Submit::Enter } else { Submit::None };
        let encoded = encode(&modes(b"\x1b[?2004h").map_err(case)?, InputRequest::Text { text: &text, submit: &submit })
            .map_err(case)?;
        let bytes = encoded.bytes();
        let framed = text.contains(['\n', '\r', '\t']);
        let (literal, tail) = bytes.split_at(usize::try_from(encoded.evidence().literal_bytes.get()).map_err(case)?);
        prop_assert_eq!(tail, if enter { &b"\r"[..] } else { &b""[..] });
        let payload = if framed {
            literal.strip_prefix(START).and_then(|rest| rest.strip_suffix(END))
        } else {
            Some(literal)
        };
        prop_assert_eq!(payload, Some(text.as_bytes()));
        prop_assert_eq!(encoded.evidence().bracketed, framed);
    }

    #[test]
    fn any_control_character_is_invalid_encoding_in_every_mode_and_submit(
        before in prop::collection::vec(literal_char(), 0..16),
        control in control_char(),
        after in prop::collection::vec(literal_char(), 0..16),
        bracketed in any::<bool>(),
    ) {
        let text: String = before.into_iter().chain([control]).chain(after).collect();
        let snapshot = modes(if bracketed { b"\x1b[?2004h" } else { b"" }).map_err(case)?;
        for submit in submits() {
            let error = rejection(encode(&snapshot, InputRequest::Text { text: &text, submit: &submit }))
                .map_err(case)?;
            prop_assert_eq!(error.kind(), "invalid_encoding");
        }
        for policy in [BracketedPolicy::Auto, BracketedPolicy::Require, BracketedPolicy::Never] {
            let error = rejection(encode(&snapshot, InputRequest::Paste { text: &text, policy }))
                .map_err(case)?;
            prop_assert_eq!(error.kind(), "invalid_encoding");
        }
    }

    #[test]
    fn line_breaks_without_bracketed_paste_are_refused_whatever_the_submit(
        head in "[a-z ;|&$()]{0,16}",
        brk in prop::sample::select(vec!["\n", "\r", "\r\n", "\t"]),
        tail in "[a-z ;|&$()]{0,16}",
    ) {
        let text = format!("{head}{brk}{tail}");
        for submit in submits() {
            let error = rejection(encode(&modes(b"").map_err(case)?, InputRequest::Text { text: &text, submit: &submit }))
                .map_err(case)?;
            prop_assert_eq!(error.kind(), "paste_refused");
        }
    }
}

#[test]
fn paste_policy_frames_only_where_the_program_asked_for_bracketed_paste() -> TestResult {
    let plain = modes(b"")?;
    let bracketed = modes(b"\x1b[?2004h")?;
    let paste = |snapshot: &ModeSnapshot, text, policy| {
        encode(snapshot, InputRequest::Paste { text, policy })
    };

    let auto_plain = paste(&plain, "one line", BracketedPolicy::Auto)?;
    assert_eq!(auto_plain.bytes(), b"one line");
    let auto_framed = paste(&bracketed, "one line", BracketedPolicy::Auto)?;
    assert_eq!(auto_framed.bytes(), b"\x1b[200~one line\x1b[201~");
    assert!(auto_framed.evidence().bracketed);
    let never = paste(&bracketed, "one line", BracketedPolicy::Never)?;
    assert_eq!(never.bytes(), b"one line");

    for (snapshot, text, policy) in [
        (&plain, "one line", BracketedPolicy::Require),
        (&plain, "a\nb", BracketedPolicy::Auto),
        (&bracketed, "a\nb", BracketedPolicy::Never),
    ] {
        assert_eq!(
            rejection(paste(snapshot, text, policy))?.kind(),
            "paste_refused",
            "{text:?} {policy:?}"
        );
    }
    Ok(())
}

#[test]
fn an_empty_payload_is_refused_but_a_bare_enter_is_a_submission() -> TestResult {
    let plain = modes(b"")?;
    let empty = rejection(encode(
        &plain,
        InputRequest::Text {
            text: "",
            submit: &Submit::None,
        },
    ))?;
    assert_eq!(empty.kind(), "invalid_params");
    let paste = rejection(encode(
        &plain,
        InputRequest::Paste {
            text: "",
            policy: BracketedPolicy::Auto,
        },
    ))?;
    assert_eq!(paste.kind(), "invalid_params");
    let enter = encode(
        &plain,
        InputRequest::Text {
            text: "",
            submit: &Submit::Enter,
        },
    )?;
    assert_eq!(enter.bytes(), b"\r");
    assert_eq!(enter.evidence().literal_bytes.get(), 0);
    Ok(())
}
