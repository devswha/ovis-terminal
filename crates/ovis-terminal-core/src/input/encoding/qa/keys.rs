//! Named keys encode for the program's negotiated keyboard modes, all or nothing.

use ovis_terminal_core::input::encoding::{EncodingError, InputRequest, ModeSnapshot, encode};
use ovis_terminal_protocol::schema::methods::input::Submit;
use proptest::prelude::*;

use super::fixture::vt;
use super::{TestResult, rejection};

const NAMES: &[&str] = &[
    "enter",
    "esc",
    "tab",
    "shift+tab",
    "backspace",
    "a",
    "shift+a",
    "7",
    "ctrl+c",
    "alt+x",
    "ctrl+backslash",
    "space",
    "up",
    "left",
    "home",
    "end",
    "insert",
    "delete",
    "pageup",
    "pagedown",
    "f1",
    "f4",
    "f5",
    "f12",
    "ctrl+up",
    "shift+f5",
    "alt+enter",
];

fn keys(output: &[u8], names: &[&str]) -> TestResult<Vec<String>> {
    let snapshot = ModeSnapshot::capture(&vt(output)?);
    names
        .iter()
        .map(|name| {
            let encoded = encode(&snapshot, InputRequest::Keys(&[(*name).to_owned()]))?;
            Ok(String::from_utf8(encoded.bytes().to_vec())?)
        })
        .collect()
}

#[test]
fn names_encode_like_a_terminal_in_normal_cursor_mode() -> TestResult {
    assert_eq!(
        keys(b"", NAMES)?,
        [
            "\r",
            "\x1b",
            "\t",
            "\x1b[Z",
            "\x7f",
            "a",
            "A",
            "7",
            "\x03",
            "\x1bx",
            "\x1c",
            " ",
            "\x1b[A",
            "\x1b[D",
            "\x1b[H",
            "\x1b[F",
            "\x1b[2~",
            "\x1b[3~",
            "\x1b[5~",
            "\x1b[6~",
            "\x1bOP",
            "\x1bOS",
            "\x1b[15~",
            "\x1b[24~",
            "\x1b[1;5A",
            "\x1b[15;2~",
            "\x1b\r",
        ]
    );
    Ok(())
}

#[test]
fn application_cursor_mode_changes_unmodified_arrows_only() -> TestResult {
    assert_eq!(
        keys(b"\x1b[?1h", &["up", "left", "home", "ctrl+up", "f1"])?,
        ["\x1bOA", "\x1bOD", "\x1bOH", "\x1b[1;5A", "\x1bOP"]
    );
    Ok(())
}

#[test]
fn kitty_disambiguation_encodes_ambiguous_keys_as_csi_u() -> TestResult {
    assert_eq!(
        keys(
            b"\x1b[>1u",
            &[
                "esc",
                "ctrl+c",
                "ctrl+backslash",
                "alt+x",
                "ctrl+shift+a",
                "shift+tab",
                "enter",
                "a",
                "up"
            ]
        )?,
        [
            "\x1b[27u",
            "\x1b[99;5u",
            "\x1b[92;5u",
            "\x1b[120;3u",
            "\x1b[97;6u",
            "\x1b[9;2u",
            "\r",
            "a",
            "\x1b[A"
        ]
    );
    Ok(())
}

#[test]
fn keyboard_protocols_this_runtime_cannot_encode_are_refused() -> TestResult {
    for output in [&b"\x1b[>8u"[..], b"\x1b[?9001h"] {
        let snapshot = ModeSnapshot::capture(&vt(output)?);
        let error = rejection(encode(&snapshot, InputRequest::Keys(&["enter".to_owned()])))?;
        assert!(
            matches!(error, EncodingError::KeyboardProtocol { revision } if revision == snapshot.revision())
        );
        assert_eq!(error.kind(), "invalid_encoding");
    }
    Ok(())
}

#[test]
fn unknown_names_and_unencodable_chords_are_typed() -> TestResult {
    let snapshot = ModeSnapshot::capture(&vt(b"")?);
    for name in [
        "hello",
        "hyper+x",
        "f13",
        "ctrl+",
        "",
        "ctrl+ctrl+c",
        "Enter\n",
    ] {
        let error = rejection(encode(&snapshot, InputRequest::Keys(&[name.to_owned()])))?;
        assert!(
            matches!(&error, EncodingError::UnknownKey { key } if key == name),
            "{name:?}"
        );
        assert_eq!(error.kind(), "unknown_key");
    }
    for name in [
        "shift+1",
        "ctrl+enter",
        "ctrl+shift+a",
        "shift+esc",
        "ctrl+%",
    ] {
        let error = rejection(encode(&snapshot, InputRequest::Keys(&[name.to_owned()])))?;
        assert!(
            matches!(&error, EncodingError::KeyUnencodable { key, .. } if key == name),
            "{name:?}"
        );
        assert_eq!(error.kind(), "invalid_encoding");
    }
    let empty = rejection(encode(&snapshot, InputRequest::Keys(&[])))?;
    assert_eq!(empty.kind(), "invalid_params");
    Ok(())
}

#[test]
fn a_submit_chord_is_appended_after_the_literal_in_one_entry() -> TestResult {
    let snapshot = ModeSnapshot::capture(&vt(b"")?);
    let submit = Submit::Keys(vec!["ctrl+j".to_owned(), "enter".to_owned()]);
    let encoded = encode(
        &snapshot,
        InputRequest::Text {
            text: "ls",
            submit: &submit,
        },
    )?;
    assert_eq!(encoded.bytes(), b"ls\n\r");
    assert_eq!(encoded.evidence().submit_bytes.get(), 2);
    let bad = Submit::Keys(vec!["enter".to_owned(), "nope".to_owned()]);
    let error = rejection(encode(
        &snapshot,
        InputRequest::Text {
            text: "ls",
            submit: &bad,
        },
    ))?;
    assert_eq!(error.kind(), "unknown_key");
    Ok(())
}

proptest! {
    #[test]
    fn a_key_list_is_the_concatenation_of_its_keys_or_nothing(
        picks in prop::collection::vec(prop::sample::select(NAMES), 1..12),
        bad_at in any::<prop::sample::Index>(),
        poison in any::<bool>(),
    ) {
        let snapshot = ModeSnapshot::capture(&vt(b"").map_err(|e| TestCaseError::fail(e.to_string()))?);
        let mut names: Vec<String> = picks.iter().map(|name| (*name).to_owned()).collect();
        let one = |name: &str| encode(&snapshot, InputRequest::Keys(&[name.to_owned()])).map(|e| e.bytes().to_vec());
        let expected: Vec<u8> = picks
            .iter()
            .map(|name| one(name))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| TestCaseError::fail(e.to_string()))?
            .concat();
        if poison {
            names.insert(bad_at.index(names.len().saturating_add(1)), "nope".to_owned());
            let result = encode(&snapshot, InputRequest::Keys(&names));
            let poisoned = matches!(result, Err(EncodingError::UnknownKey { ref key }) if key == "nope");
            prop_assert!(poisoned, "an unknown name must refuse the whole list");
        } else {
            let encoded = encode(&snapshot, InputRequest::Keys(&names)).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(encoded.bytes(), &expected[..]);
        }
    }
}
