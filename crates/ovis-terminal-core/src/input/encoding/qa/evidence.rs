//! Encoding evidence carries its mode revision as a decimal string; raw bytes stay exact.

use ovis_terminal_core::input::encoding::{
    EncodingEvidence, InputKind, InputRequest, ModeSnapshot, encode,
};
use ovis_terminal_protocol::schema::error_kinds::DomainErrorKind;
use ovis_terminal_protocol::schema::methods::input::{BracketedPolicy, Submit};
use ovis_terminal_vt::ModeRevision;
use proptest::prelude::*;
use serde_json::json;

use super::fixture::{Fixture, NOW, OPERATOR, PLAIN_INTERACTOR, vt};
use super::{TestResult, rejection};

#[test]
fn the_maximal_mode_revision_round_trips_as_a_decimal_string() -> TestResult {
    let max: ModeRevision = serde_json::from_value(json!("18446744073709551615"))?;
    let snapshot = ModeSnapshot::new(max, *ModeSnapshot::capture(&vt(b"\x1b[?2004h")?).modes());
    let encoded = encode(
        &snapshot,
        InputRequest::Paste {
            text: "x",
            policy: BracketedPolicy::Require,
        },
    )?;
    assert_eq!(encoded.receipt_revision().get(), u64::MAX);
    let wire = serde_json::to_value(encoded.evidence())?;
    assert_eq!(
        wire.get("mode_revision"),
        Some(&json!("18446744073709551615"))
    );
    assert_eq!(wire.get("literal_bytes"), Some(&json!("13")));
    assert_eq!(wire.get("kind"), Some(&json!("paste")));
    let back: EncodingEvidence = serde_json::from_value(wire.clone())?;
    assert_eq!(&back, encoded.evidence());
    let mut numeric = wire;
    numeric
        .as_object_mut()
        .ok_or("evidence is not an object")?
        .insert("mode_revision".to_owned(), json!(1));
    assert!(serde_json::from_value::<EncodingEvidence>(numeric).is_err());
    Ok(())
}

#[test]
fn every_refusal_kind_is_a_published_protocol_token() -> TestResult {
    let plain = ModeSnapshot::capture(&vt(b"")?);
    let refusals = [
        encode(
            &plain,
            InputRequest::Text {
                text: "a\nb",
                submit: &Submit::None,
            },
        ),
        encode(
            &plain,
            InputRequest::Text {
                text: "\x1b",
                submit: &Submit::None,
            },
        ),
        encode(&plain, InputRequest::Keys(&["nope".to_owned()])),
        encode(&plain, InputRequest::Mouse(b"\x1b[M !!")),
    ];
    let mut kinds = Vec::new();
    for refusal in refusals {
        let error = rejection(refusal)?;
        kinds.push(error.kind());
    }
    let encoded = encode(&plain, InputRequest::Keys(&["a".to_owned()]))?;
    let changed = ModeSnapshot::capture(&vt(b"\x1b[?1h")?);
    kinds.push(rejection(encoded.ensure_current(changed.revision()))?.kind());
    let fixture = Fixture::new()?;
    kinds.push(rejection(fixture.raw_grant(&fixture.session(PLAIN_INTERACTOR)?))?.kind());
    for kind in &kinds {
        let parsed: DomainErrorKind = serde_json::from_value(json!(kind))?;
        assert_eq!(serde_json::to_value(parsed)?, json!(kind));
    }
    assert_eq!(
        kinds,
        [
            "paste_refused",
            "invalid_encoding",
            "unknown_key",
            "invalid_encoding",
            "mode_mismatch",
            "raw_not_authorized"
        ]
    );
    Ok(())
}

#[test]
fn a_raw_grant_is_rechecked_at_the_write_boundary() -> TestResult {
    let fixture = Fixture::new()?;
    let grant = fixture.raw_grant(&fixture.session(OPERATOR)?)?;
    grant.recheck(fixture.authority(), NOW)?;
    let snapshot = ModeSnapshot::capture(&vt(b"")?);
    let empty = rejection(encode(
        &snapshot,
        InputRequest::Raw {
            bytes: b"",
            grant: &grant,
        },
    ))?;
    assert_eq!(empty.kind(), "invalid_params");
    Ok(())
}

proptest! {
    #[test]
    fn authorized_raw_bytes_are_never_rewritten_or_dropped(
        bytes in prop::collection::vec(any::<u8>(), 1..256),
        bracketed in any::<bool>(),
    ) {
        let fail = |e: Box<dyn std::error::Error>| TestCaseError::fail(e.to_string());
        let fixture = Fixture::new().map_err(fail)?;
        let session = fixture.session(OPERATOR).map_err(fail)?;
        let grant = fixture.raw_grant(&session).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let terminal = vt(if bracketed { b"\x1b[?2004h" } else { b"" }).map_err(fail)?;
        let encoded = encode(&ModeSnapshot::capture(&terminal), InputRequest::Raw { bytes: &bytes, grant: &grant })
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(encoded.bytes(), &bytes[..]);
        prop_assert_eq!(encoded.evidence().kind, InputKind::Raw);
        prop_assert!(!encoded.evidence().bracketed);
    }
}
