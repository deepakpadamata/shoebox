#!/usr/bin/env python3
"""Bulk-create GitHub labels, milestones and issues from planning/issues/*.md (and optionally planning/epics/V2-*.md).

Dry run by default: prints what it would do and touches nothing.

    python3 planning/scripts/import_github.py --repo OWNER/NAME            # dry run
    python3 planning/scripts/import_github.py --repo OWNER/NAME --apply    # create for real
    python3 planning/scripts/import_github.py --repo OWNER/NAME --apply --include-epics

Requires the `gh` CLI, authenticated with write access to the repo. It's idempotent: created issue numbers
are recorded in planning/scripts/github-map.json and skipped on re-runs. After creation, a second pass
rewrites each issue's dependency footer so "M1-03" becomes "#12".
"""
import argparse, json, os, re, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MAP_FILE = os.path.join(ROOT, "scripts", "github-map.json")
LABEL_COLORS = {"area": "c8691c", "type": "2f6db5", "epic": "7a5bb5", "hosted": "b54a6a"}


def parse(path):
    text = open(path).read()
    m = re.match(r"---\n(.*?)\n---\n\n?(.*)", text, re.S)
    if not m:
        sys.exit(f"no front-matter in {path}")
    meta = {}
    for line in m.group(1).splitlines():
        k, _, v = line.partition(":")
        v = v.strip()
        if v.startswith("[") and v.endswith("]"):
            v = [x.strip() for x in v[1:-1].split(",") if x.strip()]
        meta[k.strip()] = v
    return meta, m.group(2)


def gh(args, apply, capture=False):
    print(("  $ " if apply else "  (dry) ") + "gh " + " ".join(args))
    if not apply:
        return ""
    r = subprocess.run(["gh"] + args, check=True, text=True, capture_output=capture)
    return r.stdout if capture else ""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True)
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--include-epics", action="store_true")
    a = ap.parse_args()

    files = sorted(os.path.join(ROOT, "issues", f) for f in os.listdir(os.path.join(ROOT, "issues")) if f.endswith(".md"))
    if a.include_epics:
        files += sorted(os.path.join(ROOT, "epics", f) for f in os.listdir(os.path.join(ROOT, "epics")) if f.startswith("V2-"))
    items = [parse(f) for f in files]

    mapping = json.load(open(MAP_FILE)) if os.path.exists(MAP_FILE) else {}

    print("Labels")
    labels = sorted({l for m, _ in items for l in m.get("labels", [])})
    for l in labels:
        color = LABEL_COLORS.get(l.split(":")[0], "9a968e")
        gh(["label", "create", l, "--repo", a.repo, "--color", color, "--force"], a.apply)

    print("Milestones")
    existing = set()
    if a.apply:
        out = gh(["api", f"repos/{a.repo}/milestones?state=all&per_page=100", "--jq", ".[].title"], True, capture=True)
        existing = set(out.split("\n"))
    for ms in sorted({m["milestone"] for m, _ in items}):
        if ms not in existing:
            gh(["api", f"repos/{a.repo}/milestones", "-f", f"title={ms}"], a.apply)

    print("Issues")
    for meta, body in items:
        iid = meta["id"]
        if iid in mapping:
            print(f"  skip {iid} (#{mapping[iid]})")
            continue
        args = ["issue", "create", "--repo", a.repo, "--title", f"{iid} · {meta['title']}", "--milestone", meta["milestone"]]
        for l in meta.get("labels", []):
            args += ["--label", l]
        with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as t:
            t.write(body)
        out = gh(args + ["--body-file", t.name], a.apply, capture=True)
        os.unlink(t.name)
        if a.apply:
            mapping[iid] = int(out.strip().rstrip("/").split("/")[-1])
            json.dump(mapping, open(MAP_FILE, "w"), indent=2)

    print("Dependency links")
    for meta, body in items:
        deps = meta.get("depends_on", [])
        if not deps or meta["id"] not in mapping and a.apply:
            continue
        linked = body
        for d in deps:
            if d in mapping:
                linked = re.sub(rf"(_Depends on: [^\n]*?)\b{re.escape(d)}\b", rf"\g<1>{d} (#{mapping[d]})", linked)
        if linked != body or not a.apply:
            with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as t:
                t.write(linked)
            gh(["issue", "edit", str(mapping.get(meta["id"], "<new>")), "--repo", a.repo, "--body-file", t.name], a.apply)
            os.unlink(t.name)

    if not a.apply:
        print("\nDry run only. Re-run with --apply to create everything.")


if __name__ == "__main__":
    main()
