use ovis_terminal_protocol::domain::agent_tree::{ChildState, EndReason};
use ovis_terminal_protocol::schema::ids::PromptId;
use ovis_terminal_protocol::schema::methods::status::{
    PromptDefinition, PromptKind, PromptMechanism, PromptState,
};
use ovis_terminal_protocol::schema::records::ParentRef;
use ovis_terminal_protocol::schema::scalar::{EventSeq, OpenEnum};

use super::{
    DIE, Harness, INDEPENDENT, KEEP, OPTED_IN, PARENT, TestResult, harness, parent_ref, token,
};
use crate::prompts::Open;
use crate::tree::store::{self, ParentKey};

fn states(
    h: &Harness,
    parent: ParentRef,
) -> Result<Vec<(u64, ChildState)>, Box<dyn std::error::Error>> {
    let key = ParentKey {
        machine: h.tree.machine.clone(),
        parent,
    };
    let row = h.read(|snapshot| store::parent(snapshot, (h.tree.runtime, &key)))??;
    Ok(row
        .value
        .links
        .iter()
        .filter_map(|link| {
            let id = link.child.terminal_id.as_str().strip_prefix("terminal:")?;
            let OpenEnum::Known(state) = link.state else {
                return None;
            };
            Some((id.parse().ok()?, state))
        })
        .collect())
}

#[test]
fn only_the_opted_in_child_is_ordered_to_end_and_ends_only_on_its_own_exit() -> TestResult {
    let h = harness()?;
    let parent = h.launch(PARENT, 1)?;
    let opted = h.launch(OPTED_IN, 1)?;
    let independent = h.launch(INDEPENDENT, 1)?;
    h.link(parent_ref(PARENT, 1)?, (opted, DIE))??;
    h.link(parent_ref(PARENT, 1)?, (independent, KEEP))??;

    let exit = h.exited(parent)?;
    assert!(!exit.replayed);
    assert_eq!(exit.end_orders, vec![opted]);
    assert_eq!(
        states(&h, parent_ref(PARENT, 1)?)?,
        vec![
            (OPTED_IN, ChildState::Ending),
            (INDEPENDENT, ChildState::Live)
        ],
        "an ordered end is not reported as an exit"
    );

    let replay = h.exited(parent)?;
    assert!(
        replay.replayed && replay.end_orders.is_empty(),
        "a replayed death orders nothing"
    );

    h.exited(opted)?;
    assert_eq!(
        states(&h, parent_ref(PARENT, 1)?)?,
        vec![
            (OPTED_IN, ChildState::Ended),
            (INDEPENDENT, ChildState::Live)
        ]
    );
    let key = ParentKey {
        machine: h.tree.machine.clone(),
        parent: parent_ref(PARENT, 1)?,
    };
    let row = h.read(|snapshot| store::parent(snapshot, (h.tree.runtime, &key)))??;
    let ended = row.value.links.first().and_then(|link| link.ended.clone());
    assert_eq!(
        ended.map(|end| end.reason),
        Some(OpenEnum::Known(EndReason::WithParent))
    );
    let death = row.value.death.ok_or("the death is durable")?;
    assert_eq!(death.ending.len(), 1);
    assert_eq!(death.kept.len(), 1);
    Ok(())
}

#[test]
fn a_replayed_death_of_a_reused_address_never_ends_the_new_incarnations_child() -> TestResult {
    let h = harness()?;
    let first = h.launch(PARENT, 1)?;
    h.exited(first)?;
    h.launch(PARENT, 2)?;
    let child = h.launch(OPTED_IN, 1)?;
    h.link(parent_ref(PARENT, 2)?, (child, DIE))??;

    let replay = h.exited(first)?;
    assert!(replay.replayed);
    assert_eq!(replay.end_orders.len(), 0);
    assert_eq!(
        states(&h, parent_ref(PARENT, 2)?)?,
        vec![(OPTED_IN, ChildState::Live)]
    );
    assert_eq!(h.children(PARENT)?.children.len(), 1);
    Ok(())
}

#[test]
fn an_ended_child_expires_its_open_prompts_in_the_same_unit() -> TestResult {
    let h = harness()?;
    let parent = h.launch(PARENT, 1)?;
    let child = h.launch(OPTED_IN, 1)?;
    h.link(parent_ref(PARENT, 1)?, (child, DIE))??;
    let definition = PromptDefinition::new(
        PromptKind::Question,
        "Proceed?".to_owned(),
        PromptMechanism::Rpc,
    );
    let mut open = Open::new(child.terminal, token(OPTED_IN, 1), definition);
    open.prompt_id = Some(PromptId::new("child-question".to_owned()));
    h.op(|snapshot| h.prompts.open(snapshot, open))?;

    h.exited(parent)?;
    let still_open = h.read(|snapshot| {
        h.prompts
            .get(snapshot, &PromptId::new("child-question".to_owned()))
    })??;
    assert!(still_open.is_open(), "the end order alone expires nothing");
    h.exited(child)?;
    let prompt = h.read(|snapshot| {
        h.prompts
            .get(snapshot, &PromptId::new("child-question".to_owned()))
    })??;
    assert_eq!(prompt.state, OpenEnum::Known(PromptState::Expired));
    assert!(
        h.read(|snapshot| h.prompts.admission(snapshot, child.terminal))?
            .is_ok()
    );
    Ok(())
}

#[test]
fn notices_reach_only_parents_that_asked_and_are_acknowledged_once() -> TestResult {
    let h = harness()?;
    h.launch(PARENT, 1)?;
    let opted = h.launch(OPTED_IN, 1)?;
    let independent = h.launch(INDEPENDENT, 1)?;
    h.link(parent_ref(PARENT, 1)?, (opted, DIE))??;
    h.link(parent_ref(PARENT, 1)?, (independent, KEEP))??;
    h.exited(opted)?;
    h.exited(independent)?;
    let unseen: Vec<u32> = h
        .children(PARENT)?
        .children
        .iter()
        .map(|c| c.unseen_news)
        .collect();
    assert_eq!(unseen, vec![1, 0], "notify:false records no news");

    let through = EventSeq::new(u64::MAX);
    let ack = h
        .tree
        .acknowledge(&h.core, (parent_ref(PARENT, 1)?, through))?;
    assert_eq!(ack.acknowledged.len(), 1);
    let unseen: Vec<u32> = h
        .children(PARENT)?
        .children
        .iter()
        .map(|c| c.unseen_news)
        .collect();
    assert_eq!(unseen, vec![0, 0]);
    let replay = h
        .tree
        .acknowledge(&h.core, (parent_ref(PARENT, 1)?, through))?;
    assert!(
        replay.acknowledged.is_empty(),
        "a replayed acknowledgement covers nothing new"
    );
    Ok(())
}
