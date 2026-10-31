//! An unmodified bash on a real holder PTY: resize ordering, input after the runtime's
//! readiness, and repaint without a signal.

use std::time::{Duration, Instant};

use ovis_terminal_core::events::Event;
use ovis_terminal_core::geometry::{GeometryEpoch, GeometryEvent};
use ovis_terminal_core::input::encoding::ModeSnapshot;
use ovis_terminal_core::input::receipts::{Delivery, Receipt};
use ovis_terminal_core::input::sequencer::{Moment, Payload, QueueLimits, Sequencer, View};
use ovis_terminal_holder::protocol::Resize;
use ovis_terminal_protocol::schema::methods::input::Submit;
use ovis_terminal_protocol::schema::methods::server::WriteSequence;
use ovis_terminal_protocol::schema::scalar::ByteOffset;
use ovis_terminal_vt::Geometry;
use serde_json::json;

use super::TestResult;
use super::fixture::{NOW, admit, process, submission};
use super::pty::Pty;
use super::rig::{Rig, dispatched, op};

/// Bound on the marker after input-ready, from the plan's QA scenario.
const MARKER_BOUND: Duration = Duration::from_secs(10);
const BOUND: Duration = Duration::from_secs(20);

/// An unmodified program on a real PTY, the operator's leases and its live sequencer.
struct Session {
    pty: Pty,
    rig: Rig,
    sequencer: Sequencer,
}

impl Session {
    /// Spawns `argv` at 100x30 and waits for its first prompt or marker.
    fn start(argv: &[&str]) -> TestResult<Self> {
        let mut pty = Pty::start(argv, (100, 30))?;
        pty.until(BOUND, |pty| {
            pty.printed(b"ARMED") || pty.printed(b"PROMPT$ ")
        })?;
        let rig = Rig::new(pty.screen.clone())?;
        let mut sequencer = rig.fixture.sequencer(QueueLimits::DEFAULT)?;
        sequencer.stream_ready(process(pty.process())?, WriteSequence::new(0));
        Ok(Self {
            pty,
            rig,
            sequencer,
        })
    }

    /// Admits `line` plus Enter under the operator's interact lease and writes it.
    fn send(&mut self, line: &str) -> TestResult<Receipt> {
        let paint = self.pty.screen.first_paint()?;
        let modes = ModeSnapshot::new(paint.mode_revision, paint.modes);
        let at = Moment::new(NOW, ByteOffset::new(paint.offset.get()));
        let view = View::new(&self.rig.leases, &modes, at);
        let payload = Payload::Text {
            text: line,
            submit: &Submit::Enter,
        };
        let writer = self.rig.operator.writer().presenting(self.rig.input);
        let id = op();
        let entry = submission(id.as_str(), self.rig.fixture.expected()?, payload);
        admit(&mut self.sequencer, &view, (writer, entry))?;
        let input = self
            .sequencer
            .next(&view)?
            .dispatch
            .ok_or("nothing to write")?;
        let sequence = input.sequence;
        self.pty.holder.input(input)?;
        let acked = |pty: &Pty| pty.acks.iter().any(|ack| ack.sequence == sequence);
        self.pty.until(BOUND, acked)?;
        let ack = self
            .pty
            .acks
            .iter()
            .find(|ack| ack.sequence == sequence)
            .ok_or("no ack")?;
        self.sequencer
            .acknowledge(ack)?
            .ok_or_else(|| "no receipt".into())
    }

    /// One resize through the runtime boundary: commit, VT apply, holder acknowledgement.
    /// Returns the holder message and the input gate's answer before the acknowledgement.
    fn resize(&mut self, size: (u32, u32)) -> TestResult<(Resize, Option<&'static str>)> {
        let dispatch = dispatched(self.rig.resize(&op(), size)??)?;
        let epoch = dispatch.epoch();
        let (message, _) = self.rig.start(dispatch)?;
        let before_ack = self
            .rig
            .geometry
            .input_ready()
            .err()
            .map(|error| error.kind());
        self.pty.holder.resize(&message)?;
        let settled = self.rig.acknowledged(epoch)?;
        self.rig.publish(&settled)?;
        Ok((message, before_ack))
    }
}

