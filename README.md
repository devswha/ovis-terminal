# ovis-terminal

An agent-native terminal runtime written in Rust. It gives coding agents and the
Ovis phone/browser client a typed, persistent,
machine-readable contract for terminals: explicit targets, typed errors,
durable operation receipts, cursor-based waits, declared agent status and
explicit input/geometry leases. It draws no UI of its own.

**Status: scaffold.** This repository currently holds the research, the design
ledgers, the work plan and a compiling workspace whose product crates carry no
runtime behavior yet. The only working command is `ovis-terminal --version`.
Feature work lands one plan todo per pull request; see
[`.omo/plans/ovis-terminal-v1.md`](.omo/plans/ovis-terminal-v1.md).

## Layout

| Path | Contents |
|---|---|
| `crates/ovis-terminal-protocol` | wire framing, JSON-RPC envelopes, boundary errors, OpenRPC schema |
| `crates/ovis-terminal-store` | atomic SQLite state, receipts and quarantine |
| `crates/ovis-terminal-vt` | pure-Rust VT screen and history state |
| `crates/ovis-terminal-holder` | PTY holder protocol and process ownership |
| `crates/ovis-terminal-authz` | principals, roles and scoped authority |
| `crates/ovis-terminal-core` | control core: identities, events, leases, input, layout |
| `crates/ovis-terminal-agent`, `-agent-adapters` | declared agents, prompts, conversations |
| `crates/ovis-terminal-services`, `-remote` | worktrees, plugins, updates, documents; remote machines |
| `crates/ovis-terminal-server` | endpoints, dispatch, streams, WebSocket |
| `crates/ovis-terminal-cli` | the `ovis-terminal` binary |
| `conformance/` | real-socket conformance runner (`conformance` binary) and scenarios |
| `bench/`, `probes/` | benchmark harnesses and native platform probes |
| `packages/client-ts/` | OpenRPC-generated TypeScript types and Bun client |
| `scripts/check.sh`, `scripts/hygiene/` | local quality gate and the workspace hygiene checker |
| `docs/research`, `docs/design`, `docs/evidence` | herdr research, ideal-state and gap ledgers, baseline evidence |

## Build and check

```sh
cargo build --workspace --locked
cargo run -p ovis-terminal-cli -- --version   # ovis-terminal 0.1.0
bash scripts/check.sh                          # fmt, clippy -D warnings, nextest, doctests, hygiene
```

`scripts/check.sh` needs the pinned toolchain from `rust-toolchain.toml`, a
nightly `rustfmt` (import grouping) and `cargo-nextest`. CI runs the same gates
on Linux, macOS and Windows, plus rustdoc, MSRV, `cargo deny` and unused-dependency
checks.

## Platforms

Linux and macOS are first-class. Windows is contract-complete (named pipes,
ConPTY) and proven by CI smoke tests, not by full functional QA.

## License

MIT, see [`LICENSE`](LICENSE). Designs from herdr and herdr-core (Apache-2.0)
were studied; attribution and the terms for any mirrored material are in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
