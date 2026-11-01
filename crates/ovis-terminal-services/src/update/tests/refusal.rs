use ovis_terminal_protocol::domain::update::RejectReason;
use ovis_terminal_protocol::schema::ids::UpdateId;
use ovis_terminal_protocol::schema::methods::update::UpdateState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use tokio::sync::watch;

use super::fixture::{
    Entry, Fixture, TestResult, exists_below, manifest, release, script, stage_params, step_params,
    tar,
};
use crate::update::{Interrupt, UpdateError};

const ZEROS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[test]
fn a_wrong_digest_stages_nothing_and_keeps_the_active_build() -> TestResult {
    let fixture = Fixture::new()?;
    let updates = fixture.open()?;
    let before = updates.query()?;
    fixture.publish(("2.0.0", &super::host()), &release("2.0.0", 0))?;
    let refused = updates.stage(&stage_params("2.0.0", ZEROS)?, &Interrupt::never());
    let Err(error) = refused else {
        return Err("a mismatching archive was staged".into());
    };
    assert_eq!(error.kind(), "integrity_failed");
    assert_eq!(updates.query()?, before);
    assert!(!exists_below(
        &fixture.install_root().join("staging"),
        "archive.tar"
    ));
    Ok(())
}

#[test]
fn another_target_or_a_missing_archive_is_refused_before_copying() -> TestResult {
    let fixture = Fixture::new()?;
    let updates = fixture.open()?;
    let mut params = stage_params("2.0.0", ZEROS)?;
    params.target = Some("plan9-mips".parse()?);
    let wrong = updates.stage(&params, &Interrupt::never());
    assert_eq!(
        wrong.map_err(|error| error.kind()).err(),
        Some("incompatible")
    );
    let missing = updates.stage(&stage_params("9.9.9", ZEROS)?, &Interrupt::never());
    assert_eq!(
        missing.map_err(|error| error.kind()).err(),
        Some("transport_failed")
    );
    assert_eq!(updates.query()?.updates, []);
    Ok(())
}

#[test]
fn a_cancelled_stage_removes_its_partial_copy() -> TestResult {
    let fixture = Fixture::new()?;
    let updates = fixture.open()?;
    let digest = fixture.publish(("2.0.0", &super::host()), &release("2.0.0", 0))?;
    let (cancel, requested) = watch::channel(true);
    let stopped = updates.stage(&stage_params("2.0.0", &digest)?, &Interrupt::new(requested));
    assert!(matches!(stopped, Err(UpdateError::Cancelled)));
    drop(cancel);
    assert!(!exists_below(
        &fixture.install_root().join("staging"),
        "archive.tar.partial"
    ));
    assert_eq!(updates.query()?.updates, []);
    Ok(())
}

/// Stages `archive`, verifies it, and returns the recorded rejection.
fn rejection(fixture: &Fixture, archive: &[u8]) -> TestResult<(UpdateError, RejectReason)> {
    let updates = fixture.open()?;
    let digest = fixture.publish(("2.0.0", &super::host()), archive)?;
    let staged = updates.stage(&stage_params("2.0.0", &digest)?, &Interrupt::never())?;
    let Err(error) = updates.verify(&step_params(&staged.update_id)?) else {
        return Err("an unsafe build was verified".into());
    };
    let record = updates
        .query()?
        .updates
        .into_iter()
        .next()
        .ok_or("no record")?;
    assert_eq!(record.state, OpenEnum::Known(UpdateState::Rejected));
    let reason = record.rejection.ok_or("no rejection")?.reason;
    let OpenEnum::Known(reason) = reason else {
        return Err("unknown rejection reason".into());
    };
    assert!(
        !fixture
            .install_root()
            .join("builds")
            .join(staged.update_id.as_str())
            .exists()
    );
    Ok((error, reason))
}

