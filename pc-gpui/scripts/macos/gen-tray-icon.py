#!/usr/bin/env python3
"""生成 macOS 菜单栏图标（模板图标）：`pc-gpui/crates/app/assets/tray-template.png`。

模板图标只用 alpha 通道（RGB 一律黑色），系统按菜单栏深浅色自动着色；tray-icon 把高度固定为 18pt，
所以画 36×36（18pt@2x）。字形取应用图标的"清单"：两行勾选框 + 一行空框，右侧三条横线。
先按 8 倍分辨率画，再缩小得到抗锯齿边缘。

依赖 Pillow：`python3 pc-gpui/scripts/macos/gen-tray-icon.py`（仓库根目录执行）。改完重新运行并提交 PNG。
"""

from pathlib import Path

from PIL import Image, ImageDraw

SIZE = 36  # 18pt @2x
SCALE = 8
OUT = Path(__file__).resolve().parents[2] / "crates" / "app" / "assets" / "tray-template.png"


def s(v: float) -> int:
    return round(v * SCALE)


def main() -> None:
    big = SIZE * SCALE
    alpha = Image.new("L", (big, big), 0)
    draw = ImageDraw.Draw(alpha)

    rows = (8.0, 18.0, 28.0)  # 每行中心 y
    box = 9.0  # 勾选框边长
    box_left = 3.0
    radius = 2.0
    stroke = 2.4

    for index, cy in enumerate(rows):
        top = cy - box / 2
        rect = (s(box_left), s(top), s(box_left + box), s(top + box))
        checked = index < 2
        if checked:
            # 实心框，勾挖空
            draw.rounded_rectangle(rect, radius=s(radius), fill=255)
            check = [
                (box_left + 2.0, cy + 0.2),
                (box_left + 3.9, cy + 2.1),
                (box_left + 7.1, cy - 2.0),
            ]
            draw.line([(s(x), s(y)) for x, y in check], fill=0, width=s(1.9), joint="curve")
            for x, y in (check[0], check[2]):
                r = 0.95
                draw.ellipse((s(x - r), s(y - r), s(x + r), s(y + r)), fill=0)
        else:
            draw.rounded_rectangle(rect, radius=s(radius), outline=255, width=s(stroke * 0.85))

        # 右侧横线（圆头）
        line_left, line_right, thickness = 16.0, 33.0, 3.0
        draw.rounded_rectangle(
            (s(line_left), s(cy - thickness / 2), s(line_right), s(cy + thickness / 2)),
            radius=s(thickness / 2),
            fill=255,
        )

    alpha = alpha.resize((SIZE, SIZE), Image.LANCZOS)
    image = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    image.putalpha(alpha)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    image.save(OUT, optimize=True)
    print(f"已生成 {OUT}（{SIZE}x{SIZE}）")


if __name__ == "__main__":
    main()
