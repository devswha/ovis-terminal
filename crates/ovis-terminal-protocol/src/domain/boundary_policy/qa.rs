use std::collections::BTreeMap;

use proptest::prelude::*;

use super::{
    BoundaryViolation, EndpointAllowlist, LaunchArgv, LaunchEnvironment, Malformation,
    PayloadLimit, RelativePath, StoreRef,
};
use crate::schema::methods::machine::MachineTransport;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn shell_metacharacters_stay_literal_argv_bytes() {
    let argv = strings(&["/bin/echo", "a;b", "$(id)", "`x`", "c|d", "e && f", "'q'"]);

    let parsed = LaunchArgv::parse(&argv).map(|argv| argv.as_slice().to_vec());

    assert_eq!(parsed.ok(), Some(argv));
}

#[test]
fn an_argv_without_a_program_or_with_a_nul_is_malformed() {
    let kinds = [
        LaunchArgv::parse(&[]),
        LaunchArgv::parse(&strings(&["", "x"])),
        LaunchArgv::parse(&strings(&["/bin/echo", "a\0b"])),
    ]
    .map(|parsed| parsed.err().map(|error| (error.kind(), error.field_path())));

    assert_eq!(kinds, [Some(("invalid_params", "params.program.argv")); 3]);
}

#[test]
fn argv_limits_hold_at_their_boundary_and_refuse_one_past_it() {
    let max = usize::try_from(PayloadLimit::ArgvElementBytes.value()).unwrap_or(usize::MAX);
    let at_limit = vec!["p".to_owned(), "x".repeat(max)];
    let past_limit = vec!["p".to_owned(), "x".repeat(max + 1)];
    let elements = vec!["p".to_owned(); 4097];

    assert!(LaunchArgv::parse(&at_limit).is_ok());
    let refused = [past_limit, elements].map(|argv| match LaunchArgv::parse(&argv) {
        Err(BoundaryViolation::TooLarge { limit, .. }) => Some(limit),
        _ => None,
    });
    assert_eq!(
        refused,
        [
            Some(PayloadLimit::ArgvElementBytes),
            Some(PayloadLimit::ArgvElements)
        ]
    );
}

#[test]
fn the_total_argv_size_is_bounded_even_when_every_element_fits() {
    let element = "x".repeat(32 * 1024);
    let argv: Vec<String> = std::iter::once("p".to_owned())
        .chain(std::iter::repeat_n(element, 8))
        .collect();

    let refused = LaunchArgv::parse(&argv).err();

    assert!(matches!(
        refused,
        Some(BoundaryViolation::TooLarge {
            limit: PayloadLimit::ArgvTotalBytes,
            ..
        })
    ));
}

#[test]
fn runtime_variables_cannot_be_set_by_a_caller_in_any_letter_case() {
    let refused = [
        "OVIS_TERMINAL_PROCESS_TOKEN",
        "ovis_terminal_token",
        "ovıs_termınal_token",
    ]
    .map(|key| LaunchEnvironment::parse(&env(&[(key, "spoof")])).err())
    .map(|error| error.map(|error| error.kind()));
    let ordinary = LaunchEnvironment::parse(&env(&[("ROLE", "build"), ("OVIS", "x")]));

    assert_eq!(refused, [Some("forbidden"); 3]);
    assert_eq!(ordinary.map(|env| env.as_map().len()).ok(), Some(2));
}

#[test]
fn malformed_environment_names_and_values_are_refused() {
    let kinds = [
        env(&[("", "x")]),
        env(&[("A=B", "x")]),
        env(&[("A", "x\0y")]),
    ]
    .map(|env| match LaunchEnvironment::parse(&env) {
        Err(BoundaryViolation::Malformed { reason, .. }) => Some(reason),
        _ => None,
    });

    assert_eq!(
        kinds,
        [
            Some(Malformation::EnvKey),
            Some(Malformation::EnvKey),
            Some(Malformation::NulByte)
        ]
    );
}

#[test]
fn a_store_reference_may_not_climb_restart_at_a_root_or_use_a_backslash() {
    let refused = [
        "../../secret",
        "a/../../b",
        "/etc/passwd",
        "a\\..\\b",
        "./a",
    ]
    .map(|text| StoreRef::parse(text).err().map(|error| error.kind()));

    assert_eq!(refused, [Some("forbidden"); 5]);
}

#[test]
fn a_nested_store_reference_keeps_its_text_and_components() {
    let parsed = StoreRef::parse("claude/projects/demo");

    assert_eq!(
        parsed.as_ref().map(StoreRef::as_str).ok(),
        Some("claude/projects/demo")
    );
    assert_eq!(
        parsed
            .map(|store| store.path().as_path().components().count())
            .ok(),
        Some(3)
    );
}

#[test]
fn an_oversized_store_reference_reports_its_bound() {
    let refused = StoreRef::parse(&"a".repeat(4097)).err();
    let wire = refused.as_ref().map(BoundaryViolation::to_boundary);
    let data = wire.map(|error| serde_json::to_value(error.data()).ok());

    assert_eq!(refused.map(|error| error.kind()), Some("too_large"));
    let data = data.flatten().unwrap_or_default();
    let field = |pointer: &str| data.pointer(pointer).cloned().unwrap_or_default();
    assert_eq!(field("/kind"), "too_large");
    assert_eq!(field("/field_path"), "params.store");
    assert_eq!(field("/bound/limit"), "path_bytes");
    assert_eq!(field("/bound/value"), "4096");
}

#[test]
fn an_allowlist_admits_only_listed_ssh_hosts_and_no_argv_transport() {
    let allow = EndpointAllowlist::hosts(["build.example".to_owned()]);
    let outcomes = [
        MachineTransport::Ssh("dev@build.example".to_owned()),
        MachineTransport::Ssh("build.example".to_owned()),
        MachineTransport::Ssh("dev@BUILD.Example".to_owned()),
        MachineTransport::Ssh("dev@evil.example".to_owned()),
        MachineTransport::Argv(strings(&["/usr/bin/nc", "build.example", "22"])),
    ]
    .map(|transport| allow.admit(&transport).err().map(|error| error.kind()));

    assert_eq!(
        outcomes,
        [None, None, None, Some("forbidden"), Some("forbidden")]
    );
    let open = EndpointAllowlist::default().admit(&MachineTransport::Ssh("any".to_owned()));
    assert!(open.is_ok());
}

proptest! {
    #[test]
    fn any_parsed_relative_path_has_only_normal_components(text in "[a-z./\\\\]{1,24}") {
        if let Ok(path) = RelativePath::parse("params.path", &text) {
            let normal = path
                .as_path()
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_)));
            prop_assert!(normal);
            prop_assert!(!text.split('/').any(|part| part == ".."));
        }
    }

    #[test]
    fn a_refusal_never_echoes_the_refused_value(secret in "[A-Za-z0-9]{12,40}") {
        let refusals = [
            LaunchEnvironment::parse(&env(&[("OVIS_TERMINAL_X", &secret)])).err(),
            StoreRef::parse(&format!("../{secret}")).err(),
            LaunchArgv::parse(&[format!("{secret}\0")]).err(),
        ];
        for refusal in refusals.into_iter().flatten() {
            let wire = serde_json::to_string(refusal.to_boundary().data()).unwrap_or_default();
            prop_assert!(!wire.contains(&secret));
            prop_assert!(!refusal.to_string().contains(&secret));
        }
    }
}
