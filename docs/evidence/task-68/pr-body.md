## Summary

Plan todo 68: `install.sh` installs a release into one explicit directory, verified against its manifest, and refuses everything else with a typed exit (closes GAP-001, GAP-017, GAP-127, GAP-141; delivers IDEAL-001, IDEAL-014, IDEAL-100, IDEAL-114).

- **`install.sh`** (POSIX sh). Overrides: `OVIS_TERMINAL_INSTALL_DIR` (isolated mode: the only directory written), `OVIS_TERMINAL_VERSION` (pin) and `OVIS_TERMINAL_RELEASES_URL` (GitHub layout `latest/download/` or `download/v<version>/`; https only, http only on loopback). It reads `release-manifest.json` (`ovis-release-manifest/1`, todo 67's promote format) and picks this host's `platform`. It downloads the archive and its `.sha256` sidecar, then requires the actual bytes, the sidecar and the length to match the manifest. It lists the archive before unpacking and refuses members outside `<name>/`, `..` components and links. It unpacks into a workspace inside the install dir and runs the unpacked binary with an empty environment, which must report the manifest version. The new binary then replaces the old one with a single `rename(2)`, the last step; the receipt is written the same way. `--check` re-hashes the installed binary against the receipt, and `--uninstall` removes only what the receipt lists.
- Typed exits: 1 `checksum_mismatch` / `unsafe_archive` / `version_mismatch` / `manifest_malformed`; 2 `usage` / `insecure_url`; 3 `unwritable_destination` / `download_failed` / `write_failed` / `missing_tool` / `not_installed`; 4 `unknown_target`; 5 `runtime_running`. Stdout carries exactly one JSON report with `kind`, `hint`, `exit` and details.
- Isolated mode writes no shell profile, global symlink or service, and never touches the default home install. The only runtime command it runs is the read-only `ovis-terminal --json server status`. If the runtime this environment resolves to is live, the installer does not replace its binary: it keeps the verified release in `releases/<name>/` and exits 5 with `delegate: "update.stage"` (see Risks).
- **`probes/install/`**: the `install-probe` binary and library. The good release is the binary under test, staged and sealed in-process by `release-stage fixture native-artifact` (todo 67's real entry/archive/sidecar format), published with a manifest and served by a loopback HTTP server inside the probe. Adversarial copies: one flipped byte; a hand-written ustar with a `<name>/../../escaped` member; an unshipped platform; a 99.0.0 release whose binary reports 0.1.0; and a `stall/` route that sends half the archive and holds the socket. Each of the 10 cases runs the literal `sh install.sh` command lines with an isolated environment (private `HOME` seeded with `.profile`/`.bashrc`/`.zshrc`, XDG tree, attempt directory, a 0700 socket directory) and compares inventories of the install directory and everything outside it before and after.
- **`conformance/scenarios/release/install.json`**: `happy` (1 planned probe run) and `failure` (the QA inject, which is the literal `CORRUPT_RELEASES_URL` command line, plus an `adversarial` run of the other 8 cases), run in-process through the existing `release` step.
- **`docs/operator-guide.md`**: overrides, the isolated-mode guarantees, update, running-runtime delegation, service ownership, removal, rollback, corruption, disk-full and interrupted-download diagnosis, redacted support evidence, and Windows install through the CI artifact contract (PowerShell `Get-FileHash` + `tar`).

## Before

`absent`: there was no installer and no comparable baseline capability; this is not an executed failing test. Gap baseline: GAP-001, GAP-017, GAP-127, GAP-141 (`docs/design/gap-list.md:19`, `:35`, `:151`, `:167`). The herdr installer the plan cites (`/tmp/ulw-ovis-yCJzPH/repos/herdr-core/distribution/install.sh:12,23,58`) is no longer present on this host; the repo cites `RT-HC-04` (status auto-starts a core) at `docs/evidence/baseline/smoke/hc-s18-stop-cleanup.log`.

## After

Host: Linux x86_64 (`uname -sm`). Logs are in `docs/evidence/task-68/`.

| Command | Exit | Result |
|---|---|---|
| `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` | 0 | `acceptance-build.log` |
| `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario release/install --isolated --case all --evidence "$ATTEMPT_DIR/task-68-ovis-terminal-v1.json"` | 0 | `result: passed`, assertions 156, failed 0, skipped 0, 2 cases (`acceptance-conformance-all.log`, `acceptance-evidence.json`) |
| `cargo run --locked --manifest-path probes/Cargo.toml --bin install-probe -- --script ./install.sh --isolated --evidence "$ATTEMPT_DIR/task-68-ovis-terminal-v1.json"` | 0 | `verdict: pass`, 10/10 cases, 0 failed checks (`acceptance-install-probe.log`, which holds every literal command with its exit, stdout and stderr) |
| `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario release/install --isolated --case happy --evidence "$ATTEMPT_DIR/task-68-ovis-terminal-v1.json"` | 0 | assertions 29, failed 0, skipped 0 (`qa-happy.log`) |
| `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario release/install --isolated --case failure --inject 'OVIS_TERMINAL_INSTALL_DIR="$ATTEMPT_DIR/install" OVIS_TERMINAL_RELEASES_URL="$CORRUPT_RELEASES_URL" sh install.sh' --evidence "$ATTEMPT_DIR/task-68-ovis-terminal-v1.json" --append` | 0 | assertions 26, failed 0, skipped 0; only the injected run executed (`qa-failure.log`, `qa-evidence.json`) |
| same `--case failure --inject 'sh install.sh --bogus'` (an unplanned command) | 1 | 1 failed assertion `inject/planned` (`qa-unplanned-inject.log`) |

(`target/debug/ovis-terminal` resolves to the lane's RAM target dir through a scratch cwd; the worktree's `target/` is never written.)

Materialized commands the probe ran, with what they returned:

- happy: `OVIS_TERMINAL_INSTALL_DIR="$ATTEMPT_DIR/install" OVIS_TERMINAL_RELEASES_URL="$FIXTURE_RELEASES_URL" sh install.sh` → exit 0, `{"outcome":"installed","mode":"isolated",...,"binary_sha256":<sha of the binary under test>}`. Then `"$ATTEMPT_DIR/install/bin/ovis-terminal" --version` → `ovis-terminal 0.1.0`; the pinned reinstall → exit 0; `--check` → exit 0 `verified`; `"$ATTEMPT_DIR/install/bin/ovis-terminal" --json server status` → `"state":"stopped"`; `--uninstall` → exit 0 `removed`, and the install dir no longer exists. The outside inventory (private home, XDG, attempt dir, checkout top level) is unchanged.
- failure: `OVIS_TERMINAL_INSTALL_DIR="$ATTEMPT_DIR/install" OVIS_TERMINAL_RELEASES_URL="$CORRUPT_RELEASES_URL" sh install.sh` → exit 1, `{"outcome":"refused","kind":"checksum_mismatch","exit":1,...,"recorded":<manifest digest>,"actual":<digest of the downloaded bytes>}`. Observed: `binary_unchanged: true`, `install_unchanged: true`, `outside_unchanged: true`, `service_started: false` (`server status` → `stopped`).

The scenario asserts each of those fields per case: exit, kind or outcome, `hint` present, `binary_unchanged`, `install_unchanged`, `outside_unchanged`, `service_started: false`, `delegate: update.stage` and `escaped_files: []`. The probe itself checks 67 conditions across the 10 cases (happy 10, checksum-mismatch 6, traversal 7, unknown-target 6, version-mismatch 6, unwritable 8, interrupted-download 7, running-runtime 9, foreign-uninstall 2, corruption 6). Binary under test sha256 `ee5ac827397e82d34e36600b47df429a99c29e79cfdd85039120a5a973d1cb28`; the injected refusal reported `recorded: 9d0f5f5b…` (manifest) vs `actual: 1181f0db…` (downloaded bytes).

Adversarial classes and mutants: `adversarial.md`, `mutants.log`. Four mutants of `install.sh` (dropping the digest comparison, the `..` check or the version check, or forcing `runtime_state` to `stopped`) each turn their targeted case red (probe exit 1).

Gates: `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` → exit 0 (`check.sh: all steps passed`; nextest 1620 tests run: 1620 passed, 2 skipped; hygiene, source-check and evidence-audit clean). Windows pre-check (`RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked`, `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings`) → exit 0 (both).

## Seams

Files outside the write scope (`install.sh`, `probes/install/`, `conformance/scenarios/release/install/`, `docs/operator-guide.md`):

- `conformance/scenarios/release/install.json`: the runner maps `--scenario release/install` to `release/install.json` (`conformance/src/ids.rs:41`), so the scenario is a file beside the write-scope directory name, not inside it.
- `conformance/src/scenario/release.rs`, `conformance/src/engine/release.rs`: an optional `"tool": "install-probe"` on the existing `release` step (default `release-stage`, so `release/artifacts.json` is unchanged) runs `ovis_terminal_probes::install::execute` in-process, the same way todo 67's checks run.
- `conformance/src/runner_tests/suite.rs`: `release/install.json` added to `EXECUTABLE` in sorted position (it was in neither `EXECUTABLE` nor `declared.json`).
- `conformance/fixtures/ovis/manifest.json`, `conformance/fixtures/ovis/predicates.json`: `ovis-replay refresh` re-pinned them for the new scenario (12 refreshed).
- `probes/Cargo.toml`, `probes/src/lib.rs`, `Cargo.lock`: the `install-probe` bin, `pub mod install`, and the `release-stage` dependency (its machete entry too); the lock change is one dependency edge.
- `docs/evidence/task-68/`: logs, evidence and this body.

## Risks

- **Running-runtime delegation is a typed handoff, not an automatic `update.stage` call.** Todo 47's `update.stage` reads `ovis-terminal-<version>-<os>-<arch>.tar` with a `build.json` that declares `protocol_generations` and `holder_generations`. Todo 67's artifacts carry no `build.json`, and the holder generation is not observable from the CLI, so an installer that wrote one would be asserting compatibility it cannot check. The installer therefore refuses to replace a live runtime's binary (exit 5, `delegate: "update.stage"`, verified release kept) and the operator guide gives the `update.stage`/`verify`/`activate`/`rollback` path. Follow-up: ship `build.json` in the release archive (todo 67 scope), after which the installer can call `update.stage` itself.
- The `unwritable` case relies on POSIX permissions and would not fail as expected when run as root; GitHub runners are not root.
- `origin/integration` moved to f71a8db (P4) after this branch's base. `suite.rs` and `conformance/fixtures/ovis/manifest.json` touch different lines (no textual conflict), but the fixture pins should be refreshed once (`ovis-replay refresh`) after the union merge, because both sides re-pinned different sources.
- Releases are not signed (`signing.performed: false` from todo 67); integrity rests on the manifest digest fetched over https.

DEFERRED VERIFICATION: none for this scenario; it executes in `check.sh` through the conformance runner suite. The macOS stress job is dispatched separately (run URL in the DoneClaim).
