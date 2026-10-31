use proptest::prelude::*;
use serde_json::json;

use super::{
    CrashRecovery, IncidentId, IncidentKind, LossReason, OutputGap, RecoveryIncident,
    ScreenRecovery, Survival, TerminalRecovery, UnknownReason,
};
use crate::domain::restart::identity::Cause;
use crate::rpc::parse_request;
use crate::schema::decode_params;
use crate::schema::ids::{RuntimeLifetimeId, ServerBootId, TerminalId};
use crate::schema::methods::terminal::TerminalRunParams;
use crate::schema::records::TerminalOrigin;
use crate::schema::scalar::{ByteOffset, Incarnation, TimestampMs};

fn survival() -> impl Strategy<Value = Survival> {
    let unknown = prop::sample::select(vec![
        UnknownReason::BootEvidenceUnreadable,
        UnknownReason::HolderUnreachable,
        UnknownReason::ProcessEvidenceUnreadable,
    ]);
    let lost = prop::sample::select(vec![
        LossReason::Exited,
        LossReason::Replaced,
        LossReason::NotHeld,
        LossReason::Gone,
    ]);
    prop_oneof![
        Just(Survival::Survived {}),
        Just(Survival::RestoredMetadata {}),
        any::<u64>().prop_map(|n| Survival::Relaunched {
            process_incarnation: Incarnation::new(n)
        }),
        lost.prop_map(|reason| Survival::Lost { reason }),
        unknown.prop_map(|reason| Survival::Unknown { reason }),
    ]
}

fn terminal() -> impl Strategy<Value = TerminalRecovery> {
    let screen = (any::<u64>(), any::<u64>(), any::<bool>(), any::<bool>()).prop_map(
        |(from, head, exact, checkpoint)| ScreenRecovery {
            checkpoint_offset: checkpoint.then_some(ByteOffset::new(from)),
            replayed_from: ByteOffset::new(from),
            head: ByteOffset::new(head),
            gaps: vec![OutputGap::new(ByteOffset::new(from), ByteOffset::new(head))],
            exact,
        },
    );
    (
        "terminal:[0-9]{1,4}",
        any::<u64>(),
        survival(),
        prop::option::of(screen),
    )
        .prop_map(|(id, incarnation, survival, screen)| TerminalRecovery {
            terminal_id: TerminalId::new(id),
            terminal_incarnation: Incarnation::new(incarnation),
            survival,
            screen,
        })
}

proptest! {
    #[test]
    fn a_crash_recovery_round_trips_exactly(terminals in prop::collection::vec(terminal(), 0..4)) {
        let written = CrashRecovery {
            cause: Some(Cause::CoreCrash),
            runtime_lifetime: RuntimeLifetimeId::new("runtime:7".to_owned()),
            server_boot_id: ServerBootId::new("server-boot:2".to_owned()),
            terminals,
            incidents: vec![IncidentId::new("0001".to_owned())],
        };
        let bytes = serde_json::to_vec(&written).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let read: CrashRecovery = serde_json::from_slice(&bytes).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(read, written);
    }

    #[test]
    fn only_proven_loss_permits_a_relaunch_and_only_survivors_are_restored(survival in survival()) {
        let proven_gone = matches!(survival, Survival::RestoredMetadata {} | Survival::Lost { .. });
        prop_assert_eq!(survival.may_relaunch(), proven_gone);
        let recovery = CrashRecovery {
            cause: Some(Cause::HostReboot),
            runtime_lifetime: RuntimeLifetimeId::new("runtime:8".to_owned()),
            server_boot_id: ServerBootId::new("server-boot:1".to_owned()),
            terminals: vec![TerminalRecovery {
                terminal_id: TerminalId::new("terminal:1".to_owned()),
                terminal_incarnation: Incarnation::new(3),
                survival,
                screen: None,
            }],
            incidents: Vec::new(),
        };
        let restored = recovery.evidence(TimestampMs::new(1)).restored;
        let listed = matches!(survival, Survival::Survived {} | Survival::Relaunched { .. });
        prop_assert_eq!(restored.len(), usize::from(listed));
        prop_assert_eq!(survival.origin() == Some(TerminalOrigin::Live), survival == Survival::Survived {});
    }
}

