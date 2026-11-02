feat(agent-cli): assemble the single binary and embedded producer manual

Plan todo 66 (wave 18). Closes GAP-137, GAP-138, GAP-141, GAP-145, GAP-146, GAP-147, GAP-148, GAP-149. Delivers IDEAL-014, IDEAL-064, IDEAL-089, IDEAL-110, IDEAL-111, IDEAL-114.

## What changed

- **Typed agent verbs.** `ovis-terminal agent|status|prompt|turn|template <verb>` plus `server metrics|stop|restart`: 23 verbs, each sending one published method this build's `serve` routes, plus the offline `op new` that mints a current `op_id` (`src/agent.rs`, `src/commands.rs`). A verb takes exactly the flags its closed params type takes (`--op-id` for a mutation, `--terminal-id|--address|--name` for a terminal-scoped method, `--params` for the rest). The merged params decode as the closed type before any connection. A terminal-scoped verb without a target exits 7 with `needs_target` and the live candidates; a field given twice is `usage`. A test checks every verb against the published schema (op_id iff mutation, process_token use, target required/optional/absent).
- **Producer token.** Reporting verbs fill `process_token` from `OVIS_TERMINAL_PROCESS_TOKEN` (the variable the runtime sets for a spawned program) when `--params` names none. The token is never printed (redacted Debug) and is listed in `schema cli`.
- **`not_owner` exits 12 (forbidden).** It used to fall through to exit 1.
- **Embedded manual and contract.** The manual moved to `src/skill/manual.md`; `docs/skill/ovis-terminal.md` is a byte-equal shipped copy (tested). `ovis-terminal schema contract` renders `docs/runtime-contract.md` from this build's types: methods with effect, targeting, params/result types and error kinds; events; every server error kind with recovery; limits; roles and the permission each action needs; exit codes with kinds, the binary's own exit-1 kinds and the environment. It is platform-neutral: the per-platform commands (`serve`, `hold` and `terminal attach` are Unix only, Windows `service` verbs differ) live in the manual and `schema cli`, and a test pins that the rendering does not depend on the command grammar. The stated behaviour is `src/skill/contract.md`, from the recovered full advisory with the plan's binding overrides applied and listed. The shipped file must equal the rendering (tested).
- **Producer onboarding.** `docs/producer-onboarding.md` covers registration, authenticated reports, declared readiness, typed prompts, history, ownership and revocation, trusted-extension boundaries and a synthetic producer. Machine-consumed examples are tested, prose is not: every `sh` example line in the manual and the guide parses and plans against this build, and every `// params of <method>` JSON block decodes as that method's closed params type.
- **Text mode is inert.** Text output escapes control characters and backslashes in string values, so program text cannot fake lines or send terminal sequences.
- **Conformance `cli/manual`** (executable, real served binary) plus runner support: `--inject` also accepts one `ovis-terminal` command line (non-JSON text; malformed JSON stays a usage error), a `commands` step runs planned command lines (or only the injected one), and binary runs resolve `$op:`/`$value:` labels and keep `$capture` values.

## Before

`docs/evidence/baseline/smoke/hc-s02-run-ls.log:16-18` (RT-HC-08; source `/tmp/ulw-ovis-yCJzPH/evidence/smoke/hc-s02-run-ls.log:18`):

```
### ls human (tty-less)
$ <run-root>/bin/hc ls
{"hidden_finished":0,"terminals":[{"address":"t1","agent":"bash",...,"state":"live","turn":0}]}
```

Piped `hc ls` prints JSON without `--json` while `run`/`read` print text: the output mode depends on isatty. Path correction: the plan cites the durable copy as `docs/evidence/baseline/hc-s02-run-ls.log`, which does not exist; todo 1 stored it as `docs/evidence/baseline/smoke/hc-s02-run-ls.log` (the copy cited above, lines 16-18). Gap baseline: GAP-137 (isatty output mode), GAP-138 (focus fallback and unsafe targetless automation), GAP-141 (hidden flags and undiscoverable failure kinds), GAP-145..149 (chrome and prefix modes, brand manifests, toasts and sounds, discarded unknown input, foreground promotion).

