# ovis-terminal

A headless terminal runtime for coding agents and the Ovis client. It runs
programs in PTYs, keeps them alive across control-server restarts and answers
over a typed, versioned API. It draws nothing.

The work plan is `.omo/plans/ovis-terminal-v1.md`; the binding ledgers are
`docs/design/ideal-state.md` (IDEAL-001..117, EXCL-001..027) and
`docs/design/gap-list.md` (GAP-001..150). Read the todo you own in full before
editing; stay inside its write scope.

## Commands

```sh
bash scripts/check.sh                                     # the gate every PR passes
cargo run -p ovis-terminal-cli -- --version
cargo run -p workspace-hygiene -- check --root .          # 250 pure LOC / 3 parameters
cargo run -p workspace-hygiene -- check --fixture excessive-loc-and-parameters   # exits 1
```

## Rules that the code will not tell you

- No production `unwrap`/`expect`/`panic!`, no numeric `as` casts (`try_from` /
  `From`), no `#[allow]`: silence a lint only with
  `#[expect(lint, reason = "...")]` on the smallest item.
- At most 250 pure lines of code per `.rs` file (tests and generated code
  included) and at most three parameters per function, receiver included.
  Split by responsibility, never into numbered fragments.
- Every Cargo package declares `[lints] workspace = true` and no lint tables of
  its own; `scripts/hygiene/manifests.sh` enforces it.
- One `thiserror` enum per boundary with a stable kind and hint; never turn a
  failure into a default or a success.
- Status comes only from declared OSC 7501 reports or the owning process.
  Never infer agent state from screen text, never add brand detection.
- Reads never inject input, start a daemon or change a viewport.
- No fixed sleeps in tests: register for the event, then trigger it, then await
  with a bound.
- Each PR body has `## Before` (a cited line from `docs/evidence/baseline/` or
  the word `absent`) and `## After` (the real `ovis-terminal` invocation, its
  output, exit status and OS/arch).

## Bridge traps this runtime must not reproduce

Reproduced against herdr 0.9.5 / hc 0.1.14; transcripts are in
`docs/evidence/baseline/smoke/`.

| Trap | Evidence | Contract that replaces it |
|---|---|---|
| One request per connection; the server closes after one answer (RT-HD-01) | `herdr-h02-connection.log` | persistent multiplexed connection (IDEAL-004, EXCL-014) |
| A second `events.subscribe` silently kills the stream (RT-HD-08) | `herdr-h06-events.log` vs `herdr-h06b-single-sub.log` | acknowledged filter changes (IDEAL-018, EXCL-015) |
| A misnamed target field answers for the focused pane (RT-HD-02, RT-HD-03) | `herdr-h05-process-info-trap.log` | unknown fields rejected, explicit targets (IDEAL-008/009, EXCL-005/006) |
| `--takeover` cuts off the current holder; refusals are printed as terminal text (RT-HD-07) | `herdr-h08-attach.log` | explicit authorized takeover, typed attach outcomes (IDEAL-028/031, EXCL-007/016) |
| Screen heuristics overwrite an explicit agent report (RT-HD-09) | `herdr-h06b-single-sub.log` | declared status is authoritative (IDEAL-064+, EXCL-008/009) |
| `hc send` waits 5 s; `wait --output` misses output printed during send (RT-HC-01/02) | `hc-s06-wait-output.log` | admission receipts and cursor waits (IDEAL-051/053, EXCL-024/025) |
| `hc server status` starts a daemon (RT-HC-04) | `hc-s18-stop-cleanup.log` | side-effect-free status (IDEAL-014, EXCL-027) |

## Local toolchain

Hosts without a system Rust toolchain use an isolated one; export these before
any `cargo` command (CI installs its own toolchains):

```sh
export RUSTUP_HOME=/tmp/ulw-ovis-yCJzPH/toolchain/rustup
export CARGO_HOME=/tmp/ulw-ovis-yCJzPH/toolchain/cargo
export PATH=/tmp/ulw-ovis-yCJzPH/toolchain/cargo/bin:$PATH
```
