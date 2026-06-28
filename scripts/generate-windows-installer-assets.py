#!/usr/bin/env python3
"""Generate Windows NSIS assets that match Mira's liquid-glass theme."""

from __future__ import annotations

from pathlib import Path
import math

from PIL import Image, ImageDraw, ImageFilter, ImageFont


ROOT = Path(__file__).resolve().parents[1]
ASSET_DIR = ROOT / "scripts" / "assets"
ICON_PATH = ROOT / "src-tauri" / "icons" / "icon.png"

SF = Path("/System/Library/Fonts/SFNS.ttf")
SF_ROUNDED = Path("/System/Library/Fonts/SFNSRounded.ttf")
ARIAL = Path("/System/Library/Fonts/Supplemental/Arial.ttf")
DEJAVU = Path("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf")

INK = (40, 38, 49)
MUTED = (109, 104, 120)
ACCENT = (216, 176, 183)
COOL = (143, 168, 228)
PAPER = (247, 244, 250)
PAPER_2 = (238, 234, 243)
WHITE = (255, 255, 255)


def font(size: int, *, rounded: bool = False) -> ImageFont.FreeTypeFont:
    candidates = []
    if rounded:
        candidates.append(SF_ROUNDED)
    candidates.extend([SF, ARIAL, DEJAVU])
    for candidate in candidates:
        if candidate.exists():
            return ImageFont.truetype(str(candidate), size)
    return ImageFont.load_default()


def lerp(a: int, b: int, t: float) -> int:
    return round(a + (b - a) * t)


def mix(c1: tuple[int, int, int], c2: tuple[int, int, int], t: float) -> tuple[int, int, int]:
    return tuple(lerp(a, b, t) for a, b in zip(c1, c2))


def background(size: tuple[int, int]) -> Image.Image:
    width, height = size
    img = Image.new("RGB", size, PAPER)
    pixels = img.load()
    for y in range(height):
        ty = y / max(1, height - 1)
        for x in range(width):
            tx = x / max(1, width - 1)
            base = mix(PAPER, PAPER_2, ty)
            warm = math.sin((tx * 0.9 + ty * 1.1) * math.pi)
            pixels[x, y] = mix(base, (242, 232, 238), max(0, warm) * 0.12)
    return img.convert("RGBA")


def add_material_wash(
    img: Image.Image,
    center: tuple[float, float],
    radius: float,
    color: tuple[int, int, int],
    opacity: float,
) -> None:
    overlay = Image.new("RGBA", img.size, (0, 0, 0, 0))
    pixels = overlay.load()
    cx, cy = center
    for y in range(max(0, int(cy - radius)), min(img.height, int(cy + radius) + 1)):
        for x in range(max(0, int(cx - radius)), min(img.width, int(cx + radius) + 1)):
            distance = math.hypot(x - cx, y - cy) / radius
            if distance >= 1:
                continue
            alpha = int(255 * opacity * (1 - distance) ** 2.35)
            if alpha:
                pixels[x, y] = (*color, alpha)
    img.alpha_composite(overlay.filter(ImageFilter.GaussianBlur(7)))