/// The happy QA case: unmodified bash, attach at 100x30, resize to 120x40, marker command;
/// returns the transcript of events, holder message, receipt and timing.
///
/// # Errors
///
/// Fails when any step is refused.
///
/// # Panics
///
/// When an ordering, delivery or grid assertion does not hold.
pub fn unmodified_bash_receives_input_at_the_new_grid() -> TestResult<serde_json::Value> {
    // Given an unmodified interactive bash at 100x30 and a subscriber registered first.
    let mut session = Session::start(&["bash", "--norc", "--noprofile", "-i"])?;
    let subscription = session.rig.core.subscribe(None)?;

    // When the operator resizes to 120x40, then types the marker command once ready.
    let (message, before_ack) = session.resize((120, 40))?;
    let ready_at = Instant::now();
    let ready = session.rig.geometry.input_ready()?;
    let receipt = session.send("printf 'MARK_RESIZE_OK\\n'")?;
    let printed_marker = |pty: &Pty| pty.printed(b"MARK_RESIZE_OK\r\n");
    session.pty.until(MARKER_BOUND, printed_marker)?;
    let elapsed = ready_at.elapsed();
    session.send("stty size")?;
    session.pty.until(BOUND, |pty| pty.printed(b"40 120"))?;
    let pty = &session.pty;

    // Then change, checkpoint and input_ready were committed in order before the marker.
    let events: Vec<Event<GeometryEvent>> = (0..3)
        .map(|_| subscription.delivery.recv_timeout(BOUND))
        .collect::<Result<_, _>>()?;
    let kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect();
    assert_eq!(
        kinds,
        [
            "geometry.changed",
            "geometry.checkpoint",
            "geometry.input_ready"
        ]
    );
    let Some(GeometryEvent::Changed { from_offset, .. }) = events.first().map(|e| &e.data) else {
        return Err("no change event".into());
    };
    let marker = pty
        .offset_of(b"MARK_RESIZE_OK\r\n")
        .ok_or("marker missing")?;
    assert!(usize::try_from(from_offset.get())? <= marker);
    assert_eq!(before_ack, Some("not_ready"));
    assert_eq!(ready, GeometryEpoch::new(1));
    assert_eq!(message.geometry, (120, 40));
    // And bash wrote the marker on the 120x40 screen without any readiness report.
    assert!(
        matches!(receipt.delivery, Delivery::PtyWritten { .. }),
        "{receipt:?}"
    );
    assert!(elapsed < MARKER_BOUND);
    let paint = pty.screen.first_paint()?;
    assert_eq!(paint.geometry, Geometry::new(120, 40)?);
    assert!(String::from_utf8_lossy(&paint.bytes).contains("MARK_RESIZE_OK"));
    assert_eq!(pty.osc, 0);
    let transcript = json!({
        "events": events, "holder_resize": message, "receipt": receipt,
        "marker_offset": marker.to_string(), "marker_ms_after_input_ready": elapsed.as_millis().to_string(),
        "stty_size": "40 120", "osc_reports": pty.osc.to_string(),
    });
    Ok(transcript)
}

/// Bash that counts SIGWINCH and reports the count and the kernel grid on each line.
const COUNTER: &str = "n=0; trap 'n=$((n+1))' WINCH; printf 'ARMED\\n'; \
    while :; do if read -r line; then printf 'winch=%s size=%s\\n' \"$n\" \"$(stty size)\"; fi; done";

#[test]
fn repaint_leaves_the_childs_grid_and_signal_count_unchanged() -> TestResult {
    // Given a bash counting SIGWINCH at 100x30.
    let mut session = Session::start(&["bash", "--norc", "--noprofile", "-c", COUNTER])?;

    // When a viewer asks for two repaints and the program is asked to report.
    let first = session.rig.geometry.repaint()?;
    let second = session.rig.geometry.repaint()?;
    session.send("report")?;
    session
        .pty
        .until(BOUND, |pty| pty.printed(b"size=30 100"))?;

    // Then both paints are at the unchanged grid and the program saw no signal.
    assert_eq!(first.paint.geometry, Geometry::new(100, 30)?);
    assert_eq!(second.epoch, GeometryEpoch::new(0));
    assert!(session.pty.printed(b"winch=0 size=30 100"));

    // When the operator really resizes (the control that the counter works).
    session.resize((120, 40))?;
    session.send("report")?;
    session
        .pty
        .until(BOUND, |pty| pty.printed(b"size=40 120"))?;

    // Then the resize was signalled and repaint still sends nothing.
    assert!(!session.pty.printed(b"winch=0 size=40 120"));
    let repaint = session.rig.geometry.repaint()?;
    assert_eq!(repaint.paint.geometry, Geometry::new(120, 40)?);
    Ok(())
}

#[test]
fn the_holder_refuses_an_epoch_older_than_it_applied() -> TestResult {
    // Given the holder applied epoch 1.
    let mut session = Session::start(&["bash", "--norc", "--noprofile", "-c", COUNTER])?;
    let (applied, _) = session.resize((120, 40))?;

    // When a resize at the old epoch 0 reaches it.
    let stale = Resize {
        geometry_epoch: 0,
        geometry: (100, 30),
        ..applied
    };
    let refused = session
        .pty
        .holder
        .resize(&stale)
        .err()
        .map(|error| error.kind());

    // Then it is stale_geometry and the program still sees the epoch-1 grid.
    assert_eq!(refused, Some("stale_geometry"));
    session.send("report")?;
    session
        .pty
        .until(BOUND, |pty| pty.printed(b"size=40 120"))?;
    Ok(())
}
