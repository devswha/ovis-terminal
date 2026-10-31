//! No survival is invented: every evidence combination maps to one verdict, and only
//! matching boot, holder and operating-system evidence yields `survived`.

use std::cell::Cell;

use ovis_terminal_holder::protocol::{
    HeldTerminal, ProcessIncarnation, ProcessStartToken, TerminalId, TerminalIncarnation,
    TerminalTarget,
};
use ovis_terminal_protocol::domain::recovery::{LossReason, Survival, UnknownReason};
use proptest::prelude::*;

use crate::recovery::{BootVerdict, Evidence, GroupId, HolderAnswer, Liveness, Recorded, verdict};

fn recorded(pid: u32) -> Recorded {
    let target = TerminalTarget {
        terminal_id: TerminalId::new("terminal:9".to_owned()),
        terminal_incarnation: TerminalIncarnation::new(2),
        process_incarnation: Some(ProcessIncarnation::new(5)),
    };
    let token = ProcessStartToken::new("linux:boot:100".to_owned());
    Recorded::new(target, (pid, token), GroupId::shared("s", "b"))
}

fn held(recorded: &Recorded, (pid, token): (u32, &str)) -> HeldTerminal {
    HeldTerminal {
        terminal_id: recorded.target.terminal_id.clone(),
        terminal_incarnation: recorded.target.terminal_incarnation,
        process_incarnation: recorded.target.process_incarnation,
        pid: Some(pid),
        start_token: Some(ProcessStartToken::new(token.to_owned())),
        output_offset: 0,
        input_sequence: 0,
        exit: None,
    }
}

fn liveness() -> impl Strategy<Value = Liveness> {
    prop_oneof![
        Just(Liveness::Running(ProcessStartToken::new(
            "linux:boot:100".to_owned()
        ))),
        Just(Liveness::Running(ProcessStartToken::new(
            "linux:boot:999".to_owned()
        ))),
        Just(Liveness::Gone),
        Just(Liveness::Unreadable),
    ]
}

proptest! {
    #[test]
    fn survival_requires_same_boot_same_holder_identity_and_the_os_token(
        boot in prop::sample::select(vec![BootVerdict::Same, BootVerdict::Rebooted, BootVerdict::Inconclusive]),
        reachable in any::<bool>(),
        holds in any::<bool>(),
        held_token in prop::sample::select(vec!["linux:boot:100", "linux:boot:999"]),
        pid in prop::sample::select(vec![0_u32, 1, 4242]),
        live in liveness(),
    ) {
        let record = recorded(pid);
        let holding = held(&record, (pid, held_token));
        let holder = match (reachable, holds) {
            (false, _) => HolderAnswer::Unreachable,
            (true, false) => HolderAnswer::Held(None),
            (true, true) => HolderAnswer::Held(Some(&holding)),
        };
        let probed = Cell::new(false);
        let os_says = live.clone();
        let survival = verdict(Evidence { boot, holder }, &record, |_| {
            probed.set(true);
            os_says
        });
        let matches = boot == BootVerdict::Same && reachable && holds && held_token == "linux:boot:100"
            && pid > 1 && live == Liveness::Running(ProcessStartToken::new("linux:boot:100".to_owned()));
        prop_assert_eq!(survival == Survival::Survived {}, matches);
        prop_assert!(!(probed.get() && pid <= 1), "PIDs 0 and 1 are never probed");
        match boot {
            BootVerdict::Rebooted => prop_assert_eq!(survival, Survival::RestoredMetadata {}),
            BootVerdict::Inconclusive => prop_assert_eq!(
                survival,
                Survival::Unknown { reason: UnknownReason::BootEvidenceUnreadable }
            ),
            BootVerdict::Same if !reachable => prop_assert_eq!(
                survival,
                Survival::Unknown { reason: UnknownReason::HolderUnreachable }
            ),
            BootVerdict::Same => {
                let boot_or_link = matches!(
                    survival,
                    Survival::Unknown { reason: UnknownReason::HolderUnreachable | UnknownReason::BootEvidenceUnreadable }
                );
                prop_assert!(!boot_or_link, "a reachable holder on the same boot is never unknown for boot or link");
            }
        }
        if matches!(survival, Survival::Unknown { .. }) {
            prop_assert!(!survival.may_relaunch(), "inconclusive evidence never permits a relaunch");
        }
    }
}

#[test]
fn a_reused_pid_with_another_start_token_is_replaced_never_survived() {
    let record = recorded(4242);
    let holding = held(&record, (4242, "linux:boot:100"));
    let evidence = Evidence {
        boot: BootVerdict::Same,
        holder: HolderAnswer::Held(Some(&holding)),
    };
    let reused = Liveness::Running(ProcessStartToken::new("linux:boot:777".to_owned()));
    assert_eq!(
        verdict(evidence, &record, |_| reused),
        Survival::Lost {
            reason: LossReason::Replaced
        }
    );
    let gone = verdict(evidence, &record, |_| Liveness::Gone);
    assert_eq!(
        gone,
        Survival::Lost {
            reason: LossReason::Gone
        }
    );
}
