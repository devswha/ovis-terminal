use ovis_terminal_core::ops::Completion;
use ovis_terminal_protocol::domain::agent_tree::{OrphanAck, OrphanNotice};
use ovis_terminal_protocol::schema::ids::MachineId;

use super::{DIE, INDEPENDENT, KEEP, OPTED_IN, PARENT, TestResult, harness, parent_ref};
use crate::tree::Link;

fn peer() -> MachineId {
    MachineId::new("machine:0191f1a2-0000-7000-8000-0000000000b2".to_owned())
}

#[test]
fn a_death_is_queued_for_each_holding_machine_until_it_acknowledges() -> TestResult {
    let h = harness()?;
    let parent = h.launch(PARENT, 1)?;
    let first = parent_ref(PARENT, 1)?;
    h.run(|snapshot| {
        let writes = h.tree.link_remote(snapshot, (first, peer()))?;
        Ok(writes
            .into_iter()
            .fold(Completion::new(serde_json::json!({})), Completion::write))
    })?;
    let exit = h.exited(parent)?;
    let (machine, notice) = exit.deliveries.first().cloned().ok_or("one delivery")?;
    assert_eq!(machine, peer());
    assert_eq!(
        h.read(|snapshot| h.tree.outbox(snapshot, &peer()))??,
        vec![notice.clone()]
    );

    let replayed = h.exited(parent)?;
    assert!(
        replayed.deliveries.is_empty(),
        "a replayed exit queues nothing twice"
    );
    assert_eq!(
        h.read(|snapshot| h.tree.outbox(snapshot, &peer()))??.len(),
        1
    );

    let ack = OrphanAck::new(notice, peer());
    assert!(h.tree.orphan_acked(&h.core, &ack)?);
    assert_eq!(
        h.read(|snapshot| h.tree.outbox(snapshot, &peer()))??.len(),
        0
    );
    assert!(
        !h.tree.orphan_acked(&h.core, &ack)?,
        "a replayed acknowledgement changes nothing"
    );
    Ok(())
}

#[test]
fn a_received_death_ends_only_children_of_that_exact_incarnation_once() -> TestResult {
    let h = harness()?;
    let opted = h.launch(OPTED_IN, 1)?;
    let independent = h.launch(INDEPENDENT, 1)?;
    for (child, policy) in [(opted, DIE), (independent, KEEP)] {
        let parent = parent_ref(PARENT, 2)?;
        h.run(|snapshot| {
            let link = Link::new(parent, child, policy);
            let linked = h.tree.link_to_remote(snapshot, (peer(), link))?;
            Ok(linked
                .writes
                .into_iter()
                .fold(Completion::new(serde_json::json!({})), Completion::write))
        })?;
    }
    let notice = |incarnation| -> Result<OrphanNotice, Box<dyn std::error::Error>> {
        Ok(serde_json::from_value(serde_json::json!({
            "parent_machine": peer(),
            "parent": parent_ref(PARENT, incarnation)?,
            "finished_seq": "9",
        }))?)
    };
    let older = h.tree.orphaned(&h.core, &notice(1)?)?;
    assert!(
        older.end_orders.is_empty(),
        "another incarnation's death ends nothing here"
    );

    let applied = h.tree.orphaned(&h.core, &notice(2)?)?;
    assert_eq!(applied.end_orders, vec![opted]);
    let replay = h.tree.orphaned(&h.core, &notice(2)?)?;
    assert!(replay.replayed && replay.end_orders.is_empty());

    let late = h.launch(OPTED_IN, 2)?;
    let finished = parent_ref(PARENT, 2)?;
    let refused = h.run(|snapshot| {
        let link = Link::new(finished, late, DIE);
        let linked = h.tree.link_to_remote(snapshot, (peer(), link))?;
        Ok(Completion::new(serde_json::to_value(linked.link)?))
    })?;
    let kind = refused.error.map(|error| error.kind.token().to_owned());
    assert_eq!(kind.as_deref(), Some("parent_finished"));
    Ok(())
}
