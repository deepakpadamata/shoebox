"""S2-03 spike: test images with known text, a reference reader, and the comparison.

    # 1. about 50 images (photos of receipts, signs, screenshots, pages, posters) and what they say
    uv run --quiet --no-project --with pillow --with numpy python -I scripts/ocr_spike_eval.py make <out-dir>
    # 2. the reference: RapidOCR (a port of PaddleOCR's pipeline on onnxruntime) running the same
    #    PP-OCRv5 files with PaddleOCR's own settings for them (`inference.yml`)
    uv run --quiet --no-project --with rapidocr --with onnxruntime --with pyyaml --with pillow \
        python -I scripts/ocr_spike_eval.py reference <image-dir> <models/build> <reference.jsonl>
    # 3. what the tract spike printed (cargo run --release -p shoebox-ml --example ocr_check), scored
    python3 -I scripts/ocr_spike_eval.py compare <image-dir> <tract.jsonl> <reference.jsonl>

The images are made here and stay untracked: their text is known, so both readers are scored against
the truth (per-line character error) and against each other.
"""
import json
import math
import pathlib
import random
import sys

FONTS = {
    "sans": "/System/Library/Fonts/Helvetica.ttc",
    "arial": "/System/Library/Fonts/Supplemental/Arial.ttf",
    "arial-bold": "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
    "mono": "/System/Library/Fonts/Supplemental/Courier New.ttf",
    "serif": "/System/Library/Fonts/Supplemental/Georgia.ttf",
    "verdana": "/System/Library/Fonts/Supplemental/Verdana.ttf",
}

WORDS = (
    "market coffee station bakery central museum garden harbour river street tickets platform exit "
    "welcome open closed pharmacy bridge library cinema airport hotel lunch dinner menu today "
    "Café Straße Müller Österreich señor mañana crème brûlée Zürich Málaga Köln façade naïve "
    "Total Subtotal Tax Cash Change Thank you Invoice Order Receipt Delivery Account Balance"
).split()
PHRASES = [
    "Opening hours 9:00 - 18:30",
    "Please keep your ticket",
    "No entry without a mask",
    "Rue de la Paix 12, 75002 Paris",
    "Hauptstraße 45, 80331 München",
    "www.example-shop.com/offers",
    "info@harbour-cafe.org",
    "Order #48213 shipped on 2026-03-14",
    "Battery 87% - 4G - 14:52",
    "Meeting moved to Thursday at 3 pm",
    "Gracias por su visita",
    "Prix net 24,90 EUR TVA incluse",
    "Flight LH 4417 gate B22 boarding",
    "Next train to Zürich HB in 6 min",
    "Wi-Fi password: blue-harbor-2026",
]