## After

Linux x86_64, binary sha256 `0e67c343c2ec74cfdda9a01ccda1582572b245dc232b0c25c97e18c18ad145c7` (`$CARGO_TARGET_DIR/debug/ovis-terminal`; the lane target lives on RAM, never `<worktree>/target`).

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal                      # exit 0
$ cargo test --locked -p ovis-terminal-cli --all-targets                             # exit 0, 108 passed
$ cargo clippy --locked -p ovis-terminal-cli --all-targets -- -D warnings            # exit 0
$ cargo clippy --locked --manifest-path conformance/Cargo.toml --all-targets -- -D warnings   # exit 0
```

Acceptance (`docs/evidence/task-66/acceptance-all.log`):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario cli/manual --isolated --case all --evidence "$ATTEMPT_DIR/task-66-ovis-terminal-v1.json"
{"case":"all",...,"result":"passed","scenario":"cli/manual","totals":{"assertions":"122","cases":"2","declared_only":"0","failed":"0","passed":"2","skipped":"0"}}
EXIT=0
```

QA happy (`qa-happy.log`):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario cli/manual --isolated --case happy --evidence "$ATTEMPT_DIR/task-66-ovis-terminal-v1.json"
{"case":"happy",...,"result":"passed","totals":{"assertions":"71","cases":"1","failed":"0","passed":"1","skipped":"0"}}
EXIT=0
```

QA failure with the plan's literal inject (`qa-failure.log`):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario cli/manual --isolated --case failure --inject 'ovis-terminal agent get --unknown-flag --json' --evidence "$ATTEMPT_DIR/task-66-ovis-terminal-v1.json" --append
{"case":"failure",...,"result":"passed","totals":{"assertions":"29","cases":"1","failed":"0","passed":"1","skipped":"0"}}
EXIT=0
```

Materialized CLI runs from the evidence (`binary-runs.log`, generated from the complete `--case all` run in `docs/evidence/task-66/task-66-all.json`; the literal happy + inject runs are `docs/evidence/task-66/task-66-ovis-terminal-v1.json`; the evidence audit checks both):

| argv | exit | channel |
|---|---|---|
| `--json --skill` (no endpoint) | 0 | stdout `{version, generation, manual}`; `/version` captured |
| `--json schema` (no endpoint) | 0 | equals `schema/ovis-terminal.openrpc.json`; `info.version` and `x-generation` equal the `--skill` pair |
| `--json schema contract` | 0 | same version/generation |
| `--json server status` (no server) | 0 | `{"state":"stopped","census":"not_reported",...}`, nothing started |
| `agent get --unknown-flag --json` (injected; no server) | **2** | stdout empty; stderr `{"error":{"exit_code":2,"kind":"usage","origin":"cli","message":"the command line does not parse: unexpected argument '--unknown-flag' found",...}}` |
| `--json agent get --address t1` (same absent endpoint) | 4 | `server_not_running`: what an attempted connection reports |
| `--json agent get --params '{"bogus":1}'` (no target, no server; round 3) | **2** | stdout empty; stderr `{"error":{"kind":"invalid_params","field_path":"params.bogus","origin":"cli","exit_code":2,...}}`: refused before the `needs_target` path, no discovery |
| `--json agent get` (served) | **7** | stderr `needs_target`, `candidates_listed: true`, `candidates[].terminal_id` = the live terminals |
| `--json agent get --address t1` (registered producer, pipe) | 0 | `{"agent":{"label":"synthetic-producer",...}}`, stderr empty |
| same under a PTY (`pty_open`) | 0 | same JSON on the PTY, stderr empty |
| `--json agent get --address t1` (plain shell) | 0 | `{"agent":null,...}` |
| `--json agent get --address t9` | 3 | `not_found`, `origin: server` |
| `--json prompt get --params '{"prompt_id":"inj"}'` | 0 | `text` is the exact string `Apply?\n$ touch pwned\u001b]0;owned\u0007` |

