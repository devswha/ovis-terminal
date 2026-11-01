feat(agent-cli): assemble the single binary and embedded producer manual

Plan todo 66 (wave 18). Closes GAP-137, GAP-138, GAP-141, GAP-145, GAP-146, GAP-147, GAP-148, GAP-149. Delivers IDEAL-014, IDEAL-064, IDEAL-089, IDEAL-110, IDEAL-111, IDEAL-114.

## What changed

- **Typed agent verbs.** `ovis-terminal agent|status|prompt|turn|template <verb>` plus `server metrics|stop|restart`: 27 verbs, each sending one published method (`src/agent.rs`, `src/commands.rs`). A verb takes exactly the flags its closed params type takes (`--op-id` for a mutation, `--terminal-id|--address|--name` for a terminal-scoped method, `--params` for the rest). The merged params decode as the closed type before any connection. A terminal-scoped verb without a target exits 7 with `needs_target` and the live candidates; a field given twice is `usage`. A test checks every verb against the published schema (op_id iff mutation, process_token use, target required/optional/absent).
- **Producer token.** Reporting verbs fill `process_token` from `OVIS_TERMINAL_PROCESS_TOKEN` (the variable the runtime sets for a spawned program) when `--params` names none. The token is never printed (redacted Debug) and is listed in `schema cli`.
- **`not_owner` exits 12 (forbidden).** It used to fall through to exit 1.
- **Embedded manual and contract.** The manual moved to `src/skill/manual.md`; `docs/skill/ovis-terminal.md` is a byte-equal shipped copy (tested). `ovis-terminal schema contract` renders `docs/runtime-contract.md` from this build's types: methods with effect, targeting, params/result types and error kinds; events; every server error kind with recovery; limits; roles and the permission each action needs; commands, exit codes with kinds, and the binary's own exit-1 kinds. The stated behaviour is `src/skill/contract.md`, from the recovered full advisory with the plan's binding overrides applied and listed. The shipped file must equal the rendering (tested).
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

Piped `hc ls` prints JSON without `--json` while `run`/`read` print text: the output mode depends on isatty. Gap baseline: GAP-137 (isatty output mode), GAP-138 (focus fallback and unsafe targetless automation), GAP-141 (hidden flags and undiscoverable failure kinds), GAP-145..149 (chrome and prefix modes, brand manifests, toasts and sounds, discarded unknown input, foreground promotion).

## After

Linux x86_64, binary sha256 `623ca8409d1905f35f975872aa099d50e4588bfe1c223f1f3eee7b05eb72397c` (`$CARGO_TARGET_DIR/debug/ovis-terminal`; the lane target lives on RAM, never `<worktree>/target`).

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal                      # exit 0
$ cargo test --locked -p ovis-terminal-cli --all-targets                             # exit 0, 108 passed
$ cargo clippy --locked -p ovis-terminal-cli --all-targets -- -D warnings            # exit 0
$ cargo clippy --locked --manifest-path conformance/Cargo.toml --all-targets -- -D warnings   # exit 0
```

Acceptance (`docs/evidence/task-66/acceptance-all.log`):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario cli/manual --isolated --case all --evidence "$ATTEMPT_DIR/task-66-ovis-terminal-v1.json"
{"case":"all",...,"result":"passed","scenario":"cli/manual","totals":{"assertions":"116","cases":"2","declared_only":"0","failed":"0","passed":"2","skipped":"0"}}
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

Materialized CLI runs from the evidence (`binary-runs.log`):

| argv | exit | channel |
|---|---|---|
| `--json --skill` (no endpoint) | 0 | stdout `{version, generation, manual}`; `/version` captured |
| `--json schema` (no endpoint) | 0 | equals `schema/ovis-terminal.openrpc.json`; `info.version` and `x-generation` equal the `--skill` pair |
| `--json schema contract` | 0 | same version/generation |
| `--json server status` (no server) | 0 | `{"state":"stopped","census":"not_reported",...}`, nothing started |
| `agent get --unknown-flag --json` (injected; no server) | **2** | stdout empty; stderr `{"error":{"exit_code":2,"kind":"usage","origin":"cli","message":"the command line does not parse: unexpected argument '--unknown-flag' found",...}}` |
| `--json agent get --address t1` (same absent endpoint) | 4 | `server_not_running`: what an attempted connection reports |
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
     Summary [ 114.127s] 1504 tests run: 1504 passed (3 slow), 2 skipped
check.sh: all steps passed                                                     # exit 0
$ RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked      # exit 0
$ cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings          # exit 0
```

