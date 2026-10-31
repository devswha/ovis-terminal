feat(windows-service): serve named pipes with windows service isolation (plan todo 54)

Closes, on the parts this PR implements: GAP-001, GAP-002 and GAP-129 (pipe endpoint, discovery and authentication), and GAP-121 (holders outlive the launching client's job). GAP-048 and GAP-050 are only partly addressed: see Unmet. Delivers IDEAL-001, IDEAL-002, IDEAL-093 and IDEAL-102 on Windows; IDEAL-034 only at the holder-wire level. Enforces EXCL-014, EXCL-016 and EXCL-027. Does not reproduce V-017 (every `unsafe` block is a single operation with a `SAFETY:` comment), V-250 (no fixed sleeps in tests) or V-4077 (a failed accept ends only that client).

## What changed

- `crates/ovis-terminal-protocol/src/domain/windows_service.rs` (new)
  - `PipeName`: a validated `\\.\pipe\` name, at most 256 characters, with no backslash or NUL in the name part.
  - `ServiceAccount`: `.\name` or `DOMAIN\name`. Built-in service accounts (`LocalSystem`, `NetworkService`, `NT AUTHORITY\*`, `SYSTEM`) are refused, so programs run as the owning user.
  - `HolderPlacement` (shared / dedicated) and `ScmState`.
  - `HostRecord` (`ovis-terminal-windows-host/1`). It wraps a `DiscoveryRecord` whose transport is `pipe` and whose address is the pipe name, never a marker path. Parsing refuses a non-pipe address with `descriptor_malformed`.
  - `WindowsServiceStatus` and `ServiceRecord::pipe_registration`.
  - `WindowsServiceRecordError` with `kind()`/`hint()`.
- `crates/ovis-terminal-holder/src/service/windows/` (new): the service host.
  - Platform-neutral parts, tested on Linux:
    - `paths`: label `ovis-terminal-holder-<session>`, state directory, and the pipe `\\.\pipe\ovis-terminal-holder-<owner SID>-<session>`.
    - `definition`: the command line, quoted by `CommandLineToArgvW` rules (proptest round trip), and the description marker `label=…; owner=<SID>`.
    - `record`: atomic replace, and withdraw only by the boot proof the record names.
    - `observe::judge`: a record whose process is gone or replaced is `stale_endpoint` even with no pipe left; no record plus no pipe is `stopped`.
    - `HostError`.
  - Windows-only:
    - `pipe`:
      - The security descriptor is `O:<sid>D:P(A;;GA;;;<sid>)`, built from the process token. A conversion failure fails the bind; there is no default or null-DACL fallback.
      - `FILE_FLAG_FIRST_PIPE_INSTANCE` and `PIPE_REJECT_REMOTE_CLIENTS` are set on the instance.
      - The client's SID is read by impersonation after its first bytes.
      - The applied descriptor is read back with `GetSecurityInfo`, and `WaitNamedPipeW` gives a presence probe that opens nothing.
    - `scm`: install (it grants `SeServiceLogonRight` through LSA), uninstall, start, stop and installation. Each verb opens the service by label and refuses it as `foreign_registration` unless the description marker matches; state waits are bounded.
    - `host`: an overlapped accept loop (tokio named pipes). The next instance exists before an accepted one is handed off; a failed accept is counted, not fatal.
    - `link` / `terminals`: the holder wire (generation 2, unchanged) over the pipe, backed by `ConptyTerminal`:
      - Hello → Welcome with held terminals;
      - Spawn → Spawned / SpawnFailed;
      - Output; Input with a process-incarnation check and a fence epoch (a stale fence gets a zero-byte `InputWritten`); InputRejected on overflow;
      - Resize; Signal (Interrupt = Ctrl+C byte, Terminate/Kill = job); Tail; Forget; Drain; Goodbye.
      - A dedicated placement owns at most one terminal.
    - `service_main`: `StartServiceCtrlDispatcherW` entry point. Before the holder starts, the service process puts itself in a kill-on-close job. Every ConPTY program and any grandchild it creates is then born inside the holder's job, which closes the escape window between `CreateProcessW` and the per-terminal `AssignProcessToJobObject` that todo 53 recorded.
- `crates/ovis-terminal-server/src/listen/pipe/` (+ `listen/pipe.rs` module root):
  - `PipeEndpoint::bind/accept`: an instance whose applied descriptor is not owner-only is `endpoint_insecure`, and a name another process holds is `already_running`.
  - `PipeConnection::admit`: the applied DACL plus the client token are checked through the landed authz `EndpointPolicy` / `PipeNamespace`.
  - `serve_pipe` drives the same `Connections` the Unix socket uses, so hello, framing, limits and `$/closing` are identical.
  - `evidence` (neutral) maps operating-system facts to `PeerEvidence`.
- `crates/ovis-terminal-server/src/dispatch/lanes/windows_service.rs`: `service.describe` over this owner's installed services. A neutral `describe` runs everywhere; the `Method` impl and the SCM lookup are Windows-only. A foreign same-named service is not described.
- `crates/ovis-terminal-cli/src/service/windows/`: `service status | install --account | uninstall | start | stop | run(hidden)` with `--session`, `--state-dir` and `--owner-sid`. The password comes only from `OVIS_TERMINAL_SERVICE_PASSWORD`. `status` changes nothing.
- `conformance/scenarios/platform/windows-service.json`: a declared-only descriptor, listed in `declared.json` with owner 54 (see DEFERRED VERIFICATION).
- `.github/workflows/probe-stress-macos.yml`: the option `windows-service` and one `windows-2025` job, appended at the end.
  - It creates a local account with a random masked password (no login).
  - It copies the binary to `C:\ovis54` and runs the native tests 3 times.
  - It removes the service and the account afterwards.

## Seams

- New public API: `ovis_terminal_protocol::domain::windows_service`; `ovis_terminal_holder::service::windows::*`; `ovis_terminal_server::listen::pipe::*`; `ovis_terminal_server::dispatch::lanes::windows_service`; `ovis_terminal_cli::service::windows`.
- Additive inherent constructors on landed protocol types, in the new file: `DiscoveryRecord` is built through `HostRecord::new`, plus `ServiceRecord::pipe_registration`.
- `ovis-terminal-cli`: `crate::service::{ServiceCommand, execute, ServiceError}` now re-exports the Unix or Windows module. The invocation slot lines (`args.rs`, `plan.rs`, `error.rs`, `report.rs`, `invocation.rs`) changed `#[cfg(unix)]` to `#[cfg(any(unix, windows))]` and use that path; Unix behaviour is unchanged.
- `exit.rs` gains the kinds `invalid_pipe_name`, `invalid_account`, `privileged_account` and `password_required` (usage, 2) and `service_exists` (conflict, 11).
- Manifests:
  - holder: `[target.'cfg(windows)'.dependencies]` gains `tokio` and windows-sys features (Pipes, Services, Authorization, Authentication_Identity, Storage_FileSystem);
  - server and cli: windows-only `windows-sys` dev-dependencies, version 0.61, already in Cargo.lock.
- Holder wire generation stays 2; no message shape changed. `#[cfg(unix)] pub mod unix_service` is untouched.

## Before

`absent`: there was no comparable baseline capability, and this is not an executed failing test. Gap baseline: GAP-001, GAP-002, GAP-048, GAP-050, GAP-121, GAP-129. In herdr-core, `platform/windows.rs:174` names a pipe DACL that allows `SY` plus the user, `windows.rs:221` creates instances, and `windows.rs:532` uses `spawn_detached` breakaway rather than a service manager.

## After

Local host: `Linux 7.0.0-38-generic x86_64`. Native Windows results come from run https://github.com/devswha/ovis-terminal/actions/runs/41236769424 (job `windows-service` on windows-2025, `MINGW64_NT-10.0-26100 x86_64`, rustc 1.99.1, commit 79ddd3c), which concluded `success`; the log is `windows-service-run1.log`.

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal                                   # exit 0
$ cargo test -p ovis-terminal-server -p ovis-terminal-holder -p ovis-terminal-cli -p ovis-terminal-protocol --all-targets   # exit 0, no failures
  lane tests on Linux: protocol domain::windows_service 9, holder service::windows 10, server listen::pipe::evidence 4 + lanes::windows_service 1
$ cargo clippy -p ovis-terminal-server -p ovis-terminal-holder -p ovis-terminal-cli -p ovis-terminal-protocol --all-targets -- -D warnings   # exit 0
$ RUSTFLAGS=-Dwarnings cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings  # exit 0
$ RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked           # WINCHECK_EXIT 0
$ bash scripts/check.sh   # 823 tests run: 823 passed (1 slow), 1 skipped; check.sh: all steps passed; exit 0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario all/all --isolated --case all --evidence all-all.json
  {"result":"passed","totals":{"assertions":"685","cases":"24","declared_only":"31","failed":"0","passed":"24","skipped":"0"}}   # exit 0
```

Native Windows, 3 of 3 iterations, every listed test `ok` each time. Holder: 13 passed; server: 11 passed; CLI fixture: 1 passed (19.95 s, 0.26 s, 0.19 s; my unverified guess for the slow first run is first-logon profile creation for the fresh account). What the tests assert:

- `fixture_tests::the_service_holder_outlives_its_launch_job_and_serves_only_its_owner`: the real `ovis-terminal.exe` under the SCM, as the CI account. In order:
  - `service status` → `stopped`, `installed: null`.
  - `install --account .\ovis54ci` → exit 0.
  - `start` → exit 0. Its result shows `state: running`, `installed.state: running`, `record.transport: pipe` and a `record.address` starting `\\.\pipe\ovis-terminal-holder-`. The record's boot-proof pid equals the service process pid.
  - Opening the pipe as runneradmin, an administrator but not the owner, fails with `PermissionDenied`.
  - A launcher process (impersonating the account) runs inside a kill-on-close job. It starts `cmd /c echo OVIS-54-SERVICE-MARKER & ping …` through the holder pipe and reports `SPAWNED <pid> <start_token>` and `MARKER-SEEN`. `TerminateJobObject` then ends it.
  - A new core adopts: Welcome lists exactly one terminal with the same pid and start token and no exit, and `Tail` from 0 contains the marker.
  - `stop` → `state: stopped`, with `ended_holder.pid` equal to the record's pid.
  - `uninstall` → exit 0; then `start` → exit 3 `not_registered`.
- `host_tests::a_core_restart_re_adopts_the_same_incarnation_and_its_output_survives` (in-process host, real pipe and ConPTY):
  - spawn, output until the marker, drop the core;
  - a second core's Welcome has the same pid, start token and process incarnation, and no exit;
  - Tail contains the marker; Kill → Exit with `final_offset` ≥ the tail's end;
  - stop removes the record, and the observation is `stopped`.
- `host_tests::a_dedicated_holder_owns_one_terminal_and_refuses_another`: a second Spawn gets `SpawnFailed` for `two`.
- `host_tests::a_claimed_pipe_is_never_joined_and_a_dead_holder_record_is_stale`:
  - a second host is `already_running`;
  - a record naming a reaped pid observes `stale_endpoint` (`process_gone`/`process_replaced`) and is left in place;
  - a new host then starts.
- `listen::pipe::tests::hello_and_pipelined_requests_share_one_owner_only_pipe_connection`:
  - the applied owner is the current SID, and the DACL is exactly one allow ACE for it (not null);
  - hello plus 3 pipelined `doc.get` requests on one connection get 4 replies, ids a/b/c, and the handler ran 3 times;
  - the report shows 1 accepted and 0 failures.
- `a_credential_of_another_runtime_is_refused_at_hello_and_nothing_runs`: hello with `other-runtime-token` gets `error.data.kind` ∈ {`invalid_credential`, `unauthenticated`}, no result, and the handler ran 0 times.
- `an_anonymous_token_cannot_open_the_pipe`: `CreateFileW` under `ImpersonateAnonymousToken` fails with OS error 5.
- `a_second_listener_never_joins_a_claimed_name`: `already_running`.
- `cancelled_pending_accepts_leave_the_endpoint_serving`: 50 accepts are cancelled mid-`ConnectNamedPipe` (zero-duration timeout); a client then completes hello.
- `lanes::windows_service::native::tests::…`: an uninstalled session is not described, and no state directory is created.

## DEFERRED VERIFICATION (runner lands in Todo 21)

The runner has landed, but `platform/windows-service` cannot be executable yet, for three reasons:

- its only fixture serves a Unix socket;
- its isolated environment (`env_clear` plus a Unix `OVIS_TERMINAL_SOCKET`) removes what a pipe name and the service need;
- a windows-only case would be `skipped` on every runner it has.

`terminal.run`, attach and leases over the pipe are served only once todo 31 assembles the binary. The scenario is therefore declared (owner 54), like `platform/conpty` (todo 53). The plan's commands were run verbatim:

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario platform/windows-service --isolated --case happy --evidence "$ATTEMPT_DIR/task-54-ovis-terminal-v1.json"
{"case":"happy","error":{"detail":"platform/windows-service is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},...,"result":"failed","scenario":"platform/windows-service","totals":{"assertions":"0","cases":"0","declared_only":"0","failed":"0","passed":"0","skipped":"0"}}
exit 1
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario platform/windows-service --isolated --case failure --inject '{"method":"hello","params":{"credential":"other-runtime-token"}}' --evidence "$ATTEMPT_DIR/task-54-ovis-terminal-v1.json" --append
{"case":"failure","error":{"detail":"platform/windows-service is a declaration, not a registered executable scenario",...,"kind":"scenario_not_executable"},"result":"failed",...}
exit 1
```

The behaviours these commands would check are proven natively by the tests above:

- unauthenticated / invalid_credential;
- the wrong user cannot open the pipe (`PermissionDenied`, and OS error 5 for the anonymous token);
- no fallback to a global ACL (applied DACL = one owner ACE; null / Everyone / foreign-owner descriptors refused in `evidence::tests`).

## QA

- Happy and failure ran natively on Windows: run 41236769424, 3 of 3 iterations, `RESULT pass`. The conformance scenario runs are DEFERRED, as above.
- PR state: integration moved to 436506e after this lane branched. A dry `git merge-tree` shows a single conflict, in the shared `.github/workflows/probe-stress-macos.yml` (options list and job union). Per the lane rule I did not merge, so PR CI waits for the orchestrator's union merge.

## Adversarial

- Stale state: a marker without a live pipe or process is `stale_endpoint` (`process_gone` / `process_replaced`), never `running`. Observation never removes it, and start replaces only a proven-dead record (Linux `observe::tests` and native `host_tests`).
- Repeated interruptions: 50 pending overlapped accepts were cancelled, and the endpoint still serves a client.
- Squatting: a first-instance claim on a held name is refused (`already_running`) for both the control pipe and the holder pipe.
- Foreign administration: every SCM verb checks the description marker for label and owner SID; a mismatch is `foreign_registration`, and `describe` skips it.
- Privilege: `LocalSystem`, `NetworkService` and `NT AUTHORITY\*` accounts are refused before install (`privileged_account`).
- Flaky tests: there are no fixed sleeps; every wait is a bounded read of a real signal (pipe readiness, a holder message, a process exit). The product's SCM state wait is a bounded poll (25 ms), since SCM offers no blocking wait without APC callbacks.

## Contract checklist

1. New wire counters: none. Holder-wire u64 fields keep their decimal-string codec, and generation stays 2. PIDs stay `ProcessPid` JSON numbers, as in the landed `BootProof`.
2. `WindowsServiceRecordError`, `HostError` and `LinkError` are thiserror enums with `kind()`/`hint()` and `#[source]`; Display does not repeat the kind (asserted for the protocol enum).
3. Newtypes: `PipeName`, `ServiceAccount`, `HostSession`; incarnation and offset identities reuse the holder protocol types.
4. Tests assert behaviour (wire round trips, typed kinds, OS-applied security, process identity), with proptest for pipe names and command-line quoting.
5. No `#[allow]`/`#[expect]` and no TODO; hygiene reports 0 violations.
6. Evidence above states only what the tests assert.

## Unmet / Risks

- Not delivered here: from the happy QA, "two clients attach, owner inputs/resizes, observer is denied" and "core restart preserves child incarnation" through the real control server. Attach, leases and served `terminal.run` need todo 31 assembly; core-restart incarnation preservation is proven at the holder-wire level only. GAP-050 (geometry event before the new-grid screen) is not addressed.
- The fence is checked on arrival only. Input already queued in `ConptyTerminal` is not revoked when a newer fence arrives, unlike the Unix holder; `InputQueue` exposes no revocation.
- A grandchild born in the per-terminal assignment window stays in the holder job, not the terminal job. `Kill` of that terminal does not reach it; stopping the service does.
- The service account's password is passed through an environment variable at install; the SCM stores it as an LSA secret, and nothing persists it.
- The applied descriptor carries no mandatory integrity label (herdr-core added one); authz evidence does not model it.
- The service host serves one holder per service process; dedicated placement is selected at install (hidden flag), not per launch.