fn screen_json(offset: &serde_json::Value) -> serde_json::Value {
    json!({
        "checkpoint_offset": offset, "replayed_from": "0", "head": "18446744073709551615",
        "gaps": [], "exact": true
    })
}

#[test]
fn offsets_are_decimal_strings_and_a_numeric_offset_is_refused() {
    let max = ScreenRecovery {
        checkpoint_offset: Some(ByteOffset::new(u64::MAX)),
        replayed_from: ByteOffset::new(0),
        head: ByteOffset::new(u64::MAX),
        gaps: Vec::new(),
        exact: true,
    };
    let encoded = serde_json::to_value(&max).unwrap_or_default();
    assert_eq!(encoded, screen_json(&json!("18446744073709551615")));
    let decoded: Result<ScreenRecovery, _> =
        serde_json::from_value(screen_json(&json!("18446744073709551615")));
    assert_eq!(decoded.ok(), Some(max));
    let numeric: Result<ScreenRecovery, _> = serde_json::from_value(screen_json(&json!(7)));
    assert!(numeric.is_err(), "a numeric offset must be refused");
}

#[test]
fn an_unknown_verdict_or_field_is_refused_never_read_as_survival() {
    let guessed = serde_json::from_value::<Survival>(json!({"survival": "probably_alive"}));
    assert!(guessed.is_err());
    let extra = serde_json::from_value::<Survival>(json!({"survival": "survived", "pid": "9"}));
    assert!(extra.is_err());
    let unknown = serde_json::to_value(Survival::Unknown {
        reason: UnknownReason::HolderUnreachable,
    })
    .unwrap_or_default();
    assert_eq!(
        unknown,
        json!({"survival": "unknown", "reason": "holder_unreachable"})
    );
}

#[test]
fn an_incident_keeps_its_kind_and_quarantine_location() {
    let incident = RecoveryIncident {
        incident_id: IncidentId::new("00000000000000000001".to_owned()),
        kind: IncidentKind::CheckpointCorrupted,
        server_boot_id: ServerBootId::new("server-boot:2".to_owned()),
        recorded_ms: TimestampMs::new(u64::MAX),
        terminals: vec![TerminalId::new("terminal:4".to_owned())],
        quarantine: Some("quarantine/terminal-4.ckpt".to_owned()),
    };
    let value = serde_json::to_value(&incident).unwrap_or_default();
    assert_eq!(value.get("kind"), Some(&json!("checkpoint_corrupted")));
    assert_eq!(
        value.get("recorded_ms"),
        Some(&json!("18446744073709551615"))
    );
    let read: Result<RecoveryIncident, _> = serde_json::from_value(value);
    assert_eq!(read.ok(), Some(incident));
}

#[test]
fn dedicated_isolation_is_not_a_terminal_run_field_and_is_refused_at_its_path() {
    let line = json!({
        "jsonrpc": "2.0", "id": "run", "method": "terminal.run",
        "params": {
            "op_id": "0191f1a2-0000-7000-8000-000000000049",
            "program": {"argv": ["/bin/sh"]},
            "geometry": {"cols": 80, "rows": 24},
            "isolation": "dedicated"
        }
    })
    .to_string();
    let request = parse_request(&line).map_err(|error| error.kind().to_owned());
    let refused = request.and_then(|request| {
        decode_params::<TerminalRunParams>(&request)
            .map(drop)
            .map_err(|error| format!("{}@{}", error.kind(), error.field_path()))
    });
    assert_eq!(refused, Err("invalid_params@params.isolation".to_owned()));
    let schema = serde_json::to_value(schemars::schema_for!(TerminalRunParams)).unwrap_or_default();
    let properties = schema.pointer("/properties").cloned().unwrap_or_default();
    assert!(properties.get("program").is_some(), "{schema}");
    assert!(properties.get("isolation").is_none(), "{schema}");
}
