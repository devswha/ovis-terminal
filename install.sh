#!/bin/sh
# Installs an ovis-terminal release into one directory, verified against its manifest.
#
#   sh install.sh               install (or reinstall) the selected release
#   sh install.sh --check       re-hash the installed binary against its install receipt
#   sh install.sh --uninstall   remove exactly what the install receipt lists
#
# Environment:
#   OVIS_TERMINAL_INSTALL_DIR    the only directory written (isolated mode). Unset means
#                                ${XDG_DATA_HOME:-$HOME/.local/share}/ovis-terminal.
#   OVIS_TERMINAL_VERSION        pin a release version; unset means the latest release.
#   OVIS_TERMINAL_RELEASES_URL   release base URL (GitHub layout: latest/download/<file>,
#                                download/v<version>/<file>). https, or http on loopback.
#
# The installer writes the install directory and a temporary workspace inside it, and
# nothing else: no shell profile, no symlink outside it, no service, no running runtime.
# The binary is replaced by one rename after every check passed, so a refusal leaves the
# installed binary byte for byte. A runtime this environment resolves to that is running
# is never replaced under it; the install hands over to the staged update (update.stage).
#
# Stdout carries one JSON report; progress goes to stderr. Exit status:
#   0 installed, verified or removed
#   1 refused release: checksum_mismatch, unsafe_archive, version_mismatch, manifest_malformed
#   2 usage: unknown argument, insecure_url, invalid version
#   3 io: unwritable_destination, download_failed, write_failed, missing_tool, not_installed
#   4 unknown_target: the release has no artifact for this OS and architecture
#   5 runtime_running: a live runtime uses this installation; use the staged update
set -eu

DEFAULT_RELEASES_URL=https://github.com/devswha/ovis-terminal/releases
BINARY=ovis-terminal
RECEIPT=install-receipt.json

work=""
new_binary=""
new_previous=""
new_receipt=""

cleanup() {
    [ -z "$new_binary" ] || rm -f "$new_binary"
    [ -z "$new_previous" ] || rm -f "$new_previous"
    [ -z "$new_receipt" ] || rm -f "$new_receipt"
    [ -z "$work" ] || rm -rf "$work"
}
trap cleanup EXIT
trap 'cleanup; trap - EXIT; exit 130' INT
trap 'cleanup; trap - EXIT; exit 143' TERM
trap 'cleanup; trap - EXIT; exit 129' HUP

say() {
    printf 'install.sh: %s\n' "$*" >&2
}

json() {
    printf '"%s"' "$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g' | tr -d '\n\r')"
}

hint_of() {
    case "$1" in
        checksum_mismatch) echo "the download does not hash to the release manifest; nothing was replaced, retry or report the release" ;;
        unsafe_archive) echo "the archive holds a path outside its own directory or a link; it was not unpacked" ;;
        version_mismatch) echo "the unpacked binary does not report the release version it was published as" ;;
        manifest_malformed) echo "the release manifest is not an ovis-release-manifest/1 document" ;;
        usage) echo "run sh install.sh [--check|--uninstall]" ;;
        insecure_url) echo "OVIS_TERMINAL_RELEASES_URL must be https, or http on 127.0.0.1, localhost or [::1]" ;;
        unwritable_destination) echo "the install directory cannot be created or written; choose a writable OVIS_TERMINAL_INSTALL_DIR" ;;
        download_failed) echo "a release file could not be downloaded completely; rerun the installer" ;;
        write_failed) echo "writing the install directory failed (disk full or permissions); the installed binary is unchanged" ;;
        missing_tool) echo "install curl or wget, tar, gzip and sha256sum or shasum" ;;
        not_installed) echo "the directory holds no install receipt, so nothing in it is known to be ours" ;;
        unknown_target) echo "this release publishes no artifact for this OS and architecture" ;;
        runtime_running) echo "a running runtime uses this installation; stage the verified release with update.stage instead" ;;
        *) echo "see docs/operator-guide.md" ;;
    esac
}

# fail KIND EXIT MESSAGE [EXTRA_JSON_FIELDS]
fail() {
    say "error: $1: $3"
    printf '{"outcome":"refused","kind":%s,"exit":%s,"hint":%s,"message":%s%s}\n' \
        "$(json "$1")" "$2" "$(json "$(hint_of "$1")")" "$(json "$3")" "${4:-}"
    exit "$2"
}

