"""Write the character list of a PaddleOCR recogniser, one character per line, from its inference.yml.

    uv run --quiet --no-project --with pyyaml==6.0.2 python -I scripts/ocr_dict.py <inference.yml> <out.txt>

Used by `cargo xtask models` (S2-03). The output is deterministic: UTF-8, "\\n" line ends, one trailing newline.
"""
import pathlib
import sys

import yaml

yml = yaml.safe_load(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
chars = yml["PostProcess"]["character_dict"]
assert all(len(c) == 1 and "\n" not in c for c in chars), "a dictionary entry is not one character"
pathlib.Path(sys.argv[2]).write_bytes(("\n".join(chars) + "\n").encode("utf-8"))
print(f"{sys.argv[2]}: {len(chars)} characters")
