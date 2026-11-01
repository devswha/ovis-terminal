test(quality-gates): enforce rust quality and violation checklist gates

Todo 61 (wave 16). Closes GAP-014, GAP-139, GAP-140, GAP-142, GAP-144. Delivers IDEAL-008, IDEAL-013, IDEAL-107, IDEAL-112, IDEAL-113, IDEAL-115 and IDEAL-117 (the quality-gate slice of each). Must not reproduce EXCL-006, EXCL-020 or EXCL-021, nor V-001, V-002, V-003, V-059, V-3895, V-3896 or V-4400. Each one is a checked row in `docs/quality-checklist.md`.

## Before

`absent`: there was no comparable baseline capability. This is not an executed failing test. `docs/evidence/task-61/before.log` (base edfdb58) shows that none of `probes/quality`, `conformance/scenarios/quality`, `docs/quality-checklist.md`, `conformance/ideal-map.json`, `conformance/traceability.json` or `.github/workflows/quality.yml` existed, and `probes/Cargo.toml` declared no binary (`grep [[bin]]` exit 1).

Gap baseline: GAP-014, GAP-139, GAP-140, GAP-142, GAP-144. Legacy defects the gates now reject (`docs/research/violations.md`):
- `:165` V-001: no `[lints]` table.
- `:168` V-003: 875 pure LOC.
- `:224` V-059: eight parameters.
- `:4109` V-3896: the lint table lacks unwrap_used and similar lints.
- `:4614` V-4400: `assert!(t.status_ms >= Some(since))`, an assertion that cannot fail.

Seam baselines:
- `conformance/src/invocation.rs:166` (base) printed `"evidence_written": written.as_ref().err().map(top_failure)`, which reads `null` on success.
- `packages/client-ts/test/cases/real-socket.ts:31` (base) called `fixture.stop()` after the `finally` block, so a failing test leaked its fixture root.
- `docs/evidence/task-52/pr-body.md:246`: "The `x-effect: control` methods are still classed as reads locally by remote dispatch; this is a todo-61 note".

## After

Host: Linux 7.0.0-38-generic x86_64. `CARGO_TARGET_DIR=/dev/shm/ovt-target-lane-t61` is the host's RAM target, so `--binary` is `/dev/shm/ovt-target-lane-t61/debug/ovis-terminal`, the equivalent of `target/debug/ovis-terminal`. Logs are in `docs/evidence/task-61/` and the raw evidence is `task-61-ovis-terminal-v1.json` plus `acceptance-evidence.json`.

Acceptance:
- `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` → exit 0 (`acceptance-build.log`).
- `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary …/ovis-terminal --scenario quality/gates --isolated --case all --evidence "$ATTEMPT_DIR/task-61-ovis-terminal-v1.json"` → exit 0, `{"result":"passed","evidence_written":true,"totals":{"assertions":"113","cases":"2","failed":"0","passed":"2","skipped":"0"}}`. The evidence carries the binary sha256 (`137205bce6a04f8d…`), host linux/x86_64, every gate's argv, numeric exit and full JSON report.
- `cargo run --locked --manifest-path probes/Cargo.toml --bin source-check -- --roots crates packages/client-ts conformance bench probes --max-pure-loc 250 --max-params 3 --violations docs/research/violations.md --checklist docs/quality-checklist.md --evidence "$ATTEMPT_DIR/task-61-ovis-terminal-v1.json"` → exit 0, `verdict: pass` with 0 findings. It checked 1294 Rust files and found a fresh checklist (64 rule ids, 5174 V-rows).
- Seeded negative self-test: `source-check -- --fixture oversized-file --max-pure-loc 250 --max-params 3` → exit 1, `kind: quality_violation`, finding `max-pure-loc` at `seed/oversized-file.rs:1` (`acceptance-seeded-negative.log`).

