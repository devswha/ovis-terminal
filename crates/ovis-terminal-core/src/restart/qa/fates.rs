//! The published identity table checked against what each cause actually does.

use ovis_terminal_protocol::domain::restart::identity::{Cause, Fate, Field, fate};

use super::*;
use crate::registry::TerminalGeneration;
use crate::restart::Boot;

/// The identities a client holds for one terminal, as control reports them.
struct Seen {
    identity: RuntimeIdentity,
    terminal: TerminalGeneration,
    process: Option<ProcessIdentity>,
    seq: u64,
}

fn seen(control: &Control, id: TerminalId) -> TestResult<Seen> {
    let identity = control.identity()?;
    let (record, seq) = control.with_terminals(|terminals, session| {
        let record = terminals.get(session, id)?;
        Ok((
            record,
            terminals.coordinator().read_snapshot()?.cursor.seq.get(),
        ))
    })?;
    Ok(Seen {
        identity,
        terminal: record.lifecycle.generation(),
        process: record.lifecycle.process().cloned(),
        seq,
    })
}

/// Asserts that every identity observable here moved as the table says for `cause`.
fn assert_fates(cause: Cause, (before, after): (&Seen, &Seen)) -> TestResult {
    let kept = |same: bool| if same { Fate::Kept } else { Fate::Replaced };
    let (old, new) = (before.identity, after.identity);
    let process = match (&before.process, &after.process) {
        (Some(was), Some(now)) if was == now && running(now)? => Fate::Kept,
        _ => Fate::Replaced,
    };
    let seq = if old.runtime_id() != new.runtime_id() {
        Fate::Rescoped
    } else if after.seq > before.seq {
        Fate::Advances
    } else {
        Fate::Kept
    };
    for (field, observed) in [
        (Field::MachineId, kept(old.machine_id() == new.machine_id())),
        (
            Field::RuntimeLifetime,
            kept(old.runtime_id() == new.runtime_id()),
        ),
        (
            Field::ServerBootId,
            kept(old.server_boot_id() == new.server_boot_id()),
        ),
        (
            Field::TerminalIncarnation,
            kept(before.terminal == after.terminal),
        ),
        (Field::ProcessIncarnation, process),
        (Field::Seq, seq),
    ] {
        assert_eq!(observed, fate(cause, field), "{cause:?} {field:?}");
    }
    Ok(())
}

#[test]
fn a_planned_restart_moves_identities_as_the_control_restart_row_says() -> TestResult {
    let fixture = Fixture::new()?;
    let (control, _) = fixture.open()?;
    let record = spawn(&control, fixture.launch("printf ARMED; read gate")?)?;
    let before = seen(&control, record.id())?;
    control.restart(op())?;
    assert_fates(
        Cause::ControlRestart,
        (&before, &seen(&control, record.id())?),
    )?;
    control.close()?;
    Ok(())
}

#[test]
fn a_control_that_dies_without_an_intent_reopens_unplanned_as_the_core_crash_row_says() -> TestResult
{
    // Given: a live child and a control that ends without any restart intent.
    let fixture = Fixture::new()?;
    let (control, _) = fixture.open()?;
    let record = spawn(&control, fixture.launch("printf ARMED; read gate")?)?;
    let before = seen(&control, record.id())?;
    control.close()?;

    // When: a successor opens under the next boot.
    let next = RuntimeIdentity::new(
        HostIdentity::new(
            fixture.identity.machine_id(),
            fixture.identity.host_boot_id(),
        ),
        fixture.identity.runtime_id(),
        ServerBootId::try_from(fixture.identity.server_boot_id().get() + 1)?,
    );
    let (control, boot) = Control::open(fixture.plan()?, next)?;

    // Then: the crash path is taken, not the planned one, and the process is adopted.
    assert!(matches!(boot, Boot::Unplanned), "{boot:?}");
    assert_eq!(fixture.plan()?.intent().load()?, None);
    assert_fates(Cause::CoreCrash, (&before, &seen(&control, record.id())?))?;
    control.close()?;
    Ok(())
}

#[test]
fn a_lost_holder_ends_the_process_and_recovery_mints_new_incarnations() -> TestResult {
    // Given: a kept child whose holder dies while control keeps its boot.
    let mut fixture = Fixture::new()?;
    let (control, _) = fixture.open()?;
    let mut launch = fixture.launch("printf ARMED; read gate")?;
    launch.keep = true;
    let record = spawn(&control, launch)?;
    let before = seen(&control, record.id())?;
    control.close()?;
    fixture.holder.shutdown()?;
    fixture.holder = Holder::new(HolderConfig::new(
        HolderDescriptor::new(
            HolderBuildId::new("task-48-replacement".into()),
            HolderInstanceId::new(Uuid::now_v7().to_string()),
        ),
        HolderLimits::DEFAULT,
    ));

    // When: control adopts the replacement holder and relaunches the kept terminal.
    let (control, _) = fixture.open()?;
    control.with_terminals(|terminals, session| {
        terminals.recover(Call::new(session, op()), record.id())?;
        terminals.await_spawn(record.id(), WAIT)
    })?;

    // Then: the old process is gone and every incarnation is new, as the row says.
    assert_fates(Cause::HolderLoss, (&before, &seen(&control, record.id())?))?;
    control.close()?;
    Ok(())
}
