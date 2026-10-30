Plan todo 8: atomic SQLite storage and quarantine primitives in `crates/ovis-terminal-store`.
Closes GAP-125. Delivers the store slice of IDEAL-097, IDEAL-098 and IDEAL-101.

## What changed

- `Store::open` probes an existing `state.db` before writing anything. The probe is
  non-mutating: it reads the header magic, then opens a read-only connection with
  `SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE` to read `user_version` and run `PRAGMA quick_check`, so
  classifying a database never folds its WAL into the main file or deletes the WAL. Only a
  database classified as current gets a writable connection, which switches to WAL with
  `synchronous=FULL` and applies versioned migrations (`SCHEMA_VERSION = 1`).
- A dedicated writer thread sits behind a bounded `sync_channel` (`writer_queue`, default 64).
  It commits each `Mutation` in one `BEGIN IMMEDIATE` transaction: compare-and-swap resource
  writes, their events and the operation receipt. The `Ticket` resolves only after `COMMIT`
  returns. A full queue answers `backpressure`; an oversized payload answers `too_large`
  before anything is queued.
- `Snapshot` is a read transaction pinned to one revision (WAL snapshot). It offers
  `resource`, `receipt`, `events_after` and `recovery_evidence`.
- Retention: when events or receipts exceed the advertised limits, the oldest are evicted
  and the stored floor advances. Reads report `EventPage::Expired { floor }` /
  `ReceiptLookup::NotRetained { floor }` instead of an empty success. Deleting a resource
  leaves a versioned `Tombstone`.
- Quarantine:
  - A damaged database writes durable `evidence.json` first, then moves `state.db`,
    `-wal` and `-shm` into `quarantine/<id>/`. The open returns `store_corrupted`.
  - A newer schema is copied there byte for byte (`state.db`, `state.db-wal` and
    `state.db-shm` when present) and left in place. The open returns `store_unsupported`.
  - Each incident is identified by an `IncidentId` newtype, which is also the key of the
    rebuilt recovery table. `detected_at_unix_ms` is a `UnixMillis` serialized as a canonical
    decimal string; deserialization rejects JSON numbers and non-canonical text such as
    `"007"` or `"+7"`.
  - Later opens keep returning the same typed error; they never produce an empty store.
  - Reading the evidence never clears it.
  - `Store::rebuild_after_quarantine` is the explicit path to an empty store, and it carries
    the evidence into the new database.
  - Every incident records `process_survival: Unknown` and
    `relaunch: RequiresIndependentEvidence`; the store never decides that programs survived.
- `CommitBarrier::{Applied, Committed}` hooks are the named fault-injection barriers for
  crash tests.

## Before

RT-HD-05 / GAP-125: `docs/evidence/baseline/smoke/attempt2-herdr-h03-workspace.log:5` shows
`workspaces before: 1`: herdr silently restored state from a previous server
(source `<run-root>/evidence/smoke/attempt2-herdr-h03-workspace.log:5`). Under GAP-125, herdr
also replaces corrupt or newer persisted state with healthy-looking defaults: corrupt
`docs.json` becomes `docs.json.bad` and the core starts empty (CAT-3481,
`herdr-core/tests/docs.rs:48`).

## After

The plan's `After` command is not runnable yet. The `conformance` binary is still the
todo-1 scaffold (it registers no scenarios and exits 2), and `ovis-terminal` only answers
`--version`. The real-socket run of `part-a/task-8` therefore waits for todo 21 (runner)
and todo 31 (binary assembly). `conformance/scenarios/part-a/task-8.json` records the
component cases those todos will rerun. Here is what did run, on
`Linux 7.0.0-38-generic x86_64`, `rustc 1.99.1`, bundled SQLite `3.54.1`
(libsqlite3-sys 0.39.1, rusqlite 0.41.1):

| Command | Exit | Result |
|---|---|---|
| `bash scripts/check.sh` | 0 | fmt, clippy `-D warnings`, nextest 33 passed / 1 skipped (the ignored crash-child fixture), doctests, hygiene 0 violations in 69 files, manifests, JSON |
| `cargo nextest run -p ovis-terminal-store` | 0 | 22 passed, 1 skipped |
| `cargo clippy -p ovis-terminal-store --all-targets -- -D warnings` | 0 | clean |
| `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features --workspace --locked` | 0 | clean |

Windows CI history:

1. Run 41109382393, `test (windows-latest)` red. Clippy rejected the non-unix no-op
   `sync_dir` stub (`missing_const_for_fn`, `unnecessary_wraps`, quarantine.rs:159).
   Commit `35aab6f` defines `sync_dir` only on Unix, calls it under `#[cfg(unix)]` and adds
   no `#[allow]`. Windows clippy has passed since then.
