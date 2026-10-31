use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;

use ovis_terminal_protocol::domain::process_info::UnsupportedReason;
use proptest::prelude::*;

use super::{Liveness, NativeProcess, ProcessTable, Session, Terminal, liveness, observe};
use crate::registry::{ProcessHandle, ProcessPid, ProcessStartToken, TerminalId};
use crate::terminals::{Field, Unavailable};

/// A deterministic process table; `swap` replaces one record after `at` reads.
#[derive(Default)]
struct Scripted {
    records: RefCell<BTreeMap<u64, Result<NativeProcess, Unavailable>>>,
    reads: Cell<usize>,
    swap: RefCell<Option<(usize, NativeProcess)>>,
}

impl ProcessTable for Scripted {
    fn read(&self, pid: ProcessPid) -> Result<NativeProcess, Unavailable> {
        self.reads.set(self.reads.get().saturating_add(1));
        let due = self
            .swap
            .borrow()
            .as_ref()
            .is_some_and(|(at, _)| self.reads.get() > *at);
        if due && let Some((_, next)) = self.swap.borrow_mut().take() {
            self.records
                .borrow_mut()
                .insert(next.process.pid().get(), Ok(next));
        }
        self.records
            .borrow()
            .get(&pid.get())
            .cloned()
            .unwrap_or(Err(Unavailable::ProcessUnavailable))
    }

    fn pids(&self) -> Result<Vec<ProcessPid>, Unavailable> {
        Ok(self
            .records
            .borrow()
            .keys()
            .filter_map(|pid| ProcessPid::try_from(*pid).ok())
            .collect())
    }

    fn argv(&self, pid: ProcessPid) -> Field<Vec<String>> {
        Field::Known(vec![format!("p{}", pid.get())])
    }

    fn cwd(&self, _: ProcessPid) -> Field<PathBuf> {
        Field::Unsupported(Unavailable::Platform)
    }
}

fn handle(pid: u64, token: &str) -> ProcessHandle {
    ProcessHandle::new(
        ProcessPid::try_from(pid).expect("nonzero pid"),
        ProcessStartToken::new(token).expect("token"),
    )
}

fn record(pid: u64, token: &str, terminal: Terminal) -> NativeProcess {
    NativeProcess::new(handle(pid, token), true, terminal)
}

fn on_tty(group: u32, foreground: u32) -> Terminal {
    Terminal::Controlling(Session::new(group, 7, foreground))
}

fn table(records: Vec<NativeProcess>) -> Scripted {
    let scripted = Scripted::default();
    for native in records {
        scripted
            .records
            .borrow_mut()
            .insert(native.process.pid().get(), Ok(native));
    }
    scripted
}

fn pids(field: &Field<Vec<crate::terminals::ProcessSample>>) -> Option<Vec<u64>> {
    match field {
        Field::Known(samples) => Some(samples.iter().map(|s| s.process.pid().get()).collect()),
        Field::Unsupported(_) => None,
    }
}

fn terminal() -> TerminalId {
    TerminalId::try_from(1).expect("terminal id")
}

#[test]
fn a_reused_pid_with_another_start_token_is_replacement_never_the_owned_process() {
    let owned = handle(40, "boot:100");
    let reused = table(vec![record(40, "boot:900", on_tty(40, 40))]);

    assert_eq!(liveness(&reused, &owned), Liveness::Replaced);
    assert_eq!(
        observe(&reused, terminal(), &owned).err(),
        Some(Liveness::Replaced)
    );
}

#[test]
fn a_vanished_pid_or_a_zombie_with_the_owned_token_is_an_exit() {
    let owned = handle(40, "boot:100");
    let mut zombie = record(40, "boot:100", on_tty(40, 40));
    zombie.running = false;

    assert_eq!(liveness(&table(Vec::new()), &owned), Liveness::Exited);
    assert_eq!(liveness(&table(vec![zombie]), &owned), Liveness::Exited);
}

#[test]
fn unsupported_membership_reports_the_launcher_and_never_an_exit() {
    let owned = handle(40, "windows-created:1");
    let windows = table(vec![record(40, "windows-created:1", Terminal::Unsupported)]);

    let info = observe(&windows, terminal(), &owned).expect("launcher is current");
    let roles = info.roles();

    assert_eq!(liveness(&windows, &owned), Liveness::Current);
    assert!(matches!(info.launcher, Field::Known(ref sample) if sample.process == owned));
    assert_eq!(info.foreground, Field::Unsupported(Unavailable::Platform));
    assert_eq!(info.background, Field::Unsupported(Unavailable::Platform));
    assert_eq!(
        roles.supported_fields(),
        ["launcher", "pid", "start_token", "argv"]
    );
}

#[test]
fn foreground_and_background_follow_the_terminal_foreground_group() {
    let owned = handle(40, "t40");
    let mut other_tty = record(70, "t70", on_tty(70, 70));
    other_tty.terminal = Terminal::Controlling(Session::new(70, 8, 70));
    let session = table(vec![
        record(40, "t40", on_tty(40, 50)),
        record(50, "t50", on_tty(50, 50)),
        record(60, "t60", on_tty(60, 50)),
        record(80, "t80", Terminal::Detached),
        other_tty,
    ]);

    let info = observe(&session, terminal(), &owned).expect("launcher is current");

    assert_eq!(pids(&info.foreground), Some(vec![50]));
    assert_eq!(pids(&info.background), Some(vec![40, 60]));
    assert_eq!(info.shell, Field::Unsupported(Unavailable::UndeclaredRole));
    let wire = info.roles();
    assert_eq!(
        wire.shell,
        ovis_terminal_protocol::domain::process_info::Observed::unsupported(
            UnsupportedReason::UndeclaredRole
        )
    );
}

#[test]
fn a_foreground_change_during_the_pass_is_reported_as_raced_not_as_a_role() {
    let owned = handle(40, "t40");
    let session = table(vec![
        record(40, "t40", on_tty(40, 40)),
        record(50, "t50", on_tty(50, 40)),
    ]);
    *session.swap.borrow_mut() = Some((4, record(40, "t40", on_tty(40, 50))));

    let info = observe(&session, terminal(), &owned).expect("launcher is current");

    assert_eq!(info.foreground, Field::Unsupported(Unavailable::Raced));
    assert_eq!(info.background, Field::Unsupported(Unavailable::Raced));
}

#[test]
fn an_unreadable_launcher_is_unknown_and_every_role_says_why() {
    let owned = handle(40, "t40");
    let refused = Scripted::default();
    refused
        .records
        .borrow_mut()
        .insert(40, Err(Unavailable::Permission));

    let info = observe(&refused, terminal(), &owned).expect("not proven ended");

    assert_eq!(
        liveness(&refused, &owned),
        Liveness::Unknown(Unavailable::Permission)
    );
    assert_eq!(info.launcher, Field::Unsupported(Unavailable::Permission));
    assert_eq!(info.foreground, Field::Unsupported(Unavailable::Permission));
}

proptest! {
    #[test]
    fn only_a_different_start_token_on_a_running_pid_is_replacement(
        pid in 2_u64..u64::MAX, owned in "[a-z0-9:]{1,20}", seen in "[a-z0-9:]{1,20}",
    ) {
        let reading = table(vec![record(pid, &seen, Terminal::Unsupported)]);
        let expected = if owned == seen { Liveness::Current } else { Liveness::Replaced };
        prop_assert_eq!(liveness(&reading, &handle(pid, &owned)), expected);
    }
}
