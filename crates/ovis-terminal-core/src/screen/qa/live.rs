//! Happy path over a real holder: reads pin a revision while a viewer keeps
//! receiving bytes and the program keeps printing.

use std::io::{self, Write};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ovis_terminal_core::screen::{
    HistoryRequest, PageCursor, QueryReply, Screen, ScreenBounds, ScreenConfig, SearchRequest,
    SelectionRequest,
};
use ovis_terminal_holder::protocol::{
    CoreBuildId, Exit, HOLDER_PROTOCOL_GENERATION, Hello, HolderBuildId, HolderDescriptor,
    HolderInstanceId, Input, Message, PreviousCoreState, Spawn, TerminalId, TerminalIncarnation,
    TerminalTarget, WriteGrant,
};
use ovis_terminal_holder::unix::{CoreLink, Holder, HolderConfig, HolderLimits};
use ovis_terminal_vt::{LineSpan, OutputOffset};

use crate::fixture::{Fallible, TestResult, limit, needle, vt_config};

const WAIT: Duration = Duration::from_secs(20);
const SCRIPT: &str = "i=0; while [ $i -lt 120 ]; do echo \"line $i\"; i=$((i+1)); done; \
    printf '\\033[6n'; echo READY; read go; \
    i=0; while [ $i -lt 40 ]; do echo \"tick $i\"; i=$((i+1)); done; exit 3";

enum Fed {
    Advanced(OutputOffset),
    Exited(Box<Exit>),
}

fn holder() -> Result<(Holder, CoreLink), Box<dyn std::error::Error>> {
    let holder = Holder::new(HolderConfig::new(
        HolderDescriptor::new(
            HolderBuildId::new("task-17".into()),
            HolderInstanceId::new(uuid::Uuid::now_v7().to_string()),
        ),
        HolderLimits::DEFAULT,
    ));
    let link = holder.adopt(&Hello {
        generation: HOLDER_PROTOCOL_GENERATION,
        core_build: CoreBuildId::new("task-17".into()),
        previous: PreviousCoreState::Crash,
    })?;
    Ok((holder, link))
}

/// Pumps every holder message of the terminal into the screen, in order.
fn pump(link: &CoreLink, screen: &Screen, events: &mpsc::Sender<Fed>) {
    while let Ok(message) = link.recv_timeout(WAIT) {
        let fed = match message {
            Message::Output(output) => match screen.ingest(&output) {
                Ok(ingested) => Fed::Advanced(ingested.offset),
                Err(_) => return,
            },
            Message::Exit(exit) => Fed::Exited(Box::new(exit)),
            _ => continue,
        };
        let exited = matches!(fed, Fed::Exited(_));
        if events.send(fed).is_err() || exited {
            return;
        }
    }
}

/// A viewer that follows raw output by cursor, independent of every read.
fn view(
    screen: &Screen,
    until: &mpsc::Receiver<OutputOffset>,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut seen = Vec::new();
    while let Ok(target) = until.recv() {
        loop {
            let from = OutputOffset::new(u64::try_from(seen.len())?);
            if from >= target {
                break;
            }
            let request = ovis_terminal_core::screen::OutputRequest::new(from, limit(4096));
            seen.extend_from_slice(&screen.output(&request)?.bytes);
        }
    }
    Ok(seen)
}

fn await_offset(
    events: &mpsc::Receiver<Fed>,
    viewer: &mpsc::Sender<OutputOffset>,
) -> Fallible<Fed> {
    let fed = events.recv_timeout(WAIT)?;
    if let Fed::Advanced(offset) = &fed {
        viewer.send(*offset)?;
    }
    Ok(fed)
}

/// Starts the fixture program on the real holder.
fn spawn_program(holder: &Holder) -> Fallible<TerminalTarget> {
    let spawned = holder.spawn_wire(&Spawn {
        terminal_id: TerminalId::new("terminal:17".into()),
        terminal_incarnation: TerminalIncarnation::new(1),
        argv: vec!["/bin/sh".into(), "-c".into(), SCRIPT.into()],
        cwd: std::env::temp_dir(),
        env: [("PATH".into(), "/usr/bin:/bin".into())].into(),
        geometry: (40, 5),
    })?;
    Ok(TerminalTarget {
        terminal_id: spawned.terminal_id,
        terminal_incarnation: spawned.terminal_incarnation,
        process_incarnation: Some(spawned.process_incarnation),
    })
}

