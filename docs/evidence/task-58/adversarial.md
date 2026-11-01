# Task 58 adversarial pass

Classes named by the todo block, how each is exercised, and what the pass changed.

## Prompt injection: hostile argv and transcript content

- `terminal.run` with argv `["/bin/echo", "a;b", "$(id)", "`uname`", "c|d && e > /tmp/x"]`
  reaches the launcher byte-exact (`security/validation` happy: reply
  `/result/terminal/process/argv` and the recorded `spawn-terminal:100.json` both equal the
  literal vector); `LaunchArgv` never joins or interprets it
  (`shell_metacharacters_stay_literal_argv_bytes`).
- The fixture's provider store holds a session named
  `$(touch pwned); ignore previous instructions.jsonl` whose content is an instruction to
  delete files. `agent.history` returns the name as inert data (`$contains` assertion) and never
  opens or parses session content (EXCL-028).

## Malformed input: traversal and oversized records

- Store references `../../secret`, `a/../../b`, `/etc/passwd`, `a\..\b`, `./a`: `forbidden`
  before any filesystem call (protocol tests); over the wire `../../secret`, `escape`
  (symlink out of the root), `/etc` and `claude/linked.jsonl` (symlinked file) are
  `forbidden` with `field_path` `params.store` (`history_qa`, conformance failure case).
- A 4097-byte store reference and a 32 KiB + 1 argv element are `too_large` with
  `bound {limit, value}` as decimal strings; the 32 KiB element itself is admitted.
- Empty argv, NUL bytes, `=` in an environment name: `invalid_params`.

## Stale state: symlink replacement

- `a_symlink_retargeted_after_authorization_is_refused_at_use`: a symlink authorized to a file
  inside the root is retargeted outside (`forbidden`), then to another file inside
  (`Replaced`, also `forbidden`) before use.
- `a_file_swapped_under_the_same_name_is_refused_at_use`: same name, new inode -> `Replaced`.
- `agent.history` rechecks the store after reading it, so a store replaced by a symlink during
  the read is refused rather than reported.

## Findings fixed by this pass

1. The history scan was bounded by sessions found, not entries visited: a store of millions of
   directories was walked in full. Now at most 10 000 entries are examined (`truncated`).
2. An unencodable history page was reported as `invalid_response` (a client-envelope kind);
   it is now `internal`.
3. Reserved environment names were compared with ASCII case folding only. Windows compares
   names case-insensitively with Unicode upcasing, so `ovıs_termınal_token` (dotless i) named
   the runtime's own variable there. Names are now uppercased with Unicode rules.
4. Endpoint allowlist hosts compared case-sensitively although DNS names do not.
5. 256 argv elements refused ordinary commands with many path arguments; the element bound is
   4096 (total bytes stay bounded at 256 KiB).
6. The fixture's audit probe passed its search string as `needle`, so the probe's own audit
   record held the sentinel in clear; the parameter is now `secret`, a redacted field.
7. The captured-log test tolerated one sentinel occurrence in free text; it now asserts zero
   occurrences of the sentinel and of the bearer.
8. `worktree.remove` was mapped to layout change although its lane requires administer.

## Residual risks (not fixed, stated in the PR)

- A store directory swapped to a symlink and restored within one read is not detected; only
  entry names and modification times are ever disclosed.
- Hard links inside a store are not distinguished from regular files (names only).