#[test]
fn traversal_absolute_and_link_entries_refuse_the_whole_archive() -> TestResult {
    let manifest = manifest("2.0.0", &super::host(), "2");
    for (name, kind) in [
        ("../escape", b'0'),
        ("/escape", b'0'),
        ("dir/../../escape", b'0'),
        ("link", b'2'),
    ] {
        let fixture = Fixture::new()?;
        let archive = tar(&[
            Entry {
                name: "build.json",
                kind: b'0',
                body: &manifest,
            },
            Entry {
                name,
                kind,
                body: b"payload",
            },
        ]);
        let (error, reason) = rejection(&fixture, &archive)?;
        assert_eq!(
            (error.kind(), reason),
            ("integrity_failed", RejectReason::UnsafeArchive),
            "{name}"
        );
        assert!(!exists_below(&fixture.root, "escape"), "{name} escaped");
    }
    Ok(())
}

#[test]
fn manifests_that_do_not_fit_this_runtime_are_rejected_by_reason() -> TestResult {
    let binary = script("2.0.0", 0);
    let cases = [
        (
            manifest("2.0.0", "plan9-mips", "2"),
            true,
            "integrity_failed",
            RejectReason::ManifestMismatch,
        ),
        (
            manifest("2.0.0", &super::host(), "1"),
            true,
            "incompatible",
            RejectReason::HolderIncompatible,
        ),
        (
            manifest("2.0.0", &super::host(), "2"),
            false,
            "integrity_failed",
            RejectReason::MissingEntry,
        ),
    ];
    for (manifest, with_binary, kind, expected) in cases {
        let fixture = Fixture::new()?;
        let mut entries = vec![Entry {
            name: "build.json",
            kind: b'0',
            body: &manifest,
        }];
        if with_binary {
            entries.push(Entry {
                name: "ovis-terminal",
                kind: b'0',
                body: &binary,
            });
        }
        let (error, reason) = rejection(&fixture, &tar(&entries))?;
        assert_eq!((error.kind(), reason), (kind, expected));
    }
    Ok(())
}

#[test]
fn a_staged_archive_changed_on_disk_fails_verification() -> TestResult {
    let fixture = Fixture::new()?;
    let updates = fixture.open()?;
    let digest = fixture.publish(("2.0.0", &super::host()), &release("2.0.0", 0))?;
    let staged = updates.stage(&stage_params("2.0.0", &digest)?, &Interrupt::never())?;
    let archive = fixture
        .install_root()
        .join("staging")
        .join(staged.update_id.as_str())
        .join("archive.tar");
    std::fs::write(&archive, release("2.0.0", 1))?;
    let refused = updates.verify(&step_params(&staged.update_id)?);
    assert!(matches!(
        refused,
        Err(UpdateError::Rejected(RejectReason::DigestMismatch))
    ));
    Ok(())
}

#[test]
fn steps_out_of_order_or_on_foreign_ids_change_nothing() -> TestResult {
    let fixture = Fixture::new()?;
    let updates = fixture.open()?;
    let digest = fixture.publish(("2.0.0", &super::host()), &release("2.0.0", 0))?;
    let staged = updates.stage(&stage_params("2.0.0", &digest)?, &Interrupt::never())?;
    let early = {
        let params = step_params(&staged.update_id)?;
        fixture.step(|| updates.activate(&params))
    };
    assert_eq!(
        early.map_err(|error| error.kind()).err(),
        Some("invalid_update_state")
    );
    for id in [
        "../../active",
        "update-not-a-uuid",
        "update-0191f1a2-0000-7000-8000-000000000047",
    ] {
        let foreign = updates.verify(&step_params(&UpdateId::new(id.to_owned()))?);
        assert_eq!(
            foreign.map_err(|error| error.kind()).err(),
            Some("no_staged_update")
        );
    }
    let record = updates
        .query()?
        .updates
        .into_iter()
        .next()
        .ok_or("no record")?;
    assert_eq!(record.state, OpenEnum::Known(UpdateState::Staged));
    Ok(())
}