The first check.sh run failed on three findings of this branch, all fixed structurally before the passing run: `docs/evidence/task-66/binary-runs.json` did not match the evidence manifest schema (it is now `binary-runs.log`), and `src/agent.rs` had 254 pure LOC (the verb table moved to `src/agent/table.rs`). The `quality/gates` scenario inside `all/all` reported the same LOC finding. No schema change, so no SDK regeneration.

## Seams

Every file outside the write scope (`docs/skill/`, `src/skill/`, `src/agent/`, `conformance/scenarios/cli/manual/`, `src/main.rs`, `src/commands.rs`, `docs/producer-onboarding.md`, `docs/runtime-contract.md`), and why:

- `crates/ovis-terminal-cli/src/lib.rs`: `pub mod agent; pub mod commands; pub mod skill;` and the `PROCESS_TOKEN_ENV` re-export. The modules are `pub` with `pub(crate)` items, the `assembly` pattern; `unreachable_pub` and `redundant_pub_crate` leave no other shape.
- `crates/ovis-terminal-cli/src/agent.rs`, `src/skill.rs`, `src/commands/tests.rs`: module files of the write-scope directories (`mod_module_files` forbids `mod.rs`).
- `crates/ovis-terminal-cli/src/invocation/{args,plan,context,error,report,discovery,exit,text}.rs`, `invocation.rs`, `invocation/manual_tests.rs`: the dispatch registration (flattened `DomainCommand`, `server metrics|stop|restart`, `schema contract`), planning of typed verbs, the inherited process token, the `Command` error variant, the published exit-1 kind list, `not_owner` -> exit 12, inert text rendering, and the grammar tests, which must live inside `invocation` to reach the private grammar.
- `crates/ovis-terminal-cli/src/invocation/skill.md` -> `src/skill/manual.md` (moved).
- `crates/ovis-terminal-cli/Cargo.toml`: `ovis-terminal-authz` is a dependency on every platform (the contract renders the role table on Windows too); it was unix-only plus a dev-dependency. `Cargo.lock` unchanged.
- `crates/ovis-terminal-cli/tests/assembly.rs` + `tests/assembly/producer.rs`: the synthetic producer process test. `tests/cli.rs`: three agent-verb refusals added to the idle-listener parse-before-connect loop.
- `conformance/scenarios/cli/manual.json`: the runner maps `--scenario cli/manual` to this file; the `cli/manual/` directory would not be found.
- `conformance/src/{invocation.rs,invocation/tests.rs,scenario/model.rs,engine.rs,engine/steps.rs,engine/commands.rs}`: the plan's literal failure inject is a command line, not JSON, so the runner accepts it and selects a planned command run. Binary runs gained label resolution and captures, so `--skill` and `schema` versions are compared without pinning a version string.
- `conformance/src/runner_tests/suite.rs`: `cli/manual.json` added to `EXECUTABLE` in sorted position. It was never in `declared.json`.
- `probes/platform-qa/stress/cli-manual.{json,sh}`: the macOS stress lane for this todo (25 iterations of the CLI lib, producer and cli process tests plus `cli/manual` all and the literal failure inject); the workflow resolves lanes from these files, so it needs no edit.
- `docs/evidence/task-66/`: logs and this body.

Seams owed by todo 66 according to landed PRs: #57 already routes `machine.add` through `MachineAddAttached` and calls through `RemoteDispatch` in the served binary. `ServiceLanes`, `MachineAdd::restricted`, reconnect and the second fixture dispatcher for `worktree.*` are untouched here; see Risks.

## Risks

- `ServiceLanes`, `reconnect` and `MachineAdd::restricted` are still not constructed by the served binary (the task-52 seam note), and `worktree.*` / `doc.*` keep their fixture dispatchers. This PR assembles the CLI surface and documents what `hello` routes; it does not change the served lane set.
- Text-mode output now escapes `\` and control characters in strings. Any person-facing script that parsed text mode would see `\\`; JSON is the contract and is unchanged.
- The runner's `--inject` now also accepts command-line text. Malformed JSON (text starting with `{` or `[`) is still a usage error.
- Host incident: one local build/test run sourced the toolchain env from the wrong cwd and wrote into `/dev/shm/ovt-target-main`. Nothing there was deleted (removing hashed artifacts could break a concurrent run). The owner may want to prune it.

## macOS stress

`gh workflow run probe-stress-macos.yml -f probe=cli-manual --ref lane/t66-assemble-the-single-binary-a`: https://github.com/devswha/ovis-terminal/actions/runs/41358489287 (dispatched once; result reported when it completes).

## DEFERRED VERIFICATION

None: the runner executes `cli/manual` on this branch (exit 0, failed 0, skipped 0).
