#!/usr/bin/env python3
"""How much of each library's blob store is previews, by kind: AVIF previews, the older JPEG
previews, JPEG thumbnails and everything else (originals). Shows how far the conversion of old
previews to AVIF (A-150) has got.

    scripts/preview-storage.py                       # this Mac's app library
    scripts/preview-storage.py user@host:/srv/shoebox ~/Library/...  # any mix of local and ssh

A library is a keeper's `--storage` folder or the app's data folder (the one holding `node/`).
Remote libraries are read over ssh with the remote's python3; nothing is installed there.

Only headers are read, and only of files under 4 MB (anything larger is an original). A file is a
Shoebox-made JPEG when it has no Exif: up to 640 px on its long side a thumbnail, up to 2048 px a
preview. An original without Exif that small (a screenshot, a messaging app's copy) counts as a
preview, so the JPEG figures are a little high. Blobs under 16 KB live inside `blobs.db`, not as
files, and are not counted (most thumbnails).

The first run reads every small file (on a keeper's spinning disks, about 15 minutes for 75,000
blobs); what it learns is cached in ~/.cache/shoebox on the machine that holds the library, so
later runs read only the blobs added since."""

import hashlib
import os
import struct
import subprocess
import sys

MAX_RENDITION = 4 << 20
KINDS = ["avif preview", "jpeg preview", "jpeg thumb", "original"]
DEFAULT = "~/Library/Application Support/org.shoebox.desktop"


def jpeg_edge_and_exif(f):
    f.seek(2)
    exif = False
    while True:
        b = f.read(4)
        if len(b) < 4 or b[0] != 0xFF:
            return None, exif
        marker, length = b[1], struct.unpack(">H", b[2:4])[0]
        if marker == 0xE1:
            exif = exif or f.read(6).startswith(b"Exif")
            f.seek(length - 2 - 6, 1)
        elif marker in (0xC0, 0xC1, 0xC2):
            h, w = struct.unpack(">xHH", f.read(5))
            return max(w, h), exif
        else:
            f.seek(length - 2, 1)


def kind_of(path, size):
    if size > MAX_RENDITION:
        return "original"
    with open(path, "rb") as f:
        head = f.read(12)
        if head[4:12] in (b"ftypavif", b"ftypavis"):
            return "avif preview"
        if head[:2] != b"\xff\xd8":
            return "original"
        edge, exif = jpeg_edge_and_exif(f)
    if exif or edge is None or edge > 2048:
        return "original"
    return "jpeg thumb" if edge <= 640 else "jpeg preview"


def blob_dir(library):
    library = os.path.expanduser(library)
    for d in (os.path.join(library, "node", "blobs", "data"), os.path.join(library, "blobs", "data")):
        if os.path.isdir(d):
            return d
    sys.exit(f"no blob store under {library}")


def cache_path(d):
    key = hashlib.sha256(os.path.realpath(d).encode()).hexdigest()[:16]
    base = os.environ.get("XDG_CACHE_HOME") or os.path.expanduser("~/.cache")
    return os.path.join(base, "shoebox", f"preview-storage-{key}.tsv")


def measure(library):
    """Count one local library: prints `kind files bytes` lines. A blob's name is its hash, so
    its kind and size never change: they are cached, and a later run reads only new files."""
    d = blob_dir(library)
    cache_file = cache_path(d)
    cache = {}
    try:
        with open(cache_file) as f:
            for line in f:
                name, kind, size = line.rstrip("\n").split("\t")
                cache[name] = (kind, int(size))
    except (OSError, ValueError):
        cache = {}
    seen = {}
    with os.scandir(d) as it:
        for e in it:
            if not e.name.endswith(".data"):
                continue
            hit = cache.get(e.name)
            if hit is None:
                size = e.stat().st_size
                hit = (kind_of(e.path, size), size)
            seen[e.name] = hit
    try:
        os.makedirs(os.path.dirname(cache_file), exist_ok=True)
        tmp = cache_file + ".tmp"
        with open(tmp, "w") as f:
            f.writelines(f"{n}\t{k}\t{s}\n" for n, (k, s) in seen.items())
        os.replace(tmp, cache_file)
    except OSError:
        pass
    totals = {k: [0, 0] for k in KINDS}
    for kind, size in seen.values():
        totals[kind][0] += 1
        totals[kind][1] += size
    for k in KINDS:
        print(k.replace(" ", "_"), *totals[k])


def collect(library):
    if ":" in library and not os.path.exists(os.path.expanduser(library)):
        host, path = library.split(":", 1)
        with open(os.path.abspath(__file__), "rb") as src:
            out = subprocess.run(
                ["ssh", "-o", "BatchMode=yes", host, "python3", "-I", "-", "--measure", path],
                stdin=src, capture_output=True, text=True,
            )
    else:
        out = subprocess.run(
            [sys.executable, "-I", os.path.abspath(__file__), "--measure", library],
            capture_output=True, text=True,
        )
    if out.returncode != 0:
        return None, out.stderr.strip() or f"exit {out.returncode}"
    rows = {}
    for line in out.stdout.splitlines():
        k, n, b = line.split()
        rows[k.replace("_", " ")] = (int(n), int(b))
    return rows, None


def gb(b):
    return f"{b / 1e9:.2f} GB"


def main(args):
    if args[:1] == ["--measure"]:
        measure(args[1])
        return
    libraries = args or [DEFAULT]
    header = ["Library", "AVIF previews", "JPEG previews", "Converted", "Thumbs", "Originals"]
    table = []
    from concurrent.futures import ThreadPoolExecutor

    with ThreadPoolExecutor() as pool:
        results = list(pool.map(collect, libraries))
    for lib, (rows, err) in zip(libraries, results):
        name = lib if len(lib) < 40 else "…" + lib[-39:]
        if err:
            table.append([name, f"error: {err}", "", "", "", ""])
            continue
        a, j = rows["avif preview"], rows["jpeg preview"]
        done = a[0] / (a[0] + j[0]) if a[0] + j[0] else 1.0
        cell = lambda r: f"{r[0]:,} · {gb(r[1])}"
        table.append([name, cell(a), cell(j), f"{done:.0%}", cell(rows["jpeg thumb"]), cell(rows["original"])])
    widths = [max(len(r[i]) for r in [header] + table) for i in range(len(header))]
    line = lambda r: "  ".join(c.ljust(w) if i == 0 else c.rjust(w) for i, (c, w) in enumerate(zip(r, widths)))
    print(line(header))
    print("  ".join("-" * w for w in widths))
    for r in table:
        print(line(r))


if __name__ == "__main__":
    main(sys.argv[1:])