QA, happy (`qa-happy.log`):
- `… --scenario quality/gates --isolated --case happy --evidence "$ATTEMPT_DIR/task-61-ovis-terminal-v1.json"` → exit 0, 65 assertions.
- It materializes these real binary commands:
  - `ovis-terminal schema openrpc` → exit 0, stdout equals `schema/ovis-terminal.openrpc.json` (`$equals_file`).
  - `ovis-terminal --json schema openrpc --target focused` → exit 2, empty stdout, stderr `{"error":{"kind":"usage","origin":"cli","exit_code":2,"hint":…}}`. An unknown targeting flag is a typed error before any connection (EXCL-006).
- The gates then run on the repository: `source-check` exits 0 and `quality-matrix --mode verify` exits 0 (121 IDEAL, 154 GAP, 29 EXCL, 104 methods).
- Each clean seed exits 0.
- Each seeded defect exits 1 with its rule:
  - the oversized file → `max-pure-loc`
  - the fourth parameter → `max-parameters`
  - the dropped Result → `dropped-result` at `seed/dropped-result.rs:2`
  - the missing QA scenario → `missing-scenario`
  - the fake exit-zero evidence → `exit-forged`

QA, failure (`qa-failure.log`):
- `… --case failure --inject '{"fixture":"missing-receipt-and-pass-banner"}' --evidence "$ATTEMPT_DIR/task-61-ovis-terminal-v1.json" --append` → exit 0, 7 assertions.
- The materialized gate command is `evidence-audit --fixture missing-receipt-and-pass-banner`, with process `exit_code: 1`. Its report is `{"verdict":"fail","kind":"evidence_invalid","finding_count":"2","findings":[{"rule":"binary-hash"},{"rule":"receipt-missing"}]}`.
- The run passes only because the auditor returned evidence_invalid with exit 1. The fixture's `"banner": "PASS: all gates green"` is not counted.
- An unplanned `--inject` fails the run with exit 1 (`injection` mismatch).
- Audited by itself: `evidence-audit -- --input "$ATTEMPT_DIR/task-61-ovis-terminal-v1.json"` → exit 0 (`qa-evidence-audit.log`). Every assertion was recomputed from its expected and actual values, and every `binary/exit` was matched to a recorded process exit.

Tests (`cargo nextest run -p ovis-terminal-probes`): 22 run, 22 passed.
- `quality_gates`:
  - every one of the 18 seeded defects fails with exit 1 and its rule;
  - every clean seed passes;
  - a limit above 250/3 is `limit_above_contract` (exit 2), while tightening fails the clean seed;
  - an unknown fixture or flag is a usage error (exit 2).
- `quality_repository`:
  - the repository passes source-check with a fresh checklist;
  - the matrices verify;
  - a drifted IDEAL predicate and a removed schema method fail as `predicate-drift` and `contract-removed`;
  - reports append in invocation order.
- Unit tests:
  - `assert_ne!(fresh(), fresh())` is not flagged, while `assert!(x >= x)` is;
  - the cfg_attr `allow` is found;
  - the receiver counts as a parameter;
  - same-line `Closes | Delivers` fields are parsed;
  - removed todos are accounted for;
  - matcher operators are recomputed.

Server: `the_control_list_names_exactly_the_published_control_methods` and `a_remote_control_call_needs_the_observing_permission_not_a_read` pass. client-ts: `bun run generate:check && bun run typecheck && bun test` → 30 pass, 0 fail (`client-ts.log`).

Gates over real inputs:
- `evidence-audit` accepts every runner-format evidence file committed under `docs/evidence/` (64 files).
- It rejects the 11 pre-runner free-form files of todos 7-19 (`manifest-schema`/`host`).

### What this ships

- `probes/quality/` holds five tools (`source-check`, `evidence-audit`, `plan-audit`, `ideal-audit`, `quality-matrix`). Each is a thin binary over `ovis_terminal_probes::quality::execute`, with `--fixture` seeds for every rule.
- `source-check` reuses the workspace-hygiene token-based pure-LOC counter. Its syn rules:
  - `max-parameters`, receiver included, the current binding contract;
  - `dropped-result` (`let _ =` over a call, await, `?` or macro);
  - `allow-attribute`, including cfg_attr;
  - `numeric-cast`;
  - `tautological-assert`, for pure self-comparisons only.
