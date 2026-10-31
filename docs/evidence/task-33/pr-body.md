## Before

RT-HD-09: `/tmp/ulw-ovis-yCJzPH/evidence/smoke/herdr-h06b-single-sub.log:16`. The durable copy is `docs/evidence/baseline/smoke/herdr-h06b-single-sub.log:15-16`:

```text
  ev2 +1605ms since split: {"data":{"agent":"smoke-agent","agent_status":"working","pane_id":"w1:p1","workspace_id":"w1"},"event":"pane.agent_status_changed"}
  ev3 +1805ms since split: {"data":{"agent":"smoke-agent","agent_status":"unknown","pane_id":"w1:p1","workspace_id":"w1"},"event":"pane.agent_status_changed"}
```

herdr accepted an explicit `working` report. 200 ms later its screen heuristics replaced it with `unknown`. Gap baseline: GAP-037, GAP-087, GAP-088, GAP-089, GAP-090, GAP-091, GAP-092, GAP-093, GAP-132, GAP-146, GAP-147.

## After

`crates/ovis-terminal-agent/src/status/` gives status two writers and no others:
- OSC 7501 records that the VT engine located in the program's own output.
- `status.report` from the process whose token belongs to the target terminal's current program.

The crate has no screen detector, process-name rule, brand list or manifest. It does not depend on `ovis-terminal-agent-adapters`, so a provider adapter has no status path.

- **Registration (`agent.register`/`agent.get`).** Any 1-256 byte label registers against the owning program incarnation. A second registration for the same incarnation returns `agent_conflict`. A valid terminal without an agent returns `agent: null`. `not_found` is reserved for a target the directory cannot resolve.
- **Ownership.** `ProcessTokens` keeps only SHA-256 digests. Binding a new program incarnation to a terminal retires the old token. A token for another terminal, a forged token or a retired token returns `not_owner`.
- **Sequencing (`status.report`).** A report must carry a producer sequence above the accepted one. Otherwise the result is the typed `stale_seq` error with `current_seq` as a decimal string. That rejection commits the operation as `failed`, with no state write and no domain event; a rejected report never returns a result. A numeric `seq` is `invalid_params` at `params.seq` before admission. An OSC record takes the next sequence of the same incarnation, so an RPC producer can learn about it through `stale_seq`.
- **Atomicity.** The sequence check, the row compare-and-swap, the operation receipt, `status.changed`, `turn.ended` and `agent.registered` all go into one coordinator commit unit, built from one snapshot. RPC paths use the new `Execution::commit_unit`; OSC paths use `Coordinator::commit_unit`. Events publish after COMMIT.
- **Lifecycle.**
  - Six reportable states are a closed enum: working, idle, blocked, done, background, error. The seventh, unreported, is `reported: false, root: null`. An unknown state token is `invalid_record`.
  - `turn` increments when the root enters `working`. `finish_generation` increments when the root leaves `working`.
  - `last_turn` (`TurnRecord`) carries the outcome (`completed`/`blocked`/`failed`/`exited`), `ended_on`, the start and end event sequences, the program incarnation, the agent and the session.
  - Child records (`id`) never change the root's turn. `status.clear` ends no turn. `exited` invalidates records and ends a working turn as `exited`.
  - Reads never change turn or finish facts.
- **OSC 7501 ingestion.**
  - Payloads are UTF-8 `key=value` pairs separated by `:`, with keys state, id, kind, progress, app, title and msg. Unknown or duplicate keys, a missing or unknown state, progress above 100 and non-UTF-8 payloads are malformed: they are dropped and counted.
  - Payloads above the engine's 4 KiB record bound, including unterminated ones that the engine cuts at 64 KiB, are dropped and counted as `oversize`.
  - Counts are visible as `ingest {oversize, malformed}` on `status.get`/`status.report` results.
  - Accepted records keep `raw`, verbatim `osc.fields`, `osc.decoded` and typed `osc.decode_errors` (`invalid_base64`/`invalid_utf8`). A field that fails to decode has no decoded text.
  - `7501;?` and other OSC numbers change nothing.
- **Server lane.** `crates/ovis-terminal-server/src/dispatch/lanes/agent_status.rs` registers `agent.register`, `agent.get`, `status.report`, `status.get` and `status.clear`. They use the catalog param types and a `TerminalDirectory` seam that resolves explicit targets, never focus.