/// Lets the program print its second phase by answering its `read`.
fn release(holder: &Holder, target: TerminalTarget) -> Fallible<()> {
    let process_incarnation = target.process_incarnation.ok_or("spawned process")?;
    holder.input(Input {
        target,
        sequence: 1,
        grant: WriteGrant {
            grant_epoch: 1,
            fence_epoch: 1,
            process_incarnation,
        },
        bytes: b"go\n".to_vec(),
    })?;
    Ok(())
}

/// Pages, searches and selects at a pinned revision while a real program and a viewer continue.
///
/// # Errors
///
/// Returns holder, screen or encoding failures.
///
/// # Panics
///
/// Panics when an observed read violates the acceptance contract.
pub fn happy() -> TestResult {
    // Given: a real holder program, a pump thread and an independent raw viewer.
    let (holder, link) = holder()?;
    let target = spawn_program(&holder)?;
    let screen = Screen::new(
        target.clone(),
        ScreenConfig::new(vt_config(40, 5)?, ScreenBounds::DEFAULT),
    )?;
    let (fed_tx, fed_rx) = mpsc::channel();
    let (view_tx, view_rx) = mpsc::channel();
    let pumper = thread::spawn({
        let screen = screen.clone();
        move || pump(&link, &screen, &fed_tx)
    });
    let viewer = thread::spawn({
        let screen = screen.clone();
        move || view(&screen, &view_rx)
    });
    loop {
        await_offset(&fed_rx, &view_tx)?;
        let ready = screen.search(&SearchRequest::new(needle("READY")?, limit(1)))?;
        if !ready.matches.is_empty() {
            break;
        }
    }

    // When: history is paged and searched while the program prints more.
    let first = screen.history(&HistoryRequest::new(limit(50)))?;
    let cursor: PageCursor = first.next.ok_or("history has a second page")?;
    release(&holder, target)?;
    let mut exit = None;
    while exit.is_none() {
        if let Fed::Exited(done) = await_offset(&fed_rx, &view_tx)? {
            exit = Some(done);
        }
    }
    let second = screen.history(&HistoryRequest::new(limit(50)).at(cursor))?;
    let found =
        screen.search(&SearchRequest::new(needle("line 7")?, limit(64)).at(first.meta.revision))?;
    let selected = screen.selection(&SelectionRequest::new(
        first.meta.revision,
        LineSpan::new(0, 2),
    ))?;
    let exit = exit.ok_or("exit observed")?;
    screen.finish(&exit)?;
    drop(view_tx);
    let seen = viewer
        .join()
        .map_err(|_| "viewer panicked")?
        .map_err(|error| error.to_string())?;
    pumper.join().map_err(|_| "pump panicked")?;

    // Then: both pages share the pinned revision, the live bottom moved on,
    // the viewer received every byte and the query reply took the internal lane.
    assert_eq!(second.meta.revision, first.meta.revision);
    assert!(second.meta.live.revision > first.meta.revision);
    assert!(!second.meta.live.at_live_bottom);
    assert!(first.meta.live.at_live_bottom);
    assert_eq!(
        Some(second.returned.first),
        first.returned.first.checked_add(first.returned.count)
    );
    assert_eq!(
        first.lines.first().map(|line| line.text.as_str()),
        Some("line 0")
    );
    assert!(found.matches.len() >= 11, "line 7 and line 70..79");
    assert_eq!(selected.text, "line 0\nline 1\n");
    assert_eq!(
        OutputOffset::new(u64::try_from(seen.len())?),
        OutputOffset::new(exit.final_offset)
    );
    let ticks = String::from_utf8_lossy(&seen).matches("tick ").count();
    assert_eq!(
        ticks, 40,
        "viewer kept receiving bytes after the pinned read"
    );
    let replies = screen.drain_replies()?;
    assert!(matches!(
        replies.replies.as_slice(),
        [QueryReply::Reply { bytes, .. }] if bytes.starts_with(b"\x1b[") && bytes.ends_with(b"R")
    ));
    assert!(
        screen.drain_replies()?.replies.is_empty(),
        "replies are taken once"
    );
    writeln!(
        io::stdout(),
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "first_page": {"meta": first.meta, "requested": first.requested, "returned": first.returned, "next": first.next},
            "second_page": {"meta": second.meta, "returned": second.returned, "truncated": second.truncated},
            "search_revision": found.meta.revision, "search_hits": found.matches.len(),
            "selection": selected.text,
            "viewer_bytes": seen.len(), "final_offset": exit.final_offset.to_string(), "exit_code": exit.code,
        }))?
    )?;
    Ok(())
}