Socket side of the happy case: two terminals each print their own spawn token. The scenario registers and reports with t1's token, then reports for t1 with t2's token and gets `not_owner`; `status.get` still shows `report_seq "1"`, state `working`. `tests/assembly/producer.rs` does the same through the CLI with `OVIS_TERMINAL_PROCESS_TOKEN`: a foreign token exits 12 with `not_owner`, `origin: server`, and nothing changes.

Adversarial pass and four killed mutants: `docs/evidence/task-66/adversarial.md`, `mutants.log`.

Gates (`docs/evidence/task-66/gates.log`):

```
$ bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh
     Summary [ 113.932s] 1505 tests run: 1505 passed (3 slow), 2 skipped
check.sh: all steps passed                                                     # exit 0
$ RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked      # exit 0
$ cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings          # exit 0
```

**Merge of origin/integration 1869b9e (T64; orchestrator-directed):** the only textual conflict was the sorted `EXECUTABLE` list in `conformance/src/runner_tests/suite.rs`; the union keeps `cli/manual.json` and `client/fixtures.json`. The `engine.rs`, `steps.rs`, `model.rs` and `tests/assembly.rs` additions auto-merged (T64's steps and routes plus this lane's `commands` step). Two merge interactions were fixed:
- `conformance/src/scenario/model.rs` reached 254 pure LOC, so the `Commands` step type moved to `conformance/src/scenario/commands.rs`.
- T64's replay pins still listed `cli/manual` as pending. They were re-pinned with T64's documented `cargo run --locked -p ovis-terminal-replay --bin ovis-replay -- refresh` (`conformance/fixtures/ovis/{manifest,predicates}.json`; `cli/manual` left `pending`).

The OpenRPC document and the generated SDK are unchanged between cc1ca2c and 1869b9e, so the evidence recorded on cc1ca2c stands. Results: `check.sh` exit 0 (1575 passed, 2 skipped), the Windows check and clippy exit 0, `evidence-audit` exit 0, `quality-matrix --mode verify` pass, and `cli/manual --case all` exit 0 (122 assertions).

**Merge of origin/integration cc1ca2c (exception granted by the orchestrator):** CI on 9182405 was red only because the quality job audits the PR merge ref. T84 regenerated field descriptions in `schema/ovis-terminal.openrpc.json`, so this lane's recorded `cli/manual` `$equals_file` steps no longer matched the merged schema. I merged cc1ca2c (no conflicts; `Cargo.lock` unchanged, `cargo metadata --locked` ok) and re-recorded all three `cli/manual` runs on the merged binary (122 / 71 / 29, exit 0). Local `evidence-audit` exits 0 with 0 findings, `docs/runtime-contract.md` is unchanged, `check.sh` exits 0 (1557 passed, 2 skipped) and the Windows check and clippy exit 0.

**Round 5 (gate r2 REJECT on 83d8528):** every documented mutation example (`agent register`, `status report`, the readiness report, the revocation `status clear`, and the `// params of` blocks) exited 1 with `op_expired`. Their fixed `01900000-...` ids are 2024 UUIDv7s, below the server's retention floor (`ops/admission.rs:89-91`). The executed-examples test still passed because its deny-list omitted `op_expired`. Fix:
- **`ovis-terminal op new`** (offline) prints a fresh `UUIDv7` (`{"op_id": ...}` with `--json`). Every documented mutation now writes `--op-id "$(ovis-terminal op new)"`, the manual explains why a hard-coded id is refused, and params blocks use the placeholders `<op id from ovis-terminal op new>` and `<OVIS_TERMINAL_PROCESS_TOKEN>`.
- **The test is strict.** `tests/assembly/manual_examples.rs` runs each document in order against its own `serve` with the owning shell `t1`. Every example must exit 0 unless it ends with `# exits <code> <kind>`, which must then match exactly. No kind is waved through.
- **The owning-token flow is asserted** (`the_onboarding_flow_registers_reports_and_clears_with_the_owning_token`): register, then `agent get` shows `synthetic-producer`, then report (`status get` shows `working`), then the revocation `status clear`, then `status get` shows `root: null`.
- **Doc defects the strict test exposed and fixed.** The guide registered the same incarnation twice (the second answers `agent_conflict`), so the socket example is now `agent.update_session` and the guide states the first-registration rule. The example session `kind` `session_id` was invalid (`path` or `id`). The guide read `agent.records` before appending any (`conversation_unavailable`), so it now appends records first.
- **Mutant M9** puts a stale `01900000` id back in the revocation example. The test fails with `expected exit 0 None, got Some(1) Some("op_expired")` (`mutants.log`).
- **Lane result:** `cargo test -p ovis-terminal-cli --all-targets` exit 0, 113 passed, including `op_new_prints_a_fresh_v7_id_each_time`. Clippy exit 0, source-check 0 findings. `cli/manual` all / happy / failure exit 0 (122 / 71 / 29 assertions).

