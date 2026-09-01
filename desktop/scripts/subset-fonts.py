#!/usr/bin/env python3
"""Rebuild the two bundled newspaper faces in desktop/ui/public/fonts/.

The high-fidelity design (design/Tidy 高保真 报纸风.dc.html) is set in
Noto Serif SC and IBM Plex Mono. The app cannot load them from a CDN — its
CSP is `default-src 'self'` and a desktop cleaner has to render identically
offline — so both are subset and shipped with the bundle. Both are SIL OFL
1.1; the licence texts sit beside the woff2 files.

Sizing decision: the full Noto Serif SC CJK block is ~8 MB. This script keeps
Google Fonts' most-frequent slices (chunk index >= COMMON_CHUNK_FLOOR, about
3.4k hanzi) plus every character the repo itself prints, which lands near
1.3 MB. A rare hanzi in a file or app name simply falls through to the
system Song face named next in --font-serif.

Requires: fonttools + brotli (`pip install fonttools brotli`), network access.
Run from the repo root:  python3 desktop/scripts/subset-fonts.py
"""

from __future__ import annotations

import re
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
OUT = REPO / "desktop/ui/public/fonts"

NOTO_TTF = "https://github.com/google/fonts/raw/main/ofl/notoserifsc/NotoSerifSC%5Bwght%5D.ttf"
NOTO_CSS = "https://fonts.googleapis.com/css2?family=Noto+Serif+SC:wght@600&display=swap"
PLEX_TTF = "https://github.com/google/fonts/raw/main/ofl/ibmplexmono/IBMPlexMono-{style}.ttf"
LICENCES = {
    "OFL-NotoSerifSC.txt": "https://github.com/google/fonts/raw/main/ofl/notoserifsc/OFL.txt",
    "OFL-IBMPlexMono.txt": "https://github.com/google/fonts/raw/main/ofl/ibmplexmono/OFL.txt",
}

# Google slices CJK by frequency and numbers the slices in the woff2 filename:
# the HIGHEST index holds the most common characters (119 contains 的 and 一).
# Everything from this floor upwards is roughly the 3.5k common-hanzi table.
COMMON_CHUNK_FLOOR = 100

# A desktop browser UA is required or the CSS endpoint answers with ttf.
UA = (
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/120.0 Safari/537.36"
)

# Punctuation, arrows and marks the newspaper skin prints as furniture.
EXTRA_RANGES = [
    (0x0020, 0x007E),  # ASCII
    (0x00A0, 0x00FF),  # Latin-1 supplement (· lives here)
    (0x2000, 0x206F),  # general punctuation (— … › ‧)
    (0x2190, 0x21FF),  # arrows (→ ↑ ↗ ⇅)
]
EXTRA_CHARS = "✓✔▴▾▸•"


def fetch(url: str, dest: Path, *, ua: bool = False) -> None:
    """Download `url` to `dest`, optionally pretending to be a browser."""
    request = urllib.request.Request(url, headers={"User-Agent": UA} if ua else {})
    with urllib.request.urlopen(request) as response:
        dest.write_bytes(response.read())


def common_codepoints(css: str) -> set[int]:
    """Union the unicode-ranges of the most frequent Noto Serif SC slices."""
    codepoints: set[int] = set()
    for block in re.findall(r"@font-face \{(.*?)\}", css, re.S):
        index = re.search(r"\.(\d+)\.woff2", block)
        if not index or int(index.group(1)) < COMMON_CHUNK_FLOOR:
            continue
        ranges = re.search(r"unicode-range: (.*?);", block, re.S)
        if not ranges:
            continue
        for part in ranges.group(1).split(","):
            part = part.strip()
            if not part.startswith("U+"):
                continue
            body = part[2:]
            if "-" in body:
                low, high = body.split("-")
                codepoints |= set(range(int(low, 16), int(high, 16) + 1))
            else:
                codepoints.add(int(body, 16))
    return codepoints


def repo_codepoints() -> set[int]:
    """Every character the shipped UI and its docs can print."""
    codepoints: set[int] = set()
    for root, suffixes in ((REPO / "desktop/ui/src", {".ts", ".css"}),
                           (REPO / "desktop/docs", {".md"})):
        for path in root.rglob("*"):
            if path.is_file() and path.suffix in suffixes:
                codepoints |= {ord(c) for c in path.read_text(errors="ignore") if ord(c) > 31}
    return codepoints


def subset(source: Path, dest: Path, unicodes_file: Path) -> None:
    """Run pyftsubset with the flags that keep the output small but correct."""
    subprocess.run(
        [
            "pyftsubset", str(source),
            f"--output-file={dest}",
            "--flavor=woff2",
            f"--unicodes-file={unicodes_file}",
            "--layout-features=",
            "--no-hinting",
            "--desubroutinize",
        ],
        check=True,
    )


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmpdir:
        work = Path(tmpdir)

        css = work / "nssc.css"
        fetch(NOTO_CSS, css, ua=True)
        codepoints = common_codepoints(css.read_text()) | repo_codepoints()
        for low, high in EXTRA_RANGES:
            codepoints |= set(range(low, high + 1))
        codepoints |= {ord(c) for c in EXTRA_CHARS}

        unicodes = work / "unicodes.txt"
        unicodes.write_text(",".join(f"U+{c:04X}" for c in sorted(codepoints)))
        hanzi = len([c for c in codepoints if 0x4E00 <= c <= 0x9FFF])
        print(f"charset: {len(codepoints)} codepoints ({hanzi} hanzi)")

        noto = work / "NotoSerifSC.ttf"
        fetch(NOTO_TTF, noto)
        subset(noto, OUT / "NotoSerifSC-subset.woff2", unicodes)

        for style, weight in (("Regular", 400), ("Medium", 500)):
            plex = work / f"IBMPlexMono-{style}.ttf"
            fetch(PLEX_TTF.format(style=style), plex)
            subset(plex, OUT / f"IBMPlexMono-{weight}.woff2", unicodes)

        for name, url in LICENCES.items():
            fetch(url, OUT / name)

    for path in sorted(OUT.iterdir()):
        print(f"{path.stat().st_size / 1024:9.1f} KB  {path.name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