2. Run 41109843350 (job 124249113140), `test (windows-latest)` red in nextest:
   `ovis-terminal-store::recovery rebuild_leaves_a_newer_schema_in_place` panicked at
   `crates\ovis-terminal-store\tests\recovery.rs:76:5` with `left: "persistence_failed"`,
   `right: "store_unsupported"` (14 passed, 1 failed, 1 skipped; nextest stopped at the
   first failure). That log carried only the kind, not the OS error text.
   - Classification itself was correct and happens before any write: `probe` reads the
     header and `user_version`, and drops its connection before `preserve` runs.
   - The failure came from the step that preserves the newer-schema copy. It reopened the
     copy with `File::open` (read-only) and called `sync_all`. On Windows that is
     `FlushFileBuffers`, which requires write access, so it fails with access denied. This
     cause is inferred from the code path and the Windows API contract; the OS message was
     not in the log.
   - Commit `903a644` reopens the copy with `OpenOptions::new().write(true)` before
     `sync_all`. It also makes the kind assertions print the full `{error:?}`, so any
     future failure shows the OS error.
3. Run 41110394392 on `903a644`: every job passed, including `test (windows-latest)`, `test (macos-latest)`,
   `test (ubuntu-latest)`, `scripts/check.sh (ubuntu)`, `msrv (1.89)`, `rustdoc -D warnings`,
   `cargo deny` and `unused dependencies (cargo-machete)`.

Gate review round 1 (5 blockers), fixed in `8d513d2`:

- **B1, unsupported-schema preservation**: the probe used to open an ordinary read-write
  connection. Closing it checkpointed the WAL into `state.db` and deleted the WAL before
  quarantine, so the "preserved original" no longer matched the bytes on disk. The probe is
  now read-only with checkpoint-on-close disabled, and the newer-schema case preserves
  `state.db`, `state.db-wal` and `state.db-shm`. The new regression test is
  `a_newer_schema_with_a_live_wal_is_classified_without_touching_it`. It builds a WAL-mode
  database with `SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE` and `user_version = 999`, then asserts
  that the original DB and WAL bytes are unchanged and that both quarantine copies are
  byte-identical. Against the old read-write probe it fails (`mutation-probe-b1.log`).
- **B2, canonical decimal strings**: `detected_at_unix_ms` is a `UnixMillis` serialized as
  a decimal string. Unit tests round-trip `u64::MAX` as `"18446744073709551615"` and reject
  `"007"`, `"+7"` and the JSON number `7`. The integration test
  `durable_evidence_stores_its_timestamp_as_a_decimal_string` parses the real
  `evidence.json` file.
- **B3, incident identity**: `RecoveryEvidence.id` is an `IncidentId` newtype (serde
  transparent) and keys the recovery table.
- **B4, lint scope**: the crate-wide `#[expect(clippy::redundant_pub_crate)]` is removed.
  Opening machinery moved to `store/{apply,quarantine,schema,writer}.rs`, so its
  `pub(super)` items are no longer crate-wide and need no exception. The four shared
  test-fixture items each carry their own scoped `#[expect(..., reason)]`.
- **B5, pinned native C toolchains** (`.github/workflows/ci.yml`, under a scope extension
  the orchestrator granted). Runner images are pinned and each job asserts its exact compiler
  version, failing on drift. Job names are unchanged.
  - Linux (`ubuntu-24.04`; test, check.sh, rustdoc and MSRV jobs): gcc 14.2.0, with
    `CC=gcc-14`, `AR=gcc-ar-14` and linker `gcc-14`, guarded by
    `gcc-14 -dumpfullversion == 14.2.0`.
  - macOS (`macos-15`): `xcode-select -s /Applications/Xcode_16.4.app`, asserting
    `xcodebuild -version` reports `Xcode 16.4`.
  - Windows (`windows-2025`): `ilammy/msvc-dev-cmd@v1` with `toolset: 14.44`, asserting
    `cl.exe` reports `Version 19.44.`. The linker is that toolset's `link.exe`, by absolute path.
- CI history on the fix commits:
  - Run 41112639123 on `8d513d2` was red on macOS and Windows, both in the new pin steps:
    - macOS: `xcodebuild -version | grep -qx`. The early pipe close killed `xcodebuild`
      with SIGPIPE (`NSFileHandleOperationException ... Broken pipe`, exit 134).
      `151dd9f` captures the whole output and compares its first line in bash, with no pipe.
      The same applies to the Windows `cl.exe` banner check.
    - Windows: `msvc-dev-cmd` sets `VCINSTALLDIR`, so rustc takes `link.exe` from PATH. In
      the `shell: bash` binary-version step, Git Bash's coreutils `/usr/bin/link` came first
      and failed with `missing operand after '\377\376'`. Nextest had already passed.
      `ca1e4e4` sets `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER` to the pinned toolset's
      `${VCToolsInstallDir}bin\HostX64\x64\link.exe`, so the Windows linker is pinned too.
  - Run 41113068677 on `ca1e4e4`: all 8 jobs passed. The logs show gcc 14.2.0, Xcode 16.4
    (Apple clang 17.0.0, clang-1700.0.13.5) and MSVC `VCToolsVersion 14.44.35207`.