need() {
    for tool in "$@"; do
        command -v "$tool" >/dev/null 2>&1 || fail missing_tool 3 "$tool is not on PATH"
    done
}

sha256_of() {
    if [ "$sha_tool" = sha256sum ]; then
        sha256sum "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 "$1" | cut -d ' ' -f 1
    fi
}

bytes_of() {
    wc -c <"$1" | tr -d ' '
}

# The releases URL without credentials or query, for reports and support evidence.
redact_url() {
    printf '%s' "$1" | sed 's#^\([a-zA-Z][a-zA-Z0-9+.-]*://\)[^/@]*@#\1#; s#[?].*$##'
}

check_url() {
    case "$1" in
        https://*) ;;
        http://127.0.0.1[:/]* | http://127.0.0.1 | http://localhost[:/]* | http://localhost | http://\[::1\]*) ;;
        *) fail insecure_url 2 "$(redact_url "$1") is neither https nor loopback http" ;;
    esac
}

fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$2" "$1"
    else
        fail missing_tool 3 "neither curl nor wget is on PATH"
    fi
}

# Flattens a JSON document into `path<TAB>value` lines (`targets.0.platform.os	linux`).
flatten() {
    awk '
    BEGIN { RS = "\001" }
    function path(   p, k) {
        p = ""
        for (k = 1; k <= depth; k++) p = (k == 1) ? key[k] : p "." key[k]
        return p
    }
    {
        s = $0; n = length(s); i = 1; depth = 0
        while (i <= n) {
            c = substr(s, i, 1)
            if (c == "{") { depth++; kind[depth] = "o"; want[depth] = "key"; i++ }
            else if (c == "[") { depth++; kind[depth] = "a"; at[depth] = 0; key[depth] = 0; i++ }
            else if (c == "}" || c == "]") { depth--; i++ }
            else if (c == ",") {
                if (kind[depth] == "a") { at[depth]++; key[depth] = at[depth] } else want[depth] = "key"
                i++
            }
            else if (c == ":") { want[depth] = "value"; i++ }
            else if (c == "\"") {
                j = i + 1; v = ""
                while (j <= n) {
                    d = substr(s, j, 1)
                    if (d == "\\") { v = v substr(s, j, 2); j += 2; continue }
                    if (d == "\"") break
                    v = v d; j++
                }
                i = j + 1
                if (kind[depth] == "o" && want[depth] == "key") key[depth] = v
                else print path() "\t" v
            }
            else if (c ~ /[ \t\r\n]/) { i++ }
            else {
                j = i
                while (j <= n && substr(s, j, 1) !~ /[,}\] \t\r\n]/) j++
                print path() "\t" substr(s, i, j - i)
                i = j
            }
        }
    }'
}

field() {
    printf '%s\n' "$1" | awk -F '\t' -v p="$2" '$1 == p { print $2; exit }'
}

host_platform() {
    case "$(uname -s)" in
        Linux) os=linux ;;
        Darwin) os=macos ;;
        MINGW* | MSYS* | CYGWIN*) os=windows ;;
        *) os=$(uname -s) ;;
    esac
    case "$(uname -m)" in
        x86_64 | amd64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) arch=$(uname -m) ;;
    esac
}

# runtime_state BINARY: `running` when the runtime this environment resolves to answers.
# `server status` is read-only: it never starts, installs or restarts anything.
runtime_state() {
    [ -x "$1" ] || { echo none; return; }
    status_out=$("$1" --json server status 2>"$work/status.err") || :
    if printf '%s' "$status_out" | grep -q '"state":"running"'; then
        echo running
    elif grep -q '"origin":"server"' "$work/status.err"; then
        # A live server refused this unauthenticated read: it is running.
        echo running
    else
        echo stopped
    fi
}

