#!/usr/bin/env python3
"""Regenerate docs/assets/uteke-banner.png from the evergreen SVG source.

The banner is intentionally EVERGREEN: no version, star, or test counts are
baked in — dynamic figures belong to the shields.io badges in README.md.
Edit docs/assets/uteke-banner.svg, then run:

    python3 scripts/regen-banner.py

Renders @2x (2560x1280) for retina crispness (README displays at width=640).
Requires: cairosvg (pip install cairosvg).
"""

from pathlib import Path

import cairosvg

ROOT = Path(__file__).resolve().parent.parent
SVG = ROOT / "docs" / "assets" / "uteke-banner.svg"
PNG = ROOT / "docs" / "assets" / "uteke-banner.png"


def main() -> None:
    if not SVG.exists():
        raise SystemExit(f"missing source: {SVG}")
    cairosvg.svg2png(url=str(SVG), write_to=str(PNG), output_width=2560, output_height=1280)
    size_kb = PNG.stat().st_size // 1024
    print(f"rendered {PNG.relative_to(ROOT)} ({PNG.stat().st_size} bytes, {size_kb} KB)")


if __name__ == "__main__":
    main()