- `source-check` also checks the strict Cargo configuration: workspace lint levels, member `[lints] workspace = true`, the rust-toolchain pin, deny.toml and the clippy thresholds.
- `docs/quality-checklist.md` is generated from all 64 Rule totals ids. It lists every V-row of each rule, separates 36 mechanically gated rules, 27 review items and 1 justified N/A, and lists the cited V-rows and EXCL-006/020/021 with what enforces them. `source-check` fails when the file is stale, a rule has no disposition, or a named gate does not exist.
- `conformance/traceability.json` and `conformance/ideal-map.json` cover all 121 IDEAL, 154 GAP and 29 EXCL rows, with their todos and scenarios. They are generated from the plan, sha-pinned, and use verbatim ledger predicates.
- The `contract` section is regenerated from `schema/ovis-terminal.openrpc.json`, not from advisory text. `--mode verify` fails on predicate drift or on a contract element the schema no longer has; additions stay compatible (IDEAL-113).
- `plan-audit` and `ideal-audit` implement the F1 and F4 command lines.
- `.github/workflows/quality.yml` and a `quality-gates` job appended last in `probe-stress-macos.yml`.

Plan findings for F1/F4, reported and not fixed here because the plan is not in this write scope:
- `plan-audit` on the live plan exits 1 (`plan-audit-live.log`).
- Todos 1, 71-76 and 80-82 lack a happy+failure QA line or a `--scenario` for the rows they claim.
- 13 same-wave write-scope overlaps: todo 77/79 `dispatch/` against the lanes, and 83/84/85/86 `graphics.rs`.
- `ideal-audit` on this attempt directory proves 0 of 117 rows (`ideal-audit-live.log`), as designed. Only quality/gates evidence is there, with no Windows smoke and no soak receipt.


## Round 2 (gate review and adversarial findings)

Base unchanged at edfdb58. These are new commits on top of 0670a42; nothing was force-pushed.

- **B1, evidence-audit accepted assertions it could not judge.** `matchers::holds` now returns a typed `Judgement`.
  - An unknown operator is a `unknown-matcher` finding (exit 1).
  - `$present` and `$capture` are judged from the recorded `null`: `{"$present": true}` with `actual: null` is `assertion-forged`.
  - `$equals_file` is recomputed against `--repo`. An unreadable operand is `unjudgeable-assertion`.
  - The unit test that pinned the old behaviour is replaced by `presence_is_judged_from_the_recorded_null` and `an_unknown_or_unreadable_operator_is_never_accepted`.
  - New seeds `forged-matcher` (`{"$bogus":"never"}`) and `present-on-null` fail with exit 1 in `quality_gates` and in the `quality/gates` failure case.
- **B2, the contract was names-only.** `conformance/ideal-map.json` `contract` now records:
  - every method verbatim (params, result, errors, `x-effect`, `x-targeting`, `x-params`, summary, `paramStructure`);
  - every error kind, event and notification entry;
  - `x-capabilities`, `x-compatibility`, `x-error-data`, `x-limits` and `x-stream`;
  - a sha256 per component schema (428).

  `quality-matrix --mode verify` regenerates the contract from `schema/ovis-terminal.openrpc.json` and diffs it in both directions: `contract-missing`, `contract-removed`, `contract-drift`, `contract-stale`. Tests:
  - `a_method_missing_from_the_recorded_contract_fails_verify` deletes `worktree.remove` from the record and gets exit 1 with `contract-missing methods/worktree.remove`.
  - `a_removed_or_changed_contract_entry_and_a_weakened_predicate_fail_verify` gets `contract-drift methods/doc.set`, `contract-removed` and `predicate-drift`.
  - `quality-matrix --mode contract` rewrites only the contract from the schema, without the plan. A lane that changes the schema runs it, the same way the SDK is regenerated.
