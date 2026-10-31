## Before

RT-HC-01, GAP-072, GAP-084, GAP-148: `/tmp/ulw-ovis-yCJzPH/evidence/smoke/hc-s06-wait-output.log:6`, durable copy `docs/evidence/baseline/smoke/hc-s06-wait-output.log:6`:

```text
--- exit=0 elapsed_ms=5213
```

`hc send` took 5213 ms to return and reported no terminal modes, so a client can't tell whether multiline text will be framed. herdr preflights only canonical-mode line length (GAP-084), sends mouse reports and keys down different paths (GAP-072), and `extract_one_event` drops unknown sequences (GAP-148, CAT-2586).

## After

`crates/ovis-terminal-core/src/input/encoding/` adds `encode(&ModeSnapshot, InputRequest) -> Result<Encoded, EncodingError>`. It is a pure encoder: it turns one `input.*` request into the exact bytes of one queue entry. Each decision is made against one frozen `ModeSnapshot`, which holds the vt `TerminalModes` plus the `ModeRevision` they were read at. The rules (also stated in the module doc):

- Literal text is data. A line break (`\n`, `\r`) or a tab keeps its literal meaning only inside a bracketed paste. Without bracketed paste such text is refused with `paste_refused` (`NeedsBracketedPaste{revision}`), whatever the submission is, `none` included. Any other control character (C0, DEL, C1) is `invalid_encoding`, and so is an embedded paste marker (`ESC[200~`/`ESC[201~`, and the 8-bit forms `U+009B 200~`/`201~`). Both are checked before any byte exists.
- `input.send`: text that has no line breaks or tabs is written as typed. Text with them, under bracketed paste, is framed `ESC[200~ … ESC[201~`. The submission (`none`/`enter`/`keys[]`) is encoded separately and appended after the frame in the same entry.
- `input.paste`: `auto` frames when the program enabled bracketed paste, `require` refuses without it, and `never` never frames, so it refuses line breaks.
- `input.keys`: names follow the negotiated DECCKM cursor mode and kitty disambiguation (flag 1). Kitty "report all keys" (flag 8) and win32-input mode return `invalid_encoding` (`KeyboardProtocol`) rather than guessed bytes. An unknown name is `unknown_key`, and a chord the modes can't carry is `invalid_encoding`. A key list is all or nothing.
- Mouse reports must match the negotiated tracking (not `off`) and encoding (X10, UTF-8 1005, SGR 1006) as whole reports. They are then passed through byte-identical.
- Raw bytes need a `RawGrant`. Only `RawGrant::decide` creates one, by calling the landed `Authority::authorize` with `Action::RawInput`. `RawGrant::recheck` calls `Authority::recheck` at the write boundary. Raw bytes are passed through byte-identical, unknown sequences included; nothing is swallowed (EXCL-017).
- `EncodingEvidence {kind, mode_revision, bracketed, literal_bytes, submit_bytes}` travels with the bytes. All counters are decimal strings. `Encoded::receipt_revision()` maps the mode revision to the protocol `InputReceipt.mode_revision`. `Encoded::ensure_current(current)` returns `mode_mismatch` once the terminal's mode revision has moved, which fences stale bytes.

No brand inference: the module reads only the program's negotiated modes. Holder wire generation stays 2.

DEFERRED VERIFICATION (runner lands in Todo 21)

Executed on Linux x86_64, rustc 1.99.1 (58cf8f9c3 2026-10-13), `ATTEMPT_DIR=/home/devswha/workspace/ovis-terminal-wt/lane-t32/.omo/evidence/ovis-terminal-v1/task-32`:

```text
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-32 --isolated --case happy --evidence "$ATTEMPT_DIR/task-32-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
CONFORMANCE_EXIT=2
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-32 --isolated --case failure --evidence "$ATTEMPT_DIR/task-32-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
CONFORMANCE_EXIT=2
```

No CLI or socket request was dispatched, and no runner evidence JSON was produced (`conformance.log`). `conformance/scenarios/part-a/task-32.json` records the happy steps (multiline `input.send` with Enter under bracketed paste, expecting `written_hex` `1b5b3230307e66697273740a7365636f6e643b20726d202d7266207e1b5b3230317e0d`; an SGR mouse report and an unknown raw sequence through `input.raw`, each expected byte-identical) and the failure steps (`paste_refused`, two `invalid_encoding`, `unknown_key` and `raw_not_authorized`, each with `written_bytes` `"0"`). The `input.*` RPC routes that call this encoder land with todo 25, so no `ovis-terminal` CLI/socket transcript exists yet.

