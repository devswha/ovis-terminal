"""Fail unless every conformance run in the given evidence files passed with no
failed and no skipped case; print one line per run."""

import json
import sys


def runs(path):
    document = json.load(open(path))
    return document if isinstance(document, list) else [document]


def main():
    bad = 0
    for path in sys.argv[1:]:
        for run in runs(path):
            totals = run.get("totals", {})
            line = (
                f"{path}: {run.get('scenario')} case={run.get('case')} result={run.get('result')} "
                f"cases={totals.get('cases')} passed={totals.get('passed')} "
                f"failed={totals.get('failed')} skipped={totals.get('skipped')} "
                f"binary_sha256={(run.get('binary') or {}).get('sha256')}"
            )
            print(line)
            ok = (
                run.get("result") == "passed"
                and totals.get("failed") == "0"
                and totals.get("skipped") == "0"
                and totals.get("cases") not in (None, "0")
            )
            bad += 0 if ok else 1
    if bad:
        print(f"{bad} run(s) failed, skipped a case or ran nothing", file=sys.stderr)
    sys.exit(1 if bad else 0)


main()
