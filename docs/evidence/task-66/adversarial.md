# Task 66 adversarial pass

Binary sha256 `c955ba1920cd2418c346229a9fb358fdc6f6d92e615845fdeaca59fa144c9296`, Linux x86_64
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

## Gate round 1: advertised surface equals served surface

- Every `sh` example line in the manual and the onboarding guide runs through `/bin/sh` against `serve`, using the real spawn token and the binary on `PATH`. Every `// params of` block is sent with `call`. None may answer `method_not_found`, `unknown_method`, `usage`, `invalid_params` or unparsable output.
- `hello.methods` contains the method of every typed verb `schema cli` lists.
- Every `<group> <verb>` the two documents name is a command of the build.
- Removed unserved verbs: `agent list`, `agent note`, `agent release`, `agent wait-children`. They are named as not served in the manual and as a gap in the PR.

| mutant | killed by |
|---|---|
| M6 re-add an unserved `agent list` verb | `manual_examples::hello_advertises_the_method_of_every_typed_verb` |
| M7 document `agent list` again in the onboarding guide | `invocation::manual_tests::every_typed_verb_the_docs_name_is_a_command_of_this_build`, `manual_examples::every_documented_command_and_params_example_is_served_by_this_build` (`usage`) |
| M8 `schema cli` names no method (vacuous hello check) | `invocation::manual_tests::every_typed_verb_command_names_the_method_it_sends` (and the hello test) |

## Gate round 2: documented mutations must succeed

- Misleading-success class: the executed-examples test no longer uses a deny-list. Every example must exit 0 unless the docs declare `# exits <code> <kind>`, and that declared outcome is asserted exactly. `op_expired`, `agent_conflict`, `invalid_session` and `conversation_unavailable` were all found this way.
- The owning-token flow is asserted end to end: register, label read back, report, clear, then the cleared status read back.

| mutant | killed by |
|---|---|
| M9 stale `01900000` op id back in the revocation example | `manual_examples::the_onboarding_flow_registers_reports_and_clears_with_the_owning_token` (`op_expired`, exit 1, expected 0) |
