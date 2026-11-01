# Task 66 adversarial pass

Binary sha256 `c7919923c742395e70329438dc7a7472240cfd143a138368a07ff76dae8997bd`, Linux x86_64
(`binary-runs.log` holds every materialized run with argv, exit code, stdout and stderr).

## Malformed input: refused before any connection

`cli/manual` failure case, with no server in the isolated root (a well-formed
`--json agent get --address t1` against the same endpoint exits 4 `server_not_running`, so an
attempted connection would have shown):

| invocation | exit | stderr `error.kind` |
|---|---|---|
| `agent get --unknown-flag --json` (the plan's literal inject) | 2 | `usage`, `origin: cli` |
| `--json agent get --address t1 --params '{"bogus":1}'` | 2 | `invalid_params` with `field_path` |
| `--json agent get --params '{"bogus":1}'` (no target; round 3) | 2 | `invalid_params` with `field_path`: checked before the `needs_target` path, no discovery |
| `--json status report ... --params '{"process_token":"x","seq":1,...}'` | 2 | `invalid_params`, `field_path: params.seq` (numeric u64) |
| `--json --skill agent list` | 2 | `usage` |

`tests/cli.rs` (`failure`) runs these, plus the targetless `--json agent get --params '{"bogus":1}'`
and `--json turn wait` (missing `timeout_ms`), against a bound idle listener and asserts exit 2
and that `accept()` is still `WouldBlock` after each.

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
| M5 targetless verb answers `needs_target` without checking the supplied fields (round 3) | `invocation::manual_tests::a_targetless_verb_needs_a_target_only_when_every_supplied_field_is_valid` (`Ok(true)` vs `Err("invalid_params")`), `tests/cli.rs` `failure` (exit 13 `deadline_exceeded` after 15 s on the idle listener, not 2), `cli/manual` failure case (exit 7, not 2) |
