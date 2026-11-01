//! Write-boundary rechecks: authority, lease, incarnation and literal encoding.

use ovis_terminal_protocol::domain::template_launch::FirstMessageReason;
use ovis_terminal_protocol::schema::ids::ClientId;

use super::fixture::{Fixture, NOW, TestResult, modes};
use crate::launch::WriteBoundary;
use crate::leases::{Acquire, Caller, ControlSlot, LeaseView};
use crate::registry::{LeaseId, ProcessGeneration};

fn admit(
    fixture: &Fixture,
    (view, process): (&LeaseView, (u64, u64)),
    text: &str,
) -> TestResult<Result<Vec<u8>, FirstMessageReason>> {
    let decision = fixture.decision()?;
    let modes = modes(b"")?;
    let process = (
        ProcessGeneration::new(process.0)?,
        ProcessGeneration::new(process.1)?,
    );
    let boundary = WriteBoundary::new(
        (&fixture.authority, &decision),
        NOW,
        (view, process, &modes),
    );
    Ok(boundary.admit(text).map(|encoded| encoded.bytes().to_vec()))
}

#[test]
fn an_unchanged_terminal_gets_the_literal_text_and_one_enter() -> TestResult {
    let fixture = Fixture::new()?;
    let view = fixture.leases()?.view();
    let bytes = admit(&fixture, (&view, (1, 1)), "$(touch PWNED); `id`")?;
    assert_eq!(bytes, Ok(b"$(touch PWNED); `id`\r".to_vec()));
    Ok(())
}

#[test]
fn a_revoked_requester_lapses_at_the_write() -> TestResult {
    let fixture = Fixture::new()?;
    let decision = fixture.decision()?;
    fixture.revoke_launcher()?;
    let view = fixture.leases()?.view();
    let modes = modes(b"")?;
    let process = (ProcessGeneration::new(1)?, ProcessGeneration::new(1)?);
    let boundary = WriteBoundary::new(
        (&fixture.authority, &decision),
        NOW,
        (&view, process, &modes),
    );
    assert_eq!(
        boundary.admit("go").err(),
        Some(FirstMessageReason::AuthorityLapsed)
    );
    Ok(())
}

#[test]
fn an_interact_lease_or_a_replaced_process_refuses_the_write() -> TestResult {
    let fixture = Fixture::new()?;
    let mut leases = fixture.leases()?;
    let client = ClientId::new("tui".to_owned());
    leases.acquire(
        Caller::new(&fixture.owner, &client, NOW),
        Acquire::new(ControlSlot::Interact, LeaseId::try_from(1)?),
    )?;
    let held = admit(&fixture, (&leases.view(), (1, 1)), "go")?;
    assert_eq!(held, Err(FirstMessageReason::LeaseHeld));
    let free = fixture.leases()?.view();
    let replaced = admit(&fixture, (&free, (1, 2)), "go")?;
    assert_eq!(replaced, Err(FirstMessageReason::ProcessReplaced));
    Ok(())
}

#[test]
fn multiline_text_without_bracketed_paste_is_refused_not_split() -> TestResult {
    let fixture = Fixture::new()?;
    let view = fixture.leases()?.view();
    let refused = admit(&fixture, (&view, (1, 1)), "line one\nline two")?;
    assert_eq!(refused, Err(FirstMessageReason::EncodingRefused));
    Ok(())
}
