# Operator guide: installing, updating and removing ovis-terminal

Plan todo 68. Closes GAP-001, GAP-017, GAP-127 and GAP-141 for installation; delivers
IDEAL-001, IDEAL-014, IDEAL-100 and IDEAL-114 there. The artifacts come from the release
stage (todo 67), the holder service from todo 51 (Unix) and todo 54 (Windows), and staged
updates of a running runtime from todo 47.

## Install

```sh
OVIS_TERMINAL_INSTALL_DIR="$HOME/opt/ovis-terminal" sh install.sh
```

| Variable | Meaning | Unset |
|---|---|---|
| `OVIS_TERMINAL_INSTALL_DIR` | The only directory the installer writes (isolated mode) | `${XDG_DATA_HOME:-$HOME/.local/share}/ovis-terminal` (default mode) |
| `OVIS_TERMINAL_VERSION` | Pin a release (`0.1.0` or `v0.1.0`) | the latest release |
| `OVIS_TERMINAL_RELEASES_URL` | Release base URL | `https://github.com/devswha/ovis-terminal/releases` |

The base URL uses the GitHub release layout: the latest release is read from
`<base>/latest/download/<file>`, a pinned one from `<base>/download/v<version>/<file>`.
Only `https` is accepted, and plain `http` only on `127.0.0.1`, `localhost` or `[::1]`
(local mirrors and test fixtures); anything else is `insecure_url`.

The installer needs `sh`, `curl` or `wget`, `tar`, `gzip`, `uname` and `sha256sum` or
`shasum`. It does, in order:

1. Downloads `release-manifest.json` (`ovis-release-manifest/1`, written by
   `release-stage promote`) and picks the target whose `platform` is this host's OS and
   architecture. A release without one is `unknown_target` (exit 4).
2. Downloads the archive and its `.sha256` sidecar and requires the archive's actual bytes
   to hash to the manifest digest, the sidecar to name the same digest, and the length to
   be the manifest's `bytes`. Any difference is `checksum_mismatch` (exit 1).
3. Lists the archive before unpacking it: every member must lie below
   `<name>/`, no member may have a `..` component, and only regular files and directories
   are accepted. Anything else is `unsafe_archive` (exit 1), and nothing is unpacked.
4. Unpacks into a workspace inside the install directory and runs the unpacked binary
   with an empty environment: it must report the manifest version (`version_mismatch`,
   exit 1, otherwise). A pinned version must also be the manifest's version.
5. Asks the unpacked binary, read-only, whether the runtime this environment resolves to
   is running (see "Updating a running runtime").
6. Moves the release into `releases/<name>/`, copies the binary next to `bin/`, re-hashes
   the copy, keeps the old binary as `bin/ovis-terminal.previous` and renames the new one
   over `bin/ovis-terminal`. That rename is the only step that changes the installed
   binary, and it is the last one, so every refusal leaves the old binary byte for byte.
7. Writes `install-receipt.json` the same way (temporary file, then rename).

```text
<dir>/bin/ovis-terminal            the installed binary
<dir>/bin/ovis-terminal.previous   the binary it replaced, if any
<dir>/releases/<name>/             the unpacked release: binary, schema, manual, SDK, notices
<dir>/install-receipt.json         version, target, archive and binary digests, redacted URL
<dir>/.ovis-install.XXXXXX/        workspace of a run in progress; removed on exit and on INT/TERM/HUP
```

Stdout carries exactly one JSON object; progress goes to stderr. Every refusal names its
`kind`, `hint` and exit status:

| Exit | Kinds |
|---|---|
| 0 | `installed`, `verified` (`--check`), `removed` (`--uninstall`) |
| 1 | `checksum_mismatch`, `unsafe_archive`, `version_mismatch`, `manifest_malformed` |
| 2 | `usage`, `insecure_url` |
| 3 | `unwritable_destination`, `download_failed`, `write_failed`, `missing_tool`, `not_installed` |
| 4 | `unknown_target` |
| 5 | `runtime_running` |

### What isolated mode never touches

