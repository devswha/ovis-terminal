//! The real binary as a real Windows service, run by the CI-owned fixture account.
//!
//! The workflow creates a local account, copies the binary where that account can run
//! it and sets `OVIS54_ACCOUNT`, `OVIS54_PASSWORD`, `OVIS54_BINARY` and `OVIS54_STATE`;
//! nobody logs in. A launcher process inside a kill-on-close job asks the service's
//! holder for a program, then the job is terminated; the program must survive with the
//! same incarnation because the service manager, not the launcher, started its holder.

use std::io::{BufRead as _, BufReader};
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::process::{Command, Stdio};

use ovis_terminal_holder::protocol::{Message, TailRequest, TerminalTarget};
use ovis_terminal_holder::service::windows::account_sid;
use ovis_terminal_protocol::domain::windows_service::ServiceAccount;
use serde_json::Value;
use tokio::net::windows::named_pipe::ClientOptions;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, TerminateJobObject,
};

const TEST: &str = "service::windows::fixture_tests::the_service_holder_outlives_its_launch_job_and_serves_only_its_owner";
const MARKER: &str = "OVIS-54-SERVICE-MARKER";

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set by the service fixture"))
}

fn at(value: &Value, pointer: &str) -> Value {
    value.pointer(pointer).cloned().unwrap_or(Value::Null)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// `ovis-terminal --json service <verb> <scope...>`: exit code and parsed stdout or stderr.
fn service(verb: &str, extra: &[&str]) -> (i32, Value) {
    let scope = ["--session", "ci54", "--state-dir", &var("OVIS54_STATE")];
    let owner = account_sid(&ServiceAccount::parse(&var("OVIS54_ACCOUNT")).unwrap()).unwrap();
    let mut args = vec!["--json", "service", verb];
    args.extend(scope);
    if verb != "install" {
        args.extend(["--owner-sid", owner.as_str()]);
    }
    args.extend(extra);
    let output = Command::new(var("OVIS54_BINARY"))
        .args(&args)
        .env("OVIS_TERMINAL_SERVICE_PASSWORD", var("OVIS54_PASSWORD"))
        .output()
        .unwrap();
    let code = output.status.code().unwrap();
    let text = if code == 0 {
        &output.stdout
    } else {
        &output.stderr
    };
    let parsed = serde_json::from_slice(text).unwrap_or_else(|_| {
        panic!(
            "{args:?} exited {code}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (code, parsed)
}

mod holder_link;

use holder_link::{Core, launch};

/// Installs and starts the service; returns its status and pipe name.
fn start_service() -> (Value, String) {
    let (code, status) = service("status", &[]);
    let stopped = (code, at(&status, "/state"), at(&status, "/installed"));
    assert_eq!(stopped, (0, "stopped".into(), Value::Null), "{status}");
    let (code, installed) = service("install", &["--account", &var("OVIS54_ACCOUNT")]);
    assert_eq!(code, 0, "{installed}");
    let (code, started) = service("start", &[]);
    let running = (
        code,
        at(&started, "/state"),
        at(&started, "/installed/state"),
    );
    assert_eq!(
        running,
        (0, "running".into(), "running".into()),
        "{started}"
    );
    assert_eq!(at(&started, "/record/transport"), "pipe", "{started}");
    let holder = at(&started, "/record/boot_proof/pid");
    assert_eq!(
        holder,
        at(&started, "/installed/pid"),
        "the service process is the holder"
    );
    let pipe = at(&started, "/record/address").as_str().unwrap().to_owned();
    assert!(
        pipe.starts_with(r"\\.\pipe\ovis-terminal-holder-"),
        "{pipe}"
    );
    (started, pipe)
}

/// Runs the launcher in a kill-on-close job, terminates the job, returns `SPAWNED ...`.
fn launch_in_job(pipe: &str) -> String {
    // SAFETY: null attributes and name create an unnamed job.
    let raw_job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    assert!(!raw_job.is_null(), "{}", std::io::Error::last_os_error());
    // SAFETY: CreateJobObjectW returned a new handle nothing else owns.
    let job = unsafe { OwnedHandle::from_raw_handle(raw_job) };
    let mut launcher = Command::new(std::env::current_exe().unwrap())
        .args([TEST, "--exact", "--ignored", "--nocapture"])
        .env("OVIS54_ROLE", "launcher")
        .env("OVIS54_PIPE", pipe)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    // SAFETY: both handles are open for the call.
    let assigned =
        unsafe { AssignProcessToJobObject(job.as_raw_handle(), launcher.as_raw_handle()) };
    assert_ne!(assigned, 0);
    let stdout = BufReader::new(launcher.stdout.take().unwrap());
    let mut lines = stdout.lines().map(Result::unwrap);
    let spawned = lines
        .find(|line| line.starts_with("SPAWNED "))
        .expect("no program reported");
    assert!(
        lines.any(|line| line == "MARKER-SEEN"),
        "the launcher saw no marker"
    );
    // SAFETY: the job handle is open; this ends the launcher, the client that asked.
    let terminated = unsafe { TerminateJobObject(job.as_raw_handle(), 9) };
    assert_ne!(terminated, 0);
    assert!(
        !launcher.wait().unwrap().success(),
        "the launcher outlived its job"
    );
    spawned
}

/// Adopts the holder as a new core and returns its single terminal's retained output.
async fn adopt_survivor(pipe: &str, spawned: &str) -> Vec<u8> {
    let (mut link, terminals) = Core::adopt(pipe).await;
    let [terminal] = terminals.as_slice() else {
        panic!("{terminals:?}")
    };
    let pid = terminal.pid.map(|pid| pid.to_string());
    let token = terminal
        .start_token
        .as_ref()
        .map(|token| token.as_str().to_owned());
    let reported: Vec<String> = spawned.split(' ').skip(1).map(str::to_owned).collect();
    assert_eq!(
        [pid, token],
        [reported.first().cloned(), reported.get(1).cloned()],
        "a new incarnation answered"
    );
    assert!(
        terminal.exit.is_none(),
        "the program died with its launch job"
    );
    let target = TerminalTarget {
        terminal_id: terminal.terminal_id.clone(),
        terminal_incarnation: terminal.terminal_incarnation,
        process_incarnation: terminal.process_incarnation,
    };
    link.send(Message::TailRequest(TailRequest {
        target,
        from_offset: 0,
    }))
    .await;
    loop {
        if let Message::Tail(tail) = link.next().await {
            break tail.bytes;
        }
    }
}

#[test]
#[ignore = "needs the CI-owned Windows service fixture account (OVIS54_* variables)"]
fn the_service_holder_outlives_its_launch_job_and_serves_only_its_owner() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _entered = runtime.enter();
    if std::env::var("OVIS54_ROLE").as_deref() == Ok("launcher") {
        return runtime.block_on(launch(&var("OVIS54_PIPE")));
    }
    let (started, pipe) = start_service();
    let denied = ClientOptions::new()
        .open(&pipe)
        .map(drop)
        .map_err(|e| e.kind());
    assert_eq!(
        denied,
        Err(std::io::ErrorKind::PermissionDenied),
        "another account opened it"
    );
    let spawned = launch_in_job(&pipe);
    let retained = runtime.block_on(adopt_survivor(&pipe, &spawned));
    assert!(
        String::from_utf8_lossy(&retained).contains(MARKER),
        "the survivor's output was lost"
    );
    let (code, stopped) = service("stop", &[]);
    assert_eq!(
        (code, at(&stopped, "/state")),
        (0, "stopped".into()),
        "{stopped}"
    );
    assert_eq!(
        at(&stopped, "/ended_holder/pid"),
        at(&started, "/record/boot_proof/pid")
    );
    let (code, removed) = service("uninstall", &[]);
    assert_eq!(code, 0, "{removed}");
    let (code, refused) = service("start", &[]);
    assert_eq!(
        (code, at(&refused, "/error/kind")),
        (3, "not_registered".into()),
        "{refused}"
    );
}
