//! Open-prompt and retained-history bounds.

use ovis_terminal_protocol::schema::methods::status::PromptState;

use super::*;

#[test]
fn a_terminal_holds_at_most_sixteen_open_prompts() -> TestResult {
    let h = harness()?;
    for index in 0..16 {
        let opened = h.open(
            AGENT_TOKEN,
            (&format!("p{index}"), question(PromptMechanism::Rpc)),
        )?;
        assert_eq!(kind(&opened), None);
    }
    let over = h.open(AGENT_TOKEN, ("p16", question(PromptMechanism::Rpc)))?;
    h.answer("p0", (1, text("done")))?;
    let after = h.open(AGENT_TOKEN, ("p16", question(PromptMechanism::Rpc)))?;
    assert_eq!(kind(&over), Some("limit_exceeded"));
    assert_eq!(kind(&after), None);
    Ok(())
}

#[test]
fn closed_prompts_beyond_the_retained_bound_are_deleted_oldest_first() -> TestResult {
    let h = harness()?;
    for index in 0..65 {
        let id = format!("c{index}");
        h.open(AGENT_TOKEN, (&id, question(PromptMechanism::Rpc)))?;
        h.answer(&id, (1, text("ok")))?;
    }
    let listed = h
        .read(|snapshot| {
            h.prompts
                .list(
                    snapshot,
                    (
                        Some(terminal(AGENT).ok()?),
                        Some(&[PromptState::Answered][..]),
                    ),
                )
                .ok()
        })?
        .ok_or("list")?;
    assert_eq!(listed.len(), 64);
    assert_eq!(
        listed.first().map(|record| record.prompt_id.clone()),
        Some(prompt_id("c1"))
    );
    let oldest = h.read(|snapshot| h.prompts.get(snapshot, &prompt_id("c0")))?;
    assert_eq!(oldest.map_err(|error| error.kind()), Err("not_found"));
    Ok(())
}
