//! The closed scenario format: every key is typed, an unknown key is an error.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use super::matcher::Expect;
pub(crate) use super::serve::{Announced, FixtureKind, Serve, Stop};
use crate::ids::{CaseName, ConnName, ScenarioId};

/// Marker that makes a file a registered, runner-executable scenario.
pub(crate) const FORMAT: &str = "ovis-conformance/1";

/// One registered scenario file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScenarioFile {
    /// Always [`FORMAT`].
    pub(crate) format: String,
    pub(crate) scenario: ScenarioId,
    pub(crate) title: String,
    /// Plan todo that owns the scenario.
    pub(crate) todo: u16,
    #[serde(default)]
    pub(crate) closes: Vec<String>,
    #[serde(default)]
    pub(crate) delivers: Vec<String>,
    #[serde(default)]
    pub(crate) excludes: Vec<String>,
    pub(crate) platforms: Vec<Platform>,
    #[serde(default)]
    pub(crate) cases: Vec<Case>,
    /// Present only on suites such as `all/all`.
    #[serde(default)]
    pub(crate) suite: Option<Suite>,
}

/// Operating systems a scenario runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Platform {
    Linux,
    Macos,
    Windows,
}

impl Platform {
    pub(crate) const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// Which scenarios a suite runs.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Suite {
    /// Every registered non-suite scenario found under `conformance/scenarios/`.
    Registered,
}

/// One case of a scenario.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Case {
    #[serde(rename = "case")]
    pub(crate) name: CaseName,
    pub(crate) summary: String,
    pub(crate) steps: Vec<Step>,
}

/// One action or check; exactly one key per step object.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    /// Start the fixture server in the isolated root and await its ready signal.
    Serve(Serve),
    /// Open a connection through the binary's `connect` bridge or a direct socket.
    Connect(Connect),
    /// Send `hello` with the fixture credential.
    Hello(Hello),
    /// Write requests with an explicit write segmentation.
    Send(Send),
    /// Write the raw bytes of a named wire fixture.
    SendWire(SendWire),
    /// Await exactly one reply per listed id.
    Replies(Replies),
    /// Await one notification.
    Notification(Notification),
    /// Await end of stream with every received byte decoded.
    Closed(Closed),
    /// Close this side's write half.
    Hangup(ConnRef),
    /// Send every request of a protocol fixture and check each typed rejection.
    Rejections(Rejections),
    /// Run the binary once in the isolated root.
    Run(Run),
    /// Compare the fixture's effect directory with the expected files.
    Effects(Effects),
    /// Read a logical result page by page and check every byte.
    Pages(Pages),
    /// Stop the fixture and check its serve report.
    Stop(Stop),
    /// Await the fixture's next stdout announcement, such as a successor boot.
    Announced(Announced),
    /// Splice the steps of another scenario's case.
    Include(Include),
    /// Resend one read until its reply matches, a bounded number of times.
    PollReply(PollReply),
    /// Require the inner step to fail with this kind.
    ExpectFailure(ExpectFailure),
    /// Send planned requests, or only the one `--inject` names, and check each reply.
    Inject(Inject),
    /// Start the binary on a fresh outer PTY.
    PtyOpen(super::pty::PtyOpen),
    /// Type bytes on an outer PTY.
    PtyType(super::pty::PtyType),
    /// Require output on an outer PTY.
    PtyExpect(super::pty::PtyExpect),
    /// Require output to be absent from what an outer PTY delivered so far.
    PtyAbsent(super::pty::PtyAbsent),
    /// Resize an outer PTY.
    PtyResize(super::pty::PtyResize),
    /// Await the PTY client's exit and check its terminal settings.
    PtyExit(super::pty::PtyExit),
    /// Run planned quality gates in-process, or only the one `--inject` names.
    Gates(super::gates::Gates),
    /// Read one attach stream until a marker and check its bytes and control records.
    GraphicsTranscript(crate::fixture::graphics::GraphicsTranscript),
    /// Collect one attach stream's bytes from an offset until a marker.
    StreamData(super::stream::StreamData),
    /// Audit the Ovis consumer fixture set and replay its cases of one kind.
    Replay(Replay),
}

