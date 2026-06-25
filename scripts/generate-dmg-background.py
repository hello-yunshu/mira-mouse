#!/usr/bin/env python3
"""Generate the macOS DMG installer background assets."""

from __future__ import annotations

from pathlib import Path
import math
import xml.sax.saxutils as xml

from PIL import Image, ImageDraw, ImageFilter, ImageFont


ROOT = Path(__file__).resolve().parents[1]
ASSET_DIR = ROOT / "scripts" / "assets"
WIDTH = 660
HEIGHT = 400

TITLE = "Mira"
SUBTITLE = "鼠标配置工具 · Mouse Configuration"
PRIMARY = "拖动 Mira 到 Applications"
SECONDARY = "完成安装"
ENGLISH = "Drag Mira to Applications to install"

SF = Path("/System/Library/Fonts/SFNS.ttf")
SF_ROUNDED = Path("/System/Library/Fonts/SFNSRounded.ttf")
ARIAL_UNICODE = Path("/System/Library/Fonts/Supplemental/Arial Unicode.ttf")
ARIAL = Path("/System/Library/Fonts/Supplemental/Arial.ttf")


def font(size: int, *, rounded: bool = False, unicode: bool = False) -> ImageFont.FreeTypeFont:
    candidates = []
    if unicode:
        candidates.append(ARIAL_UNICODE)
    if rounded:
        candidates.append(SF_ROUNDED)
    candidates.extend([SF, ARIAL])
    for candidate in candidates:
        if candidate.exists():
            return ImageFont.truetype(str(candidate), size)
    return ImageFont.load_default()


def lerp(a: int, b: int, t: float) -> int:
    return round(a + (b - a) * t)


def mix(c1: tuple[int, int, int], c2: tuple[int, int, int], t: float) -> tuple[int, int, int]:
    return tuple(lerp(a, b, t) for a, b in zip(c1, c2))


def add_radial(
    img: Image.Image,
    center: tuple[float, float],
    radius: float,
    color: tuple[int, int, int],
    opacity: float,
) -> None:
    overlay = Image.new("RGBA", img.size, (0, 0, 0, 0))
    px = overlay.load()
    cx, cy = center
    for y in range(max(0, int(cy - radius)), min(img.height, int(cy + radius) + 1)):
        for x in range(max(0, int(cx - radius)), min(img.width, int(cx + radius) + 1)):
            d = math.hypot(x - cx, y - cy) / radius
            if d >= 1:
                continue
            alpha = int(255 * opacity * (1 - d) ** 2.2)
            if alpha:
                px[x, y] = (*color, alpha)
    img.alpha_composite(overlay)


def text_center(
    draw: ImageDraw.ImageDraw,
    xy: tuple[int, int],
    text: str,
    fnt: ImageFont.FreeTypeFont,
    fill: tuple[int, int, int, int],
) -> None:
    draw.text(xy, text, font=fnt, fill=fill, anchor="mm")


def draw_background(scale: int) -> Image.Image:
    w = WIDTH * scale
    h = HEIGHT * scale
    img = Image.new("RGBA", (w, h), (0, 0, 0, 255))
    px = img.load()
    top = (22, 19, 30)
    mid = (34, 29, 44)
    bottom = (42, 35, 54)
    for y in range(h):
        ty = y / max(1, h - 1)
        for x in range(w):
            tx = x / max(1, w - 1)
            base = mix(top, bottom, ty)
            color = mix(base, mid, 0.22 + 0.18 * math.sin((tx * 1.2 + ty * 0.8) * math.pi))
            vignette = min(0.28, math.hypot(tx - 0.5, ty - 0.52) * 0.34)
            px[x, y] = (*mix(color, (7, 7, 12), vignette), 255)

    add_radial(img, (88 * scale, 72 * scale), 188 * scale, (216, 176, 183), 0.20)
    add_radial(img, (522 * scale, 304 * scale), 220 * scale, (128, 104, 160), 0.24)
    add_radial(img, (334 * scale, 205 * scale), 176 * scale, (104, 137, 168), 0.10)

    overlay = Image.new("RGBA", img.size, (0, 0, 0, 0))
    draw = ImageDraw.Draw(overlay, "RGBA")

    # Crisp accent rails inspired by the app aura, kept away from icon labels.
    for offset, alpha in [(0, 84), (2, 38)]:
        draw.line(
            [
                (0, (145 + offset) * scale),
                (160 * scale, (145 + offset) * scale),
                (218 * scale, (132 + offset) * scale),
                (284 * scale, (132 + offset) * scale),
                (370 * scale, (96 + offset) * scale),
            ],
            fill=(216, 176, 183, alpha),
            width=max(1, scale),
            joint="curve",
        )
        draw.line(
            [
                (276 * scale, (308 + offset) * scale),
                (392 * scale, (308 + offset) * scale),
                (460 * scale, (286 + offset) * scale),
                (620 * scale, (286 + offset) * scale),
                (660 * scale, (250 + offset) * scale),
            ],
            fill=(128, 145, 220, alpha),
            width=max(1, scale),
            joint="curve",
        )

    img.alpha_composite(overlay)
    draw = ImageDraw.Draw(img, "RGBA")

    # Direction arrow.
    arrow_y = 198 * scale
    x1 = 248 * scale
    x2 = 412 * scale
    dash = 9 * scale
    gap = 7 * scale
    x = x1
    while x < x2 - 12 * scale:
        draw.line(
            [(x, arrow_y), (min(x + dash, x2 - 16 * scale), arrow_y)],
            fill=(216, 176, 183, 170),
            width=max(2, 2 * scale),
        )
        x += dash + gap
    draw.polygon(
        [
            (x2, arrow_y),
            ((x2 - 20 * scale), (arrow_y - 10 * scale)),
            ((x2 - 20 * scale), (arrow_y + 10 * scale)),
        ],
        fill=(216, 176, 183, 190),
    )

    # Title and exact installation guidance.
    text_center(draw, (330 * scale, 42 * scale), TITLE, font(24 * scale, rounded=True), (232, 228, 238, 245))
    text_center(draw, (330 * scale, 64 * scale), SUBTITLE, font(10 * scale, unicode=True), (154, 145, 168, 220))
    text_center(draw, (330 * scale, 334 * scale), PRIMARY, font(15 * scale, unicode=True), (232, 226, 238, 245))
    text_center(draw, (330 * scale, 354 * scale), SECONDARY, font(12 * scale, unicode=True), (199, 190, 214, 230))
    text_center(draw, (330 * scale, 374 * scale), ENGLISH, font(10 * scale), (130, 122, 145, 225))

    return img.convert("RGB")


