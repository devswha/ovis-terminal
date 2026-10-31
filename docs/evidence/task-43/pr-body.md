## Summary

Todo 43 (wave 10): `machine.add`, `machine.get` and `machine.rename` over stable machine identities. A remote runtime is added only after it answers a fresh boot challenge with a proof keyed by the credential it issued, and only if it proves the explicitly configured `expected_machine_id`. SSH and custom-argv transports carry protocol bytes over stdin/stdout unchanged; stderr is drained into bounded diagnostics. The handshake is bounded, and every failure is a typed kind. Names and aliases resolve once into a `Binding` on the machine id, and a renamed machine keeps its old name as an alias that no other machine can take.

Closes GAP-106, GAP-107, GAP-120, GAP-129, GAP-134. Delivers IDEAL-002, IDEAL-080, IDEAL-081, IDEAL-102, IDEAL-107.

## Before

`absent` (no comparable baseline capability; not an executed failing test). Gap baseline: GAP-106, GAP-107, GAP-120, GAP-129, GAP-134.

## After

Native OS/arch: `Linux x86_64`. Binary `target/debug/ovis-terminal` sha256 `1530fb97d153617d4a7c5fbb1a3cae19e04b3eff20d4d4af32988085a26226d3`.

### DEFERRED VERIFICATION (runner lands in Todo 21)

The todo-21 runner has landed, but it cannot execute this scenario yet:
- No served binary registers the machine lane until todo 31 assembles the real server.
- The scenario needs two private runtimes joined by a stdio transport. The runner starts one fixture per case; todo 52 composes remote dispatch across two private runtimes.
- The plan's `--inject` and `--append` flags do not exist in the runner.

So `conformance/scenarios/remote/identity.json` is a declaration owned by todo 43 and listed in `conformance/scenarios/declared.json` with that reason. These are the exact attempts and exit codes (`docs/evidence/task-43/conformance-attempts.log`):

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal                      -> 0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario remote/identity --isolated --case all --evidence "$ATTEMPT_DIR/task-43-ovis-terminal-v1.json"
{"case":"all","error":{"detail":"remote/identity is a declaration, not a registered executable scenario",...,"kind":"scenario_not_executable"},...,"result":"failed",...}
EXIT=1
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario remote/identity --isolated --case happy --evidence "$ATTEMPT_DIR/task-43-ovis-terminal-v1.json"
{"case":"happy","error":{...,"kind":"scenario_not_executable"},...}
EXIT=1
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario remote/identity --isolated --case failure --inject '{"method":"machine.add","params":{"expected_machine_id":"wrong-machine"}}' --evidence "$ATTEMPT_DIR/task-43-ovis-terminal-v1.json" --append
error: unexpected argument '--inject' found
EXIT=2
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario all/all --isolated --case all --evidence "$ATTEMPT_DIR/all.json"
ALL_EXIT=0   totals: cases 16, passed 16, failed 0, skipped 0, declared_only 24 (remote/identity.json listed), assertions 345
```

### What runs instead, and what it asserts

The happy and failure observables run as unit tests in `crates/ovis-terminal-server/src/dispatch/lanes/machine_identity/qa.rs`. Each test builds two isolated runtimes:
- **Local:** real SQLite store, authority and dispatcher with the lane, served by `serve_unix` and driven with literal NDJSON.
- **Remote:** its own authority and runtime identity behind the real `Connections` layer.
- **Transport:** a real child process (`sh -c 'cat < "$2" & exec cat > "$1"'`, with FIFO paths passed as positional arguments) relaying its stdio to the remote over two FIFOs.

`cargo nextest run ... -E 'test(/machine_identity|identity::|transport::|attest|domain::machine/)'` gives 37 passed, exit 0 (`docs/evidence/task-43/lane-tests.log`). The two-runtime tests assert:
- `a_machine_keeps_its_destination_and_proven_boot_across_a_rename`:
  - `machine.add` returns `machine_id` = the remote's id, `remote_boot_id` = the remote's boot, `state` `connected` and `revision` `"1"`.
  - `machine.rename` box→box2 returns `aliases` `["box"]` and `revision` `"2"`.
  - `machine.get` by `{name: box}`, `{name: box2}` and `{machine_id}` all return the same `machine_id` and `remote_boot_id`.
- `a_runtime_that_proves_another_machine_is_refused_before_anything_is_recorded`: with `expected_machine_id: "wrong-machine"` the reply kind is `trust_mismatch`. `machine.get {name: box}` is then `not_found`, and a retry under the same name with the right identity succeeds, which shows nothing was recorded and the name was released.
- `a_credential_the_remote_did_not_issue_fails_authentication`: the reply kind is `auth_failed`.
- `the_same_machine_cannot_be_added_under_a_second_name`: the kinds are `machine_exists`, then `duplicate_name`.
- `adds_without_trust_or_grant_or_with_a_silent_or_chatty_transport_are_typed`:
  - no expected id gives `trust_required`;
  - an observe-only credential gives `forbidden`;
  - a silent transport gives `handshake_timeout` (300 ms bound);
  - a `Last login:` banner on stdout gives `transport_failed`;
  - an observer `machine.get` gives `not_found` (reads need only observe).

## QA

- `bash scripts/check.sh`: exit 0. fmt, workspace clippy `-D warnings`, nextest 589 passed / 1 skipped, doc tests and hygiene all pass (`docs/evidence/task-43/check-summary.log`).
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked`: exit 0 (`windows-check.log`).
- `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --workspace --all-features --locked`: exit 0. The first CI round failed on a public doc linking to the private `Link::received`; fixed in the second commit.
- CI round 2: `test (macos-latest)` failed binding the lane fixture's socket under the Darwin `$TMPDIR` (`/var/folders/...`), which exceeds the 104-byte `sun_path` limit. The fixture root is now `/tmp/ovt43-<pid>-<n>` (mode 0700, removed after each test) on every platform. The lane binds no socket in product code; only the test fixture calls `UnixEndpoint::bind`.
- `cargo nextest run -p ovis-terminal-conformance --all-targets --locked`: 24 passed, including `every_scenario_file_is_executed_or_a_listed_declaration` with the new declaration.
- `openrpc write schema/ovis-terminal.openrpc.json` regenerated; `openrpc compat schema/released.openrpc.json` reports `compatible_with`.
- macOS stress: new `machine-identity` option and job in `probe-stress-macos.yml` (100 iterations of the remote and lane tests): https://github.com/devswha/ovis-terminal/actions/runs/41211223775 (dispatched on the first head).