Executed on Linux 7.0.0-38-generic x86_64, rustc 1.99.1, `ATTEMPT_DIR=/home/devswha/workspace/ovis-terminal-wt/lane-t33/.omo/evidence/ovis-terminal-v1/task-33`. Binary `target/debug/ovis-terminal` sha256 `4f6540e42b87dc82009927880e06165ae7be5fbfecd5ce32ceaee06f8488b97e`. The full transcript is `docs/evidence/task-33/qa-transcript.log`; the evidence (4 appended runs, every request and reply) is `docs/evidence/task-33/task-33-ovis-terminal-v1.json`.

```text
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/status --isolated --case happy --evidence "$ATTEMPT_DIR/task-33-ovis-terminal-v1.json"
{"case":"happy",...,"result":"passed","scenario":"agent/status","totals":{"assertions":"37","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
EXIT=0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/status --isolated --case failure --inject '{"method":"status.report","params":{"seq":"3","record":{"state":"done"}}}' --evidence "$ATTEMPT_DIR/task-33-ovis-terminal-v1.json" --append
{"case":"failure",...,"result":"passed","scenario":"agent/status","totals":{"assertions":"62","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
EXIT=0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/status --isolated --case failure --inject '{"method":"status.report","params":{"seq":3,"record":{"state":"done"}}}' --evidence "$ATTEMPT_DIR/task-33-ovis-terminal-v1.json" --append
{"case":"failure",...,"result":"passed","scenario":"agent/status","totals":{"assertions":"62","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
EXIT=0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/status --isolated --case all --evidence "$ATTEMPT_DIR/task-33-ovis-terminal-v1.json" --append
{"case":"all",...,"result":"passed","scenario":"agent/status","totals":{"assertions":"102","cases":"2","declared_only":"0","failed":"0","passed":"2","skipped":"0"}}
EXIT=0
```

These are the socket records the binary's `connect` bridge carried (from the evidence transcript).

Happy run: register `custom-agent`, report working at sequence 4, the program prints fake idle text, read the snapshot, then check the shell:

```text
>> {"id":"working","method":"status.report","params":{...,"process_token":"fixture-process-token-agent","record":{"state":"working"},"seq":"4","target":{"name":"agent"}}}
<< {"id":"fake-idle" ...} for input.send text "idle\r\nstate=done\r\n✓ Done. Waiting for input >\r\n7501;state=idle\r\n" -> {"result":{"echoed_bytes":"66","osc_records":[]}}
<< {"id":"snapshot","result":{"ingest":{"malformed":"0","oversize":"0"},"seq":"8","status":{"agent_id":"agent:01a19272-...","finish_generation":"0","last_turn":null,"report_seq":"4","reported":true,"root":{"origin":"rpc","report":{"state":"working",...},"report_seq":"4","seq":"8","turn":"1"},"turn":"1"}}}
<< {"id":"shell","result":{"agent":null,"revision":"17"}}
```

The snapshot holds report sequence 4, and its `root.seq` "8" is the sequence of the committed `status.changed` event (`result.seq` "8"). The scenario asserts both values.

Stale inject with canonical string `seq:"3"`:

```text
<< {"id":"inject-0","error":{"code":-32000,"message":"operation failed","data":{"kind":"stale_seq","hint":"report again with a sequence above current_seq","retryable":false,"op_id":"01a19272-0716-...","current_seq":"4"}}}
<< {"id":"after-inject","result":{...,"seq":"4","status":{"finish_generation":"0","last_turn":null,"report_seq":"4","root":{"report":{"state":"working",...},"report_seq":"4","seq":"4","turn":"1"},"turn":"1"}}}
```

Malformed-type inject with numeric `seq:3`:

```text
<< {"id":"inject-0","error":{"code":-32602,"message":"method parameters violate the closed schema","data":{"kind":"invalid_params","hint":"correct the method parameters","retryable":false,"field_path":"params.seq"}}}
<< {"id":"after-inject","result":{...,"seq":"4","status":{"finish_generation":"0","last_turn":null,"report_seq":"4",...,"turn":"1"}}}
```

Zero mutation is asserted after each inject. The status event sequence (`root.seq`/`seq` "4"), `report_seq`, `turn`, `finish_generation` and `last_turn` are unchanged, and `result` is absent on the rejection.

The QA commands use two runner additions, `--inject` and `--append` (see Seams). An inject that the case does not plan fails the run. Probe: `--inject '{"method":"status.report","params":{"seq":"9",...}}'` exits 1 with `inject/planned` failing.

