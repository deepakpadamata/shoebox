#!/usr/bin/env python3
"""Writes site/downloads/release.json from a directory of release files, so the download page only
links to files that exist (M7-02).

    python3 site/release.py <version> <dist dir> <base url>

It computes each file's SHA-256, writes release.json, and (with --verify) fetches every URL and
compares the checksum, which is the acceptance check for "all download links verified".
"""
import hashlib
import json
import pathlib
import sys
import urllib.request

def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if len(args) != 3:
        sys.exit(__doc__)
    version, dist, base = args[0], pathlib.Path(args[1]), args[2].rstrip("/")
    files = []
    for f in sorted(dist.iterdir()):
        if f.is_file():
            files.append({"name": f.name, "url": f"{base}/{f.name}", "sha256": hashlib.sha256(f.read_bytes()).hexdigest()})
    out = pathlib.Path(__file__).resolve().parent / "downloads" / "release.json"
    out.write_text(json.dumps({"version": version, "files": files}, indent=2))
    print(f"wrote {out} with {len(files)} files")
    if "--verify" in sys.argv:
        for f in files:
            data = urllib.request.urlopen(f["url"], timeout=60).read()
            ok = hashlib.sha256(data).hexdigest() == f["sha256"]
            print(("ok  " if ok else "BAD ") + f["url"])
            if not ok:
                sys.exit(1)

if __name__ == "__main__":
    main()
