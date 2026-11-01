//! Wire forms of the credential rotation records.

use proptest::prelude::*;
use serde_json::{Value, json};

use super::{
    BearerText, CommitRace, CredentialRevokeParams, CredentialRevokeResult, CredentialRotateParams,
    FenceToken, PrincipalStanding, RacedWrite, WithdrawnCredential,
};
use crate::errors::OpId;
use crate::schema::ids::{CredentialId, PrincipalName};
use crate::schema::scalar::{ByteCount, ByteOffset, CredentialEpoch, EventSeq, PrincipalEpoch};

fn rotate(epoch: &Value) -> Value {
    json!({"op_id": "o1", "credential": "c3", "principal_epoch": epoch})
}

#[test]
fn rotate_params_take_the_principal_epoch_only_as_a_decimal_string() {
    let top =
        serde_json::from_value::<CredentialRotateParams>(rotate(&json!("18446744073709551615")));
    let numeric = serde_json::from_value::<CredentialRotateParams>(rotate(&json!(1)));
    let unknown = serde_json::from_value::<CredentialRevokeParams>(
        json!({"op_id": "o1", "credential": "c3", "force": true}),
    );

    assert_eq!(
        top.ok().map(|params| params.principal_epoch.get()),
        Some(u64::MAX)
    );
    assert!(numeric.is_err());
    assert!(unknown.is_err());
}

#[test]
fn bearer_text_stays_out_of_debug_output() {
    let bearer = BearerText::new("ovt1.c4.secret".to_owned());

    assert_eq!(format!("{bearer:?}"), "BearerText(<redacted>)");
    assert_eq!(bearer.as_str(), "ovt1.c4.secret");
}

#[test]
fn each_race_outcome_is_tagged_and_already_committed_carries_its_delivery() -> serde_json::Result<()>
{
    let committed = CommitRace::AlreadyCommitted {
        written_bytes: ByteCount::new(2),
        output_offset: ByteOffset::new(u64::MAX),
    };
    let cancelled = CommitRace::CancelledBeforeCommit {
        reason: FenceToken::new("credential_revoked".to_owned()),
    };

    assert_eq!(
        serde_json::to_value(&committed)?,
        json!({"outcome": "already_committed", "written_bytes": "2", "output_offset": "18446744073709551615"})
    );
    assert_eq!(
        serde_json::to_value(&cancelled)?,
        json!({"outcome": "cancelled_before_commit", "reason": "credential_revoked"})
    );
    assert_eq!(
        serde_json::to_value(CommitRace::OutcomeUnknown)?,
        json!({"outcome": "outcome_unknown"})
    );
    Ok(())
}

fn race() -> impl Strategy<Value = CommitRace> {
    prop_oneof![
        Just(CommitRace::OutcomeUnknown),
        "[a-z_]{1,20}".prop_map(|token| CommitRace::CancelledBeforeCommit {
            reason: FenceToken::new(token)
        }),
        (any::<u64>(), any::<u64>()).prop_map(|(written, offset)| CommitRace::AlreadyCommitted {
            written_bytes: ByteCount::new(written),
            output_offset: ByteOffset::new(offset),
        }),
    ]
}

proptest! {
    #[test]
    fn revoke_results_round_trip(
        (credential, epoch, principal_epoch) in (any::<u64>(), any::<u64>(), any::<u64>()),
        writes in proptest::collection::vec((any::<u64>(), any::<u64>(), race()), 0..4),
    ) {
        let result = CredentialRevokeResult::new(
            WithdrawnCredential::new(
                CredentialId::new(format!("c{credential}")),
                CredentialEpoch::new(epoch),
            ),
            PrincipalStanding::new(
                PrincipalName::new("writer".to_owned()),
                PrincipalEpoch::new(principal_epoch),
            ),
            writes
                .into_iter()
                .map(|(seq, bytes, race)| RacedWrite::new(
                    (OpId::new(format!("op-{seq}")), EventSeq::new(seq)),
                    ByteCount::new(bytes),
                    race,
                ))
                .collect(),
        );
        let text = serde_json::to_string(&result).map_err(|error| TestCaseError::fail(error.to_string()))?;
        let back: CredentialRevokeResult =
            serde_json::from_str(&text).map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(back, result);
    }
}