## Adversarial

| Class | Test | Asserted outcome |
|---|---|---|
| Malformed input: transport banner on stdout | `transport::handshake::tests::a_banner_before_the_reply_is_not_protocol`; server `..._chatty_transport_are_typed` | `transport_failed`; `NotProtocol.first_bytes == b"Welcome to box"` |
| Credential prompt on stdout | `a_credential_prompt_times_out_typed_with_the_prompt_as_evidence` | `handshake_timeout`; received bytes `b"Password: "`; nothing is answered |
| Stderr diagnostics | `stderr_chatter_never_reaches_the_protocol_stream`, `a_transport_that_exits_reports_its_status_and_stderr_only_as_diagnostics` | the reply parses past stderr chatter; exit 255 and stderr are kept only as diagnostics (not parsed for auth) |
| Byte transparency | proptest `every_byte_except_the_record_end_passes_unchanged_under_any_split` (32 cases through `cat`) | every byte value except `\n`, under any chunking, comes back identical |
| Prompt injection: hostname metacharacters | `destinations_that_a_shell_or_ssh_would_reinterpret_are_refused` | `-oProxyCommand=id`, `box;id`, `$(id)`, backticks, whitespace, newline, `|`, `&`, `>`, non-ASCII, empty user or host are all `invalid_params` |
| Prompt injection: names and task payloads | `shell_metacharacters_addresses_and_ids_are_invalid_names`; `argv_is_kept_verbatim_and_never_joined_into_a_shell_line`; `ssh_runs_a_fixed_remote_command_without_prompts` | names are `invalid_name`; argv passes verbatim with no shell; ssh argv is exactly `ssh -T -e none -o BatchMode=yes -o StrictHostKeyChecking=yes -- <dest> ovis-terminal connect`, and protocol payloads travel only on stdin |
| Stale state: replayed cached boot proof | `a_replayed_proof_of_an_earlier_challenge_is_stale` | `boot_proof_stale` |
| Forged or inconsistent proof | `a_proof_not_keyed_by_the_presented_credential_is_forged`; `a_proof_for_another_boot_than_the_hello_is_inconsistent` | `auth_failed`; `remote_incompatible` |
| Alias hijack after rename | `an_old_name_cannot_be_taken_to_redirect_queued_work` | `duplicate_name`; the old name still binds the same machine |

## Contract checklist

