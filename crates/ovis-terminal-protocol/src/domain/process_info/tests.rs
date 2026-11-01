use proptest::prelude::*;
use serde_json::json;

use super::{Observed, ProcessRoles, ProcessSample, UnsupportedReason};
use crate::rpc::parse_request;
use crate::schema::decode_params;
use crate::schema::ids::{ProcessPid, StartToken, TerminalId};
use crate::schema::methods::terminal::{TerminalProcessInfoParams, TerminalProcessInfoResult};
use crate::schema::records::ProcessRecord;
use crate::schema::scalar::{Incarnation, OpenEnum};

const REASONS: [UnsupportedReason; 7] = [
    UnsupportedReason::Platform,
    UnsupportedReason::UndeclaredRole,
    UnsupportedReason::ProcessUnavailable,
    UnsupportedReason::Raced,
    UnsupportedReason::Permission,
    UnsupportedReason::Encoding,
    UnsupportedReason::Limit,
];

fn sample(pid: u32, token: &str) -> ProcessSample {
    let mut sample = ProcessSample::new(
        ProcessPid::new(pid),
        StartToken::new(token.to_owned()),
        UnsupportedReason::Platform,
    );
    sample.argv = Observed::known(vec!["sh".to_owned()]);
    sample
}

fn launcher(pid: u32, token: &str) -> Result<ProcessRecord, serde_json::Error> {
    serde_json::from_value(json!({
        "incarnation": "1", "pid": pid, "start_token": token,
        "argv": ["sh"], "cwd": null, "started_ms": "1"
    }))
}

fn observed<T: Clone>(value: Option<T>, reason: usize) -> Observed<T> {
    value.map_or_else(
        || {
            Observed::unsupported(
                REASONS
                    .get(reason)
                    .copied()
                    .unwrap_or(UnsupportedReason::Platform),
            )
        },
        Observed::known,
    )
}

fn roles() -> impl Strategy<Value = ProcessRoles> {
    let process = (
        any::<u32>(),
        "[ -~]{1,40}",
        proptest::option::of("[ -~]{0,8}"),
        0..7_usize,
    );
    let one = (proptest::option::of(process.clone()), 0..7_usize);
    let many = (
        proptest::option::of(proptest::collection::vec(process, 0..4)),
        0..7_usize,
    );
    (one.clone(), one, many.clone(), many).prop_map(|(shell, launcher, fg, bg)| {
        let build = |(pid, token, cwd, reason): (u32, String, Option<String>, usize)| {
            let mut sample = sample(pid, &token);
            sample.cwd = observed(cwd, reason);
            sample
        };
        let list =
            |group: Option<Vec<_>>| group.map(|items| items.into_iter().map(build).collect());
        ProcessRoles {
            shell: observed(shell.0.map(build), shell.1),
            launcher: observed(launcher.0.map(build), launcher.1),
            foreground: observed(list(fg.0), fg.1),
            background: observed(list(bg.0), bg.1),
        }
    })
}

proptest! {
    #[test]
    fn every_role_shape_round_trips_through_json(roles in roles()) {
        let wire = serde_json::to_string(&roles).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let back: ProcessRoles =
            serde_json::from_str(&wire).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, roles);
    }
}

#[test]
fn observed_fields_carry_their_state_and_reason_on_the_wire() -> Result<(), serde_json::Error> {
    let known = serde_json::to_value(Observed::known(sample(u32::MAX, "linux:b:7")))?;
    assert_eq!(known.pointer("/state"), Some(&json!("known")));
    assert_eq!(known.pointer("/value/pid"), Some(&json!(u32::MAX)));
    assert_eq!(known.pointer("/value/cwd/reason"), Some(&json!("platform")));
    let unsupported = serde_json::to_value(Observed::<ProcessSample>::unsupported(
        UnsupportedReason::UndeclaredRole,
    ))?;
    assert_eq!(
        unsupported,
        json!({"state": "unsupported", "reason": "undeclared_role"})
    );
    let later: Observed<ProcessSample> =
        serde_json::from_value(json!({"state": "unsupported", "reason": "sandboxed"}))?;
    assert_eq!(
        later,
        Observed::Unsupported {
            reason: OpenEnum::Unknown("sandboxed".to_owned())
        }
    );
    Ok(())
}

#[test]
fn an_unsupported_foreground_is_absent_from_supported_fields_not_reported_as_exit()
-> Result<(), serde_json::Error> {
    let mut roles = ProcessRoles::unsupported(UnsupportedReason::Platform);
    roles.launcher = Observed::known(sample(41, "windows-created:9"));
    let result = TerminalProcessInfoResult::new(
        TerminalId::new("terminal:1".to_owned()),
        launcher(41, "windows-created:9")?,
        roles,
    );
    assert_eq!(
        result.supported_fields,
        ["launcher", "pid", "start_token", "argv"]
    );
    assert_eq!(result.foreground, None);
    assert!(result.roles.launcher.value().is_some());
    Ok(())
}

#[test]
fn the_legacy_foreground_names_the_launcher_only_with_its_exact_start_token()
-> Result<(), serde_json::Error> {
    let record = launcher(41, "linux:b:7")?;
    let with = |member: ProcessSample| {
        let mut roles = ProcessRoles::unsupported(UnsupportedReason::UndeclaredRole);
        roles.launcher = Observed::known(sample(41, "linux:b:7"));
        roles.foreground = Observed::known(vec![member]);
        TerminalProcessInfoResult::new(
            TerminalId::new("terminal:1".to_owned()),
            record.clone(),
            roles,
        )
    };
    assert_eq!(
        with(sample(41, "linux:b:7")).foreground,
        Some(record.clone())
    );
    assert_eq!(with(sample(41, "linux:b:8")).foreground, None);
    assert_eq!(with(sample(42, "linux:b:7")).foreground, None);
    Ok(())
}

#[test]
fn a_misnamed_target_field_is_invalid_params_at_that_field() -> Result<(), String> {
    let request = parse_request(
        r#"{"jsonrpc":"2.0","id":"r","method":"terminal.process_info","params":{"pane_typo":"p2"}}"#,
    )
    .map_err(|error| error.to_string())?;
    let refused = decode_params::<TerminalProcessInfoParams>(&request)
        .err()
        .ok_or("a misnamed target must not decode")?;
    assert_eq!(refused.kind(), "invalid_params");
    assert_eq!(refused.field_path(), "params.pane_typo");
    Ok(())
}

#[test]
fn expected_incarnations_are_exact_decimal_strings() -> Result<(), serde_json::Error> {
    let params = |incarnation| json!({"target": {"name": "proc"}, "expect": {"process_incarnation": incarnation}});
    let decoded: TerminalProcessInfoParams =
        serde_json::from_value(params(json!("18446744073709551615")))?;
    let expect = decoded.expect.clone().unwrap_or_default();
    assert_eq!(expect.process_incarnation, Some(Incarnation::new(u64::MAX)));
    assert_eq!(
        serde_json::to_value(&decoded)?.pointer("/expect/process_incarnation"),
        Some(&json!("18446744073709551615"))
    );
    assert!(serde_json::from_value::<TerminalProcessInfoParams>(params(json!(3))).is_err());
    let status = json!({"target": {"name": "proc"}, "expect": {"status_seq": "1"}});
    assert!(serde_json::from_value::<TerminalProcessInfoParams>(status).is_err());
    Ok(())
}