def paste_icon(img: Image.Image, center: tuple[int, int], size: int) -> None:
    icon = Image.open(ICON_PATH).convert("RGBA")
    icon.thumbnail((size, size), Image.Resampling.LANCZOS)
    shadow = Image.new("RGBA", (size + 18, size + 18), (0, 0, 0, 0))
    draw = ImageDraw.Draw(shadow, "RGBA")
    draw.rounded_rectangle((9, 9, size + 9, size + 9), radius=20, fill=(54, 45, 67, 42))
    shadow = shadow.filter(ImageFilter.GaussianBlur(7))
    img.alpha_composite(shadow, (center[0] - shadow.width // 2, center[1] - shadow.height // 2 + 5))
    img.alpha_composite(icon, (center[0] - icon.width // 2, center[1] - icon.height // 2))


def glass_panel(
    draw: ImageDraw.ImageDraw,
    box: tuple[int, int, int, int],
    *,
    radius: int,
    fill_alpha: int = 86,
    outline_alpha: int = 112,
) -> None:
    draw.rounded_rectangle(box, radius=radius, fill=(*WHITE, fill_alpha), outline=(*WHITE, outline_alpha), width=1)
    x1, y1, x2, _ = box
    draw.line((x1 + radius, y1 + 1, x2 - radius, y1 + 1), fill=(*WHITE, 118), width=1)


def draw_tracks(draw: ImageDraw.ImageDraw) -> None:
    draw.line([(0, 101), (44, 101), (62, 88), (100, 88), (164, 58)], fill=(*ACCENT, 92), width=1)
    draw.line([(20, 232), (70, 232), (94, 209), (132, 209), (164, 188)], fill=(*COOL, 78), width=1)
    draw.line([(0, 274), (42, 274), (62, 260), (112, 260), (164, 244)], fill=(*ACCENT, 54), width=1)
    for x, y, color in [(62, 88, ACCENT), (94, 209, COOL), (62, 260, ACCENT)]:
        draw.ellipse((x - 2, y - 2, x + 2, y + 2), fill=(*color, 150))


def draw_sidebar() -> Image.Image:
    img = background((164, 314))
    add_material_wash(img, (-12, 26), 130, ACCENT, 0.42)
    add_material_wash(img, (172, 246), 150, COOL, 0.24)
    add_material_wash(img, (92, 140), 104, WHITE, 0.22)

    draw = ImageDraw.Draw(img, "RGBA")
    draw_tracks(draw)
    glass_panel(draw, (20, 26, 144, 262), radius=18, fill_alpha=74, outline_alpha=96)
    paste_icon(img, (82, 83), 72)

    draw.text((82, 151), "Mira", font=font(27, rounded=True), fill=(*INK, 246), anchor="mm")
    draw.text((82, 174), "Mouse Control", font=font(10), fill=(*MUTED, 226), anchor="mm")

    draw.rounded_rectangle((43, 210, 121, 216), radius=3, fill=(*ACCENT, 118))
    draw.rounded_rectangle((55, 226, 109, 231), radius=3, fill=(*COOL, 88))
    draw.rounded_rectangle((66, 241, 98, 245), radius=2, fill=(*ACCENT, 70))
    return img.convert("RGB")


def draw_header(*, uninstall: bool = False) -> Image.Image:
    img = background((150, 57))
    add_material_wash(img, (128, 18), 80, COOL if uninstall else ACCENT, 0.26)
    add_material_wash(img, (36, 44), 68, ACCENT if uninstall else COOL, 0.12)
    draw = ImageDraw.Draw(img, "RGBA")
    glass_panel(draw, (5, 6, 145, 50), radius=12, fill_alpha=58, outline_alpha=84)
    draw.line((0, 56, 150, 56), fill=(218, 210, 224, 255), width=1)
    draw.line((15, 38, 58, 38, 72, 30, 108, 30), fill=(*ACCENT, 104), width=1)
    draw.ellipse((70, 28, 75, 33), fill=(*ACCENT, 144))
    paste_icon(img, (123, 29), 31)
    draw.text((15, 22), "Mira", font=font(13, rounded=True), fill=(*INK, 244), anchor="lm")
    draw.text((15, 37), "Uninstall" if uninstall else "Setup", font=font(8), fill=(*MUTED, 230), anchor="lm")
    return img.convert("RGB")


def main() -> None:
    ASSET_DIR.mkdir(parents=True, exist_ok=True)
    draw_header().save(ASSET_DIR / "windows-nsis-header.bmp")
    draw_header(uninstall=True).save(ASSET_DIR / "windows-nsis-uninstall-header.bmp")
    draw_sidebar().save(ASSET_DIR / "windows-nsis-sidebar.bmp")


if __name__ == "__main__":
    main()
