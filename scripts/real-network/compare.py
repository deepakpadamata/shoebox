#!/usr/bin/env python3
"""Flags a regression in a real-network report: a failed step, or a step that took more than
three times as long as in the baseline (scripts/real-network/baseline.json, updated by hand after
a run that was judged normal). With no baseline it only checks for failures."""
import json
import pathlib
import sys

report = json.load(open(sys.argv[1]))
bad = [f"step failed: {s['name']}: {s.get('error')}" for s in report["steps"] if not s["ok"]]
base_path = pathlib.Path(__file__).with_name("baseline.json")
if base_path.exists():
    base = {s["name"]: s["seconds"] for s in json.load(open(base_path))["steps"]}
    for s in report["steps"]:
        was = base.get(s["name"])
        if was and s["seconds"] > max(30, 3 * was):
            bad.append(f"{s['name']} took {s['seconds']}s (baseline {was}s)")
print("paths:", report.get("paths"))
for b in bad:
    print("REGRESSION:", b)
sys.exit(1 if bad else 0)