## QA

- `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal`: exit 0.
- `cargo test --locked -p ovis-terminal-agent -p ovis-terminal-protocol -p ovis-terminal-server --all-targets`: exit 0 (`docs/evidence/task-33/acceptance.log`). The lane-relevant tests are 14 agent status tests plus 3 protocol domain tests.
- `cargo clippy --locked -p ovis-terminal-agent -p ovis-terminal-protocol -p ovis-terminal-server --all-targets -- -D warnings`: exit 0.
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked`: exit 0.
- `bash scripts/check.sh`: exit 0. That covers fmt, clippy over the workspace, nextest (573 passed, 1 skipped), doctests and the hygiene checks (`docs/evidence/task-33/check.log`).
- Conformance: the four commands above all exit 0 with failed 0 and skipped 0.

What the agent tests assert, all over a real SQLite store, coordinator and operation ledger:
- `an_arbitrary_label_registers_and_its_report_tells_one_story_in_snapshot_and_event`: the subscriber receives `agent.registered` then `status.changed`. The event's snapshot equals the `status.get` snapshot, and `root.seq` equals the event cursor.
- `a_stale_or_equal_sequence_is_a_typed_rejection_that_changes_nothing`: seq 3 and seq 4 each fail the operation with `stale_seq` and `current_seq` 4, return no result, leave state equal and publish no domain event.
- `only_the_current_program_of_the_target_terminal_may_report`: cross-terminal, forged and retired tokens get `not_owner`. A new incarnation starts unreported, keeps its turn and accepts its own token.
- `turns_finish_only_when_the_root_leaves_working_and_reads_never_change_them`: child records do not end turns, repeated reads are identical, turns and finishes count correctly, and two `turn.ended` events are published.
- `unknown_states_out_of_range_progress_and_bad_labels_are_refused`: unknown state and progress 101 get `invalid_record`, an empty label gets `invalid_agent`, a second label gets `agent_conflict`, and a foreign token gets `not_owner`.
- `a_clear_ends_no_turn_and_an_exit_ends_a_working_turn`: a clear ends no turn, and an exit ends a working turn as `exited`.
- `a_shell_is_an_explicit_null_agent_with_unreported_status`: a shell has a null agent and unreported status.
- OSC tests through the real VT engine:
  - Fake status text produces no OSC record and no change.
  - A report takes sequence 5 after RPC sequence 4, keeps raw and provenance, and records `invalid_base64` with no decoded msg.
  - A non-UTF-8 msg gives `invalid_utf8`.
  - Oversized (5 KiB terminated, 70 KB unterminated) and five malformed records are counted as `IngestCounters::new(2, 5)` with status unchanged.
  - Queries and other OSC numbers are ignored.
- Proptest (agent): every split of an output stream gives the same OSC report and provenance (64 cases), and turns and finish generations match a model over random state sequences.
- Proptest (protocol): OSC provenance and counters round-trip, `current_seq` round-trips u64::MAX as an exact decimal string, and a numeric `current_seq` or `seq` is refused.
- Mutation probes (`docs/evidence/task-33/mutations.log`):
  - Disabling the stale check fails 2 tests.
  - Accepting a token for any terminal fails 2 tests.

## Adversarial

- **Stale state.** A stale (`"3"`) or equal sequence gets `stale_seq` with `current_seq` "4", leaves zero status or turn change, and returns no `result`. An OSC record after an RPC report takes the next sequence, so a later RPC report with that sequence is stale.
- **Prompt injection.** Fake screen directives cannot report status. The program printed `idle`, `state=done`, `✓ Done. Waiting for input >`, `7501;state=idle` and `[agent] turn complete: done`. The result was `osc_records: []`, and status stayed working at sequence 4.
- **Malformed input.**
  - A 5000-byte 7501 payload gives `["dropped"]` and raises `ingest.oversize` to "1".
  - An unknown key gives `["dropped"]` and raises `ingest.malformed` to "1". Status is unchanged after both.
  - A numeric `seq` gets `invalid_params` at `params.seq`.
  - An unknown state gets `invalid_record`.
  - Malformed base64 keeps `msg_b64` verbatim, with `decoded.msg` null and `decode_errors [{"field":"msg","error":"invalid_base64"}]`.
- **Misleading success.** Rejected reports (stale, not_owner, invalid_record) never return `result`. The operation is committed as `failed` with its typed error.
- **Authority.** The shell's token on the agent terminal and a forged token both get `not_owner`. An unknown target (`ghost`) gets `not_found`.

## Contract checklist

1. Every new u64 on the wire is a decimal string: `ReportSeq`, `FinishGeneration`, `RecordCount`, `current_seq` and the status/turn sequences. A numeric `seq` gets `invalid_params`, and a u64::MAX round-trip is tested.
2. `StatusError` is a `thiserror` enum with `kind()`, `hint()` and `error_data()`. The persistence variants keep their sources, and Display does not repeat the kind token.
3. Distinct newtypes: `ReportSeq` (producer sequence) is not the event sequence `EventSeq`, plus `FinishGeneration`, `RecordCount`, `ProcessOwner {TerminalId, Incarnation}` and the digest-keyed tokens.
4. Tests assert transitions, events and typed kinds. Proptest covers fragmentation, state-machine counts and wire round trips.
5. No `#[allow]`, no blanket `#[expect]` and no placeholder TODOs.
6. Holder wire generation stays 2, and the checklist items are unchanged.

