#!/bin/sh
# Builds, stages, seals and natively checks one ovis-terminal release artifact.
#
#   sh scripts/release/build.sh --target <native|TRIPLE> [--dry-run] [--out DIR] [--runner LABEL]
#
# The target must be this host's own OS and architecture: release builds are native, and
# the stage refuses a receipt from another host (no Linux-to-macOS or other cross build).
# Without --dry-run the target must be one of the five release targets in pins.json; with
# it any native host triple is staged as a non-release artifact. This script never
# publishes anything: promotion is `release-stage promote`, which only the release
# workflow's orchestrator job runs, and only after every native target job has passed.
set -eu

repo=$(cd "$(dirname "$0")/../.." && pwd)
target=native
dry_run=0
runner=${OVIS_RELEASE_RUNNER:-local}
out=""

while [ $# -gt 0 ]; do
    case "$1" in
        --target) target=$2; shift 2 ;;
        --dry-run) dry_run=1; shift ;;
        --out) out=$2; shift 2 ;;
        --runner) runner=$2; shift 2 ;;
        *) echo "build.sh: unknown argument $1" >&2; exit 2 ;;
    esac
done

cd "$repo"
if [ "$target" = native ]; then
    target=$(rustc -vV | sed -n 's/^host: //p')
fi
if [ "$dry_run" -eq 0 ] && ! grep -q "\"$target\": {" scripts/release/pins.json; then
    echo "build.sh: $target is not a release target; stage it with --dry-run" >&2
    exit 2
fi

target_dir=${CARGO_TARGET_DIR:-$repo/target}
[ -n "$out" ] || out="$target_dir/release-stage/$target"
rm -rf "$out"
mkdir -p "$out/sdk"
# Absolute from here on: the SDK is packed from packages/client-ts, not the repository root.
out=$(cd "$out" && pwd)

stage_tool() {
    cargo run --locked --quiet -p release-stage -- "$@"
}

binary="$target_dir/$target/release/ovis-terminal"
case "$target" in *-windows-*) binary="$binary.exe" ;; esac
# A binary left by an earlier build must never be staged in place of this one.
rm -f "$binary"

echo "==> build ovis-terminal for $target (release profile)"
cargo build --locked --release -p ovis-terminal-cli --bin ovis-terminal --target "$target"
[ -f "$binary" ] || { echo "build.sh: cargo produced no $binary" >&2; exit 1; }

echo "==> pack the TypeScript SDK"
(
    cd packages/client-ts
    bun install --frozen-lockfile
    bun run generate:check
    bun run build
    bun run pack --destination "$out/sdk"
)
set -- "$out"/sdk/ovis-terminal-client-*.tgz
[ -f "$1" ] || { echo "build.sh: bun pack produced no ovis-terminal-client tarball" >&2; exit 1; }
sdk=$1

echo "==> observe the C toolchain"
set --
case "$(uname -s)" in
    Darwin)
        xcode=$(xcodebuild -version)
        set -- --observe "xcode=$(printf '%s\n' "$xcode" | sed -n 1p)" \
            --observe "sdk=$(xcrun --sdk macosx --show-sdk-version)" \
            --observe "clang=$(clang --version | sed -n 1p)"
        ;;
    Linux)
        triple_var=$(printf '%s' "$target" | tr 'a-z-' 'A-Z_')
        cc=$(eval "printf '%s' \"\${CC_$(printf '%s' "$target" | tr '-' '_'):-\${CC:-cc}}\"")
        set -- --observe "compiler=$($cc --version 2>/dev/null | sed -n 1p) ($cc)"
        case "$target" in
            *-musl)
                set -- "$@" --observe "gcc=$(${REALGCC:-gcc} -dumpfullversion)"
                musl=$(dpkg-query -W -f='${Version}' musl-tools 2>/dev/null || echo unknown)
                set -- "$@" --observe "musl=${musl%%-*}"
                ;;
        esac
        linker_var="CARGO_TARGET_${triple_var}_LINKER"
        set -- "$@" --observe "linker=$(eval "printf '%s' \"\${$linker_var:-default}\"")"
        ;;
    MINGW* | MSYS* | CYGWIN*)
        banner=$(cl.exe 2>&1 || :)
        cl=$(printf '%s\n' "$banner" | sed -n 's/.*Version \([0-9]*\.[0-9]*\).*/\1/p' | sed -n 1p)
        toolset=$(printf '%s' "${VCToolsVersion:-unknown}" | cut -d. -f1-2)
        set -- --observe "msvc_toolset=$toolset" --observe "cl=$cl"
        ;;
esac

echo "==> receipt, stage, seal, native QA, verify"
stage_tool receipt --target "$target" --runner "$runner" --repo "$repo" --out "$out/receipt.json" "$@"
stage_tool stage --target "$target" --binary "$binary" --sdk "$sdk" --receipt "$out/receipt.json" \
    --repo "$repo" --out "$out/stage"
entry=$(ls "$out"/stage/*.entry.json)
stage_tool seal --entry "$entry"
stage_tool qa --entry "$entry" --install-dir "$out/install" \
    --released "$repo/schema/released.openrpc.json" --runner "$runner"
stage_tool verify --dir "$out/stage"

if [ "$dry_run" -eq 1 ]; then
    echo "build.sh: dry run staged $target in $out/stage; nothing was published"
else
    echo "build.sh: staged $target in $out/stage for promotion; nothing was published"
fi