def make(out: pathlib.Path) -> None:
    from PIL import Image, ImageDraw, ImageFilter, ImageFont
    import numpy as np

    rnd = random.Random(7)
    out.mkdir(parents=True, exist_ok=True)
    truth = {}

    def font(name, size):
        return ImageFont.truetype(FONTS[name], size)

    def noise_bg(w, h, base, spread=30):
        arr = np.zeros((h, w, 3), dtype=np.float32)
        for c in range(3):
            small = np.random.RandomState(rnd.randrange(1 << 30)).rand(h // 64 + 2, w // 64 + 2)
            img = Image.fromarray((small * 255).astype("uint8")).resize((w, h), Image.BICUBIC)
            arr[..., c] = base[c] + (np.asarray(img, dtype=np.float32) / 255 - 0.5) * 2 * spread
        arr += np.random.RandomState(rnd.randrange(1 << 30)).randn(h, w, 3) * 4
        return Image.fromarray(np.clip(arr, 0, 255).astype("uint8"))

    def save(name, img, lines, quality=82, angle=0.0):
        if angle:
            img = img.rotate(angle, resample=Image.BICUBIC, expand=False, fillcolor=(120, 120, 120))
        img.save(out / f"{name}.jpg", quality=quality)
        truth[f"{name}.jpg"] = lines

    def sentence(n):
        return " ".join(rnd.choice(WORDS) for _ in range(n))

    # receipts: monospace lines on paper on a table
    for i in range(10):
        w, h = 1536, 2048
        img = noise_bg(w, h, (110, 95, 80), 25)
        d = ImageDraw.Draw(img)
        pw = rnd.randint(620, 800)
        x0 = (w - pw) // 2
        d.rectangle([x0, 120, x0 + pw, h - 160], fill=(245, 243, 236))
        f = font("mono", rnd.randint(30, 38))
        lines = [rnd.choice(["CORNER SHOP", "Café Central", "Bäckerei Müller", "Mercado Sur"])]
        for _ in range(rnd.randint(5, 9)):
            lines.append(f"{rnd.randint(1, 3)} x {rnd.choice(WORDS)} {rnd.randint(1, 19)}.{rnd.randint(0, 99):02d}")
        lines.append(f"TOTAL {rnd.randint(10, 99)}.{rnd.randint(0, 99):02d}")
        lines.append(rnd.choice(["Thank you!", "Danke und bis bald", "Merci de votre visite"]))
        y = 200
        for ln in lines:
            d.text((x0 + 40, y), ln, fill=(30, 30, 30), font=f)
            y += int(f.size * 1.9)
        save(f"receipt-{i:02d}", img, lines, angle=rnd.uniform(-2.5, 2.5))

    # signs: a few large words on a coloured board
    for i in range(10):
        w, h = 2048, 1536
        col = rnd.choice([(20, 90, 160), (190, 40, 40), (30, 120, 60), (230, 200, 40), (240, 240, 240)])
        img = noise_bg(w, h, col, 22)
        d = ImageDraw.Draw(img)
        fg = (255, 255, 255) if sum(col) < 450 else (20, 20, 20)
        lines = [rnd.choice(WORDS).upper(), rnd.choice(PHRASES)]
        f1, f2 = font("arial-bold", rnd.randint(220, 300)), font("arial", rnd.randint(70, 100))
        d.text((120, 300), lines[0], fill=fg, font=f1)
        d.text((120, 800), lines[1], fill=fg, font=f2)
        save(f"sign-{i:02d}", img, lines, quality=70, angle=rnd.uniform(-3, 3))

    # screenshots of a phone: small UI text, light or dark
    for i in range(10):
        w, h = 1170, 2532
        dark = i % 2 == 0
        img = Image.new("RGB", (w, h), (18, 18, 20) if dark else (250, 250, 252))
        d = ImageDraw.Draw(img)
        fg, dim = ((235, 235, 240), (150, 150, 160)) if dark else ((20, 20, 24), (110, 110, 120))
        lines = []
        y = 180
        title = sentence(2).title()
        d.text((60, y), title, fill=fg, font=font("sans", 64))
        lines.append(title)
        y += 150
        for _ in range(9):
            a, b = rnd.choice(PHRASES), sentence(rnd.randint(3, 5))
            d.text((60, y), a, fill=fg, font=font("sans", 40))
            d.text((60, y + 62), b, fill=dim, font=font("sans", 32))
            lines += [a, b]
            y += 200
        save(f"screenshot-{i:02d}", img, lines, quality=92)

    # pages: a few paragraphs of serif text
    for i in range(10):
        w, h = 1536, 2048
        img = noise_bg(w, h, (238, 234, 224), 8)
        d = ImageDraw.Draw(img)
        f = font("serif", rnd.randint(34, 42))
        lines = []
        y = 160
        for _ in range(rnd.randint(14, 18)):
            ln = sentence(rnd.randint(4, 6))
            if rnd.random() < 0.3:
                ln = rnd.choice(PHRASES)
            d.text((130, y), ln, fill=(35, 30, 28), font=f)
            lines.append(ln)
            y += int(f.size * 1.55)
        save(f"page-{i:02d}", img, lines, quality=85, angle=rnd.uniform(-1.5, 1.5))

    # posters and stickers: words of different size and colour on a textured background
    for i in range(10):
        w, h = 2048, 1536
        img = noise_bg(w, h, (rnd.randint(60, 200), rnd.randint(60, 200), rnd.randint(60, 200)), 50).filter(
            ImageFilter.GaussianBlur(2)
        )
        d = ImageDraw.Draw(img)
        lines = []
        y = 160
        for _ in range(3):
            ln = rnd.choice([rnd.choice(WORDS), sentence(2), rnd.choice(PHRASES)])
            f = font(rnd.choice(["arial-bold", "verdana", "serif"]), rnd.randint(90, 170))
            d.text((150, y), ln, fill=rnd.choice([(255, 255, 255), (15, 15, 15), (255, 230, 80)]), font=f)
            lines.append(ln)
            y += int(f.size * 1.9)
        save(f"poster-{i:02d}", img, lines, quality=60, angle=rnd.uniform(-5, 5))

    (out / "truth.json").write_text(json.dumps(truth, ensure_ascii=False, indent=1))
    print(f"{len(truth)} images and truth.json in {out}")


def reference(images: pathlib.Path, models: pathlib.Path, out: pathlib.Path) -> None:
    import time

    from rapidocr import RapidOCR

    engine = RapidOCR(
        params={
            "Global.use_cls": False,
            "Global.log_level": "error",
            "Global.use_preprocess_img": False,
            "Det.model_path": str(models / "ocr-det-ppocrv5-mobile.onnx"),
            "Det.limit_side_len": 960,
            "Det.limit_type": "max",
            "Det.mean": [0.485, 0.456, 0.406],
            "Det.std": [0.229, 0.224, 0.225],
            "Det.thresh": 0.3,
            "Det.box_thresh": 0.6,
            "Det.unclip_ratio": 1.5,
            "Det.use_dilation": False,
            "Rec.model_path": str(models / "ocr-rec-ppocrv5-latin.onnx"),
            "Rec.rec_keys_path": str(models / "ocr-dict-ppocrv5-latin.txt"),
            "Global.text_score": 0.0,
        }
    )
    with out.open("w") as f:
        for p in sorted(images.glob("*.jpg")) + sorted(images.glob("*.png")):
            t = time.time()
            r = engine(str(p))
            ms = (time.time() - t) * 1e3
            texts = list(r.txts or [])
            scores = [float(s) for s in (r.scores or [])]
            f.write(json.dumps({"file": p.name, "total_ms": round(ms), "lines": [{"text": a, "score": b} for a, b in zip(texts, scores)]}, ensure_ascii=False) + "\n")
    print("reference written to", out)


def lev(a: str, b: str) -> int:
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1]


