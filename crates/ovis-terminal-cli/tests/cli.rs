//! Process tests of the real `ovis-terminal` binary: output channels, exit codes,
//! parse-before-connect and side-effect-free discovery.
#![cfg(unix)]

#[path = "cli/support.rs"]
pub mod support;

use std::os::unix::net::UnixListener;

use support::{Isolated, MACHINE, TestResult, field, json, serve};

const STATUS_REPEATS: usize = 5;

/// Status of a stopped runtime, repeated, never starts, creates or leaves anything;
/// schema and the manual answer offline; a running fixture reports running.
#[tokio::test]
async fn happy() -> TestResult {
    let dir = Isolated::new()?;
    let before = dir.entries()?;
    for _ in 0..STATUS_REPEATS {
        let output = dir.run(&["--json", "server", "status"], &[]).await?;
        let output = output.exit(0)?;
        assert_eq!(output.stderr, b"", "stderr: {:?}", output.stderr);
        let status = json(&output.stdout)?;
        assert_eq!(field(&status, "/state"), "stopped");
        assert_eq!(*field(&status, "/holder_count"), serde_json::Value::Null);
        assert_eq!(field(&status, "/census"), "not_reported");
        assert_eq!(dir.entries()?, before, "status created a file");
        assert_eq!(dir.owned_processes()?, 0, "status left a process behind");
    }

    let schema = dir.run(&["--json", "schema"], &[]).await?;
    let schema = schema.exit(0)?;
    let published = include_bytes!("../../../schema/ovis-terminal.openrpc.json");
    assert_eq!(json(&schema.stdout)?, json(published)?);

    let skill = dir.run(&["--json", "--skill"], &[]).await?;
    let skill = skill.exit(0)?;
    let manual = json(&skill.stdout)?;
    assert_eq!(field(&manual, "/version"), env!("CARGO_PKG_VERSION"));
    assert!(
        field(&manual, "/manual")
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );
    assert_eq!(dir.entries()?, before);
    assert_eq!(dir.owned_processes()?, 0);

    let served = serve(&dir).await?;
    let socket = served.socket.to_string_lossy().into_owned();
    let env = [
        ("OVIS_TERMINAL_SOCKET", socket.as_str()),
        ("OVIS_TERMINAL_CREDENTIAL", served.bearer.as_str()),
    ];
    let running = dir.run(&["--json", "server", "status"], &env).await?;
    let running = running.exit(0)?;
    let status = json(&running.stdout)?;
    assert_eq!(field(&status, "/state"), "running");
    assert_eq!(*field(&status, "/uptime_ms"), u64::MAX.to_string());

    let hello = serde_json::json!({
        "jsonrpc": "2.0", "id": "h", "method": "hello",
        "params": {
            "generation": "1",
            "client": {"name": "bridge-test", "version": "0"},
            "credential": served.bearer,
        },
    });
    let bridged = dir.bridge(&socket, format!("{hello}\n").as_bytes()).await?;
    let bridged = bridged.exit(0)?;
    let reply = json(&bridged.stdout)?;
    assert_eq!(field(&reply, "/id"), "h");
    assert_eq!(field(&reply, "/result/machine_id"), MACHINE);
    served.handle.shutdown().await?;
    Ok(())
}

/// A targetless read exits 7 with live candidates; an unknown flag exits 2 before any
/// connection; neither writes to stdout.
#[tokio::test]
async fn failure() -> TestResult {
    let dir = Isolated::new()?;
    let served = serve(&dir).await?;
    let socket = served.socket.to_string_lossy().into_owned();
    let env = [
        ("OVIS_TERMINAL_SOCKET", socket.as_str()),
        ("OVIS_TERMINAL_CREDENTIAL", served.bearer.as_str()),
    ];
    let targetless = dir.run(&["--json", "terminal", "get"], &env).await?;
    let targetless = targetless.exit(7)?;
    assert_eq!(targetless.stdout, b"");
    let report = json(&targetless.stderr)?;
    let error = field(&report, "/error");
    assert_eq!(field(error, "/kind"), "needs_target");
    assert_eq!(field(error, "/exit_code"), 7);
    assert_eq!(field(error, "/candidates_listed"), true);
    let ids: Vec<&str> = field(error, "/candidates")
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|candidate| field(candidate, "/terminal_id").as_str())
        .collect();
    assert_eq!(ids, ["term-a", "term-b"]);
    assert_eq!(field(error, "/candidates/0/machine_id"), MACHINE);
    served.handle.shutdown().await?;

    let listener = UnixListener::bind(dir.root().join("idle.sock"))?;
    listener.set_nonblocking(true)?;
    let idle = dir.root().join("idle.sock").to_string_lossy().into_owned();
    let idle_env = [("OVIS_TERMINAL_SOCKET", idle.as_str())];
    for args in [
        &["--json", "--unknown-flag", "server", "status"][..],
        &["--json", "terminal", "get", "--name", "a", "--unknown-flag"],
        &["--json", "call", "terminal.list", "--params", "[1]"],
        &["--json", "call", "hello"],
    ] {
        let refused = dir.run(args, &idle_env).await?;
        let refused = refused.exit(2)?;
        assert_eq!(refused.stdout, b"");
        let kind = field(&json(&refused.stderr)?, "/error/kind").clone();
        assert!(
            ["usage", "invalid_params", "unknown_method"]
                .contains(&kind.as_str().unwrap_or_default()),
            "{args:?}: {kind}"
        );
        let accepted = listener.accept();
        assert!(
            matches!(&accepted, Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "{args:?} connected before parsing"
        );
    }

    let deep = dir
        .root()
        .join("d".repeat(100))
        .to_string_lossy()
        .into_owned();
    let too_long = dir
        .run(
            &["--json", "server", "status"],
            &[("XDG_RUNTIME_DIR", deep.as_str())],
        )
        .await?;
    let too_long = too_long.exit(2)?;
    assert_eq!(too_long.stdout, b"");
    let report = json(&too_long.stderr)?;
    assert_eq!(field(&report, "/error/kind"), "socket_path_too_long");

    let stopped = dir.run(&["--json", "terminal", "get"], &[]).await?;
    let stopped = stopped.exit(7)?;
    let report = json(&stopped.stderr)?;
    let error = field(&report, "/error");
    assert_eq!(field(error, "/candidates_listed"), false);
    assert_eq!(*field(error, "/candidates"), serde_json::json!([]));
    Ok(())
}
