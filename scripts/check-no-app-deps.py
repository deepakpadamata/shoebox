#!/usr/bin/env python3
"""Fail if any non-app workspace crate depends (even transitively) on an `apps/*` crate.

Apps are AGPL-3.0; libraries are Apache-2.0 OR MIT, so the dependency edge must never point
from a library to an app (decision #17). Usage: check-no-app-deps.py [metadata.json]
(reads `cargo metadata` output from a file, or runs cargo itself).
"""
import json
import subprocess
import sys


def load(path):
    if path:
        with open(path) as f:
            return json.load(f)
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        check=True, capture_output=True, text=True,
    ).stdout
    return json.loads(out)


def violations(meta):
    root = meta["workspace_root"].rstrip("/") + "/"
    members = set(meta["workspace_members"])
    pkgs = {p["id"]: p for p in meta["packages"]}

    def rel(pid):
        return pkgs[pid]["manifest_path"][len(root):] if pkgs[pid]["manifest_path"].startswith(root) else ""

    is_app = {pid: rel(pid).startswith("apps/") for pid in members}
    graph = {n["id"]: [d["pkg"] for d in n["deps"]] for n in meta["resolve"]["nodes"]}
    bad = []
    for pid in members:
        if is_app[pid]:
            continue
        seen, stack = set(), list(graph.get(pid, []))
        while stack:
            cur = stack.pop()
            if cur in seen:
                continue
            seen.add(cur)
            if cur in is_app and is_app[cur]:
                bad.append((pkgs[pid]["name"], pkgs[cur]["name"]))
            stack.extend(graph.get(cur, []))
    return sorted(set(bad))


def main():
    bad = violations(load(sys.argv[1] if len(sys.argv) > 1 else None))
    for lib, app in bad:
        print(f"error: library crate `{lib}` depends on app crate `{app}` (AGPL)", file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