1. **u64 values on the wire:** every u64 is a decimal string. `MachineRecord.revision` is `Revision` and is asserted as `"1"` / `"2"`. Boot challenges and MACs are 64-hex strings, with a proptest round trip over all 32-byte values and non-canonical rejection (uppercase, short, long, non-ASCII, number).
2. **Boundary errors:** `IdentityError`, `TransportError` and `Hex32Error` are `thiserror` with `kind()`/`hint()`; Display never repeats the kind token; `#[source]` is kept for io and serde failures. They reach the wire through the new `DispatchError::Domain { kind, hint, source }`.
3. **Identity types:** `Name`, `Binding`, `NameHold`, `Credential` (redacted `Debug`), `BootChallenge`, `ProofMac`, `BootStatement`, `Trust` and `Trusted` are distinct types. `Trusted` is built only by `Trust::admit`.
4. **Behavior tests:** tests assert behavior (typed kinds, round trips, registry state across rename/hold/release, real child processes). proptest covers hex values and transport fragmentation.
5. **No suppressions:** no `#[allow]` or `#[expect]`, no TODO placeholders. Every file is at most 250 pure LOC and every function takes at most 3 parameters.
6. **Evidence scope:** this body states only what the tests assert. The conformance attempts are recorded verbatim above.

Holder wire generation stays 2. The contract checklist items are unchanged.

## Seams

These public API changes are outside the literal write scope, each needed by the todo and disclosed here:
- **`ovis-terminal-authz`:** new `Attestation`, `AttestRequest` and `Authority::attest(&Session, AttestRequest)`, an HMAC-SHA256 keyed by the session credential's bearer, with RFC 4231 vector tests. Adds a `sha2` dependency (already in `Cargo.lock`).
- **`ovis-terminal-protocol`:**
  - new `domain::machine_identity` module;
  - `HelloParams.challenge: Option<BootChallenge>` and `HelloResult.boot_proof: Option<BootProof>`;
  - `MachineAddParams.expected_machine_id: Option<MachineId>` (optional in the schema for released-baseline compatibility; absence is refused as `trust_required`);
  - `MachineRecord.remote_boot_id: Option<ServerBootId>`;
  - error kinds `trust_required`, `trust_mismatch`, `boot_proof_stale`, `handshake_timeout`, `machine_exists`, added to `machine.add`'s list.
  - `schema/ovis-terminal.openrpc.json` is regenerated, and the compat gate against `released.openrpc.json` passes.
- **`ovis-terminal-server`:**
  - `conn/hello.rs` answers a challenge with a `BootProof` attested by the session credential;
  - `DispatchError::Domain` variant (the enum is `#[non_exhaustive]`);
  - `dispatch::lanes` module with `MachineLane`, `MachineAdd`, `MachineGet` and `MachineRename` (registered by assembly, todo 31/52);
  - lane tests live in `dispatch/lanes/machine_identity/{add.rs,qa.rs,qa/}`, beside the scoped file.
- **Remote crate module roots and conformance files:**
  - `crates/ovis-terminal-remote/src/{lib.rs,identity.rs,transport.rs}` (module roots) and its manifest (`serde_json`, `thiserror`, `tokio`, `uuid` v4, dev `proptest`).
  - The scenario file is `conformance/scenarios/remote/identity.json`, because the runner maps id `remote/identity` to that path rather than a directory.
  - `declared.json` gains one entry, and `probe-stress-macos.yml` gains one option and one job at the end.

## Risks

- **No persistence across restarts:** machine records, aliases and credentials are in memory. Persistence and reconnect are not in this todo's scope.
- **One handshake per link:** the link stays open after `machine.add`, but nothing sends further requests over it until the todo-44 actors.
- **Aliases never expire:** the spec mentions an alias grace period; this lane has none.
- **SSH failures are not split by cause:** host-key and SSH-key failures arrive as `transport_failed` with exit status and stderr diagnostics, because they cannot be typed without parsing ssh's text, which GAP-107 forbids. Protocol-level credential refusal is typed `auth_failed`, and `host_key_unknown` is not produced.
- **Commit-time races show as `outcome_unknown`:** a name or machine conflict that appears between the pre-commit check and the commit (concurrent add/rename) records `outcome_unknown`. The pre-commit holds (`NameHold`, `ensure_new`, `ensure_renamable`) catch the non-racing cases with typed kinds.
- **The real-process tests need `sh`, `cat` and `mkfifo`:** these Unix tests are `cfg(unix)`; Windows compiles the lane but does not run them.
