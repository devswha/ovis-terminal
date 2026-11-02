feat(assembly): serve every published method or exclude it explicitly (P5, plan F4)

Product-fix lane P5 closes the gap that T66 (#82) and the T65 gate (#86) named: the assembled daemon published 14 OpenRPC methods it did not route. After this PR, `hello.methods` equals the published method set minus an explicit excluded set of 10, and nothing answers `method_not_found` any more.

Every one of the 14 is in the frozen released baseline `schema/released.openrpc.json` (T67), so removing any of them would fail `openrpc compat` with `method_removed`. No method is removed. Each one is either served or kept published and explicitly excluded with the typed `not_implemented` kind. The new kind is additive: `ErrorKindName` is an open enum, and `openrpc compat schema/released.openrpc.json` still exits 0. The hint of each exclusion names its follow-up.

## Decisions (all 14)

| method | decision | reason |
|---|---|---|
| `terminal.history` | **served** | Reads the runtime-owned scrollback through the core screen's pinned `history` pages (IDEAL-035/036/047). Without `from_line` it returns the newest lines; with `from_line` it pages, and both pages pin one revision. `history_unavailable` comes from the screen. It is an unrecorded read on the actor. |
| `events.ack` | **served** | Uses the subscription cursor (IDEAL-022). Each subscription keeps delivered, scanned and acked marks, all starting at `snapshot_seq`. The forwarder raises the delivered and scanned marks *before* it writes a notification, so an owner can never see an event its ack would refuse. An ack past the delivered mark is `seq_ahead`; an older one keeps the mark (it never moves backwards). Only the subscription's owner may ack; anyone else gets `not_found`. |
| `agent.list` | **served** | Uses the agent registry (IDEAL-064/067). It returns the binding of every known terminal that has one, read exactly as `session.snapshot` reads bindings: the terminal list comes from one actor job and the bindings from the dispatch-log snapshot. It answers in one page; `page.limit` truncates and says so, and `page.after` is `invalid_params` because no token is ever issued. |
| `server.start` | **served** | A call can only reach a running control server, so it answers the method's own `already_running` (exit 11) after decoding params. Malformed params are still `invalid_params`. |
| `agent.note` | excluded | No child-to-parent note store exists. Follow-up: agent notes lane. |
| `agent.release` | excluded | The status service cannot unbind a program, and the agent crate is outside this lane's write scope. A binding ends when its program exits. Follow-up: binding release in the agent status service. |
| `agent.wait_children` | excluded | No per-parent news cursor exists. Follow-up: agent notes lane. |
| `doc.list` | excluded | `ovis-terminal-store`'s `Snapshot` has no listing by resource kind. Scanning the retained event log would silently miss documents older than the replay window, and the store is outside the write scope. Follow-up: store listing by resource kind. |
| `machine.exec` | excluded | The remote lane (T52/T55) has no bounded exec. Follow-up named in the hint. |
| `machine.file` | excluded | The remote lane has no scoped file transfer. |
| `machine.login` | excluded | The remote lane has no interactive login. |
| `machine.rm` | excluded | The remote lane cannot detach a machine and tombstone its terminals. |
| `service.install` | excluded | Holder registration would write unit files and run `systemctl --user daemon-reload` from inside the daemon. The local `ovis-terminal service install` verb already does this. Follow-up: daemon-side registration. |
| `service.remove` | excluded | Same as `service.install`; use the local `ovis-terminal service uninstall`. |

The 10 exclusions are recorded in `docs/design/traceability.json` (IDEAL-113 note, plus IDEAL-002 under todo 66 with the new scenario), in each method's published error list (`not_implemented`), in `Excluded` (`crates/ovis-terminal-cli/src/assembly/excluded.rs`: a `thiserror` enum with `kind()`/`hint()`), and in the manual.

## Before

- `.omo/evidence/ovis-terminal-v1-task-65-gate-review.md:71`: "The published-minus-served methods are exactly 14: agent.list, agent.note, agent.release, agent.wait_children, doc.list, events.ack, machine.exec, machine.file, machine.login, machine.rm, server.start, service.install, service.remove, terminal.history."
- `docs/evidence/task-P5/before.log`: the base f71a8db binary has advertised 90 and published 104. `published_not_advertised` lists exactly those 14, and each answers `method_not_found`.
- `docs/evidence/task-P5/before-scenario.log`: `cli/advertised-methods` against the base binary exits 1 with 11 failed assertions. Hello lacks the 4 served methods, and each excluded method answers `method_not_found`.
- `docs/evidence/task-P5/tdd-red.log`: the new tests against the base product code (product changes stashed, tests kept) give 4/4 FAIL, including `left: {14 names} right: {10 excluded}`.

## After

Linux x86_64, binary built from this branch:

| command | exit | result |
|---|---|---|
| `python3 probe.py <lane ovis-terminal> schema/ovis-terminal.openrpc.json calls` (`after.log`) | 0 | advertised 94, published 104; `published_not_advertised` == the 10 excluded; `advertised_not_published` = []; answers `invalid_params` 76, result 16, `not_implemented` 10, `unavailable` 1; `method_not_found` = **[]** |
| `cargo run --locked -q --manifest-path conformance/Cargo.toml --bin conformance -- --binary <lane ovis-terminal> --scenario cli/advertised-methods --isolated --case all --evidence docs/evidence/task-P5/cli-advertised-methods-all.json` | 0 | 2 cases, 138 assertions, 0 failed |
| same with `--case happy` / `--case failure` | 0 / 0 | 105 / 33 assertions |
| `cargo nextest run -p ovis-terminal-cli` | 0 | includes `hello_advertises_every_published_method_but_the_excluded_which_are_not_implemented`, `served::*` (history, agent.list + server.start), `served_events::*` (events.ack), and unit tests in `excluded`, `events::ack`, `history`, `agent_list`, `restart` |
| `openrpc write` / `check` / `compat schema/released.openrpc.json` (`openrpc.log`) | 0 / 0 / 0 | `{"compatible_with":"schema/released.openrpc.json"}`, no baseline amendment |
| `quality-matrix --mode contract` / `--mode verify` | 0 / 0 | `"finding_count": "0"`, `"verdict": "pass"` |
| `bun run generate` → `generate:check` / `typecheck` / `bun test` with `OVIS_CONFORMANCE_BIN`/`OVIS_TERMINAL_BIN` on the lane target (`sdk-replay.log`) | 0 | 31 files regenerated; 51 pass, 0 fail, 3423 expect() calls |
| `cargo run --locked -q -p ovis-terminal-replay --bin ovis-replay -- refresh` | 0 | `{"refreshed":12}` |
| T66's three literal `cli/manual` commands, re-recorded (`rerecord-task-66.log`) | 0 / 0 / 0 | 122 / 71 / 29 assertions; then `evidence-audit` gives `pass`, findings 0, runs 187 |
| `cargo nextest run --manifest-path conformance/Cargo.toml` (`conformance-suite.log`; includes all/all over every executable scenario, security/validation among them) | 0 | 35 tests run: 35 passed |
| `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` (`check.log`) | 0 | 1637 tests run: 1637 passed, 2 skipped; hygiene 0 violations; evidence-audit pass |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` / `cargo clippy ... -- -D warnings` (`windows-check.log`) | 0 / 0 | |

### What the tests assert

- `tests/assembly.rs` hello test: `published − advertised == EXCLUDED` (10 names) and `advertised ⊆ published`. Each excluded method, called through `ovis-terminal call`, exits 1 with `kind: not_implemented` and a hint beginning `follow-up: `.
- `served.rs`: 60 echoed lines fill the scrollback (waited for event-driven with `terminal.wait_output`). `from_line "0"` returns h-0/h-1/h-20 with `truncated:false`. The newest-3 read equals the last 3 lines and its `first_line` is `total-3`. `from_line "1" limit 2` returns exactly lines 1..3 with `truncated:true`. `limit 0` and a numeric `from_line` are `invalid_params`. `agent.list` is empty before `agent.register`; afterwards it shows the registered `agent_id`/label, `total` 1, `limit 0` gives `truncated:true`, and `page.after` gives `invalid_params`. `server.start` exits 11 with `already_running`.
- `served_events.rs`: after a real `status.changed` event with seq S, `ack(S+1)` is `seq_ahead`, `ack(S)` returns `seq S` with `scan_seq ≥ S`, and `ack("0")` keeps S. A numeric seq is `invalid_params`. The observer principal gets `not_found`.
- Unit tests: the cursor marks (including a `u64::MAX` round-trip), the newest-window arithmetic, one-page truncation, and the excluded set being sorted/distinct with every member answering `not_implemented` and carrying its own hint.

### Mutants (each must fail a shipped test; logs in `docs/evidence/task-P5/`)

| mutant | change | result |
|---|---|---|
| M1 | `seq > delivered` → `>=` in `Cursor::ack` | exit 100; 4 of 5 targeted tests fail |
| M2 | drop `self.advance(&id, &pulled)` before the write | exit 100; `events_ack_records…` fails |
| M3 | newest window → `0` | exit 100; 2 tests fail |
| M4 | drop the `Excluded::of` refusal (re-run at its final site in `RequestHandler::call`) | exit 100; the hello test fails (exit 4 `method_not_found` instead of 1) |
| M5 | `kept < total` → `kept <= total` | exit 100; 2 tests fail |

### Adversarial pass

- Ack race: the delivered mark is raised before `notify`, so a client that acks immediately after receiving an event cannot get a spurious `seq_ahead` (M2 proves the test sees this).
- An ack from another principal is `not_found`, the same as `events.update`/`unsubscribe`. A numeric `seq` is `invalid_params`.
- Exclusions do not bypass authorization: the refusal runs inside `RequestHandler::call`, which sits behind `Revalidated`/`BoundaryPolicy`, so a mutation-role check (for example Administer for `machine.exec`) still runs first. `security/validation` stays green.
- Hello does not advertise an excluded name (scenario failure case: `$excludes`).
- `terminal.history` reads only the screen. It never writes input or moves a viewport, and its two pages pin one revision.

## Seams (files outside the Write scope, and why)

- `crates/ovis-terminal-cli/src/assembly.rs` and `handler.rs`: module registration and routing (inside `assembly/**`). Listed here only because P6 also touched `assembly.rs`; `git merge-tree` against 4d0511d is clean.
- `crates/ovis-terminal-cli/tests/assembly.rs`, `tests/assembly/served.rs`, `tests/assembly/served_events.rs`: the existing hello test pinned the 14 as `method_not_found`, so it had to become the explicit-exclusion test. The new end-to-end tests are registered next to it.
- `crates/ovis-terminal-cli/src/skill/manual.md`: byte-identical embedded copy of `docs/skill/ovis-terminal.md` (an equality test enforces it).
- `docs/runtime-contract.md`: regenerated with `ovis-terminal schema contract` (`manual_tests` pins it to the build).
- `conformance/ideal-map.json` (quality-matrix contract), `conformance/fixtures/ovis/manifest.json` (replay refresh pins), `docs/evidence/task-66/task-66-{all,ovis-terminal-v1}.json` (re-recorded: their `$equals_file` pins the schema).
- `conformance/src/runner_tests/suite.rs`: the scenario is added in sorted position.
- `conformance/scenarios/cli/advertised-methods.json`: the runner maps `--scenario cli/advertised-methods` to this file; a `cli/advertised-methods/` directory would not be found (same as T66's note).

## Risks

- **T65 (#86) is not on this base**, so `docs/ovis-adapter.md` and its `client/migration` "Known gaps"/`unserved` rows are not in this tree. On union-merge: drop `agent.list`, `events.ack`, `server.start` and `terminal.history` from that gap list, and mark the 10 others as "excluded, answers `not_implemented`" instead of "unserved / `method_not_found`". T65's `served.rs` check flags guide-served-but-not-advertised; listing a now-served method as unserved does not fail it, but the guide would be stale.
- Schema regeneration: open lanes whose committed evidence pins `schema/ovis-terminal.openrpc.json` via `$equals_file` need re-recording on their merge ref after this lands.
- `not_implemented` maps to exit 1 (plain failure) in `invocation/exit.rs`, which is outside the write scope.
- The ack marks live in memory per subscription and are not durable across a restart. A subscription does not survive a restart either.

DEFERRED VERIFICATION: none. The runner executes `cli/advertised-methods` on the real binary (exit 0 above).