Logs: `docs/evidence/task-8/check.log`, `qa-happy.log`, `qa-failure.log`,
`mutation-probes.log`, `mutation-probe-b1.log`, `tdd-red.log` (the failing run before implementation),
`platform.txt`, `task-8-ovis-terminal-v1.json`.

## QA

Happy: `cargo test -p ovis-terminal-store --test transactions happy -- --exact --nocapture`,
exit 0. A child process commits one mutation and `abort()`s at `CommitBarrier::Committed`;
the parent then reopens the store:

```
reopened after crash: revision=1 resource=Live { version: Version(1), value: Payload([118, 49]), revision: Revision(1) } receipt=Found { revision: Revision(1), result: Payload([111, 112, 45, 49]) }
cas: committed=CommitReceipt { revision: Revision(2), first_event: Revision(2), versions: [(ResourceKey { kind: "doc", id: "app" }, Version(2))] } conflict.kind=conflict resource=Live { version: Version(2), value: Payload([118, 50]), revision: Revision(2) }
test happy ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out; finished in 0.16s
```

Failure: `cargo test -p ovis-terminal-store --test transactions failure -- --exact --nocapture`,
exit 0. It covers both a corrupt header and a future schema (`user_version = 999`):

```
Header: kind=store_corrupted ... evidence=RecoveryEvidence { id: IncidentId("1793387239179-00"), cause: CorruptHeader, location: "/tmp/ovis-store-failure-3853650-0/quarantine/1793387239179-00", preserved: ["state.db"], detected_at_unix_ms: Some(UnixMillis(1793387239179)), flags: RecoveryFlags { material: MovedToQuarantine, process_survival: Unknown, relaunch: RequiresIndependentEvidence } }
FutureSchema: kind=store_unsupported ... evidence=RecoveryEvidence { id: IncidentId("1793387239185-00"), cause: UnsupportedSchema { found: 999, supported: 1 }, location: "/tmp/ovis-store-failure-3853650-1/quarantine/1793387239185-00", preserved: ["state.db", "state.db-wal", "state.db-shm"], detected_at_unix_ms: Some(UnixMillis(1793387239185)), flags: RecoveryFlags { material: CopiedOriginalKept, process_survival: Unknown, relaunch: RequiresIndependentEvidence } }
test failure ... ok
```

The failure test also asserts:
- the quarantined bytes equal the damaged file;
- a second open is still the same typed error;
- reading the evidence twice returns the same records.

## Adversarial

- **Stale state**: a CAS with a stale version gets `conflict` and leaves no receipt (`happy`).
  A conflicting write inside a multi-write mutation rolls back the whole mutation
  (`a_conflicting_write_rolls_back_the_whole_mutation`). A proptest compares 24 random CAS
  sequences against a reference model (`versions_follow_the_reference_model`).
- **Cancel/resume**: the writer process is killed at two commit barriers.
  - At `Committed`, state, receipt and event all survive (`happy`).
  - At `Applied` (before `COMMIT`), none of them survive and the revision stays 0
    (`crash_before_commit_keeps_nothing`).
- **Misleading success**: the reopened durable store is the oracle in every crash test. A
  corrupt or newer database never opens as an empty store. Evicted events and receipts
  report their floor, not an empty page.
- **Tests can fail**: five temporary code mutations were each killed by the named test, and
  the code was restored and re-run green (`mutation-probes.log`, `mutation-probe-b1.log`). The mutants were:
  receipt not written, CAS ignoring the expected version, the barrier moved after `COMMIT`,
  the header check returning "missing", and the old read-write schema probe.

## Risks

- `Cargo.lock` changes (rusqlite, tokio and their dependencies). It is outside this todo's
  write scope, but adding the declared workspace dependencies requires it.
- The native C toolchains are pinned by runner image plus asserted compiler versions
  (gcc 14.2.0, Xcode 16.4, MSVC 14.44). An image update that drops one of these versions
  fails CI loudly; it does not silently drift. The bundled SQLite version is pinned through
  `Cargo.lock` (libsqlite3-sys 0.39.1 → SQLite 3.54.1).
- `Ticket::wait` blocks and panics inside a Tokio runtime (documented). Async callers use
  `Ticket::outcome().await`. `Snapshot` reads are blocking SQLite calls that the async
  coordinator (todo 10) must run off the executor.
- After receipt eviction, an unknown op id reports `NotRetained { floor }` rather than
  `Absent`: the store cannot tell an expired id from one that never existed.
- The shared test fixture carries four per-item `#[expect(clippy::redundant_pub_crate)]`.
  In a depth-1 test module that nursery lint contradicts `unreachable_pub`.
- On Windows, directory fsync after quarantine renames is not performed (Windows exposes no directory flush; NTFS journals the rename).
  Windows correctness is CI-smoke only.