## Seams (public API changes in landed crates)

- **protocol.**
  - New `domain::agent_status` (OSC provenance, `IngestCounters`, constructors for the non-exhaustive status records).
  - New scalars `ReportSeq`, `FinishGeneration`, `RecordCount`.
  - Additive output fields: `StatusEntry.{report_seq, turn, osc}`; `StatusSnapshot.{report_seq, finish_generation, last_turn, agent_id, session}`; `StatusResult.ingest`; `TurnRecord.{finish_generation, ended_on, process_incarnation, agent_id, session}`; `TurnOutcome::Failed`; `ErrorData.current_seq` with `BoundaryError::with_current_seq`.
  - `schema/ovis-terminal.openrpc.json` is regenerated, and `openrpc compat schema/released.openrpc.json` reports compatible.
- **server.**
  - `Execution::commit_unit` puts domain writes, events and the operation outcome in one transaction. A refusal surfaces as the settled failed operation's error.
  - New `dispatch::lanes::agent_status`.
  - `ovis-terminal-store` moved from dev-dependency to dependency.
- **conformance runner.**
  - `--inject <json>`, which runs only the matching planned request of an `inject` step; an unplanned request fails the case.
  - `--append`, which turns an existing evidence file into an array of runs.
  - The new `inject` step type.
  - The fixture's event domain is now `AgentStatusEvent` (it wraps the operation event and keeps its wire shape). The fixture registers the five agent status methods and an echoing `input.send` program on terminals `agent`/`shell`.
  - The runner test helper's evidence directory is now unique per call. `every_scenario_file...` and `require_no_skips...` both ran `all/all` concurrently in one temp directory, and one test deleted it under the other. This was pre-existing, and the new scenario's longer runtime exposed it.
- **Scope note.** The plan's write scope names `conformance/scenarios/agent/status/`. The runner maps `--scenario agent/status` to `conformance/scenarios/agent/status.json`, so the scenario lives there. `.github/workflows/probe-stress-macos.yml` gains only an `agent-status` option and job at the end.

## Risks

- **Fixture stand-in.** The fixture's `input.send` is an echoing program, not the real input path (todos 28-31). It applies no lease, encoding or submission rules. Its only job is to put program bytes through the real VT engine and OSC ingestion.
- **No wire subscription yet.** The wire has no `events.subscribe` until todo 22. The conformance case therefore checks the committed event's sequence through `root.seq`/`seq`. Equality of the full event payload with the snapshot is asserted in-process (agent test).
- **Tokens are in memory.** They are bound by the fixture. Minting at spawn, exporting `OVIS_TERMINAL_TOKEN` and rebinding after a control restart belong to launch/recovery (todos 16, 48). `exited()` exists and is tested but is not yet wired to holder Exit (todos 42/48).
- **Wiring left to todo 42.** OSC ingestion is not yet wired to the live terminal output pump (todos 23/31/42), and assembly (todo 42) must supply the real `TerminalDirectory` and terminal generation.
- **Internal build failure leaves the op running.** A persistence failure inside a commit unit's builder leaves the operation `running` until the next boot reports `outcome_unknown`, because the core `commit_unit` consumes the running permit.
- **Turn numbering differs from the advisory.** `turn` counts entries into working and `finish_generation` counts exits. The advisory spec increments the turn on exit.