**Round 4 (gate r1 REJECT on f8fc2fa, `.omo/evidence/ovis-terminal-v1-task-66-gate-review.md`):** the manual and the onboarding guide advertised `agent list`, `agent note`, `agent release` and `agent wait-children`, but `serve` routes none of `agent.list`, `agent.note`, `agent.release` or `agent.wait_children`. They answer `method_not_found` (exit 4), and the parse-only example test could not see that. No handler, lane or domain function exists for them (the only hit is a boundary-policy matrix row), so wiring them is new server work. Instead:
- The four verbs are removed from the grammar (`VERBS` is 20 plus the 3 `server` verbs). The manual names the four as catalog methods this build does not serve. The onboarding revocation step now uses served methods: `status clear`, `attach.leases` / `attach.close`, `credential.revoke`, and the registration ending with its process incarnation.
- `schema cli` now names the `method` each typed verb sends.
- New `tests/assembly/manual_examples.rs`:
  - `every_documented_command_and_params_example_is_served_by_this_build` executes every `sh` line of the manual and the guide through `/bin/sh` against `serve`, with the real spawn token, and sends every `// params of` block with `call`. It fails on `method_not_found`, `unknown_method`, `usage`, `invalid_params` or unparsed output.
  - `hello_advertises_the_method_of_every_typed_verb` requires `hello.methods` to contain every `schema cli` method.
- New unit tests: `every_typed_verb_the_docs_name_is_a_command_of_this_build` covers every `<group> <verb>` in code spans and command lines of both documents. `every_typed_verb_command_names_the_method_it_sends` guards against a vacuous hello check.
- The executed test found one more documentation bug: the manual's `turn wait` example had no `turn` or `send_op` and was refused `invalid_params`. It now names `"turn":"1"`.
- The duplicate `fenced()` copies and the test-only word splitter are gone; one `fenced()` remains, in the served-binary test.
- Mutants, all killed (`mutants.log`): M6 re-adds an unserved `agent list` verb and fails the hello test. M7 documents `agent list` again and fails both the documented-verb unit test and the executed-examples test (`usage`). M8 blanks the method field and fails the coverage test.
- Lane result: `cargo test -p ovis-terminal-cli --all-targets` exit 0, 111 passed; clippy exit 0; `cli/manual` all / happy / failure exit 0 (122 / 71 / 29 assertions).

**Round 3 (adversarial review on 8905649):** a targetless `agent get --params '{"bogus":1}'` answered `needs_target` (exit 7) after candidate discovery instead of refusing the malformed params. Supplied fields are now checked first. `commands::plan` returns the merged params with a stand-in target (`{"address": "t1"}`; every required `target` of the typed verbs is the common `Target` type), and `invocation::plan::verb` decodes that probe as the method's closed params type before it answers `needs_target`. So only an otherwise valid targetless invocation lists candidates (IDEAL-111), and a malformed one exits 2 before any connection. Tests:
- `invocation::manual_tests::a_targetless_verb_needs_a_target_only_when_every_supplied_field_is_valid` checks that `agent get` and `turn wait --params '{"timeout_ms":"1000"}'` give needs_target, while an unknown field, a numeric `seq` and a missing `timeout_ms` give invalid_params.
- `tests/cli.rs` `failure` runs the targetless `agent get --params '{"bogus":1}'` and `turn wait` against a bound idle listener: exit 2, and `accept()` is still `WouldBlock`.
- The `cli/manual` failure case has a targetless malformed run with no server: exit 2, `invalid_params` with `field_path`.

