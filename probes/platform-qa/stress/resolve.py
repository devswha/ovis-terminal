"""Resolve one stress lane into GitHub Actions job outputs.

A lane is the pair <name>.json (runner variants, timeout, setup) and <name>.sh
(the repeated commands) in this directory. Adding a lane adds two files and edits
no shared line, so concurrent lane PRs never conflict on the workflow.
"""

import json
import pathlib
import re
import sys

LANE = re.compile(r"^[a-z0-9][a-z0-9-]*$")
SETUP = {"msvc", "bun", "windows-service-account"}
KEYS = {"format", "variants", "timeout_minutes", "setup"}
VARIANT_KEYS = {"runs_on", "xcode", "variant"}


def fail(message):
    print(f"stress lane: {message}", file=sys.stderr)
    sys.exit(1)


def main():
    if len(sys.argv) != 2:
        fail("usage: resolve.py <lane>")
    name = sys.argv[1]
    if not LANE.match(name):
        fail(f"{name!r} is not a lane name")
    here = pathlib.Path(__file__).resolve().parent
    manifest, script = here / f"{name}.json", here / f"{name}.sh"
    if not manifest.is_file() or not script.is_file():
        known = sorted(p.stem for p in here.glob("*.json"))
        fail(f"unknown lane {name!r}; known lanes: {', '.join(known)}")
    lane = json.loads(manifest.read_text())
    if set(lane) != KEYS or lane["format"] != "ovis-stress-lane/1":
        fail(f"{manifest.name} must hold exactly {sorted(KEYS)} with format ovis-stress-lane/1")
    if not lane["variants"] or any(set(v) != VARIANT_KEYS for v in lane["variants"]):
        fail(f"{manifest.name} needs variants with exactly {sorted(VARIANT_KEYS)}")
    if not set(lane["setup"]) <= SETUP:
        fail(f"{manifest.name} names an unknown setup in {lane['setup']}")
    print(f"matrix={json.dumps({'include': lane['variants']})}")
    print(f"timeout={int(lane['timeout_minutes'])}")
    print(f"setup={json.dumps(lane['setup'])}")


main()