- **N1, ownership.** `verify` fails on any row whose todos or scenarios are empty (`matrix-unowned`). `an_emptied_owner_or_scenario_fails_verify` empties GAP-014's todos and EXCL-029's scenarios and gets two findings and exit 1.
  - EXCL-029 is now owned through its ledger citation (`owner decision 2026-10-31; todo 37`): todo 37 / `agent/records`. Both the matrix derivation and plan-audit count a `todo N` named in an EXCL row's citation.
  - plan-audit defaults and the documented command now use `--require-ideals 121 --require-gaps 154 --require-exclusions 29`. On the live plan `excluded_features` is `[]` (`plan-audit-live.log`).
- **N2, `.ok();` discards.** `dropped-result` now also flags every `<expr>.ok();` statement (`ok-discard` seed, unit tests). The tree has 13 such statements. Each one is classified in `probes/quality/dispositions.json` `dropped_result_exemptions`, rendered under "Accepted `.ok();` discards" in the checklist.
  - 5 production `reply.send(..).ok()` sites in `streams/scheduler/{actor/serve.rs,db.rs}`: a oneshot reply fails only when the requester dropped its receiver. No product change is needed.
  - 8 test-harness sites: barrier callbacks that cannot return, bounded hang guards, and cleanup in `Drop`.
  - An exemption that stops matching is a `stale-exemption` finding.
- **N3, committed evidence.** `evidence-audit --tree docs/evidence --exemptions probes/quality/evidence-exemptions.json` runs in check.sh and in the quality workflow. It audits 84 files and exits 0.
  - The 22 exemptions each give a reason: 11 pre-runner files (todos 7-19), 7 task-56 tuning files, and 4 all/all runs whose `$equals_file` compared the schema of their commit.
  - `without_exemptions_exactly_the_exempted_files_are_rejected` proves the list equals the rejected set. `an_exemption_for_valid_evidence_is_stale` proves a stale entry fails.
  - Quality-gate reports appended to an evidence file are now audited as gate reports, so `docs/evidence/task-61/acceptance-evidence.json` passes (`evidence-audit --input` exit 0). Their verdict, exit, kind, `finding_count` and findings must agree; seed `forged-gate-report` fails.
- **Also:** the numeric-cast unit test covers all 14 numeric targets (the adversarial `usize` mutant).

Round-2 results (`docs/evidence/task-61/*.log`, regenerated):
- conformance `quality/gates --case all` exit 0, 2 cases, 129 assertions, `failed: 0`, `skipped: 0`;
- `--case happy` exit 0 (65 assertions);
- `--case failure --inject '{"fixture":"missing-receipt-and-pass-banner"}' --append` exit 0 (7 assertions), with the gate process exit 1 `evidence_invalid`;
- source-check exit 0;
- the seeded negative exit 1;
- `evidence-audit --fixture forged-matcher` exit 1 (`unknown-matcher`);
- matrix verify exit 0;
- matrix check against the live plan exit 0;
- evidence tree exit 0;
- `cargo nextest run -p ovis-terminal-probes`: 28 run, 28 passed.

Round-2 mutants (`mutants.log`, 5 to 9), each killed by a shipped test:
- unknown matcher treated as holding;
- reverse contract comparison removed;
- ownership needing both lists empty;
- stale-exemption check disabled;
- `.ok()` rule disabled.

Not changed by round 2: the three deferred flake items in Risks.

## Seams (files outside the write scope)

- `probes/Cargo.toml`, `probes/src/lib.rs`, `Cargo.toml` (workspace deps `workspace-hygiene`, `quote`) and `Cargo.lock`: register the bins, tests and the `#[path]` module.
- `conformance/Cargo.toml`, `conformance/src/scenario{.rs,/model.rs,/gates.rs}`, `conformance/src/engine{.rs,/steps.rs,/gates.rs}`: the new `gates` runner step, which runs the tools in-process through spawn_blocking and records argv, exit and report as a `gate` process. `conformance/scenarios/quality/gates.json` is the scenario file for the id `quality/gates`, since the runner maps ids to `<family>/<name>.json`.
- `conformance/src/runner_tests/suite.rs`: registers `quality/gates.json` and asserts the summary's `evidence_written: true` / `evidence_error: null`.
- `conformance/src/invocation.rs`: `evidence_written` is now a boolean, and the failure moved to `evidence_error` (cosmetic gate note).
- `scripts/check.sh`, three new steps:
  - a `/tmp` free-space pre-check (`rule=tmp-space`, below 2 GiB);
  - one threaded `cargo test -p ovis-terminal-core --test subscriptions` run, the PUMP_GATE class;
  - the source-check gate.
