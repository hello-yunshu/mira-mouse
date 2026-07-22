#!/usr/bin/env python3
"""Generate the macOS DMG installer background assets."""

from __future__ import annotations

from pathlib import Path
import subprocess
import tempfile
import xml.sax.saxutils as xml

from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
ASSET_DIR = ROOT / "scripts" / "assets"
WIDTH = 660
HEIGHT = 400

TITLE = "Mira"
SUBTITLE = "美观，可爱的鼠标管理软件"
SUBTITLE_EN = "Beautiful, cute mouse management software"
PRIMARY = "将 Mira 拖入 Applications"
SECONDARY = "然后就可以开始啦"
ENGLISH = "Drag Mira to Applications, and you're ready to go"

def svg() -> str:
    return f'''<?xml version="1.0" encoding="UTF-8"?>
<svg viewBox="0 0 {WIDTH} {HEIGHT}" width="{WIDTH}" height="{HEIGHT}" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <linearGradient id="bg" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#2D2738"/>
      <stop offset="48%" stop-color="#40374E"/>
      <stop offset="100%" stop-color="#4C415F"/>
    </linearGradient>
    <radialGradient id="pink" cx="15%" cy="18%" r="42%">
      <stop offset="0%" stop-color="#D8B0B7" stop-opacity="0.20"/>
      <stop offset="100%" stop-color="#D8B0B7" stop-opacity="0"/>
    </radialGradient>
    <radialGradient id="violet" cx="80%" cy="75%" r="48%">
      <stop offset="0%" stop-color="#8068A0" stop-opacity="0.24"/>
      <stop offset="100%" stop-color="#8068A0" stop-opacity="0"/>
    </radialGradient>
    <filter id="shelfShadow" x="-10%" y="-20%" width="120%" height="140%">
      <feGaussianBlur stdDeviation="14"/>
    </filter>
  </defs>
  <rect width="{WIDTH}" height="{HEIGHT}" fill="url(#bg)"/>
  <rect width="{WIDTH}" height="{HEIGHT}" fill="url(#pink)"/>
  <rect width="{WIDTH}" height="{HEIGHT}" fill="url(#violet)"/>
  <path d="M0 145 H160 C198 145 202 132 284 132 C332 132 350 118 370 96" fill="none" stroke="#D8B0B7" stroke-opacity="0.33" stroke-width="1"/>
  <path d="M276 308 H392 C430 308 438 286 460 286 H620 C640 286 648 268 660 250" fill="none" stroke="#8091DC" stroke-opacity="0.32" stroke-width="1"/>
  <rect x="72" y="116" width="516" height="188" rx="38" fill="#0A0810" fill-opacity="0.28" filter="url(#shelfShadow)"/>
  <rect x="72" y="110" width="516" height="188" rx="38" fill="#FFFFFF" fill-opacity="0.27" stroke="#FFFFFF" stroke-opacity="0.38"/>
  <path d="M72 148 A38 38 0 0 1 110 110 H550 A38 38 0 0 1 588 148" fill="none" stroke="#FFFFFF" stroke-opacity="0.35"/>
  <g font-family="-apple-system, BlinkMacSystemFont, 'PingFang SC', system-ui, sans-serif" text-anchor="middle">
    <text x="330" y="46" font-size="24" font-weight="700" fill="#E8E4EE">{xml.escape(TITLE)}</text>
    <text x="330" y="67" font-size="12" font-weight="600" fill="#D3CADC">{xml.escape(SUBTITLE)}</text>
    <text x="330" y="83" font-size="10" font-weight="500" fill="#B5ABC1">{xml.escape(SUBTITLE_EN)}</text>
    <line x1="248" y1="198" x2="395" y2="198" stroke="#D8B0B7" stroke-opacity="0.67" stroke-width="2" stroke-dasharray="9 7"/>
    <polygon points="412,198 392,188 392,208" fill="#D8B0B7" fill-opacity="0.74"/>
    <text x="330" y="338" font-size="15" font-weight="700" fill="#E8E2EE">{xml.escape(PRIMARY)}</text>
    <text x="330" y="356" font-size="12" font-weight="600" fill="#C7BED6">{xml.escape(SECONDARY)}</text>
    <text x="330" y="376" font-size="10" fill="#827A91">{xml.escape(ENGLISH)}</text>
  </g>
</svg>
'''


def render_png(svg_path: Path, output_path: Path, scale: int) -> None:
    """Render with macOS ImageIO so all type and paths stay vector-sharp."""
    width = WIDTH * scale
    height = HEIGHT * scale
    with tempfile.TemporaryDirectory(prefix="mira-dmg-background-") as temp_dir:
        rendered_path = Path(temp_dir) / f"background-{scale}x.png"
        subprocess.run(
            [
                "/usr/bin/sips",
                "-s",
                "format",
                "png",
                "-z",
                str(height),
                str(width),
                str(svg_path),
                "--out",
                str(rendered_path),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        with Image.open(rendered_path) as image:
            if image.size != (width, height):
                raise RuntimeError(
                    f"unexpected rendered size {image.size}, expected {(width, height)}"
                )
            image.convert("RGB").save(output_path, optimize=True)


def main() -> None:
    ASSET_DIR.mkdir(parents=True, exist_ok=True)
    svg_path = ASSET_DIR / "dmg-background.svg"
    svg_path.write_text(svg(), encoding="utf-8")
    render_png(svg_path, ASSET_DIR / "dmg-background.png", 1)
    render_png(svg_path, ASSET_DIR / "dmg-background@2x.png", 2)


if __name__ == "__main__":
    main()
