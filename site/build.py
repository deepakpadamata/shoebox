#!/usr/bin/env python3
"""Builds the static site: turns docs/guides/*.md into site/guides/*.html (no dependencies).

    python3 site/build.py            # writes site/guides/*.html
    python3 site/build.py --check    # verifies that every local link and asset exists
"""
import html
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent
GUIDES = ROOT.parent / "docs" / "guides"
OUT = ROOT / "guides"

TITLES = {
    "user-guide": "User guide",
    "keeper-guide": "Keeper guide",
    "relay-guide": "Relay guide",
}


def inline(text: str) -> str:
    text = html.escape(text, quote=False)
    text = re.sub(r"`([^`]+)`", lambda m: "<code>" + m.group(1) + "</code>", text)
    text = re.sub(r"\*\*([^*]+)\*\*", r"<strong>\1</strong>", text)
    text = re.sub(r"(?<![\w*])\*([^*\n]+)\*(?![\w*])", r"<em>\1</em>", text)

    def link(m):
        href = m.group(2)
        if href.endswith(".md") and not href.startswith("http"):
            href = href[:-3] + ".html"
        return f'<a href="{href}">{m.group(1)}</a>'

    return re.sub(r"\[([^\]]+)\]\(([^)]+)\)", link, text)


def render(md: str) -> str:
    out, lines, i = [], md.splitlines(), 0
    while i < len(lines):
        line = lines[i]
        if line.startswith("```"):
            i += 1
            code = []
            while i < len(lines) and not lines[i].startswith("```"):
                code.append(lines[i])
                i += 1
            out.append("<pre><code>" + html.escape("\n".join(code)) + "</code></pre>")
            i += 1
        elif m := re.match(r"(#{1,3}) (.*)", line):
            n = len(m.group(1))
            out.append(f"<h{n}>{inline(m.group(2))}</h{n}>")
            i += 1
        elif line.startswith("|"):
            rows = []
            while i < len(lines) and lines[i].startswith("|"):
                rows.append([c.strip() for c in lines[i].strip().strip("|").split("|")])
                i += 1
            head, body = rows[0], [r for r in rows[2:]]
            t = "<table><thead><tr>" + "".join(f"<th>{inline(c)}</th>" for c in head) + "</tr></thead><tbody>"
            for r in body:
                t += "<tr>" + "".join(f"<td>{inline(c)}</td>" for c in r) + "</tr>"
            out.append(t + "</tbody></table>")
        elif re.match(r"\s*([*-]|\d+\.) ", line):
            ordered = bool(re.match(r"\s*\d+\. ", line))
            items = []
            while i < len(lines) and (re.match(r"\s*([*-]|\d+\.) ", lines[i]) or (lines[i].startswith("  ") and lines[i].strip() and items)):
                if re.match(r"\s*([*-]|\d+\.) ", lines[i]):
                    items.append(re.sub(r"^\s*([*-]|\d+\.) ", "", lines[i]))
                else:
                    items[-1] += " " + lines[i].strip()
                i += 1
            tag = "ol" if ordered else "ul"
            out.append(f"<{tag}>" + "".join(f"<li>{inline(x)}</li>" for x in items) + f"</{tag}>")
        elif line.startswith("> "):
            quote = []
            while i < len(lines) and lines[i].startswith(">"):
                quote.append(lines[i][1:].strip())
                i += 1
            out.append("<blockquote>" + inline(" ".join(quote)) + "</blockquote>")
        elif not line.strip():
            i += 1
        else:
            para = [line]
            i += 1
            while i < len(lines) and lines[i].strip() and not re.match(r"(#|\||```|> |\s*([*-]|\d+\.) )", lines[i]):
                para.append(lines[i])
                i += 1
            out.append("<p>" + inline(" ".join(para)) + "</p>")
    return "\n".join(out)


PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · Shoebox</title>
<link rel="icon" href="../icon.svg" type="image/svg+xml">
<link rel="stylesheet" href="../style.css">
</head>
<body>
<header class="top"><a class="brand" href="../index.html"><img src="../icon.svg" alt="" width="26" height="26">Shoebox</a><nav><a href="../index.html#downloads">Download</a><a href="user-guide.html">Guides</a></nav></header>
<main class="doc">
{body}
</main>
<footer>Shoebox collects no data. <a href="../index.html">Home</a></footer>
</body>
</html>
"""


def build():
    OUT.mkdir(exist_ok=True)
    for src in sorted(GUIDES.glob("*.md")):
        title = TITLES.get(src.stem, src.stem.replace("-", " ").title())
        (OUT / f"{src.stem}.html").write_text(PAGE.format(title=title, body=render(src.read_text())))
        print("wrote", OUT / f"{src.stem}.html")


def check():
    bad = 0
    for page in list(ROOT.glob("*.html")) + list(OUT.glob("*.html")):
        for href in re.findall(r'(?:href|src)="([^"#]+)', page.read_text()):
            if href.startswith(("http://", "https://", "mailto:")):
                continue
            if not (page.parent / href).exists():
                print(f"{page.relative_to(ROOT)}: missing {href}")
                bad += 1
    print("links ok" if not bad else f"{bad} broken local links")
    return bad == 0


if __name__ == "__main__":
    build()
    sys.exit(0 if check() or "--check" not in sys.argv else 1)