Mutant M5 (answer `needs_target` without the check) fails all three: the unit test gets `Ok(true)` instead of `Err("invalid_params")`; the process test exits 13 `deadline_exceeded` after waiting 15 s on the idle listener's hello, which is the discovery side effect; and conformance exits 7 instead of 2. Lane gates for round 3 are in `gates.log`.

**Round 2 (PR CI, windows-latest):** `the_shipped_contract_reference_is_the_one_this_build_renders` failed because the reference rendered this build's command table, and the Windows grammar differs (no `serve`, `hold` or `terminal attach`; other `service` verbs). It was fixed portably, with no cfg: the contract no longer carries per-platform commands (the manual and `schema cli` list them), and the new `the_contract_reference_does_not_depend_on_the_platform_grammar` renders the reference with and without the command list and requires identical bytes. The rest is platform-neutral by construction: the OpenRPC document (`crates/ovis-terminal-protocol/tests/schema.rs` pins it on every platform), the role and action constants, the exit-code table and the fixed environment names. Gate after the fix: `check.sh` exit 0 (1505 passed, 2 skipped), the Windows check and clippy exit 0. Then `origin/integration` 60076f7 was merged as instructed (conformance fixture only, `Cargo.lock` unchanged, `cargo metadata --locked` ok), and on the merged tree conformance clippy exits 0 and the three literal `cli/manual` runs exit 0 (116 / 71 / 29 assertions).

The first check.sh run failed on three findings of this branch, all fixed structurally before the passing run: `docs/evidence/task-66/binary-runs.json` did not match the evidence manifest schema (it is now `binary-runs.log`), and `src/agent.rs` had 254 pure LOC (the verb table moved to `src/agent/table.rs`). The `quality/gates` scenario inside `all/all` reported the same LOC finding. No schema change, so no SDK regeneration.

## Seams

Every file outside the write scope (`docs/skill/`, `src/skill/`, `src/agent/`, `conformance/scenarios/cli/manual/`, `src/main.rs`, `src/commands.rs`, `docs/producer-onboarding.md`, `docs/runtime-contract.md`), and why:

