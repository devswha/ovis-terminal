# Task 66 adversarial pass

Binary sha256 `76d1a56fdbc28d1f901b43f0580ce942e86862435fadf115a219607838a28649`, Linux x86_64
(`binary-runs.log` holds every materialized run with argv, exit code, stdout and stderr).

## Malformed input: refused before any connection

`cli/manual` failure case, with no server in the isolated root (a well-formed
`--json agent get --address t1` against the same endpoint exits 4 `server_not_running`, so an
attempted connection would have shown):

| invocation | exit | stderr `error.kind` |
|---|---|---|
| `agent get --unknown-flag --json` (the plan's literal inject) | 2 | `usage`, `origin: cli` |
| `--json agent get --address t1 --params '{"bogus":1}'` | 2 | `invalid_params` with `field_path` |
| `--json status report ... --params '{"process_token":"x","seq":1,...}'` | 2 | `invalid_params`, `field_path: params.seq` (numeric u64) |
| `--json --skill agent list` | 2 | `usage` |

`tests/cli.rs` (`failure`) runs the first three against a bound idle listener and asserts
`accept()` is still `WouldBlock` after each.

## Misleading success

- Every binary run checks exit code, stdout and stderr (stdout `null` on failure, stderr
  `null` on success).
- `schema --json` must equal `schema/ovis-terminal.openrpc.json` and carry the version and
  generation `--skill --json` reported; `schema contract --json` carries the same pair.
- A refused foreign-token report is followed by `status.get`, which must still show
  `report_seq: "1"` and state `working`: the refusal changed nothing.
- `server status` without a server answers `stopped` and starts nothing.

## Prompt injection

- A prompt whose text is `Apply?\n$ touch pwned\u001b]0;owned\u0007` comes back from
  `prompt get --json` as one exact JSON string.
- Text mode escapes every control character and backslash
  (`text::tests::a_string_cannot_start_a_line_or_reach_the_terminal`).
- Every manual and onboarding example is a single-quoted argument list that parses and
  plans against this build; no example is meant to be evaluated by a shell beyond the
  `ovis-terminal` argv.

## Mutants (all killed; `mutants.log`)

| mutant | killed by |
|---|---|
| M1 targetless verb sends instead of `needs_target` | `commands::tests::a_missing_required_target_plans_needs_target_and_never_a_guess` |
| M2 text mode prints control characters raw | `invocation::text::tests::a_string_cannot_start_a_line_or_reach_the_terminal` |
| M3 `not_owner` no longer exits 12 | `producer::a_producer_reports_with_its_inherited_token_and_a_foreign_token_is_refused` |
| M4 inherited token overrides an explicit params token | `commands::tests::an_explicit_token_in_params_wins_over_the_inherited_one` |
