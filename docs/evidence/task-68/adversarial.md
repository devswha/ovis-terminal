# Todo 68 adversarial pass

Host: Linux x86_64. Evidence: `acceptance-install-probe.log` (full probe report, every literal command with exit, stdout, stderr), `mutants.log`.

| Class | Case | Observed |
|---|---|---|
| dirty worktree: installer never touches the checkout | every case | commands run with cwd = the checkout; `outside` inventory includes every checkout top-level entry (file digests) plus private HOME (seeded .profile/.bashrc/.zshrc), XDG tree and attempt dir; unchanged in happy, every refusal and foreign-uninstall |
| malformed archive traversal | traversal | archive member `<name>/../../escaped` with matching manifest digest and sidecar: exit 1 `unsafe_archive`, no `escaped` file anywhere in the world, install dir and binary identical |
| cancel/resume during download | interrupted-download | server sends half the archive and holds the socket; probe sends TERM to the installer's process group on that event (no sleep): exit 143 via the TERM trap, install dir identical (no workspace, no temp binary), rerun installs and the binary is the one under test |
| misleading success: verify executable version/hash | version-mismatch, happy | a release claiming 99.0.0 whose binary reports 0.1.0: exit 1 `version_mismatch`, binary identical; happy checks installed sha256 == binary under test and `--version` output |
| checksum mismatch (QA inject) | checksum-mismatch | one flipped byte, intact manifest+sidecar: exit 1 `checksum_mismatch` with recorded/actual digests, binary identical, `server status` stopped |
| unknown target | unknown-target | manifest without this OS/arch: exit 4 `unknown_target` |
| unwritable destination | unwritable | 0555 install dir and 0555 parent: exit 3 `unwritable_destination`, binary identical, no directory created |
| running runtime | running-runtime | installed binary serving the same environment: exit 5 `runtime_running`, `delegate: update.stage`, binary identical, runtime keeps running until its stdin closes (exit 0) |
| removal of a foreign directory | foreign-uninstall | `--uninstall` of a dir without receipt: exit 3 `not_installed`, every file kept |
| post-install corruption | corruption | appended byte: `--check` exit 1 `checksum_mismatch`; reinstall repairs; `--check` exit 0 |
| unplanned inject | conformance | `--inject 'sh install.sh --bogus'`: runner exit 1, 1 failed assertion (`qa-unplanned-inject.log`) |

## Mutants (each applied to a scratch copy of install.sh; probe exit 1 for every one)

| Mutant | Case | Failed checks |
|---|---|---|
| m1 remove the archive digest, sidecar and length comparisons | checksum-mismatch | exit 1 kind checksum_mismatch (the corrupt gzip then fails later with another kind) |
| m2 remove the `..` member check | traversal | exit 1 kind unsafe_archive (GNU tar itself refuses the member, so the run ends as write_failed) |
| m3 remove the unpacked-version check | version-mismatch | exit 1 kind version_mismatch; install directory unchanged |
| m4 runtime_state always `stopped` | running-runtime | exit 5 kind runtime_running; install directory unchanged; delegates to update.stage |
