use proptest::prelude::*;
use serde_json::json;

use super::{
    BuildManifest, BuildTarget, HolderGeneration, ReleaseVersion, RollbackReason, RollbackReceipt,
    Sha256Digest, UpdateValueError,
};
use crate::errors::OpId;
use crate::schema::ids::UpdateId;
use crate::schema::methods::update::{UpdateRecord, UpdateStageParams, UpdateState};
use crate::schema::scalar::{OpenEnum, ProtocolGeneration, TimestampMs};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const ZEROS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn manifest_json(generation: &serde_json::Value) -> serde_json::Value {
    json!({
        "version": "2.0.0",
        "target": "linux-x86_64",
        "protocol_generations": ["1"],
        "holder_generations": [generation],
    })
}

#[test]
fn holder_generation_round_trips_u64_max_as_a_decimal_string() -> TestResult {
    let value = manifest_json(&json!(u64::MAX.to_string()));
    let manifest: BuildManifest = serde_json::from_value(value.clone())?;
    assert_eq!(
        manifest.holder_generations,
        [HolderGeneration::new(u64::MAX)]
    );
    assert_eq!(serde_json::to_value(&manifest)?, value);
    Ok(())
}

#[test]
fn numeric_generations_are_refused() {
    assert!(serde_json::from_value::<BuildManifest>(manifest_json(&json!(2))).is_err());
}

#[test]
fn a_manifest_with_an_unknown_key_is_refused() {
    let value = json!({
        "version": "2.0.0",
        "target": "linux-x86_64",
        "protocol_generations": ["1"],
        "holder_generations": ["2"],
        "extra": true,
    });
    assert!(serde_json::from_value::<BuildManifest>(value).is_err());
}

#[test]
fn versions_that_could_name_a_path_are_refused() {
    for version in ["", "../x", "a/b", "a\\b", ".hidden", "-rc", &"9".repeat(65)] {
        assert_eq!(
            version.parse::<ReleaseVersion>(),
            Err(UpdateValueError::Version),
            "{version:?}"
        );
    }
    assert!("1.2.3-rc.1+build.7".parse::<ReleaseVersion>().is_ok());
}

#[test]
fn the_host_target_is_a_valid_target_and_bare_names_are_not() -> TestResult {
    let host = BuildTarget::host();
    assert_eq!(host.as_str().parse::<BuildTarget>()?, host);
    for target in [
        "linux",
        "Linux-x86_64",
        "linux-",
        "-x86_64",
        "linux-x86-64-extra/..",
    ] {
        assert_eq!(target.parse::<BuildTarget>(), Err(UpdateValueError::Target));
    }
    Ok(())
}

#[test]
fn stage_params_parse_the_digest_and_refuse_unknown_fields() -> TestResult {
    let stage = |sha256: &str, extra: Option<&str>| {
        let mut params = json!({"op_id": "op-1", "version": "2.0.0", "target": "linux-x86_64"});
        if let Some(object) = params.as_object_mut() {
            object.insert("sha256".to_owned(), json!(sha256));
            if let Some(key) = extra {
                object.insert(key.to_owned(), json!(format!("sha256:{ZEROS}")));
            }
        }
        serde_json::from_value::<UpdateStageParams>(params)
    };
    assert_eq!(stage(ZEROS, None)?.sha256.as_str(), ZEROS);
    assert!(stage(&ZEROS.replace('0', "A"), None).is_err());
    assert!(stage(ZEROS, Some("digest")).is_err());
    Ok(())
}

#[test]
fn a_rolled_back_record_round_trips_with_its_receipt() -> TestResult {
    let sha: Sha256Digest = ZEROS.parse()?;
    let version: ReleaseVersion = "2.0.0".parse()?;
    let old: ReleaseVersion = "1.0.0".parse()?;
    let target = BuildTarget::host();
    let mut record = UpdateRecord::staged(
        UpdateId::new("update:1".to_owned()),
        (version.clone(), target.clone(), sha.clone()),
        (old.clone(), OpId::new("op-stage".to_owned())),
    );
    let new_build = super::ActiveBuild::new(version, target.clone(), sha.clone());
    let old_build = super::ActiveBuild::new(old, target, sha);
    record.state = OpenEnum::Known(UpdateState::RolledBack);
    record.rollback = Some(RollbackReceipt::new(
        OpId::new("op-activate".to_owned()),
        RollbackReason::ReadinessFailed,
        (new_build, old_build, TimestampMs::new(u64::MAX)),
    ));
    let value = serde_json::to_value(&record)?;
    assert_eq!(value.pointer("/state"), Some(&json!("rolled_back")));
    assert_eq!(
        value.pointer("/rollback/reason"),
        Some(&json!("readiness_failed"))
    );
    assert_eq!(
        value.pointer("/rollback/recorded_ms"),
        Some(&json!(u64::MAX.to_string()))
    );
    assert_eq!(serde_json::from_value::<UpdateRecord>(value)?, record);
    Ok(())
}

proptest! {
    #[test]
    fn any_lowercase_digest_round_trips(bytes in proptest::array::uniform32(any::<u8>())) {
        let text: String = bytes
            .iter()
            .flat_map(|byte| [byte >> 4, byte & 0x0f])
            .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
            .collect();
        let parsed: Sha256Digest = serde_json::from_value(json!(text))?;
        prop_assert_eq!(serde_json::to_value(&parsed)?, json!(text));
    }

    #[test]
    fn digests_of_any_other_length_are_refused(text in "[0-9a-f]{0,80}") {
        prop_assume!(text.len() != 64);
        prop_assert_eq!(text.parse::<Sha256Digest>(), Err(UpdateValueError::Sha256));
    }

    #[test]
    fn protocol_generations_round_trip(value in any::<u64>()) {
        let manifest = BuildManifest::new(
            "1.0.0".parse()?,
            BuildTarget::host(),
            (vec![ProtocolGeneration::new(value)], vec![HolderGeneration::new(value)]),
        );
        let text = serde_json::to_string(&manifest)?;
        prop_assert_eq!(serde_json::from_str::<BuildManifest>(&text)?, manifest);
    }
}
