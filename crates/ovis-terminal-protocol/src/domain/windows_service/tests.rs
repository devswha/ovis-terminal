use proptest::prelude::*;
use serde_json::json;

use super::{
    HolderPlacement, HostRecord, Installation, PipeName, ScmState, ServiceAccount,
    WindowsServiceRecordError, WindowsServiceStatus,
};
use crate::domain::unix_service::{BootProof, Observed, StaleReason};
use crate::schema::ids::{ProcessPid, ServiceLabel, StartToken};
use crate::schema::methods::service::ServiceRecord;

fn label() -> ServiceLabel {
    ServiceLabel::new("ovis-terminal-holder-default".to_owned())
}

fn pipe() -> PipeName {
    PipeName::parse(r"\\.\pipe\ovis-terminal-holder-S-1-5-21-7-default").unwrap()
}

fn proof() -> BootProof {
    BootProof::new(
        ProcessPid::new(4242),
        StartToken::new("windows-created:133".to_owned()),
    )
}

#[test]
fn pipe_names_are_local_and_bounded_and_nothing_else_is_accepted() {
    let longest = format!(r"\\.\pipe\{}", "a".repeat(247));
    assert_eq!(longest.chars().count(), 256);
    assert!(PipeName::parse(&longest).is_ok());
    for refused in [
        format!(r"\\.\pipe\{}", "a".repeat(248)),
        r"\\.\pipe\".to_owned(),
        r"\\.\pipe\a\b".to_owned(),
        r"\\server\pipe\a".to_owned(),
        "C:\\Users\\a\\herdr.sock".to_owned(),
        "\\\\.\\pipe\\a\0".to_owned(),
    ] {
        assert_eq!(
            PipeName::parse(&refused),
            Err(WindowsServiceRecordError::InvalidPipeName),
            "{refused:?}"
        );
    }
}

proptest! {
    #[test]
    fn a_pipe_name_round_trips_through_json_exactly(name in "[A-Za-z0-9._-]{1,200}") {
        let parsed = PipeName::parse(&format!(r"\\.\pipe\{name}")).unwrap();
        let wire = serde_json::to_string(&parsed).unwrap();
        prop_assert_eq!(serde_json::from_str::<PipeName>(&wire).unwrap(), parsed);
    }
}

#[test]
fn accounts_are_named_users_never_built_in_service_accounts() {
    assert_eq!(
        ServiceAccount::parse(r".\ovis").map(|a| a.as_str().to_owned()),
        Ok(r".\ovis".to_owned())
    );
    assert!(ServiceAccount::parse(r"CORP\alice").is_ok());
    for privileged in [
        r".\LocalSystem",
        r"NT AUTHORITY\NetworkService",
        r"x\SYSTEM",
    ] {
        assert_eq!(
            ServiceAccount::parse(privileged),
            Err(WindowsServiceRecordError::PrivilegedAccount)
        );
    }
    for malformed in ["alice", r"\alice", r".\", r".\a\b"] {
        assert_eq!(
            ServiceAccount::parse(malformed),
            Err(WindowsServiceRecordError::InvalidAccount)
        );
    }
}

#[test]
fn every_documented_manager_state_maps_and_others_are_refused() {
    let states: Vec<_> = (0..=8).map(ScmState::from_raw).collect();
    assert_eq!(states.first(), Some(&None));
    assert_eq!(states.last(), Some(&None));
    assert_eq!(states.get(4), Some(&Some(ScmState::Running)));
    assert_eq!(states.iter().flatten().count(), 7);
}

#[test]
fn a_host_record_names_a_pipe_address_and_round_trips() {
    let record =
        HostRecord::new(label(), &pipe(), proof()).with_placement(HolderPlacement::Dedicated);
    let value = serde_json::to_value(&record).unwrap();
    assert_eq!(value.pointer("/endpoint/transport"), Some(&json!("pipe")));
    assert_eq!(
        value.pointer("/endpoint/manager"),
        Some(&json!("windows_service"))
    );
    assert_eq!(
        value.pointer("/endpoint/address"),
        Some(&json!(pipe().as_str()))
    );
    assert_eq!(value.pointer("/placement"), Some(&json!("dedicated")));
    let bytes = serde_json::to_vec(&record).unwrap();
    assert_eq!(HostRecord::parse(&bytes).unwrap(), record);
    assert_eq!(record.pipe(), Ok(pipe()));
}

#[test]
fn a_marker_path_or_another_format_is_never_read_as_a_pipe_record() {
    let mut value = serde_json::to_value(HostRecord::new(label(), &pipe(), proof())).unwrap();
    *value.pointer_mut("/endpoint/address").unwrap() = json!(r"C:\Users\a\herdr.sock");
    let error = HostRecord::parse(&serde_json::to_vec(&value).unwrap()).unwrap_err();
    assert_eq!(error.kind(), "descriptor_malformed");
    *value.pointer_mut("/endpoint/address").unwrap() = json!(pipe().as_str());
    *value.pointer_mut("/endpoint/transport").unwrap() = json!("unix");
    let error = HostRecord::parse(&serde_json::to_vec(&value).unwrap()).unwrap_err();
    assert_eq!(error.kind(), "descriptor_malformed");
    *value.pointer_mut("/format").unwrap() = json!("ovis-terminal-holder-endpoint/1");
    let error = HostRecord::parse(&serde_json::to_vec(&value).unwrap()).unwrap_err();
    assert_eq!(error.kind(), "descriptor_unsupported");
}

#[test]
fn status_flattens_the_observation_and_reports_the_manager_view() {
    let stale = Observed::StaleEndpoint {
        record: Some(HostRecord::new(label(), &pipe(), proof()).endpoint),
        reason: StaleReason::ProcessGone,
    };
    let status = WindowsServiceStatus::new(label(), stale).with_host(
        Some(Installation::new(ScmState::Stopped, None)),
        Some(HolderPlacement::Shared),
    );
    let value = serde_json::to_value(&status).unwrap();
    assert_eq!(value.pointer("/state"), Some(&json!("stale_endpoint")));
    assert_eq!(value.pointer("/reason"), Some(&json!("process_gone")));
    assert_eq!(value.pointer("/installed/state"), Some(&json!("stopped")));
    assert_eq!(value.pointer("/installed/pid"), Some(&json!(null)));
    let back: WindowsServiceStatus = serde_json::from_value(value).unwrap();
    assert_eq!(back, status);
}

#[test]
fn a_registration_describes_the_pipe_it_serves() {
    let value = serde_json::to_value(ServiceRecord::pipe_registration(label(), &pipe())).unwrap();
    assert_eq!(
        value,
        json!({"label": "ovis-terminal-holder-default", "manager": "windows_service",
            "endpoint": {"transport": "pipe", "address": pipe().as_str(), "server_boot_id": null},
            "missing_prerequisites": []})
    );
}

#[test]
fn errors_carry_stable_kinds_and_display_never_repeats_them() {
    for error in [
        WindowsServiceRecordError::InvalidPipeName,
        WindowsServiceRecordError::InvalidAccount,
        WindowsServiceRecordError::PrivilegedAccount,
    ] {
        assert!(!error.to_string().contains(error.kind()));
        assert_ne!(error.hint(), "");
    }
}
