"""Fetch the search eval set (S1-01): the first N photos of the COCO Karpathy test split, with
their five captions each, into fixtures-out/search-eval/ (gitignored; COCO photos are under their
Flickr owners' licences, so they are never committed or shipped).

    python3 -I tools/search-eval/fetch.py [N]
"""
import json, os, sys, urllib.request, concurrent.futures

N = int(sys.argv[1]) if len(sys.argv) > 1 else 1000
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "fixtures-out", "search-eval")
API = "https://datasets-server.huggingface.co/rows?dataset=yerevann/coco-karpathy&config=default&split=test"

os.makedirs(os.path.join(OUT, "photos"), exist_ok=True)
rows = []
while len(rows) < N:
    with urllib.request.urlopen(f"{API}&offset={len(rows)}&length={min(100, N - len(rows))}", timeout=60) as r:
        batch = json.load(r)["rows"]
    if not batch:
        break
    rows += [b["row"] for b in batch]

def get(row):
    path = os.path.join(OUT, "photos", row["filename"])
    if not os.path.exists(path):
        with urllib.request.urlopen(row["url"], timeout=60) as r:
            data = r.read()
        with open(path + ".part", "wb") as f:
            f.write(data)
        os.replace(path + ".part", path)
    return {"file": row["filename"], "captions": row["sentences"]}

with concurrent.futures.ThreadPoolExecutor(8) as ex:
    items = list(ex.map(get, rows))
with open(os.path.join(OUT, "captions.json"), "w") as f:
    json.dump(items, f, indent=0)
print(f"{len(items)} photos in {OUT}")
