use proptest::prelude::*;
use serde_json::json;

use super::{
    ChildLink, ChildRef, ChildState, EndReason, NoticeKind, OrphanAck, ParentDeath, ParentIdentity,
    RemoteDelivery,
};
use crate::schema::ids::{MachineId, TerminalId};
use crate::schema::methods::terminal::{ChildPolicy, TerminalRunParams};
use crate::schema::records::{ParentRef, SessionRef, StatusSnapshot};
use crate::schema::scalar::{EventSeq, Incarnation, OpenEnum, TurnNumber};

fn machine(name: &str) -> MachineId {
    MachineId::new(name.to_owned())
}

fn link(policy: ChildPolicy, incarnation: u64) -> ChildLink {
    let child = ChildRef::new(
        TerminalId::new("terminal:7".to_owned()),
        Incarnation::new(1),
    );
    let parent = ParentRef::new(
        TerminalId::new("terminal:6".to_owned()),
        Incarnation::new(incarnation),
    );
    let identity = ParentIdentity::of(&StatusSnapshot::unreported(Incarnation::new(incarnation)));
    ChildLink::linked(
        (child, parent, machine("machine:a")),
        (policy, Some(identity)),
        EventSeq::new(3),
    )
}

#[test]
fn only_a_live_die_with_parent_child_is_ordered_to_end_and_ends_with_its_parent() {
    let mut independent = link(ChildPolicy::new(true, false), 1);
    assert!(!independent.order_end());
    assert_eq!(independent.state, OpenEnum::Known(ChildState::Live));

    let mut opted_in = link(ChildPolicy::new(false, true), 1);
    assert!(opted_in.order_end());
    assert!(!opted_in.order_end(), "an end is ordered once");
    assert!(
        opted_in.is_running(),
        "an ordered end is not an observed exit"
    );
    assert_eq!(
        opted_in.exited(EventSeq::new(9)),
        Some(EndReason::WithParent)
    );
    assert!(!opted_in.is_running());
    assert_eq!(
        opted_in.exited(EventSeq::new(10)),
        None,
        "a replayed exit changes nothing"
    );
    assert_eq!(
        opted_in.ended.map(|end| (end.reason, end.seq)),
        Some((OpenEnum::Known(EndReason::WithParent), EventSeq::new(9)))
    );
    assert_eq!(
        independent.exited(EventSeq::new(4)),
        Some(EndReason::Exited)
    );
    assert_eq!(
        NoticeKind::from(EndReason::WithParent),
        NoticeKind::EndedWithParent
    );
}

#[test]
fn a_death_acknowledges_each_remote_machine_once() {
    let mut death = ParentDeath {
        parent: ParentRef::new(
            TerminalId::new("terminal:6".to_owned()),
            Incarnation::new(2),
        ),
        parent_machine: machine("machine:a"),
        finished_seq: EventSeq::new(u64::MAX),
        ending: Vec::new(),
        kept: Vec::new(),
        remote: vec![RemoteDelivery {
            machine_id: machine("machine:b"),
            acknowledged: false,
        }],
    };
    assert!(!death.acknowledge(&machine("machine:c")));
    assert!(death.acknowledge(&machine("machine:b")));
    assert!(
        !death.acknowledge(&machine("machine:b")),
        "a replayed ack is not new"
    );
    let notice = death.notice();
    assert_eq!(notice.parent.incarnation, Incarnation::new(2));
    assert_eq!(notice.finished_seq, EventSeq::new(u64::MAX));
}

#[test]
fn incarnations_and_sequences_are_exact_decimal_strings_and_numbers_are_refused()
-> Result<(), serde_json::Error> {
    let mut record = link(ChildPolicy::new(true, true), u64::MAX);
    if let Some(identity) = record.parent_identity.as_mut() {
        identity.turn = TurnNumber::new(u64::MAX);
        identity.session = Some(SessionRef {
            source: "s".to_owned(),
            kind: "k".to_owned(),
            value: "v".to_owned(),
        });
    }
    let wire = serde_json::to_value(&record)?;
    assert_eq!(
        wire.pointer("/parent/incarnation"),
        Some(&json!("18446744073709551615"))
    );
    assert_eq!(
        wire.pointer("/parent_identity/turn"),
        Some(&json!("18446744073709551615"))
    );
    assert_eq!(serde_json::from_value::<ChildLink>(wire.clone())?, record);
    let mut numeric = wire;
    if let Some(incarnation) = numeric.pointer_mut("/parent/incarnation") {
        *incarnation = json!(1);
    }
    assert!(serde_json::from_value::<ChildLink>(numeric).is_err());
    Ok(())
}

#[test]
fn a_run_names_its_parent_by_incarnation_and_refuses_a_word_or_number() {
    let run = |incarnation: serde_json::Value| {
        serde_json::from_value::<TerminalRunParams>(json!({
            "op_id": "o", "program": {"argv": ["sh"]}, "geometry": {"cols": 80, "rows": 24},
            "parent": {"terminal_id": "terminal:6", "incarnation": incarnation},
            "child_policy": {"notify": true, "die_with_parent": true},
        }))
    };
    assert!(run(json!("3")).is_ok_and(|params| {
        params
            .parent
            .is_some_and(|parent| parent.incarnation == Incarnation::new(3))
    }));
    assert!(run(json!("retired")).is_err());
    assert!(run(json!(3)).is_err());
}

fn wire_id() -> impl Strategy<Value = String> {
    "[a-z0-9:]{1,24}"
}

proptest! {
    #[test]
    fn an_orphan_ack_round_trips(parent in wire_id(), terminal in wire_id(),
                                 incarnation in any::<u64>(), seq in any::<u64>()) {
        let death = ParentDeath {
            parent: ParentRef::new(TerminalId::new(terminal), Incarnation::new(incarnation)),
            parent_machine: MachineId::new(parent),
            finished_seq: EventSeq::new(seq),
            ending: Vec::new(),
            kept: Vec::new(),
            remote: Vec::new(),
        };
        let ack = OrphanAck::new(death.notice(), machine("machine:b"));
        let text = serde_json::to_string(&ack).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let back: OrphanAck =
            serde_json::from_str(&text).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, ack);
    }

    #[test]
    fn a_child_link_round_trips_in_every_state(incarnation in any::<u64>(), seq in any::<u64>(),
                                               die in any::<bool>(), end in any::<bool>()) {
        let mut record = link(ChildPolicy::new(!die, die), incarnation);
        record.order_end();
        if end {
            record.exited(EventSeq::new(seq));
        }
        let text = serde_json::to_string(&record).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let back: ChildLink =
            serde_json::from_str(&text).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, record);
    }
}