def norm(s: str) -> str:
    return " ".join(s.split())


def line_errors(truth_lines, pred_lines):
    """Per truth line: the lowest character error of any predicted line, or of two or three in a row joined."""
    preds = [norm(p) for p in pred_lines]
    # A line split by the detector at a wide gap ("2 x Coffee" ... "7.00") still counts as read.
    preds += [" ".join(preds[i : i + n]) for n in (2, 3) for i in range(len(preds) - n + 1)]
    out = []
    for t in truth_lines:
        t = norm(t)
        best = min((lev(t, p) / max(1, len(t)) for p in preds), default=1.0)
        out.append(min(1.0, best))
    return out


def compare(images: pathlib.Path, tract: pathlib.Path, ref: pathlib.Path) -> None:
    truth = json.loads((images / "truth.json").read_text())
    read = lambda p: {json.loads(l)["file"]: json.loads(l) for l in p.read_text().splitlines() if l.startswith("{")}
    tr, rf = read(tract), read(ref)
    cats = {}
    tot = {"tract": [], "ref": [], "agree": []}
    for name, lines in truth.items():
        cat = name.split("-")[0]
        a = [l["text"] for l in tr[name]["lines"]]
        b = [l["text"] for l in rf[name]["lines"]]
        ea, eb = line_errors(lines, a), line_errors(lines, b)
        # Agreement between the two readers: each reference line against the best tract line.
        ag = line_errors(b, a) if b else [0.0]
        c = cats.setdefault(cat, {"tract": [], "ref": [], "agree": []})
        for k, v in (("tract", ea), ("ref", eb), ("agree", ag)):
            c[k] += v
            tot[k] += v
    mean = lambda v: sum(v) / len(v) if v else float("nan")
    ok = lambda v: sum(1 for x in v if x <= 0.1) / len(v) if v else float("nan")
    print(f"{'':12} {'lines':>6} | {'tract CER':>9} {'lines ok':>8} | {'ref CER':>8} {'lines ok':>8} | {'tract vs ref CER':>16}")
    for cat, c in [*sorted(cats.items()), ("all", tot)]:
        print(f"{cat:12} {len(c['tract']):6} | {mean(c['tract']):9.3f} {ok(c['tract']):8.2f} | {mean(c['ref']):8.3f} {ok(c['ref']):8.2f} | {mean(c['agree']):16.3f}")
    print("(CER: per-line character error of the best-matching line read; 'lines ok': share of lines read with at most 10% error)")


if __name__ == "__main__":
    cmd, args = sys.argv[1], sys.argv[2:]
    if cmd == "make":
        make(pathlib.Path(args[0]))
    elif cmd == "reference":
        reference(pathlib.Path(args[0]), pathlib.Path(args[1]), pathlib.Path(args[2]))
    elif cmd == "compare":
        compare(*(pathlib.Path(a) for a in args))
    else:
        sys.exit(__doc__)
