Plan todo 67: build verified cross-platform release artifacts.
Closes GAP-127, GAP-139, GAP-140, GAP-141. Delivers IDEAL-034, IDEAL-100, IDEAL-112, IDEAL-113, IDEAL-114.

## What this adds

- **`ovis-terminal schema compat --released <path>`** (offline, side-effect free). It compares this build's OpenRPC document with a released baseline using the same-generation rules: released methods, events and required outputs stay, and inputs only widen. A break exits 1 with kind `compatibility_break` and lists every `{path, rule}` in `breaks`. A baseline of another generation is `generation_mismatch`. An unreadable file is `baseline_unreadable`. A file that is not a comparable OpenRPC document is `baseline_malformed`. `--released` with any other `schema` part, or `schema compat` without `--released`, is a usage error (exit 2). The four kinds are listed in `schema cli` `failure_kinds`.
- **`scripts/release/` (`release-stage` crate and `build.sh`).** Each step reads and writes closed JSON documents (unknown fields are refused; byte counts are decimal strings):
  - `receipt` records the build host, rustc, the bundled SQLite version (read from libsqlite3-sys's `sqlite3.h`: 3.54.1 via 0.39.1) and the C toolchain observations that `build.sh` reads from gcc/musl, Xcode/SDK/clang or MSVC/cl.
  - `stage` runs every check before it writes anything. Checks: the receipt must come from the target's own host (`cross_build_refused` otherwise); notices must carry the Apache-2.0 attribution and list every Cargo.lock registry crate; the binary, the `--skill` manual, OpenRPC `info.version`, `packages/client-ts/package.json` and the packed SDK tarball must all report one version; the binary's `schema` output must equal `schema/ovis-terminal.openrpc.json`; the built binary itself must pass `schema compat --released schema/released.openrpc.json`. It then lays out the binary, schema, manual, packed SDK, LICENSE, THIRD_PARTY_NOTICES.md and receipt with per-file sha256.
  - `seal` re-hashes the staged files and packs them into `<name>.tar.gz` (+ `.sha256`).
  - `qa` requires the target's own host (`not_native` otherwise). It unpacks the archive the way an installer would, re-hashes every file and runs the unpacked binary (version, manual, schema digest, released compatibility). It writes `<name>.qa.json` whether the checks pass or fail.
  - `verify` re-hashes every entry and archive.
  - `promote` is the orchestrator-only gate. It requires all five targets (`target_missing` names the absent ones) and receipts that match `scripts/release/pins.json` exactly (rustc 1.99.1, SQLite 3.54.1, runner, gcc/musl, Xcode/SDK, MSVC/cl; `target_receipt_invalid`). Each target needs a passing native QA receipt for that exact archive from its own host (`native_qa_missing` / `native_qa_failed`), plus the notices and a compatible schema. It then writes `release-manifest.json` and `SHA256SUMS` and decides `promote`, `dry_run` or `already_published`. An identical rerun after publication is `already_published` with `publish: false`. A rerun with different artifacts is `release_conflict`. It never uploads.
  - `notices [--check]` regenerates the crate license inventory in THIRD_PARTY_NOTICES.md from `cargo metadata`.
- **`.github/workflows/release.yml`**:
  - On **pull_request** (path-filtered) it runs one ubuntu dry run (`build.sh --target native --dry-run`), the release-stage tests, the notices freshness check and the `release/artifacts` scenario (all, then the failure injection).
  - The **five-target matrix runs only on a `v*` tag push or `workflow_dispatch`**. On a pull request it would need two macOS runners, one arm runner and Windows, each doing a fat-LTO build, which exceeds the hosted-runner budget for PR CI. Every target builds natively: x86_64-unknown-linux-musl on ubuntu-24.04, aarch64-unknown-linux-musl on ubuntu-24.04-arm (musl-gcc over gcc-14 14.2.0, musl 1.2.4), x86_64-apple-darwin on macos-15-intel, aarch64-apple-darwin on macos-15 (Xcode 16.4, SDK 15.5), and x86_64-pc-windows-msvc on windows-2025 (MSVC 14.44 / cl 19.44). There is no Linux-to-macOS cross build.
  - Each target job also runs native behaviour checks on the unpacked binary: the `cli/manual` conformance scenario on Linux and macOS, and the offline verbs on Windows.
  - `promote` runs `if: always()` after the matrix, so a failed or missing target (for example Windows) fails promotion with `target_missing` instead of producing a smaller release. It publishes only on a tag push or with `publish=true`, through the protected `release` environment. Without either, promotion is a dry run.
  - The workflow keeps `CARGO_PROFILE_{DEV,TEST}_DEBUG=line-tables-only` (F2b).
  - **Release profile:** unchanged `[profile.release]` (fat LTO, one codegen unit, stripped symbols, `panic = "abort"`). That is the profile the shipped binary should use. A faster profile would ship a different binary from the one QA ran.
- **Signing and notarization: not performed.** No signing identity, notarization service or other paid service is used. `pins.json` carries the published statement, and every promotion report and manifest repeats `signing.performed: false`.
- **THIRD_PARTY_NOTICES.md:** the existing herdr/herdr-core Apache-2.0 attribution and full license text, plus a generated inventory of all 168 crates.io crates in Cargo.lock (name, version, SPDX license, repository).
- **`schema/released.openrpc.json` is unchanged.** Promoting a new released baseline is an orchestrator decision taken with a tagged release, and it needs a BASELINE-AMENDMENTS entry. This PR adds the gate that every built artifact must pass against the current baseline.

Exclusions enforced, not copied:
- EXCL-013: no release notes or announcement modal; the release statement lives only on the GitHub release.
- EXCL-016 and EXCL-021: every refusal is a typed, queryable kind with a hint and an exit code in a JSON report, not printed text or a transient banner.
- V-3895: the toolchain is pinned in rust-toolchain.toml and checked in every receipt.
- V-3896: workspace lints apply to the new crate.
- V-002: deny.toml stays the license allow-list behind the inventory.
- V-3902: release kinds come from one `thiserror` enum with `kind()`/`hint()`, not hand-maintained strings.

## Before

`absent` (no comparable baseline capability; not an executed failing test). Gap baseline: GAP-127, GAP-139, GAP-140, GAP-141. At base 1c1527d there is no `scripts/release`, release workflow or `release/` scenario, `SchemaPart` has only `Openrpc | Cli | Contract`, and THIRD_PARTY_NOTICES.md has no crate inventory (`docs/evidence/task-67/before.log:1-17`).

## After

Host: Linux x86_64. `$ATTEMPT_DIR=/dev/shm/ovt-fixtures/t67-scratch/attempt`. Per host rules, `target/` is `CARGO_TARGET_DIR=/dev/shm/ovt-target-lane-t67`, so `--binary target/debug/ovis-terminal` materializes as `/dev/shm/ovt-target-lane-t67/debug/ovis-terminal` (sha256 `0ea230ead53b368d7030c2b94ea6c277a6f0ed8e6046132520386fd3f7381049`, 151442776 bytes).

| Command | Exit | Result |
|---|---|---|
| `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` | 0 | `build.log` |
| `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario release/artifacts --isolated --case all --evidence "$ATTEMPT_DIR/task-67-ovis-terminal-v1.json"` | 0 | `result: passed`, cases 2, assertions 63, failed 0, skipped 0 (`qa-all.log`, `evidence-all.json`) |
| `... --case happy --evidence "$ATTEMPT_DIR/task-67-ovis-terminal-v1.json"` | 0 | assertions 27, failed 0, skipped 0 |
| `... --case failure --inject '{"fixture":"same-generation-removed-required-output"}' --evidence "$ATTEMPT_DIR/task-67-ovis-terminal-v1.json" --append` | 0 | assertions 13, failed 0, skipped 0 (`qa-happy-failure.log`, `evidence-happy-failure.json`) |
| `sh scripts/release/build.sh --target native --dry-run` | 0 | x86_64-unknown-linux-gnu staged as a non-release artifact: 7 files, sealed archive sha256 `6b83e15d…`, native QA 5/5 checks passed, verify 1 entry 1 sealed, "nothing was published" (`dry-run.log`) |
| `cargo nextest run --locked -p release-stage` | 0 | 12/12 |
| `cargo nextest run --locked -p ovis-terminal-cli` | 0 | 117/117 (4 new `schema compat` tests) |

Materialized commands and outputs proving `compatibility_break`, from the failure case's evidence:

```
release-stage fixture same-generation-removed-required-output --binary /dev/shm/ovt-target-lane-t67/debug/ovis-terminal --repo <worktree> --scratch <isolated root>
  -> /dev/shm/ovt-target-lane-t67/debug/ovis-terminal --json schema compat --released <isolated root>/release-same-generation-removed-required-output/released.openrpc.json
     exit 1, stderr {"error":{"kind":"compatibility_break","exit_code":1,"breaks":[
       {"path":"server.status ->.release_fixture_required_output","rule":"output_property_removed"},
       {"path":"server.status ->.release_fixture_required_output","rule":"output_no_longer_required"}],...}}
  release stage: exit 1, kind compatibility_break, entries_written "0",
  released_unchanged true (schema/released.openrpc.json sha256 06e5e410… before and after)
```

The fixture's baseline is generation 1, the same as the binary. It adds a required `server.status` output that this build does not return, which is what removing a required output in a same-generation release looks like from a released client.

Happy observables: `ovis-terminal --json schema compat --released schema/released.openrpc.json` returns `{"compatible":true,"generation":"1",...}` with exit 0. The native artifact passes stage, seal, native QA and verify, with schema equal to source, `apache_attribution: true`, 168 inventory crates and compat exit 0. A complete five-target matrix promotes as `dry_run` with `publish: false`, supported `linux/x86_64, linux/aarch64, macos/x86_64, macos/aarch64, windows/x86_64` and `signing.performed: false`.

Adversarial (`adversarial.log`, `mutants.log`):
- Malformed input: a bad hash is `checksum_mismatch` naming the file, an entry with an unknown field is `manifest_malformed`, an unreadable directory is `io` (exit 3), a non-OpenRPC baseline is `baseline_malformed`, a generation-2 baseline is `generation_mismatch`, and `--released` without `compat` is usage (exit 2).
- Misleading success: a matrix without the Windows artifact is `target_missing ["x86_64-pc-windows-msvc"]`. Also refused: a missing QA receipt, a macOS receipt from a Linux host, an unpinned SQLite, lost Apache attribution, a cross-host receipt passed to `stage` (`cross_build_refused`, nothing written), and a non-release triple without `--dry-run` (`build.sh` exit 2).
- Repeated interruptions: an identical rerun after publication is `already_published` with `publish: false`; a rerun with rebuilt artifacts is `release_conflict`.
- Mutants, each caught by a shipped test (test exit 100): M1 promotion ignores a missing target, M2 republishing over a different published manifest, M3 accepting a cross-built receipt, M4 the CLI treating a break as compatible.

Gates: `bash with-slot.sh bash scripts/check.sh` -> exit 0, nextest 1591 passed / 2 skipped, every step passed (`check.log`). Windows pre-check: `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` -> exit 0; `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings` -> exit 0 (`windows-precheck.log`).

## Seams

Files outside the write scope, and why:
- `crates/ovis-terminal-cli/src/invocation/{args,plan,error,report,discovery}.rs`, `invocation.rs`, new `invocation/compat.rs` and `invocation/report/record.rs` (the failure-record builders moved out of report.rs to keep it under 250 pure LOC, no behaviour change): the acceptance requires the binary itself to return `compatibility_break`, so `schema compat --released` is a CLI verb. The four new kinds are in `FAILURE_KINDS` (exit 1).
- `crates/ovis-terminal-cli/src/skill/manual.md`, `docs/skill/ovis-terminal.md`, `docs/runtime-contract.md`: the manual documents the verb, and the shipped copies must equal the embedded manual and `schema contract` (their tests enforce this). The OpenRPC schema is unchanged, so neither the SDK nor the replay fixtures change.
- `conformance/src/scenario/release.rs`, `conformance/src/engine/release.rs`, plus one line each in `scenario.rs`, `engine.rs`, `scenario/model.rs` (`Step::Release`) and `engine/steps.rs`, and `conformance/Cargo.toml` (release-stage dependency): a `release` step runs release-stage in-process against `--binary`, mirroring the `gates` step.
- `conformance/scenarios/release/artifacts.json`: the runner's id grammar maps `release/artifacts` to this file, not a directory. `conformance/src/runner_tests/suite.rs` gains `release/artifacts.json` in sorted position. It was never in `declared.json`.
- `conformance/fixtures/ovis/manifest.json`, `predicates.json`: regenerated with `ovis-replay refresh` because the traceability predicates now resolve `release/artifacts` as executable.
- `Cargo.toml` (workspace member `scripts/release`, `release-stage` workspace dependency) and `Cargo.lock`.
- `docs/evidence/task-67/`.

## Risks

- The five-target matrix has not run yet. It needs a tag or a `workflow_dispatch` from the default branch (`main`), and `main` is untouched. The pins for gcc-14 on ubuntu-24.04-arm, musl-tools 1.2.4 and the Xcode 16.4 / SDK 15.5 pair on macos-15-intel come from the runner image documentation and the existing ci.yml pins. If an image drifts, the job fails at its pin assertion or `promote` fails with `target_receipt_invalid`. A drift never produces an unpinned release.
- The `release` environment must exist with required reviewers for promotion to be orchestrator-only. GitHub creates an unprotected environment on first use, so the repository owner must add the protection rule.
- The inventory names each crate's SPDX license and repository. It does not bundle every crate's copyright and license file the way cargo-about does.
- The installer (`install.sh`) is todo 68. Native QA here unpacks the archive as an installer would and runs the unpacked binary.
- The local dry run builds the host's gnu triple, not musl (this host has no musl C toolchain), so it is staged with `release_target: false` and is never promotable.

DEFERRED VERIFICATION: the five-target release matrix and the `promote` job (tag or `workflow_dispatch` only). The PR CI runs the dry-run job.