The component acceptance cases ran against the real vt engine (alacritty-backed `Vt`, modes set by feeding the program's own escape sequences) and the real authority:

```text
$ cargo test -p ovis-terminal-core --test encoding happy -- --exact --nocapture
running 1 test
send evidence: {"kind":"text","mode_revision":"3","bracketed":true,"literal_bytes":"39","submit_bytes":"1"}
send bytes: "\u{1b}[200~first\nsecond; rm -rf ~\tdone\u{1b}[201~\r"
test happy ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 20 filtered out; finished in 0.00s
QA_HAPPY_EXIT=0

$ cargo test -p ovis-terminal-core --test encoding failure -- --exact --nocapture
running 1 test
multiline in plain mode: paste_refused (line breaks and tabs stay literal only under bracketed paste, absent at mode revision 0)
"ok\u{1b}[201~\rrm -rf ~": invalid_encoding (byte 2 opens a bracketed-paste marker inside literal text)
"ok\u{9b}201~x": invalid_encoding (byte 2 opens a bracketed-paste marker inside literal text)
"bell\u{7}": invalid_encoding (byte 4 is the control character U+0007)
"esc\u{1b}[A": invalid_encoding (byte 3 is the control character U+001B)
"nul\0": invalid_encoding (byte 3 is the control character U+0000)
test failure ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 20 filtered out; finished in 0.00s
QA_FAILURE_EXIT=0
```

## QA

- `happy` (tests/encoding.rs). The emulator is fed `ESC[?2004h ESC[?1000h ESC[?1006h`. The test sends `first\nsecond; rm -rf ~\tdone` with `submit: enter`, a two-report SGR mouse sequence, and raw `ESC[999;unknown BEL 0xff NUL` under a real `RawGrant`. It asserts:
  - the send's last byte is `\r` and the rest is exactly `ESC[200~` + the text bytes + `ESC[201~`;
  - the evidence has kind `text`, `bracketed: true`, `mode_revision` equal to the emulator's, `literal_bytes` equal to the framed length and `submit_bytes` `1`;
  - the mouse bytes and the raw bytes are byte-identical to the input;
  - `ensure_current` passes at the same revision and returns `mode_mismatch` after the program sends `ESC[?2004l`.
- `failure` (tests/encoding.rs). The test asserts:
  - `first\nsecond` with `submit: none` in plain mode is `paste_refused` (`NeedsBracketedPaste` naming the snapshot revision);
  - with bracketed paste enabled, a paste containing `ESC[201~`, the C1 `U+009B 201~`, BEL, `ESC[A` or NUL is `invalid_encoding`;
  - `a ESC[201~ b` via `input.send` is `PasteMarker{at: 1}`;
  - a session holding interact but not raw input gets `raw_not_authorized` from `RawGrant::decide`.
- QA modules under `src/input/encoding/qa/` hold 19 more tests:
  - Literal (3 proptests, 2 unit tests):
    - bracketed text round-trips with the submission outside the frame;
    - any control character at any position is `invalid_encoding` for every submission and paste policy, in both modes;
    - a line break or tab without bracketed paste is `paste_refused` for `none`, `enter` and `keys`;
    - the paste policy matrix;
    - an empty payload is `invalid_params`, while a bare Enter is a submission.
  - Keys (6 unit tests, 1 proptest):
    - the normal-mode table, application cursor mode and kitty flag-1 CSI u;
    - kitty flag 8 and win32 are refused;
    - unknown names and unencodable chords give typed errors;
    - a submission chord is appended in the same entry;
    - a key list is the concatenation of its keys, and one unknown name refuses the whole list.
  - Mouse (2 unit tests, 1 proptest): the X10, UTF-8 and SGR encodings pass unchanged; a mismatched, trailing, empty-field or truncated report is refused; an SGR sequence round-trips, and cutting inside its last report is refused.
  - Evidence (3 unit tests, 1 proptest):
    - `u64::MAX` mode revision round trip;
    - each refusal kind deserializes into the protocol `DomainErrorKind` and back;
    - raw grant recheck;
    - arbitrary authorized raw bytes are never rewritten.

Gate on the merged tree (`origin/integration` be44e01 merged in): `bash scripts/check.sh` exit 0 (`check.log`: 505 tests run, 505 passed, 1 skipped; hygiene 0 violations in 507 files; 46 JSON files checked). Acceptance commands: `cargo nextest run -p ovis-terminal-core` (`core-nextest.log`: 151 tests run, 151 passed, `NEXTEST_EXIT=0`) and `cargo clippy -p ovis-terminal-core --all-targets -- -D warnings` (`core-clippy.log`: `CLIPPY_EXIT=0`). The TDD red run before the module existed is in `tdd-red.log` (unresolved `input::encoding::*` imports, exit 101).

macOS: the `probe-stress-macos` workflow gains an `encoding` input that repeats `cargo nextest run -p ovis-terminal-core --test encoding` 100 times on macos-latest. Run: https://github.com/devswha/ovis-terminal/actions/runs/41199291391 (dispatched with `gh workflow run probe-stress-macos.yml --ref lane/t32-literal-keys-paste -f probe=encoding`)

## Adversarial

- Malformed input, paste terminator and control bytes: `failure` and the control-character proptest. A literal `ESC[201~` followed by `\r` would otherwise close the paste early and execute the rest of the line; it is refused before any byte exists.
- Prompt injection, literal payload stays data: `happy` sends `; rm -rf ~` inside multiline text. The payload is recovered byte-equal from inside the frame, and the only Enter is the explicit submission, outside the frame. In plain mode the same text is refused, whatever the submission.
- Stale state, mode revision: `happy` asserts `mode_mismatch` from `ensure_current` once the program turns bracketed paste off after encoding. The evidence test asserts the receipt revision round-trips at `u64::MAX`.
- Misleading success, zero bytes on rejection: every refusal is an `Err(EncodingError)`, so no `Encoded` (and no bytes) exists. The key-list proptest asserts that one unknown name refuses the whole list rather than writing a prefix.
- Mutation probes (`mutations.log`). Each guard was removed in turn, and the named tests failed:
  - paste-marker check: `failure`;
  - line breaks allowed without bracketed paste: `failure`, the policy matrix, the line-break proptest and the kind test;
  - control characters allowed: `failure`, the control proptest and the kind test;
  - mode-revision fence always passing: `happy` and the kind test;
  - mouse check skipped: both mouse tests and the kind test;
  - kitty disambiguation ignored: the kitty test;
  - unknown keys skipped: four keys/evidence tests.

  Sources were restored after each probe. Proptest regression files written by mutants were discarded.

## Contract checklist

1. Counters on the wire are decimal strings. `EncodingEvidence.mode_revision` uses the vt `ModeRevision` decimal serde, and `literal_bytes`/`submit_bytes` use the protocol `ByteCount`. `the_maximal_mode_revision_round_trips_as_a_decimal_string` asserts that `"18446744073709551615"` round-trips, that `literal_bytes` serializes as `"13"`, and that a numeric `mode_revision` fails to decode (`deny_unknown_fields` also holds).
2. `EncodingError` is one `thiserror` enum with `kind()` and `hint()`. `RawNotAuthorized` carries `#[source] AuthzError`. `Display` never repeats a kind token, and the kinds are asserted to be published `DomainErrorKind` tokens (`invalid_params` for empty/unsupported options is the envelope kind).
3. Distinct newtypes: vt `ModeRevision`, protocol `ByteCount`/`Revision`/`TimestampMs`, registry `RegistryIdentities` targets, and a new `PayloadIndex` for byte positions inside a payload. `Option` is used only inside parsers.
4. Tests assert encoded bytes, round trips and typed kinds against the real emulator and the real authority. Proptest covers literal framing, control injection, key lists, SGR reports, raw bytes and the evidence wire form.
5. No `#[allow]` or `#[expect]`, no placeholders. Every file is at most 250 pure LOC and every function has at most 3 parameters (hygiene 0 violations).
6. The conformance attempt is recorded verbatim above as deferred to Todo 21. Holder wire generation stays 2; contract checklist items are unchanged.

## Seams

- New public API in `ovis_terminal_core::input::encoding`: `encode`, `InputRequest`, `ModeSnapshot`, `Encoded`, `EncodingEvidence`, `InputKind`, `EncodingError`, `PayloadIndex`, `RawGrant`, `RawRequest`. No landed crate's public API changed.
- The protocol `InputSendParams`/`InputPasteParams`/`InputKeysParams` have no public constructor outside the protocol crate, and `input.raw` carries base64 (core has no base64 dependency). `InputRequest` therefore borrows the decoded fields, and it reuses the protocol `Submit` and `BracketedPolicy`. Todo 25 maps params to `InputRequest`, `Encoded::receipt_revision()` to `InputReceipt.mode_revision`, and `evidence().literal_bytes + submit_bytes` to `admitted_bytes`.
- Mouse reports have no dedicated method in the published schema. `InputRequest::Mouse` is the checked path that todo 25 can use for interact-lease mouse input; `input.raw` stays the grant-gated byte path.
- `.github/workflows/probe-stress-macos.yml`: new `encoding` option and job appended at the end, union-merged after integration's `cli` job.

## Risks

- The encoder is pure. Admission in the terminal actor (the lease check via `TerminalLeases::authorize_write`, `ensure_current` and `RawGrant::recheck` in the same step as the write, and queue sequencing and receipts) belongs to todo 25, so until then no real-binary evidence exists for this todo.
- Kitty flags beyond disambiguation (flag 8) and win32-input mode are refused for `input.keys` and for `submit` keys rather than encoded. A program in those modes needs `input.raw` until an encoder lands. Under `modifyOtherKeys` the legacy encodings are sent, and xterm-compatible programs accept them.
- Tabs count as structure: without bracketed paste, `a\tb` is refused because a typed tab would trigger completion. Clients send the Tab key explicitly with `input.keys`.
