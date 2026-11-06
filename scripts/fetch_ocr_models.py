"""S2-03 spike: fetch the PP-OCRv5 files the OCR spike reads, from PaddlePaddle's own repositories on
Hugging Face at pinned commits, check them against pinned SHA-256 hashes, and write them to
`models/build/` with the recogniser's dictionary as a plain list (one character per line).

    uv run --quiet --no-project --with pyyaml python -I scripts/fetch_ocr_models.py [models-dir]

The files are Apache-2.0 (PaddlePaddle/PaddleOCR). Nothing here goes into the app yet: when the spike
passes its gate, the files join `cargo xtask models` and the hashes compiled into the app.
"""
import hashlib
import pathlib
import sys
import urllib.request

import yaml

DET = ("PaddlePaddle/PP-OCRv5_mobile_det_onnx", "e6f4fa85f00e168c862bc462aebca69eef9b3d3d")
REC = ("PaddlePaddle/latin_PP-OCRv5_mobile_rec_onnx", "89d3a50e2c27e2e7cceeab0e944c25c807d5db4f")

# (repository, file in it, SHA-256, name in models/build/)
FILES = [
    (DET, "inference.onnx", "a431985659dc921974177a95adcfbb90fd9e51989a5e04d70d0b75f597b6e61d", "ocr-det-ppocrv5-mobile.onnx"),
    (REC, "inference.onnx", "7888113072263cb471b93f66dd5e2ad70548dc526fa1ace760d0d973dd121498", "ocr-rec-ppocrv5-latin.onnx"),
    (REC, "inference.yml", "0bbe984570f597af3638e50bdf2e8276f3ab26a61966096538b3b0d1849f5c84", None),
]


def main() -> None:
    root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "models")
    src, build = root / "src" / "ocr", root / "build"
    src.mkdir(parents=True, exist_ok=True)
    build.mkdir(parents=True, exist_ok=True)
    for (repo, commit), name, sha, out in FILES:
        path = src / f"{repo.split('/')[1]}-{name}"
        if not path.exists():
            url = f"https://huggingface.co/{repo}/resolve/{commit}/{name}"
            print("fetching", url)
            part = path.with_suffix(path.suffix + ".part")
            urllib.request.urlretrieve(url, part)
            part.rename(path)
        got = hashlib.sha256(path.read_bytes()).hexdigest()
        if got != sha:
            sys.exit(f"{path} has SHA-256 {got}, expected {sha}: delete it to fetch again, or update the pin on purpose")
        if out:
            (build / out).write_bytes(path.read_bytes())
            print(f"{out}: {path.stat().st_size} bytes")
    yml = yaml.safe_load((src / f"{REC[0].split('/')[1]}-inference.yml").read_text())
    chars = yml["PostProcess"]["character_dict"]
    assert all(len(c) == 1 and "\n" not in c for c in chars), "a dictionary entry is not one character"
    (build / "ocr-dict-ppocrv5-latin.txt").write_text("\n".join(chars) + "\n", encoding="utf-8")
    print(f"ocr-dict-ppocrv5-latin.txt: {len(chars)} characters")


main()
