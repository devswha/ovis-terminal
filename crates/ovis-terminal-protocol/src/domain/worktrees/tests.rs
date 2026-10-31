use proptest::prelude::*;
use serde_json::{Value, json};

use super::{AffectedResource, BranchName, BranchNameError, WorktreeFacts};
use crate::errors::{BoundaryError, ErrorData, ErrorKind};
use crate::schema::ids::{CheckoutId, RepositoryId, WorkspaceId};
use crate::schema::methods::worktree::{
    RemoveScope, WorktreeChange, WorktreeRecord, WorktreeRemoveParams,
};
use crate::schema::scalar::{EventSeq, Revision};

fn facts(revision: u64) -> WorktreeFacts {
    WorktreeFacts {
        repository_id: RepositoryId::new("repo:main".to_owned()),
        checkout_id: CheckoutId::new("checkout:a".to_owned()),
        path: "/srv/a".to_owned(),
        branch: BranchName::parse("feature/a").ok(),
        source: Some(CheckoutId::new("checkout:main".to_owned())),
        dirty: true,
        locked: false,
        workspaces: vec![WorkspaceId::new("workspace:4".to_owned())],
        revision: Revision::new(revision),
    }
}

#[test]
fn record_and_change_carry_counters_as_exact_decimal_strings() -> Result<(), serde_json::Error> {
    let record = WorktreeRecord::from(facts(u64::MAX));
    let change = WorktreeChange::new(record.clone(), Vec::new(), EventSeq::new(u64::MAX));
    let wire = serde_json::to_value(&change)?;
    assert_eq!(wire.pointer("/seq"), Some(&json!("18446744073709551615")));
    assert_eq!(
        wire.pointer("/worktree/revision"),
        Some(&json!("18446744073709551615"))
    );
    assert_eq!(wire.pointer("/worktree/branch"), Some(&json!("feature/a")));
    assert_eq!(serde_json::from_value::<WorktreeChange>(wire)?, change);
    let mut numeric = serde_json::to_value(&record)?;
    let revision = numeric
        .pointer_mut("/revision")
        .ok_or_else(|| <serde_json::Error as serde::de::Error>::custom("record has no revision"))?;
    *revision = json!(7);
    assert!(serde_json::from_value::<WorktreeRecord>(numeric).is_err());
    Ok(())
}

#[test]
fn remove_scope_keeps_group_and_force_independent() -> Result<(), serde_json::Error> {
    for (group, force) in [(false, false), (true, false), (false, true), (true, true)] {
        let params: WorktreeRemoveParams = serde_json::from_value(json!({
            "op_id": "op-1", "checkout_id": "checkout:a",
            "scope": {"group": group, "force": force},
        }))?;
        assert_eq!(params.scope, RemoveScope::new(group, force));
    }
    let flat =
        json!({"op_id": "op-1", "checkout_id": "checkout:a", "group": false, "force": false});
    assert!(serde_json::from_value::<WorktreeRemoveParams>(flat).is_err());
    let hidden = json!({"op_id": "op-1", "checkout_id": "checkout:a", "scope": {"group": true}});
    assert!(serde_json::from_value::<WorktreeRemoveParams>(hidden).is_err());
    Ok(())
}

#[test]
fn branch_names_that_read_as_options_or_carry_controls_are_refused() {
    assert_eq!(BranchName::parse(""), Err(BranchNameError::Empty));
    assert_eq!(
        BranchName::parse("--upload-pack=x"),
        Err(BranchNameError::LeadingDash)
    );
    assert_eq!(BranchName::parse("a b"), Err(BranchNameError::Control));
    assert_eq!(BranchName::parse("a\nb"), Err(BranchNameError::Control));
    assert_eq!(BranchNameError::LeadingDash.kind(), "invalid_name");
}

#[test]
fn affected_identities_are_distinct_per_family() -> Result<(), BranchNameError> {
    let resources = [
        AffectedResource::Checkout(CheckoutId::new("checkout:a".to_owned())),
        AffectedResource::Workspace(WorkspaceId::new("workspace:4".to_owned())),
        AffectedResource::Branch(BranchName::parse("feature/a")?),
    ];
    let ids: Vec<Value> = resources
        .iter()
        .map(|resource| serde_json::to_value(resource.target_id()).unwrap_or(Value::Null))
        .collect();
    assert_eq!(
        ids,
        [
            json!("checkout:a"),
            json!("workspace:4"),
            json!("branch:feature/a")
        ]
    );
    Ok(())
}

#[test]
fn refusal_data_round_trips_its_affected_list() -> Result<(), serde_json::Error> {
    let affected = vec![
        AffectedResource::Checkout(CheckoutId::new("checkout:a".to_owned())).target_id(),
        AffectedResource::Workspace(WorkspaceId::new("workspace:4".to_owned())).target_id(),
    ];
    let refused = BoundaryError::domain(ErrorKind::from("checkout_dirty".to_owned()), false)
        .with_affected(affected);
    let wire = serde_json::to_value(refused.data())?;
    assert_eq!(
        wire.pointer("/affected"),
        Some(&json!(["checkout:a", "workspace:4"]))
    );
    assert_eq!(serde_json::from_value::<ErrorData>(wire)?, *refused.data());
    let plain = BoundaryError::domain(ErrorKind::from("not_found".to_owned()), false);
    let wire = serde_json::to_value(plain.data())?;
    assert!(wire.get("affected").is_none(), "an empty list is omitted");
    assert_eq!(serde_json::from_value::<ErrorData>(wire)?, *plain.data());
    Ok(())
}

proptest! {
    #[test]
    fn accepted_branch_names_survive_record_round_trips(name in "[a-z0-9][a-z0-9/._]{0,30}") {
        let branch = BranchName::parse(&name);
        prop_assert!(branch.is_ok());
        let mut built = facts(3);
        built.branch = branch.ok();
        let record = WorktreeRecord::from(built);
        let wire = serde_json::to_value(&record).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(wire.pointer("/branch"), Some(&json!(name)));
        let back: WorktreeRecord =
            serde_json::from_value(wire).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, record);
    }

    #[test]
    fn revisions_round_trip_exactly(value in any::<u64>()) {
        let record = WorktreeRecord::from(facts(value));
        let wire = serde_json::to_value(&record).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(wire.pointer("/revision"), Some(&json!(value.to_string())));
    }
}