With `OVIS_TERMINAL_INSTALL_DIR` set, the installer writes that directory and the
workspace inside it, and nothing else: no shell profile or rc file, no symlink in a
global `bin`, no systemd unit or launchd agent, no file below `$HOME` or the XDG
directories, and no default installation. It never starts, stops or restarts a runtime or
a service; the only runtime command it runs is `ovis-terminal --json server status`, which
is read-only (IDEAL-014, EXCL-027). Default mode writes only its default directory and
prints the `bin/` path to add to `PATH` yourself.

An unwritable or uncreatable directory is `unwritable_destination` before any download.

## Updating

Run the installer again. Reinstalling the same version replaces the files with verified
copies; a newer version (latest or pinned) replaces `bin/ovis-terminal` by rename and
removes the previous `releases/<name>/` after the new one is in place.

### Updating a running runtime

If the runtime this environment resolves to (`OVIS_TERMINAL_SOCKET`, `XDG_RUNTIME_DIR`,
...) answers `server status`, either `running` or with an authentication refusal from a
live server, the installer does not replace its binary. It keeps the verified release in
`releases/<name>/`, leaves `bin/ovis-terminal` and the receipt unchanged and exits 5:

```json
{"outcome":"refused","kind":"runtime_running","exit":5,"delegate":"update.stage",
 "version":"0.2.0","archive_sha256":"...","verified_release":"<dir>/releases/ovis-terminal-0.2.0-..."}
```

The update then goes through the runtime's staged update (todo 47), which keeps programs
running and exposes verification, activation and rollback outcomes (IDEAL-100):
`update.stage` (version and expected digest) from the runtime's release source,
`update.verify`, `update.activate`, and `update.rollback` if the new build misbehaves;
`update.query` shows the active and previous builds at every step. Stop the runtime first
if you want the installer to replace the binary directly instead.

## Services

The installer never installs or starts a service. Service ownership stays explicit:

```sh
"$DIR/bin/ovis-terminal" service install --manager systemd-user   # or launchd
"$DIR/bin/ovis-terminal" service start --manager systemd-user
"$DIR/bin/ovis-terminal" service status
```

The unit or agent names the binary path it was installed with, so a service installed
from an isolated directory belongs to that directory. See `docs/service-lifetime.md` for
labels, paths and logout survival.

## Removing

```sh
"$DIR/bin/ovis-terminal" service stop                      # if you started one
"$DIR/bin/ovis-terminal" service uninstall --manager systemd-user
OVIS_TERMINAL_INSTALL_DIR="$DIR" sh install.sh --uninstall
```

`--uninstall` removes exactly what the receipt describes: `bin/ovis-terminal`,
`bin/ovis-terminal.previous`, `releases/<name>/` and the receipt, then the `bin/`,
`releases/` and install directories if they are empty. Anything else you put there is
kept and listed in `kept`. A directory without an install receipt is `not_installed`
(exit 3) and nothing in it is removed; a running runtime is `runtime_running` (exit 5).

## Rollback

- Not running: reinstall the previous version,
  `OVIS_TERMINAL_VERSION=<previous> OVIS_TERMINAL_INSTALL_DIR="$DIR" sh install.sh`. It goes
  through the same checks and writes a matching receipt. `bin/ovis-terminal.previous` is the
  binary that was replaced, for inspection or an emergency
  `mv "$DIR/bin/ovis-terminal.previous" "$DIR/bin/ovis-terminal"` (one rename);
  `--check` reports `checksum_mismatch` until the next install rewrites the receipt.
- Running: `update.rollback` selects the previous build again without stopping programs.

## Diagnosing a broken installation

### Corruption

```sh
OVIS_TERMINAL_INSTALL_DIR="$DIR" sh install.sh --check
```

`--check` re-hashes `bin/ovis-terminal` against the receipt: `verified` (exit 0) or
`checksum_mismatch` (exit 1) with the recorded and actual digests; a missing binary is
`actual: "missing"`. Reinstalling the receipt's version repairs it. A download corrupted in
transit never reaches `bin/`: it is refused in step 2.

### Disk full or permissions

