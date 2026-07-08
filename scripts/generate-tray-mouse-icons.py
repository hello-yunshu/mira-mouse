#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later

from pathlib import Path

from PIL import Image, ImageDraw


SIZE = 64
IDLE_SIZE = 32
OUTLINE_WIDTH = 4  # 外轮廓 4px
GAP = 4  # 内留白与外轮廓等粗
OUTLINE_ALPHA = 160  # ~0.63，轮廓再透明一点点
WHEEL_WIDTH = 7  # 中键粗细 7px
WHEEL_LENGTH = 14  # 中键长度增加 2px
WHEEL_GAP = 2  # 中键四周 2px 透明边缘
CHARGING_BOLT_GAP = [
    (44, 5),
    (34, 28),
    (46, 28),
    (26, 58),
    (32, 38),
    (20, 38),
]
CHARGING_BOLT = [
    (43, 8),
    (36, 29),
    (45, 29),
    (28, 55),
    (33, 36),
    (23, 36),
]

ROOT = Path(__file__).resolve().parent.parent
ICON_DIR = ROOT / "src-tauri" / "icons"
LEVELS = range(0, 101, 10)


def outline_color(dark: bool):
    # dark=True: 深色菜单栏背景 -> 白色半透明轮廓
    # dark=False: 浅色菜单栏背景 -> 黑色半透明轮廓
    return (255, 255, 255, OUTLINE_ALPHA) if dark else (0, 0, 0, OUTLINE_ALPHA)


def wheel_color(dark: bool):
    alpha = min(255, OUTLINE_ALPHA + 60)
    return (255, 255, 255, alpha) if dark else (0, 0, 0, alpha)


def battery_color(level: int):
    if level <= 20:
        return (255, 59, 48, 255)
    if level <= 50:
        return (255, 204, 0, 255)
    return (52, 199, 89, 255)


def mouse_shape_bounds(size: int):
    # 微胖且占满画布：宽 46，高 60，上下仅留 2px 边距给轮廓。
    width = int(round(46 * size / 64))
    height = int(round(60 * size / 64))
    left = (size - width) // 2
    top = (size - height) // 2
    return (left, top, left + width, top + height)


def draw_mouse_icon(size: int, level: int, dark: bool, charging: bool = False):
    outline = outline_color(dark)
    wheel = wheel_color(dark)
    fill = battery_color(level)
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)

    scale = size / 64
    outline_width = max(1, int(round(OUTLINE_WIDTH * scale)))
    gap = max(1, int(round(GAP * scale)))

    outer = mouse_shape_bounds(size)
    radius = int(round(16 * scale))
    draw.rounded_rectangle(outer, radius=radius, outline=outline, width=outline_width)

    inset = outline_width + gap
    inner = (
        outer[0] + inset,
        outer[1] + inset,
        outer[2] - inset,
        outer[3] - inset,
    )

    center_x = size // 2
    wheel_top = inner[1] + max(2, int(round(4 * scale)))
    wheel_bottom = wheel_top + max(8, int(round(WHEEL_LENGTH * scale)))
    wheel_width = max(1, int(round(WHEEL_WIDTH * scale)))
    wheel_gap = max(1, int(round(WHEEL_GAP * scale)))
    wheel_left = center_x - wheel_width // 2
    wheel_right = wheel_left + wheel_width - 1

    fill_height = int((inner[3] - inner[1]) * level / 100)
    if fill_height > 0:
        fill_box = (inner[0], inner[3] - fill_height, inner[2], inner[3])
        draw.rounded_rectangle(
            fill_box,
            radius=max(1, int(round(6 * scale))),
            fill=fill,
        )
        # 电量填充绕过中键，四周都留出 2px 纯透明边缘。
        if wheel_bottom + wheel_gap >= inner[3] - fill_height:
            clear_top = max(inner[3] - fill_height, wheel_top - wheel_gap)
            draw.rounded_rectangle(
                (
                    wheel_left - wheel_gap,
                    clear_top,
                    wheel_right + wheel_gap,
                    wheel_bottom + wheel_gap,
                ),
                radius=wheel_gap,
                fill=(0, 0, 0, 0),
            )

    draw.rounded_rectangle(
        (wheel_left, wheel_top, wheel_right, wheel_bottom),
        radius=wheel_width // 2,
        fill=wheel,
    )

    if charging:
        gap_points = [
            (int(round(x * scale)), int(round(y * scale)))
            for x, y in CHARGING_BOLT_GAP
        ]
        draw.polygon(gap_points, fill=(0, 0, 0, 0))
        points = [
            (int(round(x * scale)), int(round(y * scale)))
            for x, y in CHARGING_BOLT
        ]
        draw.polygon(points, fill=(255, 255, 255, 255))

    return image


def draw_idle_icon(size: int, dark: bool):
    outline = outline_color(dark)
    wheel = wheel_color(dark)
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)

    scale = size / 64
    outline_width = max(1, int(round(OUTLINE_WIDTH * scale)))
    gap = max(1, int(round(GAP * scale)))

    outer = mouse_shape_bounds(size)
    radius = int(round(16 * scale))
    draw.rounded_rectangle(outer, radius=radius, outline=outline, width=outline_width)

    inset = outline_width + gap
    inner = (
        outer[0] + inset,
        outer[1] + inset,
        outer[2] - inset,
        outer[3] - inset,
    )
    draw.rounded_rectangle(
        inner,
        radius=max(1, int(round(6 * scale))),
        outline=outline,
        width=outline_width,
    )

    center_x = size // 2
    wheel_top = inner[1] + max(2, int(round(4 * scale)))
    wheel_bottom = wheel_top + max(8, int(round(12 * scale)))
    wheel_width = max(1, int(round(WHEEL_WIDTH * scale)))
    draw.line(
        [(center_x, wheel_top), (center_x, wheel_bottom)],
        fill=outline,
        width=wheel_width + 4,
    )
    draw.line(
        [(center_x, wheel_top), (center_x, wheel_bottom)],
        fill=wheel,
        width=wheel_width,
    )

    return image


def main():
    output_dirs = {
        "light": ICON_DIR / "tray-mouse-levels",
        "dark": ICON_DIR / "tray-mouse-levels-dark",
        "charging_light": ICON_DIR / "tray-mouse-charging-levels",
        "charging_dark": ICON_DIR / "tray-mouse-charging-levels-dark",
    }
    for output_dir in output_dirs.values():
        output_dir.mkdir(parents=True, exist_ok=True)

    for level in LEVELS:
        draw_mouse_icon(SIZE, level, dark=False).save(
            output_dirs["light"] / f"mouse-{level}.png"
        )
        draw_mouse_icon(SIZE, level, dark=True).save(
            output_dirs["dark"] / f"mouse-{level}.png"
        )
        draw_mouse_icon(SIZE, level, dark=False, charging=True).save(
            output_dirs["charging_light"] / f"mouse-{level}.png"
        )
        draw_mouse_icon(SIZE, level, dark=True, charging=True).save(
            output_dirs["charging_dark"] / f"mouse-{level}.png"
        )

    draw_idle_icon(IDLE_SIZE, dark=False).save(ICON_DIR / "tray-mouse-idle-light.png")
    draw_idle_icon(IDLE_SIZE, dark=True).save(ICON_DIR / "tray-mouse-idle-dark.png")

    print("generated tray mouse icons")


if __name__ == "__main__":
    main()
