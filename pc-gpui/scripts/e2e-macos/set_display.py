#!/usr/bin/env python3
"""尝试把 runner 的主显示器调大（默认 1024×768 pt @1x：1080 宽的详情窗口会被 AppKit 夹到屏幕宽）。

    brew install displayplacer && python3 set_display.py [--prefer 1920x1080,1680x1050,1440x900]

`displayplacer list` 列出主屏的全部模式，按偏好顺序挑一个（先挑非 HiDPI 的 `scaling:off`，没有再退到 HiDPI；至少 1440×900），
先用 `res:… hz:… color_depth:… scaling:…` 的写法切换，不行再用 `mode:<n>`。切完打印 NSScreen / CGDisplayBounds 读到的尺寸。
任何一步失败都只打印、退出码恒为 0（测试按"期望尺寸与屏幕可见区域取小"判定，屏幕小也能跑）。
"""
from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass


@dataclass
class Mode:
    n: int
    w: int
    h: int
    hz: str | None
    depth: str | None
    scaling: bool
    current: bool


def sh(cmd: list[str], timeout: float = 60) -> tuple[int, str]:
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return r.returncode, (r.stdout + r.stderr).strip()
    except Exception as e:  # noqa: BLE001
        return -1, f"<{' '.join(cmd)} 失败: {e}>"


def current_size() -> str:
    try:
        import AppKit  # type: ignore
        s = AppKit.NSScreen.mainScreen() or AppKit.NSScreen.screens()[0]
        f, v = s.frame(), s.visibleFrame()
        return (f"NSScreen {f.size.width:.0f}×{f.size.height:.0f}pt @{s.backingScaleFactor():g}x，"
                f"visibleFrame {v.size.width:.0f}×{v.size.height:.0f}")
    except Exception as e:  # noqa: BLE001
        try:
            import Quartz  # type: ignore
            b = Quartz.CGDisplayBounds(Quartz.CGMainDisplayID())
            return f"CGDisplayBounds {b.size.width:.0f}×{b.size.height:.0f}pt（NSScreen 失败: {e}）"
        except Exception as e2:  # noqa: BLE001
            return f"读不到屏幕尺寸：{e} / {e2}"


MODE_RE = re.compile(r"mode (\d+): res:(\d+)x(\d+)(?: hz:(\d+))?(?: color_depth:(\d+))?(?: (scaling:on))?(.*)$")


def parse_main_screen(text: str) -> tuple[str | None, list[Mode]]:
    """返回 (主屏的 persistent id, 模式列表)；只有一块屏时它就是主屏"""
    blocks = [b for b in re.split(r"\n(?=Persistent screen id:)", text) if b.startswith("Persistent screen id:")]
    if not blocks:
        return None, []
    main = next((b for b in blocks if "main display" in b), blocks[0])
    sid = re.search(r"Persistent screen id: (\S+)", main)
    modes = []
    for line in main.splitlines():
        m = MODE_RE.search(line.strip())
        if m:
            modes.append(Mode(int(m.group(1)), int(m.group(2)), int(m.group(3)), m.group(4), m.group(5),
                              bool(m.group(6)), "current mode" in (m.group(7) or "")))
    return (sid.group(1) if sid else None), modes


def choose(modes: list[Mode], prefer: list[tuple[int, int]]) -> Mode | None:
    for scaling in (False, True):
        pool = [m for m in modes if m.scaling == scaling]
        for w, h in prefer:
            hits = [m for m in pool if (m.w, m.h) == (w, h)]
            if hits:
                # 同分辨率挑刷新率 / 色深最高的
                return max(hits, key=lambda m: (int(m.hz or 0), int(m.depth or 0)))
        big = [m for m in pool if m.w >= 1440 and m.h >= 900 and m.w <= 2560]
        if big:
            return max(big, key=lambda m: (m.w * m.h, int(m.hz or 0), int(m.depth or 0)))
    return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--prefer", default="1920x1080,1920x1200,1680x1050,1600x900,1440x900")
    a = ap.parse_args()
    prefer = [tuple(int(x) for x in p.split("x")) for p in a.prefer.split(",") if "x" in p]
    print(f"调整前：{current_size()}")
    dp = shutil.which("displayplacer")
    if not dp:
        print("没有 displayplacer（brew install displayplacer），保持原分辨率")
        return 0
    rc, out = sh([dp, "list"])
    print(f"displayplacer list → rc={rc}\n{out}")
    sid, modes = parse_main_screen(out)
    cur = next((m for m in modes if m.current), None)
    print(f"主屏 id={sid}，{len(modes)} 个模式，当前 {cur}")
    target = choose(modes, prefer)
    if not sid or not target:
        print("没有合适的模式（≥1440×900），保持原分辨率")
        return 0
    if cur and (cur.w, cur.h, cur.scaling) == (target.w, target.h, target.scaling):
        print(f"已经是 {target.w}x{target.h}，不用切")
        return 0
    parts = [f"id:{sid}", f"res:{target.w}x{target.h}"]
    if target.hz:
        parts.append(f"hz:{target.hz}")
    if target.depth:
        parts.append(f"color_depth:{target.depth}")
    parts += [f"scaling:{'on' if target.scaling else 'off'}", "origin:(0,0)", "degree:0"]
    for spec in (" ".join(parts), f"id:{sid} mode:{target.n}"):
        rc, out = sh([dp, spec])
        print(f"displayplacer \"{spec}\" → rc={rc} {out[:400]}")
        time.sleep(3)
        now = current_size()
        print(f"切换后：{now}")
        if f"{target.w}×{target.h}" in now:
            return 0
    print("切换没有生效，保持现状")
    return 0


if __name__ == "__main__":
    sys.exit(main())