- `packages/client-ts/test/cases/real-socket.ts`: `fixture.stop()` now runs inside `finally`.
- `crates/ovis-terminal-server/src/dispatch/lanes/boundary_policy{.rs,/matrix.rs,/qa.rs}` and `dispatch/remote/route.rs`: `CONTROL_METHODS` names exactly the 7 `x-effect: control` methods (a test pins the list to the OpenRPC document), and remote dispatch classes them as `Action::Subscribe` instead of `Read`. Both actions require `Permission::Observe`, so no grant changes. Audit records and refusals now name the right action.
- `.github/workflows/probe-stress-macos.yml`: the `quality-gates` option and job, appended last.

## Adversarial pass (executor self-review)

This is the executor's own slop/programming pass for the block's adversarial classes; the independent terra adversarial review and the gate review run separately against this PR head.


- Flaky tests (single-run determinism): the gates are pure functions of checked-in inputs and use no clock or network. The 25x macOS stress job reruns them.
- Misleading success output:
  - `evidence-audit` recomputes every assertion, including the runner's `$at_least`/`$contains`/`$excludes` operators and `expect_failure` kind-only checks.
  - It rejects a passed claim with a skipped or failed case, a run error or no binary hash.
  - It matches each `binary/exit` to a real process exit and rejects prose banners.
  - A first draft wrongly flagged real evidence that used matcher operators. Auditing all committed evidence found this, and it was fixed.
- Malformed input: non-JSON evidence is `manifest-schema` (exit 1); unreadable inputs are exit 3; usage errors are exit 2.
- False positives found and fixed: `assert_eq!(read_os_boot()?, read_os_boot()?)` and `assert_ne!(fresh_challenge(), fresh_challenge())` are real checks. The tautology rule now flags only side-effect-free operands.
- Mutants (`mutants.log`), each killed by a shipped test:
  1. Treat calls as pure → `comparing_two_evaluations_of_a_call_is_a_real_check` fails.
  2. Skip the exit cross-check → `every_seeded_defect_fails_its_gate…` fails.
  3. Loosen the LOC limit by one → `limits_may_be_tightened_but_never_loosened` fails.
  4. Class only `attach.*` control as Subscribe → `a_remote_control_call_needs_the_observing_permission_not_a_read` fails.

## Contract checklist

- No `unwrap`/`expect`/`panic`, no `as` casts and no `#[allow]` (source-check passes on its own code).
- Boundary errors use `QualityError` (thiserror, with `kind()`, `hint()`, `#[source]`).
- Counts in reports are decimal strings.
- Every file is at most 250 pure LOC and every function takes at most 3 parameters.
- Tests assert machine values, not prose.
- No wire or schema change, so no SDK regeneration is needed.

## Risks

- The matrices are generated from the live plan (sha-pinned in `source.plan_sha256`). The tracked `.omo/plans/ovis-terminal-v1.md` is the stale initial copy, so `--mode check` must be run against the live plan, while CI runs only `--mode verify`.
- `source-check` in check.sh couples future lanes to the new syntax rules: dropped-result, allow, numeric-cast and pure tautologies. Today's tree passes.
- The quality.yml macOS job is new and has not run before this PR.

DEFERRED VERIFICATION: none for the runner, since `quality/gates` executes against the real binary today.

Not done here (flake-ledger items listed for this lane):
- the durable.rs fsync fault-injection seam (T49);
- the root cause of the deadline-sensitive server tests at 16 threads (T59);
- the bounded StartLock and holder `observe` flake.

Each needs its own root-cause run with a stress loop. Under the flake rule, root-causing comes before any fix, so they are left as follow-ups.