setup_dir() {
    if [ -n "${OVIS_TERMINAL_INSTALL_DIR:-}" ]; then
        dir=$OVIS_TERMINAL_INSTALL_DIR
        mode=isolated
    else
        dir=${XDG_DATA_HOME:-${HOME:?HOME or OVIS_TERMINAL_INSTALL_DIR must be set}/.local/share}/ovis-terminal
        mode=default
    fi
    case "$dir" in
        /*) ;;
        *) dir=$(pwd)/$dir ;;
    esac
    mkdir -p "$dir" 2>/dev/null || fail unwritable_destination 3 "cannot create $dir"
    [ -w "$dir" ] || fail unwritable_destination 3 "$dir is not writable"
    work=$(mktemp -d "$dir/.ovis-install.XXXXXX" 2>/dev/null) ||
        fail unwritable_destination 3 "cannot create a workspace in $dir"
}

read_receipt() {
    [ -f "$dir/$RECEIPT" ] || fail not_installed 3 "$dir has no $RECEIPT"
    flat=$(flatten <"$dir/$RECEIPT")
    [ "$(field "$flat" format)" = ovis-install-receipt/1 ] ||
        fail not_installed 3 "$dir/$RECEIPT is not an install receipt"
}

do_check() {
    read_receipt
    recorded=$(field "$flat" binary_sha256)
    installed=$dir/bin/$BINARY
    [ -f "$installed" ] || fail checksum_mismatch 1 "$installed is missing" \
        ",\"recorded\":$(json "$recorded"),\"actual\":\"missing\""
    actual=$(sha256_of "$installed")
    [ "$actual" = "$recorded" ] || fail checksum_mismatch 1 "$installed was changed after install" \
        ",\"recorded\":$(json "$recorded"),\"actual\":$(json "$actual")"
    printf '{"outcome":"verified","dir":%s,"version":%s,"binary_sha256":%s,"runtime":%s,"releases_url":%s}\n' \
        "$(json "$dir")" "$(json "$(field "$flat" version)")" "$(json "$actual")" \
        "$(json "$(runtime_state "$installed")")" "$(json "$(field "$flat" releases_url)")"
}

do_uninstall() {
    read_receipt
    name=$(field "$flat" release)
    case "$name" in
        ovis-terminal-*) ;;
        *) fail not_installed 3 "$dir/$RECEIPT names no release directory" ;;
    esac
    case "$name" in
        */* | *..*) fail not_installed 3 "$dir/$RECEIPT names an unsafe release $name" ;;
    esac
    [ "$(runtime_state "$dir/bin/$BINARY")" != running ] ||
        fail runtime_running 5 "the runtime of this installation is running; stop it before removing it"
    rm -f "$dir/bin/$BINARY" "$dir/bin/$BINARY.previous"
    rm -rf "${dir:?}/releases/$name"
    rm -f "$dir/$RECEIPT"
    rm -rf "$work"
    work=""
    kept=""
    for leftover in "$dir/bin" "$dir/releases" "$dir"; do
        if [ -d "$leftover" ] && ! rmdir "$leftover" 2>/dev/null; then
            kept="$kept${kept:+,}$(json "$leftover")"
        fi
    done
    printf '{"outcome":"removed","dir":%s,"release":%s,"kept":[%s]}\n' \
        "$(json "$dir")" "$(json "$name")" "$kept"
}