A failed write is `write_failed` (exit 3) naming the step; `unwritable_destination` means
the directory itself cannot be created or written. The installed binary is unchanged in
both cases, and the run's workspace is removed on exit. Free space (`df -h "$DIR"`) or
fix permissions and rerun. A workspace left by a run killed with SIGKILL or a power loss
is a `.ovis-install.XXXXXX` directory in `$DIR`; it is never used again and can be
deleted.

### Interrupted download

Ctrl-C or SIGTERM during a download removes the workspace and exits 130 or 143; the
installed binary is untouched. Rerun the same command to install.

### Support evidence

Collect, from the install directory only:

```sh
OVIS_TERMINAL_INSTALL_DIR="$DIR" sh install.sh --check
cat "$DIR/install-receipt.json"
"$DIR/bin/ovis-terminal" --version
"$DIR/bin/ovis-terminal" --json server status
uname -sm
```

The receipt and every report carry the releases URL without user information or query
string (`https://user:token@host/x?sig=...` is recorded as `https://host/x`). Do not add
`OVIS_TERMINAL_CREDENTIAL` or a bearer printed by `serve`.

## Windows

`install.sh` is POSIX; on Windows install from the same release files the CI release job
publishes (todo 67): `release-manifest.json`, `ovis-terminal-<version>-x86_64-pc-windows-msvc.tar.gz`
and its `.sha256` sidecar. The artifacts are not code-signed (`signing.performed: false` in
the manifest). In PowerShell:

```powershell
$base = "https://github.com/devswha/ovis-terminal/releases/download/v$version"
$name = "ovis-terminal-$version-x86_64-pc-windows-msvc"
$dir  = "$env:LOCALAPPDATA\ovis-terminal"
Invoke-WebRequest "$base/release-manifest.json" -OutFile manifest.json
Invoke-WebRequest "$base/$name.tar.gz" -OutFile "$name.tar.gz"
Invoke-WebRequest "$base/$name.tar.gz.sha256" -OutFile "$name.tar.gz.sha256"
$entry  = (Get-Content manifest.json | ConvertFrom-Json).targets |
          Where-Object { $_.platform.os -eq "windows" -and $_.platform.arch -eq "x86_64" }
$actual = (Get-FileHash "$name.tar.gz" -Algorithm SHA256).Hash.ToLower()
$sidecar = (Get-Content "$name.tar.gz.sha256").Split(" ")[0]
if ($actual -ne $entry.sha256 -or $sidecar -ne $entry.sha256) { throw "checksum_mismatch" }
tar -tzf "$name.tar.gz" | Where-Object { -not $_.StartsWith("$name/") -or $_ -match '(^|/)\.\.(/|$)' } |
    ForEach-Object { throw "unsafe_archive: $_" }
tar -xzf "$name.tar.gz" -C $dir
& "$dir\$name\ovis-terminal.exe" --version
```

The same rules hold: verify before unpacking, unpack into the directory you chose, change
no profile or `PATH` entry you did not decide to change, and install the holder service
explicitly (`ovis-terminal service install`, todo 54).

## What this replaces

herdr's installer and status path are not reproduced: no release-notes or announcement
prompt during install (EXCL-013); every installer run ends in one JSON outcome and exit
status rather than a detached step with discarded output (EXCL-020); and inspecting a
runtime never starts one (EXCL-027, V-RT-HC-04). Refusals are typed kinds with hints, not
free text (V-005).

## Verifying the installer

```sh
cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
cargo run --locked --manifest-path probes/Cargo.toml --bin install-probe -- \
    --script ./install.sh --isolated --evidence "$ATTEMPT_DIR/task-68-ovis-terminal-v1.json"
cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- \
    --binary target/debug/ovis-terminal --scenario release/install --isolated --case all \
    --evidence "$ATTEMPT_DIR/task-68-ovis-terminal-v1.json"
```

The probe stages the binary under test as a real release artifact, serves it and its
corrupt, traversal, unknown-target and mislabeled copies on a loopback port, and runs
the literal `sh install.sh` commands above in private home, XDG and attempt directories.
It checks the installed binary's hash and version, that nothing outside the install
directory changed, and that no runtime was started.
