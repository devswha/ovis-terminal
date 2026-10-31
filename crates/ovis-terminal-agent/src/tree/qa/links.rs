use ovis_terminal_protocol::schema::ids::ProcessToken;
use ovis_terminal_protocol::schema::records::{AgentState, SessionRef, StatusReport};
use ovis_terminal_protocol::schema::scalar::{Incarnation, OpenEnum, ReportSeq};

use super::{DIE, INDEPENDENT, KEEP, OPTED_IN, PARENT, TestResult, harness, parent_ref, token};
use crate::status::{Register, Report};

#[test]
fn a_finished_replaced_or_unknown_parent_is_refused_before_anything_is_linked() -> TestResult {
    let h = harness()?;
    let first = h.launch(PARENT, 1)?;
    let child = h.launch(OPTED_IN, 1)?;
    assert_eq!(
        h.link(parent_ref(OPTED_IN + 9, 1)?, (child, DIE))?,
        Err("parent_not_found".to_owned())
    );
    assert!(!h.exited(first)?.replayed);
    assert_eq!(
        h.link(parent_ref(PARENT, 1)?, (child, DIE))?,
        Err("parent_finished".to_owned()),
        "the finished incarnation adopts nothing"
    );
    h.launch(PARENT, 2)?;
    assert_eq!(
        h.link(parent_ref(PARENT, 3)?, (child, DIE))?,
        Err("stale_target".to_owned()),
        "an incarnation the terminal does not run is stale"
    );
    assert_eq!(
        h.link(parent_ref(PARENT, 1)?, (child, DIE))?,
        Err("parent_finished".to_owned()),
        "the reused address does not revive the finished incarnation"
    );
    assert_eq!(h.children(PARENT)?.children.len(), 0);
    let linked = h.link(parent_ref(PARENT, 2)?, (child, DIE))?;
    assert_eq!(
        linked.map(|link| link.parent.incarnation),
        Ok(Incarnation::new(2))
    );
    assert_eq!(h.children(PARENT)?.children.len(), 1);
    Ok(())
}

#[test]
fn a_link_keeps_the_parent_session_while_children_report_and_end() -> TestResult {
    let h = harness()?;
    let parent = h.launch(PARENT, 1)?;
    let session: SessionRef = serde_json::from_value(serde_json::json!({
        "source": "omo", "kind": "session", "value": "orchestrator-1",
    }))?;
    let mut register = Register::new(parent.terminal, token(PARENT, 1), "orchestrator".to_owned());
    register.session = Some(session.clone());
    h.op(|snapshot| h.status.register(snapshot, register))?;
    let opted = h.launch(OPTED_IN, 1)?;
    let independent = h.launch(INDEPENDENT, 1)?;
    let link = h.link(parent_ref(PARENT, 1)?, (opted, DIE))??;
    h.link(parent_ref(PARENT, 1)?, (independent, KEEP))??;
    let identity = link
        .parent_identity
        .ok_or("a local parent's identity is kept")?;
    assert_eq!(identity.session.as_ref(), Some(&session));

    for (child, seq) in [(OPTED_IN, 1), (INDEPENDENT, 1)] {
        let token: ProcessToken = token(child, 1);
        let mut report = Report::new(
            super::terminal(child)?,
            token,
            StatusReport::new(AgentState::Working),
        );
        report.seq = ReportSeq::new(seq);
        h.op(|snapshot| h.status.report(snapshot, report))?;
    }
    h.status.exited(&h.core, opted)?;
    h.exited(opted)?;
    let parent_status = h.read(|snapshot| {
        h.status
            .location(parent.terminal)
            .read(snapshot, Some(parent.process))
            .map(|stored| stored.row.status)
    })??;
    assert_eq!(parent_status.session, Some(session));
    assert_eq!(
        parent_status.root, None,
        "child reports never write the parent's status"
    );
    let children = h.children(PARENT)?.children;
    let working = children
        .iter()
        .filter_map(|child| child.status.root.as_ref())
        .filter(|root| root.report.state == OpenEnum::Known(AgentState::Working))
        .count();
    assert_eq!(
        working, 1,
        "only the independent child still reports working"
    );
    Ok(())
}

#[test]
fn a_parent_keeps_at_most_its_limit_of_running_children() -> TestResult {
    let h = harness()?;
    h.launch(PARENT, 1)?;
    for incarnation in 1..=64 {
        let child = h.launch(OPTED_IN, incarnation)?;
        h.link(parent_ref(PARENT, 1)?, (child, KEEP))??;
    }
    let one_more = h.launch(INDEPENDENT, 1)?;
    assert_eq!(
        h.link(parent_ref(PARENT, 1)?, (one_more, KEEP))?,
        Err("limit_exceeded".to_owned())
    );
    Ok(())
}