def svg() -> str:
    return f'''<?xml version="1.0" encoding="UTF-8"?>
<svg viewBox="0 0 {WIDTH} {HEIGHT}" width="{WIDTH}" height="{HEIGHT}" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <linearGradient id="bg" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#16131E"/>
      <stop offset="48%" stop-color="#241F30"/>
      <stop offset="100%" stop-color="#2A2336"/>
    </linearGradient>
    <radialGradient id="pink" cx="15%" cy="18%" r="42%">
      <stop offset="0%" stop-color="#D8B0B7" stop-opacity="0.20"/>
      <stop offset="100%" stop-color="#D8B0B7" stop-opacity="0"/>
    </radialGradient>
    <radialGradient id="violet" cx="80%" cy="75%" r="48%">
      <stop offset="0%" stop-color="#8068A0" stop-opacity="0.24"/>
      <stop offset="100%" stop-color="#8068A0" stop-opacity="0"/>
    </radialGradient>
  </defs>
  <rect width="{WIDTH}" height="{HEIGHT}" fill="url(#bg)"/>
  <rect width="{WIDTH}" height="{HEIGHT}" fill="url(#pink)"/>
  <rect width="{WIDTH}" height="{HEIGHT}" fill="url(#violet)"/>
  <path d="M0 145 H160 C198 145 202 132 284 132 C332 132 350 118 370 96" fill="none" stroke="#D8B0B7" stroke-opacity="0.33" stroke-width="1"/>
  <path d="M276 308 H392 C430 308 438 286 460 286 H620 C640 286 648 268 660 250" fill="none" stroke="#8091DC" stroke-opacity="0.32" stroke-width="1"/>
  <g font-family="-apple-system, BlinkMacSystemFont, 'PingFang SC', system-ui, sans-serif" text-anchor="middle">
    <text x="330" y="46" font-size="24" font-weight="700" fill="#E8E4EE">{xml.escape(TITLE)}</text>
    <text x="330" y="66" font-size="10" fill="#9A91A8">{xml.escape(SUBTITLE)}</text>
    <line x1="248" y1="198" x2="395" y2="198" stroke="#D8B0B7" stroke-opacity="0.67" stroke-width="2" stroke-dasharray="9 7"/>
    <polygon points="412,198 392,188 392,208" fill="#D8B0B7" fill-opacity="0.74"/>
    <text x="330" y="338" font-size="15" font-weight="700" fill="#E8E2EE">{xml.escape(PRIMARY)}</text>
    <text x="330" y="356" font-size="12" font-weight="600" fill="#C7BED6">{xml.escape(SECONDARY)}</text>
    <text x="330" y="376" font-size="10" fill="#827A91">{xml.escape(ENGLISH)}</text>
  </g>
</svg>
'''


def main() -> None:
    ASSET_DIR.mkdir(parents=True, exist_ok=True)
    (ASSET_DIR / "dmg-background.svg").write_text(svg(), encoding="utf-8")
    draw_background(1).save(ASSET_DIR / "dmg-background.png", optimize=True)
    draw_background(2).save(ASSET_DIR / "dmg-background@2x.png", optimize=True)


if __name__ == "__main__":
    main()
