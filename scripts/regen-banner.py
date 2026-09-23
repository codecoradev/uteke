#!/usr/bin/env python3
"""Regenerate docs/assets/uteke-banner.png from the evergreen SVG source.

The banner is intentionally EVERGREEN: no version, star, or test counts are
baked in — dynamic figures belong to the shields.io badges in README.md.
The mascot is the OFFICIAL Uteke icon from uteke-mobile
(docs/assets/uteke-icon.png), composited with rounded corners by this
script after the SVG render — cairosvg cannot embed raster images.

Usage:
    python3 scripts/regen-banner.py

Renders @2x (2560x1280) for retina crispness (README displays at width=640).
Requires: cairosvg, pillow (pip install cairosvg pillow).
"""

from pathlib import Path

import cairosvg
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
SVG = ROOT / "docs" / "assets" / "uteke-banner.svg"
ICON = ROOT / "docs" / "assets" / "uteke-icon.png"
PNG = ROOT / "docs" / "assets" / "uteke-banner.png"

RENDER_SCALE = 2  # SVG is authored at 1280x640, rendered at 2560x1280
# Avatar slot in SVG authoring coords — keep in sync with uteke-banner.svg
ICON_XY = (56, 150)
ICON_SIZE = 48  # SVG units; rendered at 96px
ICON_RADIUS = 13  # matches the app-icon corner radius of uteke-mobile


def main() -> None:
    if not SVG.exists():
        raise SystemExit(f"missing source: {SVG}")
    if not ICON.exists():
        raise SystemExit(f"missing icon: {ICON}")

    cairosvg.svg2png(url=str(SVG), write_to=str(PNG), output_width=1280 * RENDER_SCALE,
                     output_height=640 * RENDER_SCALE)
    banner = Image.open(PNG).convert("RGBA")

    size = ICON_SIZE * RENDER_SCALE
    icon = Image.open(ICON).convert("RGBA").resize((size, size), Image.Resampling.LANCZOS)
    mask = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(mask)
    draw.rounded_rectangle([0, 0, size - 1, size - 1], radius=ICON_RADIUS * RENDER_SCALE, fill=255)
    banner.paste(icon, (ICON_XY[0] * RENDER_SCALE, ICON_XY[1] * RENDER_SCALE), mask)

    banner.convert("RGB").save(PNG, "PNG", optimize=True)
    size_kb = PNG.stat().st_size // 1024
    print(f"rendered {PNG.relative_to(ROOT)} ({PNG.stat().st_size} bytes, {size_kb} KB)")


if __name__ == "__main__":
    main()