select_release() {
    base=${OVIS_TERMINAL_RELEASES_URL:-$DEFAULT_RELEASES_URL}
    base=${base%/}
    check_url "$base"
    pinned=${OVIS_TERMINAL_VERSION:-}
    if [ -n "$pinned" ]; then
        printf '%s' "$pinned" | grep -Eq '^v?[0-9A-Za-z][0-9A-Za-z.+-]*$' ||
            fail usage 2 "OVIS_TERMINAL_VERSION=$pinned is not a version"
        pinned=${pinned#v}
        files=$base/download/v$pinned
    else
        files=$base/latest/download
    fi
    say "release manifest from $(redact_url "$files")"
    fetch "$files/release-manifest.json" "$work/release-manifest.json" ||
        fail download_failed 3 "release-manifest.json could not be downloaded"
    manifest=$(flatten <"$work/release-manifest.json")
    [ "$(field "$manifest" format)" = ovis-release-manifest/1 ] ||
        fail manifest_malformed 1 "release-manifest.json has no ovis-release-manifest/1 format"
    version=$(field "$manifest" version)
    printf '%s' "$version" | grep -Eq '^[0-9A-Za-z][0-9A-Za-z.+-]*$' ||
        fail manifest_malformed 1 "the manifest version is not a version"
    if [ -n "$pinned" ] && [ "$version" != "$pinned" ]; then
        fail version_mismatch 1 "the manifest is release $version, not the pinned $pinned"
    fi
    host_platform
    index=$(printf '%s\n' "$manifest" | awk -F '\t' -v os="$os" -v arch="$arch" '
        $1 ~ /^targets\.[0-9]+\.platform\.os$/ { split($1, p, "."); o[p[2]] = $2 }
        $1 ~ /^targets\.[0-9]+\.platform\.arch$/ { split($1, p, "."); a[p[2]] = $2 }
        END { for (k in o) if (o[k] == os && a[k] == arch) { print k; exit } }')
    [ -n "$index" ] || fail unknown_target 4 "release $version has no $os/$arch artifact" \
        ",\"platform\":$(json "$os/$arch")"
    target=$(field "$manifest" "targets.$index.target")
    archive=$(field "$manifest" "targets.$index.archive")
    sha256=$(field "$manifest" "targets.$index.sha256")
    size=$(field "$manifest" "targets.$index.bytes")
    printf '%s' "$archive" | grep -Eq '^ovis-terminal-[0-9A-Za-z.+_-]+\.tar\.gz$' ||
        fail manifest_malformed 1 "the $os/$arch archive name is not a release archive name"
    printf '%s' "$sha256" | grep -Eq '^[0-9a-f]{64}$' ||
        fail manifest_malformed 1 "the $os/$arch archive digest is not a SHA-256"
    name=${archive%.tar.gz}
}

download_release() {
    say "download $archive"
    fetch "$files/$archive" "$work/$archive" ||
        fail download_failed 3 "$archive could not be downloaded completely"
    fetch "$files/$archive.sha256" "$work/$archive.sha256" ||
        fail download_failed 3 "$archive.sha256 could not be downloaded completely"
    actual=$(sha256_of "$work/$archive")
    sidecar=$(cut -d ' ' -f 1 <"$work/$archive.sha256")
    mismatch=",\"file\":$(json "$archive"),\"recorded\":$(json "$sha256"),\"actual\":$(json "$actual")"
    [ "$actual" = "$sha256" ] ||
        fail checksum_mismatch 1 "$archive does not hash to the manifest digest" "$mismatch"
    [ "$sidecar" = "$sha256" ] ||
        fail checksum_mismatch 1 "$archive.sha256 disagrees with the manifest" \
            ",\"file\":$(json "$archive.sha256"),\"recorded\":$(json "$sha256"),\"actual\":$(json "$sidecar")"
    [ "$(bytes_of "$work/$archive")" = "$size" ] ||
        fail checksum_mismatch 1 "$archive is not the manifest's $size bytes" "$mismatch"
}

unpack_release() {
    tar -tzf "$work/$archive" >"$work/listing" 2>/dev/null ||
        fail unsafe_archive 1 "$archive is not a gzip tar archive"
    tar -tvzf "$work/$archive" >"$work/types" 2>/dev/null ||
        fail unsafe_archive 1 "$archive is not a gzip tar archive"
    if grep -v "^$name/" "$work/listing" | grep -q .; then
        fail unsafe_archive 1 "$archive holds a path outside $name/" \
            ",\"path\":$(json "$(grep -v "^$name/" "$work/listing" | sed -n 1p)")"
    fi
    if grep -Eq '(^|/)\.\.(/|$)' "$work/listing"; then
        fail unsafe_archive 1 "$archive holds a .. path component" \
            ",\"path\":$(json "$(grep -E '(^|/)\.\.(/|$)' "$work/listing" | sed -n 1p)")"
    fi
    if grep -vq '^[-d]' "$work/types"; then
        fail unsafe_archive 1 "$archive holds a link or special file"
    fi
    mkdir "$work/unpack" || fail write_failed 3 "cannot create the unpack directory"
    tar -xzf "$work/$archive" -C "$work/unpack" ||
        fail write_failed 3 "unpacking $archive failed"
    unpacked=$work/unpack/$name/$BINARY
    [ -f "$unpacked" ] || fail unsafe_archive 1 "$archive has no $name/$BINARY"
    chmod 755 "$unpacked" || fail write_failed 3 "cannot mark $BINARY executable"
    reported=$(env -i "$unpacked" --json --version 2>/dev/null |
        sed -n 's/.*"version":"\([^"]*\)".*/\1/p') || :
    [ "$reported" = "$version" ] ||
        fail version_mismatch 1 "the unpacked binary reports ${reported:-nothing}, not $version" \
            ",\"recorded\":$(json "$version"),\"actual\":$(json "$reported")"
    unpacked_sha256=$(sha256_of "$unpacked")
}

delegate_running() {
    [ "$(runtime_state "$unpacked")" = running ] || return 0
    held=$dir/releases/$name
    if [ ! -d "$held" ]; then
        mkdir -p "$dir/releases" && mv "$work/unpack/$name" "$held" ||
            fail write_failed 3 "cannot keep the verified release in $dir/releases"
    fi
    fail runtime_running 5 "a running runtime uses this environment; its binary was not replaced" \
        ",\"delegate\":\"update.stage\",\"version\":$(json "$version"),\"archive_sha256\":$(json "$sha256"),\"verified_release\":$(json "$held")"
}

activate() {
    old_release=""
    if [ -f "$dir/$RECEIPT" ]; then
        old_release=$(field "$(flatten <"$dir/$RECEIPT")" release)
    fi
    mkdir -p "$dir/bin" "$dir/releases" || fail write_failed 3 "cannot create $dir/bin or $dir/releases"
    if [ -d "$dir/releases/$name" ]; then
        mv "$dir/releases/$name" "$work/replaced" || fail write_failed 3 "cannot move the old $name aside"
    fi
    mv "$work/unpack/$name" "$dir/releases/$name" || fail write_failed 3 "cannot move $name into place"
    new_binary=$dir/bin/.$BINARY.new.$$
    cp "$dir/releases/$name/$BINARY" "$new_binary" && chmod 755 "$new_binary" ||
        fail write_failed 3 "cannot copy the new binary into $dir/bin"
    [ "$(sha256_of "$new_binary")" = "$unpacked_sha256" ] ||
        fail write_failed 3 "the copied binary does not hash to the unpacked one"
    if [ -f "$dir/bin/$BINARY" ]; then
        new_previous=$dir/bin/.$BINARY.previous.$$
        cp -p "$dir/bin/$BINARY" "$new_previous" && mv -f "$new_previous" "$dir/bin/$BINARY.previous" ||
            fail write_failed 3 "cannot keep the previous binary"
        new_previous=""
    fi
    mv -f "$new_binary" "$dir/bin/$BINARY" || fail write_failed 3 "cannot rename the new binary into place"
    new_binary=""
    installed_sha256=$(sha256_of "$dir/bin/$BINARY")
    new_receipt=$dir/.$RECEIPT.$$
    printf '{"format":"ovis-install-receipt/1","mode":%s,"version":%s,"target":%s,"platform":%s,"release":%s,"archive":%s,"archive_sha256":%s,"binary_sha256":%s,"releases_url":%s}\n' \
        "$(json "$mode")" "$(json "$version")" "$(json "$target")" "$(json "$os/$arch")" \
        "$(json "$name")" "$(json "$archive")" "$(json "$sha256")" "$(json "$installed_sha256")" \
        "$(json "$(redact_url "$base")")" >"$new_receipt" && mv -f "$new_receipt" "$dir/$RECEIPT" ||
        fail write_failed 3 "cannot write $RECEIPT"
    new_receipt=""
    case "$old_release" in
        "" | "$name" | */* | *..*) ;;
        ovis-terminal-*) rm -rf "${dir:?}/releases/$old_release" ;;
    esac
}

do_install() {
    need tar gzip uname
    select_release
    download_release
    unpack_release
    delegate_running
    activate
    printf '{"outcome":"installed","mode":%s,"dir":%s,"binary":%s,"version":%s,"target":%s,"platform":%s,"archive_sha256":%s,"binary_sha256":%s,"releases_url":%s}\n' \
        "$(json "$mode")" "$(json "$dir")" "$(json "$dir/bin/$BINARY")" "$(json "$version")" \
        "$(json "$target")" "$(json "$os/$arch")" "$(json "$sha256")" "$(json "$installed_sha256")" \
        "$(json "$(redact_url "$base")")"
    if [ "$mode" = default ]; then
        say "installed $dir/bin/$BINARY; add $dir/bin to PATH yourself, no profile was edited"
    fi
}

action=install
case "${1:-}" in
    "") ;;
    --check) action=check ;;
    --uninstall) action=uninstall ;;
    *) fail usage 2 "unknown argument $1" ;;
esac
[ $# -le 1 ] || fail usage 2 "takes at most one argument"

if command -v sha256sum >/dev/null 2>&1; then
    sha_tool=sha256sum
elif command -v shasum >/dev/null 2>&1; then
    sha_tool=shasum
else
    fail missing_tool 3 "neither sha256sum nor shasum is on PATH"
fi
setup_dir
case "$action" in
    install) do_install ;;
    check) do_check ;;
    uninstall) do_uninstall ;;
esac
