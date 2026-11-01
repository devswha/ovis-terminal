//! A core killed mid-output at a signalled boundary: the successor adopts the surviving
//! holder, and its rebuilt screen plus spliced live output equals an independent engine
//! fed the whole stream.

use ovis_terminal_holder::protocol::Message;
use ovis_terminal_protocol::domain::recovery::{IncidentKind, Survival};
use ovis_terminal_protocol::domain::restart::identity::Cause;
use ovis_terminal_vt::Vt;

use super::{Served, State, TestResult, running, until};
use crate::recovery::{Adopted, GroupId, Splice, read_os_boot, recover};

const WRITER: &str = r"i=0; while [ $i -lt 1500 ]; do printf '\033[1;3%dmline %d é─\033[0m\r\n' $((i%7)) $i; [ $i -eq 200 ] && printf 'MARK\r\n'; i=$((i+1)); done; printf DONE; read gate";

#[test]
fn a_core_killed_mid_output_is_succeeded_by_an_exact_screen_without_duplicate_bytes() -> TestResult
{
    // Given: a writer under a shared holder, watched by a core that checkpoints at MARK.
    let state = State::new()?;
    let plan = state.plan()?;
    let served = Served::new(&state.0, GroupId::shared("session", "task-49"))?;
    let mut old = Adopted::adopt(&served.group, plan.core_build.clone(), plan.bound)?;
    let recorded = served.spawn("terminal:1", WRITER)?;
    let seen = until(&mut old, (&recorded.target, b"MARK"))?;
    let mut crashed = Vt::new(plan.config);
    crashed.feed(&seen);
    let checkpoint = crashed.export_checkpoint();
    plan.checkpoints().save(&recorded.target, &checkpoint)?;
    plan.stamp().record(&read_os_boot()?)?;

    // When: the core dies without a handover while the program keeps printing.
    drop(old);
    let mut recovered = recover(
        &plan,
        std::slice::from_ref(&served.group),
        std::slice::from_ref(&recorded),
    )?;

    // Then: the program survived under the same PID and start token, from the checkpoint.
    let report = &recovered.report;
    assert_eq!(report.cause, Some(Cause::CoreCrash));
    let terminal = report.terminals.first().ok_or("terminal verdict")?;
    assert_eq!(terminal.survival, Survival::Survived {});
    let screen = terminal.screen.clone().ok_or("rebuilt screen")?;
    assert_eq!(
        screen
            .checkpoint_offset
            .map(ovis_terminal_protocol::schema::scalar::ByteOffset::get),
        Some(checkpoint.offset().get())
    );
    assert!(screen.exact && screen.gaps.is_empty(), "{screen:?}");
    assert!(running(&recorded));
    let kinds: Vec<_> = plan
        .incidents()
        .list()?
        .into_iter()
        .map(|i| i.kind)
        .collect();
    assert_eq!(kinds, vec![IncidentKind::CoreCrash]);

    // And: live output splices on without repeating a byte, through DONE.
    let (_, mut restored) = recovered.screens.pop().ok_or("restored screen")?;
    let link = recovered
        .links
        .get_mut(&served.group.id)
        .ok_or("adopted link")?;
    let mut splice = Splice::at(screen.head.get());
    let replayed = link.tail(&recorded.target, screen.replayed_from.get())?;
    let replayed_len =
        usize::try_from(screen.head.get().saturating_sub(screen.replayed_from.get()))?;
    let mut fed = replayed
        .bytes
        .get(..replayed_len)
        .unwrap_or_default()
        .to_vec();
    while !fed.windows(4).any(|bytes| bytes == b"DONE") {
        if let Message::Output(output) = link.receive()? {
            let fresh = splice.admit(&output)?;
            restored.vt.feed(fresh);
            fed.extend_from_slice(fresh);
        }
    }
    let whole = link.tail(&recorded.target, 0)?;
    assert!(
        !whole.gap && whole.from_offset == 0,
        "the holder retained the whole stream"
    );
    assert_eq!(whole.through_offset, splice.next());
    assert!(
        checkpoint.offset().get() < whole.through_offset,
        "the crash cut the output"
    );
    let mut independent = Vt::new(plan.config);
    independent.feed(&whole.bytes);
    let (rebuilt, reference) = (restored.vt.screen(), independent.screen());
    assert_eq!(
        (rebuilt.offset, rebuilt.lines),
        (reference.offset, reference.lines)
    );
    assert_eq!(restored.vt.cursor(), independent.cursor());
    assert_eq!(
        restored.vt.export_checkpoint(),
        independent.export_checkpoint()
    );
    Ok(())
}

#[test]
fn a_damaged_checkpoint_is_quarantined_and_a_repeated_recovery_reaches_the_same_verdict()
-> TestResult {
    // Given: an idle program, a checkpoint that is then damaged, and a crashed core.
    let state = State::new()?;
    let plan = state.plan()?;
    let served = Served::new(&state.0, GroupId::shared("session", "task-49"))?;
    let mut old = Adopted::adopt(&served.group, plan.core_build.clone(), plan.bound)?;
    let recorded = served.spawn("terminal:2", "printf 'ARMED\\r\\n'; read gate")?;
    let seen = until(&mut old, (&recorded.target, b"ARMED"))?;
    let mut crashed = Vt::new(plan.config);
    crashed.feed(&seen);
    plan.checkpoints()
        .save(&recorded.target, &crashed.export_checkpoint())?;
    plan.stamp().record(&read_os_boot()?)?;
    drop(old);
    let dir = state.0.join("recovery").join("checkpoints");
    let file = std::fs::read_dir(&dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "ckpt"))
        .ok_or("checkpoint file")?;
    let mut bytes = std::fs::read(&file)?;
    let last = bytes.last_mut().ok_or("empty")?;
    *last ^= 0x20;
    std::fs::write(&file, &bytes)?;

    // When: recovery runs, and runs again as if it had been interrupted after adoption.
    let first = recover(
        &plan,
        std::slice::from_ref(&served.group),
        std::slice::from_ref(&recorded),
    )?;
    drop(first.links);
    let second = recover(
        &plan,
        std::slice::from_ref(&served.group),
        std::slice::from_ref(&recorded),
    )?;

    // Then: both rebuild the same exact screen from retained output alone.
    for report in [&first.report, &second.report] {
        let terminal = report.terminals.first().ok_or("verdict")?;
        assert_eq!(terminal.survival, Survival::Survived {});
        let screen = terminal.screen.clone().ok_or("screen")?;
        assert_eq!(screen.checkpoint_offset, None);
        assert!(screen.exact, "{screen:?}");
    }
    let screens = |screens: &[(_, crate::recovery::Restored)]| {
        screens.first().map(|(_, restored)| restored.vt.screen())
    };
    assert_eq!(screens(&first.screens), screens(&second.screens));
    let kinds: Vec<_> = plan
        .incidents()
        .list()?
        .into_iter()
        .map(|i| i.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            IncidentKind::CoreCrash,
            IncidentKind::CheckpointCorrupted,
            IncidentKind::CoreCrash
        ]
    );
    let kept = plan
        .incidents()
        .list()?
        .get(1)
        .and_then(|i| i.quarantine.clone())
        .ok_or("kept")?;
    assert_eq!(std::fs::read(kept)?, bytes);
    assert!(running(&recorded));
    Ok(())
}
