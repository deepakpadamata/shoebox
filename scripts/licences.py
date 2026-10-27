#!/usr/bin/env python3
"""Write the third-party licence notices each shipped app bundles (R1-05).

    python3 scripts/licences.py           # regenerate
    python3 scripts/licences.py --check   # fail if a generated file is out of date (CI)

For each app, the Rust crates it depends on (normal and build dependencies, all platforms, from
`cargo metadata`) and, for the desktop/mobile app, the npm packages its UI ships (production
entries of `package-lock.json`). Each file lists every package with its licence, then the distinct
licence and notice texts those packages carry (LICENSE*, LICENCE*, COPYING*, NOTICE*), each once.
Our own workspace crates are left out: their licences are in the repository root and apps/.

Needs the crate sources (`cargo fetch`) and the UI's node_modules (`npm ci`).
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
APPS = {
    # package name in Cargo.toml -> (output file, UI directory or None)
    "keeper": ("apps/keeper/THIRD-PARTY-LICENSES.txt", None),
    "desktop-mobile": ("apps/desktop-mobile/THIRD-PARTY-LICENSES.txt", "apps/desktop-mobile/ui"),
}
NOTICE = re.compile(r"^(licen[cs]e|copying|notice)([-_.].*)?$", re.I)


def notice_files(directory):
    """The licence and notice files at the top of a package directory, sorted."""
    try:
        names = sorted(os.listdir(directory))
    except OSError:
        return []
    return [os.path.join(directory, n) for n in names if NOTICE.match(n) and os.path.isfile(os.path.join(directory, n))]


def read(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read().strip()


# The features each app ships with: the shell and its plugins are optional in the app crate.
SHIPPED_FEATURES = {"desktop-mobile": ["desktop-mobile/tauri-app"]}


def rust_packages(app):
    features = SHIPPED_FEATURES.get(app, [])
    args = ["--features", ",".join(features)] if features else []
    meta = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--locked", *args], cwd=ROOT))
    by_id = {p["id"]: p for p in meta["packages"]}
    members = set(meta["workspace_members"])
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = next(i for i in members if by_id[i]["name"] == app)
    seen, stack = set(), [root]
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            kinds = {k["kind"] for k in dep["dep_kinds"]}
            if kinds - {"dev"}:  # normal (None) or build dependencies ship or build the app
                stack.append(dep["pkg"])
    out = []
    for pid in seen - members:
        p = by_id[pid]
        out.append({
            "name": p["name"],
            "version": p["version"],
            "license": p.get("license") or ("see " + p["license_file"] if p.get("license_file") else "UNKNOWN"),
            "dir": os.path.dirname(p["manifest_path"]),
            "authors": ", ".join(p.get("authors") or []),
        })
    return sorted(out, key=lambda p: (p["name"], p["version"]))


def npm_packages(ui_dir):
    lock = json.load(open(os.path.join(ROOT, ui_dir, "package-lock.json")))
    out = []
    for path, info in lock.get("packages", {}).items():
        if not path or info.get("dev") or info.get("devOptional"):
            continue
        directory = os.path.join(ROOT, ui_dir, path)
        if not os.path.isdir(directory):
            sys.exit(f"{directory} is missing: run `npm ci` in {ui_dir} first")
        name = path.split("node_modules/")[-1]
        pkg = json.load(open(os.path.join(directory, "package.json")))
        author = pkg.get("author")
        out.append({
            "name": name,
            "version": info.get("version", "?"),
            "license": info.get("license") or "UNKNOWN",
            "dir": directory,
            "authors": author.get("name", "") if isinstance(author, dict) else (author or ""),
        })
    return sorted(out, key=lambda p: (p["name"], p["version"]))


def _mit():
    body = read(os.path.join(ROOT, "LICENSE-MIT")).split("\n", 1)[1].strip()
    return "Copyright (c) the authors named for the package above\n\n" + body


STANDARD = {
    "Apache-2.0": lambda: read(os.path.join(ROOT, "LICENSE-APACHE")),
    "MIT": _mit,
    "MPL-2.0": lambda: (
        "Mozilla Public License 2.0: https://mozilla.org/MPL/2.0/\n"
        "The source of each MPL-2.0 package is the package itself, published on crates.io under "
        "the name and version listed above; Shoebox uses it unmodified."
    ),
}


def standard_text_for(expression):
    """Which standard text covers a package whose licence is `expression`: one of the licences
    it may be used under, preferring MIT, then Apache-2.0. None when no notice is required
    (public-domain style licences) or it is unknown."""
    ids = set(re.findall(r"[A-Za-z0-9.+-]+", expression.replace("/", " OR ")))
    for name in ("MIT", "Apache-2.0", "MPL-2.0"):
        if name in ids:
            return name
    return None


def render(app, rust, npm):
    lines = [
        f"Third-party software in Shoebox ({app})",
        "",
        "Generated by scripts/licences.py; do not edit. Shoebox itself: apps are AGPL-3.0-only",
        "(apps/LICENSE-AGPL), libraries Apache-2.0 OR MIT (LICENSE-APACHE, LICENSE-MIT).",
        "",
    ]
    texts = {}  # hash -> (text, [packages])
    used_standard = set()
    for title, packages in (("Rust crates", rust), ("JavaScript packages (the user interface)", npm)):
        if not packages:
            continue
        lines += [f"== {title} ({len(packages)}) ==", ""]
        for p in packages:
            refs = []
            for f in notice_files(p["dir"]):
                text = read(f)
                if not text:
                    continue
                h = hashlib.sha256(text.encode()).hexdigest()[:12]
                texts.setdefault(h, (text, []))[1].append(f"{p['name']} {p['version']}")
                refs.append(h)
            if refs:
                ref = f"  [{', '.join(refs)}]"
            else:
                # No file in the package: its declared licence's standard text (below) applies,
                # copyright its authors.
                standard = standard_text_for(p["license"])
                if standard:
                    used_standard.add(standard)
                by = f"; copyright {p['authors']}" if p["authors"] else ""
                if standard:
                    ref = f"  [standard {standard} text{by}]"
                elif re.search(r"CC0|Unlicense", p["license"]):
                    ref = "  [public-domain dedication: no notice required]"
                else:
                    ref = f"  [no licence text found{by}]"
            lines.append(f"{p['name']} {p['version']}: {p['license']}{ref}")
        lines.append("")
    lines += ["== Standard licence texts ==", ""]
    for name in sorted(used_standard):
        lines += [f"--- [standard {name}]", "", STANDARD[name](), ""]
    lines += ["== Licence and notice texts shipped with the packages ==", ""]
    for h, (text, users) in sorted(texts.items(), key=lambda kv: (kv[1][1][0], kv[0])):
        lines += [f"--- [{h}] used by: {', '.join(sorted(set(users)))}", "", text, ""]
    return "\n".join(lines) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    a = ap.parse_args()
    stale = []
    for app, (out, ui) in APPS.items():
        content = render(app, rust_packages(app), npm_packages(ui) if ui else [])
        path = os.path.join(ROOT, out)
        current = open(path, encoding="utf-8").read() if os.path.exists(path) else None
        if a.check:
            if current != content:
                stale.append(out)
        else:
            with open(path, "w", encoding="utf-8") as f:
                f.write(content)
            print(f"wrote {out} ({len(content) // 1024} KiB)")
    if stale:
        sys.exit("out of date (run python3 scripts/licences.py): " + ", ".join(stale))


if __name__ == "__main__":
    main()
