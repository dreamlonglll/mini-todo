#!/usr/bin/env python3
"""介绍视频的素材采集：Xvfb 下以 2 倍缩放运行真实应用（debug 构建、隔离数据目录、样例数据），截带透明通道的窗口图，
并录制几段真实交互（x11grab 区域录屏 + 光标轨迹日志，视频里的光标按日志重画）。

场景的数据准备、启动与交互沿用 `../regress.py`（坐标仍按 1 倍逻辑像素写，这里统一乘缩放倍数）。
用法见同目录 README.md。只依赖 Python 3 标准库 + Xvfb / xdotool / xwd / ffmpeg。
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import math
import os
import shutil
import struct
import subprocess
import sys
import time
import zlib
from pathlib import Path
from types import SimpleNamespace

SCRIPT_DIR = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("regress", SCRIPT_DIR.parent / "regress.py")
regress = importlib.util.module_from_spec(_spec)
sys.modules["regress"] = regress
_spec.loader.exec_module(regress)

Scene = regress.Scene
MAIN, EDITOR, SUBTASK_VIEW, SETTINGS = regress.MAIN, regress.EDITOR, regress.SUBTASK_VIEW, regress.SETTINGS
COMPLETED, NOTIFICATION, FENCE = regress.COMPLETED, regress.NOTIFICATION, regress.FENCE
DARK, QUADRANT, FIXED = regress.DARK, regress.QUADRANT, regress.FIXED

SCALE = 2  # 2 倍缩放：素材在 1080p 视频里按约 0.6~0.75 缩小显示，文字清晰
FPS = 30

# ---------------------------------------------------------------------------
# 静态截图：沿用 regress.py 的同名场景（数据、打开方式、交互都一样），另加几张视频专用的
# ---------------------------------------------------------------------------

REUSED = [
    "main-list",
    "main-quadrant",
    "main-calendar",
    "main-dark",
    "main-fixed-dark",
    "editor-view",
    "editor-notify-select",
    "subtask-view",
    "notification",
    "settings-general",
    "settings-data",
    "fence",
    "fence-collapsed",
]

EXTRA = [
    # 两个收纳箱各截一张（吸附对齐的动画用）
    Scene("fence-pair", open="main,fence:1,fence:2", window=FENCE, fences="items", park="away", wait=3.5,
          steps=[("fences", "fence-pair")], shot=False),
]

# ---------------------------------------------------------------------------
# 录屏片段：("rec", 片段名, 窗口标题) 开始录该窗口区域，("stop",) 结束；光标移动一律平滑插值并记日志
# ---------------------------------------------------------------------------

CLIPS = [
    # 主窗口：悬停 → 展开子任务 → 切到四象限 → 跨象限拖动（改分类）→ 切回列表
    Scene(
        "clip-main",
        park="away",
        wait=4.0,
        shot=False,
        steps=[
            ("rec", "clip-main", MAIN),
            ("sleep", 0.6),
            ("move", MAIN, 220, 262),
            ("sleep", 0.5),
            ("move", MAIN, 220, 196),
            ("sleep", 0.4),
            ("move", MAIN, 78, 95),
            ("sleep", 0.3),
            ("click", MAIN, 78, 95),
            ("sleep", 1.6),
            ("move", MAIN, 205, 27),
            ("sleep", 0.5),
            ("click", MAIN, 205, 27),
            ("sleep", 1.4),
            ("call", "quadrant-drag"),
            ("sleep", 1.6),
            ("click", MAIN, 205, 27),
            ("sleep", 0.6),
            ("move", MAIN, 200, 560),
            ("sleep", 1.0),
            ("stop",),
        ],
    ),
    # 详情窗口：只读详情 → [编辑] → 单击描述切到源码、补两行任务清单 → 打开放大编辑（源码 / 预览分栏，右侧即时渲染）
    Scene(
        "clip-editor",
        open="main,editor:2:view",
        window=EDITOR,
        park="away",
        wait=4.0,
        shot=False,
        steps=[
            ("rec", "clip-editor", EDITOR),
            ("sleep", 0.8),
            ("move", EDITOR, 603, 28),
            ("sleep", 0.5),
            ("click", EDITOR, 603, 28),
            ("sleep", 1.4),
            ("move", EDITOR, 300, 200),
            ("sleep", 0.3),
            ("click", EDITOR, 300, 200),
            ("sleep", 0.8),
            ("key", "ctrl+End"),
            ("key", "Return"),
            ("key", "Return"),
            ("type", "- [x] outline"),
            ("key", "Return"),
            ("type", "- [ ] **demo**: GPU pipeline"),
            ("sleep", 1.0),
            ("move", EDITOR, 63, 167),
            ("sleep", 0.4),
            ("click", EDITOR, 63, 167),
            ("sleep", 2.6),
            ("move", EDITOR, 760, 420),
            ("sleep", 1.6),
            ("stop",),
        ],
    ),
    # 收纳箱是半透明窗口，x11grab 录不到透明度：改为在每个状态拍一张带透明通道的截图（关键帧），光标照常记轨迹。
    # 悬停 → Ctrl+滚轮放大 / 缩小图标 → 选中 → 右键菜单 → 单击空白处关闭
    Scene(
        "clip-fence",
        open="main,fence:1",
        window=FENCE,
        fences="items",
        park="away",
        wait=3.5,
        shot=False,
        step_delay=0.0,  # 节奏全由下面的 sleep 决定
        steps=[
            ("krec", "clip-fence", FENCE),
            ("sleep", 0.8),
            # 悬停后 500ms 会出原生提示：拍完再等
            ("move", FENCE, 60, 90),
            ("sleep", 0.15),
            ("snap",),
            ("sleep", 0.5),
            ("move", FENCE, 150, 95),
            ("sleep", 0.15),
            ("snap",),
            ("sleep", 0.5),
            ("ctrlwheel", FENCE, 150, 95, -1),
            ("sleep", 0.15),
            ("snap",),
            ("sleep", 0.4),
            ("ctrlwheel", FENCE, 150, 95, -1),
            ("sleep", 0.15),
            ("snap",),
            ("sleep", 0.9),
            ("ctrlwheel", FENCE, 150, 95, 1),
            ("sleep", 0.15),
            ("snap",),
            ("sleep", 0.4),
            ("ctrlwheel", FENCE, 150, 95, 1),
            ("sleep", 0.15),
            ("snap",),
            ("sleep", 0.6),
            ("click", FENCE, 140, 80),
            ("sleep", 0.15),
            ("snap",),
            ("sleep", 0.3),
            ("rclick", FENCE, 141, 88),
            ("sleep", 0.2),
            ("snap",),
            ("move", FENCE, 200, 118),
            ("sleep", 0.15),
            ("snap",),
            ("sleep", 1.4),
            # 单击空白处关闭菜单（Esc 不关）
            ("click", FENCE, 250, 300),
            ("sleep", 0.2),
            ("snap",),
            ("sleep", 0.8),
            ("kstop",),
        ],
    ),
]

# 四象限里按住"回复供应商的报价邮件"的颜色圆点（拖动手柄）从"紧急不重要"（左下）拖到"重要紧急"（左上）
CALLS = {
    "quadrant-drag": ("drag", MAIN, 45, 333, 45, 240),
}


def png_write(path: Path, w: int, h: int, rgba: bytes):
    """无依赖写 RGBA PNG（避免 ImageMagick 丢掉透明通道）"""
    raw = b"".join(b"\x00" + rgba[y * w * 4 : (y + 1) * w * 4] for y in range(h))

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 6))
        + chunk(b"IEND", b"")
    )


def ease_in_out(p: float) -> float:
    return 0.5 - 0.5 * math.cos(math.pi * p)


class VideoRunner(regress.Runner):
    def __init__(self, args):
        super().__init__(args)
        self.env["GPUI_X11_SCALE_FACTOR"] = str(SCALE)
        self.cur = (100 * SCALE, 1000 * SCALE)
        self.rec = None  # 正在录的片段：dict(name, proc, x, y, w, h, events)
        (self.out / "clips").mkdir(exist_ok=True)

    # ---- X ----
    def start_xvfb(self):
        subprocess.run(["pkill", "-f", f"Xvfb {self.display} "], capture_output=True)
        self.xvfb = subprocess.Popen(
            # -noreset：最后一个客户端断开时 X 服务器会重置并把光标放回屏幕中央，新窗口就带着悬停样式出现
            ["Xvfb", self.display, "-screen", "0", f"{1920 * SCALE}x{1080 * SCALE}x24", "-nolisten", "tcp", "-noreset"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        for _ in range(100):
            if subprocess.run(["xdotool", "getdisplaygeometry"], capture_output=True, env=self.env).returncode == 0:
                return
            time.sleep(0.1)
        raise RuntimeError("Xvfb 没有起来")

    def window_ids(self, title):
        out = self.xdo("search", "--onlyvisible", "--name", ".").split()
        return [w for w in out if self.xdo("getwindowname", w) == title]

    def now(self):
        """片段时钟：关键帧模式扣掉拍照（xwd）花掉的时间，回放时节奏与真实操作一致"""
        return time.time() - (self.rec.get("paused", 0.0) if self.rec else 0.0)

    def log(self, kind, x, y):
        if self.rec is not None:
            now = self.now()
            self.rec["events"].append([now, kind, x, y])
            if kind != "move":
                self.rec["last_action"] = now

    def acted(self):
        """键盘 / 滚轮等不带坐标的输入：关键帧的显示时间从这一刻算"""
        if self.rec is not None:
            self.rec["last_action"] = self.now()

    def glide(self, x, y, speed=1400.0):
        """光标平滑移到屏幕坐标 (x, y)（物理像素），沿途记日志"""
        x0, y0 = self.cur
        dist = math.hypot(x - x0, y - y0)
        if dist < 1:
            return
        dur = min(1.1, 0.28 + dist / (speed * SCALE))
        steps = max(2, int(dur * 60))
        t0 = time.time()
        for i in range(1, steps + 1):
            p = ease_in_out(i / steps)
            px, py = round(x0 + (x - x0) * p), round(y0 + (y - y0) * p)
            self.xdo("mousemove", px, py)
            self.log("move", px, py)
            delay = t0 + dur * i / steps - time.time()
            if delay > 0:
                time.sleep(delay)
        self.cur = (x, y)
        if self.rec is not None:
            self.rec["last_action"] = self.now()

    def at(self, title, x, y):
        wid = self.window_id(title)
        if wid is None:
            raise RuntimeError(f"找不到窗口「{title}」")
        gx, gy, _, _ = self.geometry(wid)
        if self.rec is None:
            self.xdo("mousemove", gx + x * SCALE, gy + y * SCALE)
            self.cur = (gx + x * SCALE, gy + y * SCALE)
        else:
            self.glide(gx + x * SCALE, gy + y * SCALE)
        return wid

    def park(self, mode):
        if mode == "main":
            wid = self.window_id(MAIN)
            if wid:
                x, y, _, _ = self.geometry(wid)
                self.xdo("mousemove", x + 200 * SCALE, y + 135 * SCALE)
                time.sleep(0.3)
                self.xdo("mousemove", x + 100 * SCALE, y + 45 * SCALE)
                self.cur = (x + 100 * SCALE, y + 45 * SCALE)
            time.sleep(0.5)
        elif mode == "away":
            self.xdo("mousemove", 100 * SCALE, 1000 * SCALE)
            self.cur = (100 * SCALE, 1000 * SCALE)
            time.sleep(0.3)

    def screenshot(self, title, path):
        wid = self.window_id(title)
        if wid is None:
            print(f"    !! 找不到窗口「{title}」")
            return False
        self.capture_window(wid, path)
        return True

    def capture_window(self, wid, path: Path):
        """xwd 抓 32 位窗口（带透明通道），去预乘后写 PNG"""
        self.xwd_to_png(self.xwd(wid), path)

    def xwd(self, wid) -> bytes:
        return subprocess.run(["xwd", "-silent", "-id", str(wid)], capture_output=True, env=self.env, check=True).stdout

    @staticmethod
    def xwd_to_png(data: bytes, path: Path):
        hdr = struct.unpack(">25I", data[:100])
        size, w, h, bpp, bpl, ncolors = hdr[0], hdr[4], hdr[5], hdr[11], hdr[12], hdr[19]
        off = size + ncolors * 12
        px = data[off : off + bpl * h]
        if bpp != 32:
            raise RuntimeError(f"窗口不是 32 位（bpp={bpp}）")
        out = bytearray(w * h * 4)
        for y in range(h):
            row = px[y * bpl : y * bpl + w * 4]
            b, g, r, a = row[0::4], row[1::4], row[2::4], row[3::4]
            o = y * w * 4
            out[o + 0 : o + w * 4 : 4] = r
            out[o + 1 : o + w * 4 : 4] = g
            out[o + 2 : o + w * 4 : 4] = b
            out[o + 3 : o + w * 4 : 4] = a
        # GPUI 在 X11 上输出预乘 alpha；半透明像素还原成直通 alpha
        for i in range(0, len(out), 4):
            a = out[i + 3]
            if 0 < a < 255:
                out[i] = min(255, out[i] * 255 // a)
                out[i + 1] = min(255, out[i + 1] * 255 // a)
                out[i + 2] = min(255, out[i + 2] * 255 // a)
        png_write(path, w, h, bytes(out))
        print(f"    {path.name} {w}x{h}")

    # ---- 录屏 ----
    def rec_start(self, name, title):
        wid = self.window_id(title)
        if wid is None:
            raise RuntimeError(f"找不到窗口「{title}」")
        x, y, w, h = self.geometry(wid)
        clip_dir = self.out / "clips"
        # 同时存一张带透明通道的窗口图：视频里拿它当遮罩去掉录屏的黑角
        self.capture_window(wid, clip_dir / f"{name}-mask.png")
        mkv = clip_dir / f"{name}.mkv"
        proc = subprocess.Popen(
            ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "x11grab", "-draw_mouse", "0",
             "-framerate", str(FPS), "-video_size", f"{w}x{h}", "-i", f"{self.display}+{x},{y}",
             "-c:v", "libx264", "-preset", "ultrafast", "-qp", "0", "-copyts", "-fps_mode", "passthrough", str(mkv)],
            stdin=subprocess.PIPE,
            env=self.env,
        )
        self.rec = dict(name=name, proc=proc, x=x, y=y, w=w, h=h, events=[], mkv=mkv)
        self.log("move", *self.cur)
        time.sleep(0.5)

    def rec_stop(self):
        rec = self.rec
        self.rec = None
        rec["proc"].communicate(b"q", timeout=30)
        pts = subprocess.run(
            ["ffprobe", "-v", "error", "-select_streams", "v", "-show_entries", "frame=pts_time", "-of", "csv=p=0",
             str(rec["mkv"])],
            capture_output=True, text=True, check=True,
        ).stdout.split()
        pts0 = float(pts[0].strip(","))
        frames_dir = self.out / "clips" / rec["name"]
        shutil.rmtree(frames_dir, ignore_errors=True)
        frames_dir.mkdir()
        subprocess.run(
            ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-i", str(rec["mkv"]),
             "-vf", f"setpts=PTS-STARTPTS,fps={FPS}", "-q:v", "2", str(frames_dir / "%04d.jpg")],
            check=True,
        )
        count = len(list(frames_dir.glob("*.jpg")))
        events = [
            dict(t=round(t - pts0, 3), kind=kind, x=x - rec["x"], y=y - rec["y"])
            for t, kind, x, y in rec["events"]
        ]
        meta = dict(name=rec["name"], fps=FPS, frames=count, width=rec["w"], height=rec["h"], scale=SCALE, events=events)
        (self.out / "clips" / f"{rec['name']}.json").write_text(json.dumps(meta, ensure_ascii=False, indent=1))
        rec["mkv"].unlink()
        print(f"    片段 {rec['name']}：{count} 帧，{len(events)} 个光标事件")

    def krec_start(self, name, title):
        wid = self.window_id(title)
        if wid is None:
            raise RuntimeError(f"找不到窗口「{title}」")
        x, y, w, h = self.geometry(wid)
        now = time.time()
        self.rec = dict(name=name, mode="keyframes", wid=wid, x=x, y=y, w=w, h=h, t0=now, events=[], shots=[],
                        last_action=now, last_shot=now - 1)
        self.log("move", *self.cur)
        self.snap()

    def snap(self):
        rec = self.rec
        now = self.now()
        t_show = rec["last_action"] if rec["last_action"] > rec["last_shot"] else now
        began = time.time()
        rec["shots"].append((t_show, self.xwd(rec["wid"])))
        rec["paused"] = rec.get("paused", 0.0) + time.time() - began
        rec["last_shot"] = self.now()

    def krec_stop(self):
        rec = self.rec
        end = self.now()
        self.rec = None
        frames_dir = self.out / "clips" / rec["name"]
        shutil.rmtree(frames_dir, ignore_errors=True)
        frames_dir.mkdir()
        shots = []
        for i, (t, data) in enumerate(rec["shots"], 1):
            file = f"{i:02d}.png"
            self.xwd_to_png(data, frames_dir / file)
            shots.append(dict(t=round(max(0.0, t - rec["t0"]), 3), file=file))
        events = [
            dict(t=round(t - rec["t0"], 3), kind=kind, x=x - rec["x"], y=y - rec["y"])
            for t, kind, x, y in rec["events"]
        ]
        meta = dict(name=rec["name"], mode="keyframes", duration=round(end - rec["t0"], 3), width=rec["w"],
                    height=rec["h"], scale=SCALE, shots=shots, events=events)
        (self.out / "clips" / f"{rec['name']}.json").write_text(json.dumps(meta, ensure_ascii=False, indent=1))
        print(f"    关键帧片段 {rec['name']}：{len(shots)} 张，{len(events)} 个光标事件")

    # ---- 交互 ----
    def run_step(self, scene, step, ctx):
        kind = step[0]
        if kind == "call":
            step = CALLS[step[1]]
            kind = step[0]
        if kind in ("click", "rclick", "dbl"):
            self.at(step[1], step[2], step[3])
            time.sleep(0.12)
            x, y = self.cur
            if kind == "click":
                self.log("down", x, y)
                self.xdo("click", 1)
                self.log("up", x, y)
            elif kind == "rclick":
                self.log("rdown", x, y)
                self.xdo("click", 3)
            else:
                self.log("down", x, y)
                self.xdo("click", "--repeat", 2, "--delay", 80, 1)
                self.log("up", x, y)
        elif kind == "drag":
            _, title, x1, y1, x2, y2 = step
            self.at(title, x1, y1)
            time.sleep(0.25)
            self.log("down", *self.cur)
            self.xdo("mousedown", 1)
            time.sleep(0.15)
            wid = self.window_id(title)
            gx, gy, _, _ = self.geometry(wid)
            # 先小幅移动触发拖动，再平滑拖到目标
            self.glide(self.cur[0] + 8 * SCALE, self.cur[1] - 6 * SCALE, speed=400)
            self.glide(gx + x2 * SCALE, gy + y2 * SCALE, speed=700)
            time.sleep(0.35)
            self.log("up", *self.cur)
            self.xdo("mouseup", 1)
        elif kind == "ctrlwheel":
            _, title, x, y, n = step
            self.at(title, x, y)
            self.xdo("keydown", "ctrl")
            for _ in range(abs(n)):
                self.xdo("click", 4 if n < 0 else 5)
                self.acted()
                time.sleep(0.35)
            self.xdo("keyup", "ctrl")
        elif kind == "away":
            if self.rec is None:
                self.park("away")
            else:
                self.glide(100 * SCALE, 1000 * SCALE)
        elif kind == "rec":
            self.rec_start(step[1], step[2])
        elif kind == "stop":
            self.rec_stop()
        elif kind == "krec":
            self.krec_start(step[1], step[2])
        elif kind == "snap":
            self.snap()
        elif kind == "kstop":
            self.krec_stop()
        elif kind == "key":
            self.xdo("key", step[1])
            self.acted()
        elif kind == "type":
            text = step[1]
            if text.startswith("-"):  # xdotool 会把以 - 开头的文字当成选项
                self.xdo("key", "minus")
                text = text[1:]
            self.xdo("type", "--delay", 70, text)
            self.acted()
        elif kind == "fences":
            ids = self.window_ids(FENCE) + self.window_ids(f"{FENCE} 2")
            ids.sort(key=lambda w: self.geometry(w)[0])
            for i, wid in enumerate(ids, 1):
                self.capture_window(wid, self.out / f"{step[1]}-{i}.png")
                x, y, w, h = self.geometry(wid)
                print(f"      位置 {x},{y} {w}x{h}")
        else:
            super().run_step(scene, step, ctx)


def main():
    os.environ["TZ"] = "Asia/Shanghai"
    time.tzset()
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", default=str(regress.WORKSPACE / "target" / "intro-video" / "assets"),
                        help="素材输出目录（默认 target/intro-video/assets）")
    parser.add_argument("--display", default=":77", help="Xvfb 显示号（默认 :77）")
    parser.add_argument("--only", action="append", default=[], help="只跑名字匹配的场景（glob，可多次）")
    parser.add_argument("--no-build", action="store_true", help="跳过 cargo build")
    parser.add_argument("--tmp", default=None, help="临时数据目录的父目录")
    args = parser.parse_args()

    for tool in ("Xvfb", "xdotool", "xwd", "ffmpeg", "ffprobe", "convert"):
        if shutil.which(tool) is None:
            print(f"缺少命令：{tool}", file=sys.stderr)
            return 1
    if not args.no_build:
        subprocess.run(["cargo", "build", "-p", "mini-todo"], cwd=regress.WORKSPACE, check=True)
        subprocess.run(["cargo", "build", "-p", "mini-todo-core", "--example", "seed"], cwd=regress.WORKSPACE, check=True)

    by_name = {s.name: s for s in regress.SCENES}
    scenes = [by_name[n] for n in REUSED] + EXTRA + CLIPS
    if args.only:
        import fnmatch

        scenes = [s for s in scenes if any(fnmatch.fnmatch(s.name, p) for p in args.only)]

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    icon = regress.WORKSPACE / "crates" / "app" / "assets" / "icon.ico"
    subprocess.run(["convert", f"{icon}[5]", str(out / "icon.png")], check=True)  # 256×256 那一帧

    runner = VideoRunner(SimpleNamespace(display=args.display, out=args.out, tmp=args.tmp, keep_data=False, ref=[], before=[]))
    runner.start_xvfb()
    try:
        for scene in scenes:
            # regress 的场景停光标用"主窗口标题栏"会留下悬停效果，视频素材一律移开
            if scene.park == "main":
                scene.park = "away"
            runner.run_scene(scene)
            if runner.rec is not None:
                runner.krec_stop() if runner.rec.get("mode") == "keyframes" else runner.rec_stop()
    finally:
        runner.stop_xvfb()
    failed = [r for r in runner.results if not r[2]]
    for name, what, ok, detail in failed:
        print(f"  FAIL  {name}  {what}  {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