/// Replays every fixture case of `kind`, each against a fresh server in a fresh root.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Replay {
    pub(crate) kind: ovis_terminal_replay::CaseKind,
}

/// How a connection reaches the endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Via {
    /// `ovis-terminal connect`: stdin/stdout of the binary under test.
    Bridge,
    /// The runner's own socket, for exact write segmentation.
    Socket,
}

/// The fixture credential a connection's hello presents.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Principal {
    /// The bootstrap owner, holding every permission.
    #[default]
    Owner,
    /// A principal that may only observe the machine.
    Observe,
    /// A principal that may observe, type and answer prompts, and nothing else.
    Writer,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Connect {
    pub(crate) conn: ConnName,
    pub(crate) via: Via,
    #[serde(default)]
    pub(crate) principal: Principal,
    /// Presents the bearer an earlier `keep_bearer` reply kept under this name instead of
    /// the principal's fixture credential.
    #[serde(default)]
    pub(crate) bearer: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConnRef {
    pub(crate) conn: ConnName,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Hello {
    pub(crate) conn: ConnName,
    pub(crate) framed: bool,
    pub(crate) expect: Expect,
}

/// How encoded bytes are cut into writes.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Split {
    /// One write for every record of the step: coalesced.
    #[default]
    Whole,
    /// One write per byte.
    Bytewise,
    /// Writes of these sizes, cycling until every byte is sent.
    Chunks(Vec<usize>),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Send {
    pub(crate) conn: ConnName,
    pub(crate) requests: Vec<Value>,
    #[serde(default)]
    pub(crate) split: Split,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SendWire {
    pub(crate) conn: ConnName,
    /// Name of an entry in `fixtures/wire/frames.json`.
    pub(crate) frame: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Replies {
    pub(crate) conn: ConnName,
    pub(crate) expect: BTreeMap<String, Expect>,
    #[serde(default)]
    pub(crate) within_ms: Option<u64>,
    /// Reply id to the name under which its `/result/issued/bearer` is kept for a later
    /// `connect`; the reply must carry one.
    #[serde(default)]
    pub(crate) keep_bearer: BTreeMap<String, String>,
}

/// One read resent until its reply satisfies `expect`, at most `attempts` times.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PollReply {
    pub(crate) conn: ConnName,
    pub(crate) id: String,
    pub(crate) request: Value,
    pub(crate) expect: Expect,
    pub(crate) attempts: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Notification {
    pub(crate) conn: ConnName,
    pub(crate) method: String,
    pub(crate) expect: Expect,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Closed {
    pub(crate) conn: ConnName,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rejections {
    pub(crate) conn: ConnName,
    /// File below `fixtures/protocol/`.
    pub(crate) fixture: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Run {
    pub(crate) args: Vec<String>,
    /// Point the binary at the fixture endpoint and credential.
    #[serde(default)]
    pub(crate) endpoint: bool,
    pub(crate) exit: i32,
    #[serde(default)]
    pub(crate) stdout: Expect,
    #[serde(default)]
    pub(crate) stderr: Expect,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Effects {
    /// File name to its JSON content; no other file may exist.
    pub(crate) entries: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pages {
    pub(crate) conn: ConnName,
    pub(crate) target: Value,
    pub(crate) page_bytes: crate::ids::Decimal,
    pub(crate) total_bytes: crate::ids::Decimal,
    /// Lowercase hex SHA-256 of the complete logical result.
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Include {
    pub(crate) scenario: ScenarioId,
    pub(crate) case: CaseName,
}

/// Requests a failure case can inject: each merges its params over `base`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Inject {
    pub(crate) conn: ConnName,
    /// Params every injection starts from; `$op:` labels are fresh per injection.
    pub(crate) base: Value,
    pub(crate) planned: Vec<Injection>,
}

/// One injectable request and the reply it must get.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Injection {
    /// `{"method", "params"}`, compared exactly with `--inject`.
    pub(crate) request: Value,
    pub(crate) expect: Expect,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExpectFailure {
    pub(crate) kind: String,
    pub(crate) step: Box<Step>,
}