- `crates/ovis-terminal-cli/src/lib.rs`: `pub mod agent; pub mod commands; pub mod skill;` and the `PROCESS_TOKEN_ENV` re-export. The modules are `pub` with `pub(crate)` items, the `assembly` pattern; `unreachable_pub` and `redundant_pub_crate` leave no other shape.
- `crates/ovis-terminal-cli/src/agent.rs`, `src/skill.rs`, `src/commands/tests.rs`: module files of the write-scope directories (`mod_module_files` forbids `mod.rs`).
- `crates/ovis-terminal-cli/src/invocation/{args,plan,context,error,report,discovery,exit,text}.rs`, `invocation.rs`, `invocation/manual_tests.rs`: the dispatch registration (flattened `DomainCommand`, `server metrics|stop|restart`, `schema contract`), planning of typed verbs, the inherited process token, the `Command` error variant, the published exit-1 kind list, `not_owner` -> exit 12, inert text rendering, and the grammar tests, which must live inside `invocation` to reach the private grammar.
- `crates/ovis-terminal-cli/src/invocation/skill.md` -> `src/skill/manual.md` (moved).
- `crates/ovis-terminal-cli/Cargo.toml`: `ovis-terminal-authz` is a dependency on every platform (the contract renders the role table on Windows too); it was unix-only plus a dev-dependency. `Cargo.lock` unchanged.
- `crates/ovis-terminal-cli/tests/assembly.rs` + `tests/assembly/producer.rs` + `tests/assembly/manual_examples.rs`: the synthetic producer process test, and the test that executes every documented example against `serve` and checks every typed verb's method against `hello`. `tests/cli.rs`: three agent-verb refusals added to the idle-listener parse-before-connect loop.
- `conformance/scenarios/cli/manual.json`: the runner maps `--scenario cli/manual` to this file; the `cli/manual/` directory would not be found.
- `conformance/src/{invocation.rs,invocation/tests.rs,scenario/model.rs,engine.rs,engine/steps.rs,engine/commands.rs}`: the plan's literal failure inject is a command line, not JSON, so the runner accepts it and selects a planned command run. Binary runs gained label resolution and captures, so `--skill` and `schema` versions are compared without pinning a version string.
- `conformance/fixtures/ovis/{manifest,predicates}.json`: re-pinned with `ovis-replay refresh` after the 1869b9e merge (T64's documented procedure); `conformance/src/scenario/{commands.rs,model.rs}` and `conformance/src/scenario.rs`: the `Commands` step type moved to a child module to stay under 250 pure LOC.
- `conformance/src/runner_tests/suite.rs`: `cli/manual.json` added to `EXECUTABLE` in sorted position. It was never in `declared.json`.
- `probes/platform-qa/stress/cli-manual.{json,sh}`: the macOS stress lane for this todo (25 iterations of the CLI lib, producer and cli process tests plus `cli/manual` all and the literal failure inject); the workflow resolves lanes from these files, so it needs no edit.
- `docs/evidence/task-66/`: logs, the raw runner evidence and this body.

Seams owed by todo 66 according to landed PRs: #57 already routes `machine.add` through `MachineAddAttached` and calls through `RemoteDispatch` in the served binary. `ServiceLanes`, `MachineAdd::restricted`, reconnect and the second fixture dispatcher for `worktree.*` are untouched here; see Risks.

### Seams outside the write scope (named gaps)

- **Unserved agent catalog methods (gate r1), for F4 / P5:** `agent.list`, `agent.note`, `agent.release` and `agent.wait_children` are in the OpenRPC catalog, but this build's `serve` routes none of them; they answer `method_not_found` (exit 4). No handler, lane or domain function exists in `crates/ovis-terminal-server`, `crates/ovis-terminal-agent` or `crates/ovis-terminal-cli/src/assembly`. This build ships no verb for them, and the manual names them as not served. Once they are served, add the verbs back to `src/agent/table.rs`. `hello_advertises_the_method_of_every_typed_verb` then proves they are routed.

- **Event publication (from T64's DoneClaim), for F4:** the assembled daemon at this branch never publishes `lease.revoked`, `layout.changed` or `selection.changed` to event subscribers (T64, landed on integration at 1869b9e after this branch's merge base, publishes `lease.*`; `layout.changed` and `selection.changed` remain). These events are in the catalog (`## Events` of `docs/runtime-contract.md`), but the wiring lives in `crates/ovis-terminal-cli/src/assembly/` (events lane, lease table, layout service), which is outside todo 66's write scope. This PR does not change it and makes no claim that subscribers receive these three events.

## Risks

- `ServiceLanes`, `reconnect` and `MachineAdd::restricted` are still not constructed by the served binary (the task-52 seam note), and `worktree.*` / `doc.*` keep their fixture dispatchers. This PR assembles the CLI surface and documents what `hello` routes; it does not change the served lane set.
- Text-mode output now escapes `\` and control characters in strings. Any person-facing script that parsed text mode would see `\\`; JSON is the contract and is unchanged.
- The runner's `--inject` now also accepts command-line text. Malformed JSON (text starting with `{` or `[`) is still a usage error.
- Host incident: one local build/test run sourced the toolchain env from the wrong cwd and wrote into `/dev/shm/ovt-target-main`. Nothing there was deleted (removing hashed artifacts could break a concurrent run). The owner may want to prune it.

## macOS stress

`gh workflow run probe-stress-macos.yml -f probe=cli-manual --ref lane/t66-assemble-the-single-binary-a`: https://github.com/devswha/ovis-terminal/actions/runs/41358489287 (dispatched once; result reported when it completes).

## DEFERRED VERIFICATION

None: the runner executes `cli/manual` on this branch (exit 0, failed 0, skipped 0).
