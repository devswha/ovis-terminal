use std::error::Error;
use std::io;

use proptest::prelude::*;

use crate::registry::{
    MachineId, ProcessGeneration, ProcessHandle, ProcessId, ProcessIdentity, ProcessPid,
    ProcessStartToken, TerminalGeneration, TerminalId,
};

type TestResult<T> = Result<T, Box<dyn Error>>;

fn identity_roundtrips(sequence: u64) -> TestResult<()> {
    let identity = TerminalId::try_from(sequence)?;
    let encoded = serde_json::to_string(&identity)?;
    if serde_json::from_str::<TerminalId>(&encoded)? != identity {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "terminal ID changed").into());
    }
    let generation = TerminalGeneration::new(sequence)?;
    let encoded_generation = serde_json::to_string(&generation)?;
    if encoded_generation != format!("\"{sequence}\"")
        || serde_json::from_str::<TerminalGeneration>(&encoded_generation)? != generation
    {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "generation changed").into());
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]
    #[test]
    fn identity_serialization_roundtrips_full_width_decimal_strings(sequence in 1_u64..=u64::MAX) {
        let outcome = identity_roundtrips(sequence);
        prop_assert!(outcome.is_ok(), "round-trip failure: {:?}", outcome.err());
    }
}

#[test]
fn identity_serialization_preserves_u64_max() -> TestResult<()> {
    let max_generation = TerminalGeneration::new(u64::MAX)?;
    let max_terminal = TerminalId::try_from(u64::MAX)?;
    let machine = MachineId::new(uuid::Uuid::from_u128(1))?;
    assert_eq!(
        serde_json::to_string(&max_generation)?,
        format!("\"{}\"", u64::MAX)
    );
    assert_eq!(
        serde_json::to_string(&max_terminal)?,
        format!("\"terminal:{}\"", u64::MAX)
    );
    let machine_json = serde_json::to_string(&machine)?;
    assert!(machine_json.starts_with("\"machine:"));
    assert_eq!(serde_json::from_str::<MachineId>(&machine_json)?, machine);
    assert!(serde_json::from_str::<MachineId>("1").is_err());
    assert!(serde_json::from_str::<TerminalGeneration>(&u64::MAX.to_string()).is_err());
    assert!(serde_json::from_str::<ProcessPid>(&u64::MAX.to_string()).is_err());
    assert_eq!(
        max_generation.checked_next().unwrap_err().kind(),
        "counter_overflow"
    );
    let full_process = ProcessIdentity::new(
        ProcessId::try_from(1)?,
        ProcessGeneration::new(u64::MAX)?,
        ProcessHandle::new(
            ProcessPid::try_from(u64::MAX)?,
            ProcessStartToken::new("linux:boot:9223372036854775807:token")?,
        ),
    );
    let process_json = serde_json::to_string(&full_process)?;
    assert!(process_json.contains(&format!("\"{}\"", u64::MAX)));
    assert_eq!(
        serde_json::from_str::<ProcessIdentity>(&process_json)?,
        full_process
    );
    Ok(())
}

#[test]
fn malformed_machine_uuid_preserves_source_and_kind() {
    let error = "machine:not-a-uuid".parse::<MachineId>().unwrap_err();

    assert_eq!(error.kind(), "invalid_identity");
    assert!(error.source().is_some());
}

#[test]
fn malformed_resource_integer_preserves_source_and_kind() {
    let error = "terminal:18446744073709551616"
        .parse::<TerminalId>()
        .unwrap_err();

    assert_eq!(error.kind(), "invalid_identity");
    assert!(error.source().is_some());
}

#[test]
fn malformed_generation_and_pid_integers_preserve_sources() {
    let generation = "18446744073709551616"
        .parse::<TerminalGeneration>()
        .unwrap_err();
    let pid = "18446744073709551616".parse::<ProcessPid>().unwrap_err();

    assert_eq!(generation.kind(), "invalid_identity");
    assert!(generation.source().is_some());
    assert_eq!(pid.kind(), "invalid_identity");
    assert!(pid.source().is_some());
}

#[test]
fn malformed_identity_inputs_have_typed_kinds() {
    assert_eq!(
        TerminalId::try_from(0_u64).unwrap_err().kind(),
        "invalid_identity"
    );
    assert_eq!(
        MachineId::new(uuid::Uuid::from_u128(0)).unwrap_err().kind(),
        "invalid_identity"
    );
    assert_eq!(
        "workspace:7".parse::<TerminalId>().unwrap_err().kind(),
        "invalid_identity"
    );
    assert!(serde_json::from_str::<TerminalId>("\"workspace:7\"").is_err());
    assert_eq!(
        crate::registry::ResourceName::new("bad/name")
            .unwrap_err()
            .kind(),
        "invalid_name"
    );
    assert_eq!(
        ProcessStartToken::new("").unwrap_err().kind(),
        "invalid_identity"
    );
}
