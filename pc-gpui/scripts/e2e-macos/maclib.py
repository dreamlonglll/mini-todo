"""macOS e2e 的辅助函数：Quartz 窗口枚举、screencapture 截图、Vision OCR、Accessibility 探测、应用进程、用例框架。

依赖（`pip install pyobjc-framework-Quartz pyobjc-framework-Vision pyobjc-framework-ApplicationServices pillow`）：
Quartz（CGWindowListCopyWindowInfo，必需）、Vision（OCR）、ApplicationServices（AX，只做信息性探测）、AppKit（随 Quartz 的依赖
pyobjc-framework-Cocoa 安装；NSRunningApplication）、Pillow。缺了可选的模块时相关检查报"不可用"，不会让脚本崩掉。

用例框架（Report / Checks / Skip、results.json / summary.md / fail-<id>.txt / emit/*.png）与 e2e-windows/winlib.py 同一套约定，
所以失败注解与截图注解直接复用 e2e-windows/emit_annotations.py。
"""
from __future__ import annotations

import json
import math
import os
import re
import signal
import sqlite3
import subprocess
import sys
import time
import traceback
from dataclasses import dataclass
from difflib import SequenceMatcher
from pathlib import Path
from typing import Callable

from PIL import Image

try:
    import Quartz  # type: ignore
    QUARTZ_ERR = None
except Exception as e:  # noqa: BLE001
    Quartz = None
    QUARTZ_ERR = e

try:
    import Vision  # type: ignore
    from Foundation import NSURL  # type: ignore
    VISION_ERR = None
except Exception as e:  # noqa: BLE001
    Vision = None
    NSURL = None
    VISION_ERR = e

try:
    import ApplicationServices as AS  # type: ignore
    AX_ERR = None
except Exception as e:  # noqa: BLE001
    AS = None
    AX_ERR = e

try:
    import AppKit  # type: ignore
    APPKIT_ERR = None
except Exception as e:  # noqa: BLE001
    AppKit = None
    APPKIT_ERR = e


# ----------------------------------------------------------------------------- 杂项

def run(cmd: list[str], timeout: float = 60, check: bool = False) -> subprocess.CompletedProcess:
    r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
    if check and r.returncode != 0:
        raise AssertionError(f"{' '.join(cmd)} 退出码 {r.returncode}: {(r.stdout + r.stderr).strip()[:800]}")
    return r


def sh_out(cmd: list[str], timeout: float = 30) -> str:
    try:
        r = run(cmd, timeout=timeout)
        return (r.stdout + r.stderr).strip()
    except Exception as e:  # noqa: BLE001
        return f"<{' '.join(cmd)} 失败: {e}>"


class Timeout(AssertionError):
    pass


def wait_until(fn: Callable[[], object], timeout: float, what: str, interval: float = 0.5):
    deadline = time.time() + timeout
    last_exc = None
    while True:
        try:
            v = fn()
            if v:
                return v
        except Timeout:
            raise
        except AssertionError:
            raise
        except Exception as e:  # noqa: BLE001
            last_exc = e
        if time.time() >= deadline:
            extra = f"（最后一次异常：{last_exc!r}）" if last_exc else ""
            raise Timeout(f"等待超时 {timeout:.0f}s：{what}{extra}")
        time.sleep(interval)


def tail(text: str, n: int) -> list[str]:
    lines = text.splitlines()
    return lines[-n:]


# ----------------------------------------------------------------------------- 窗口（Quartz）

@dataclass
class WinInfo:
    number: int
    pid: int
    layer: int
    x: float
    y: float
    w: float
    h: float
    name: str | None
    owner: str | None
    alpha: float

    def describe(self) -> str:
        return (f"#{self.number} pid={self.pid} layer={self.layer} ({self.x:.0f},{self.y:.0f} {self.w:.0f}×{self.h:.0f}pt) "
                f"owner={self.owner!r} name={self.name!r} alpha={self.alpha:.2f}")


def _num(v, default=0.0) -> float:
    try:
        return float(v)
    except Exception:  # noqa: BLE001
        return default


def list_windows() -> list[WinInfo]:
    """屏幕上的全部窗口（不含桌面元素），按前后顺序（最前在前）。坐标是全局逻辑点、主屏左上为原点"""
    if Quartz is None:
        raise AssertionError(f"pyobjc Quartz 不可用：{QUARTZ_ERR!r}")
    opts = Quartz.kCGWindowListOptionOnScreenOnly | Quartz.kCGWindowListExcludeDesktopElements
    arr = Quartz.CGWindowListCopyWindowInfo(opts, Quartz.kCGNullWindowID) or []
    out = []
    for d in arr:
        b = d.get("kCGWindowBounds") or {}
        name = d.get("kCGWindowName")
        owner = d.get("kCGWindowOwnerName")
        out.append(WinInfo(
            number=int(d.get("kCGWindowNumber", 0)),
            pid=int(d.get("kCGWindowOwnerPID", 0)),
            layer=int(d.get("kCGWindowLayer", 0)),
            x=_num(b.get("X")), y=_num(b.get("Y")), w=_num(b.get("Width")), h=_num(b.get("Height")),
            name=str(name) if name is not None else None,
            owner=str(owner) if owner is not None else None,
            alpha=_num(d.get("kCGWindowAlpha", 1.0), 1.0),
        ))
    return out


def windows_of(pid: int, layer: int | None = 0) -> list[WinInfo]:
    return [w for w in list_windows() if w.pid == pid and (layer is None or w.layer == layer) and w.w > 1 and w.h > 1]


def dump_windows(label: str, pid: int | None = None) -> None:
    """打印窗口列表（本进程的窗口前面标 *）：layer 0 = 普通窗口，25 = 菜单栏状态项，24 = 菜单栏"""
    try:
        ws = list_windows()
    except Exception as e:  # noqa: BLE001
        print(f"  [windows] {label}: 枚举失败 {e}")
        return
    mine = [w for w in ws if pid and w.pid == pid]
    print(f"  [windows] {label}: 共 {len(ws)} 个屏幕上的窗口，本进程 {len(mine)} 个", flush=True)
    for w in ws:
        mark = "*" if pid and w.pid == pid else " "
        print(f"   {mark} {w.describe()}")


def main_display_points() -> tuple[float, float]:
    if Quartz is None:
        return (0.0, 0.0)
    try:
        b = Quartz.CGDisplayBounds(Quartz.CGMainDisplayID())
        return (float(b.size.width), float(b.size.height))
    except Exception as e:  # noqa: BLE001
        print(f"  (CGDisplayBounds 失败: {e})")
        return (0.0, 0.0)


def screen_capture_preflight() -> str:
    if Quartz is None:
        return "未知（Quartz 不可用）"
    fn = getattr(Quartz, "CGPreflightScreenCaptureAccess", None)
    if fn is None:
        return "未知（pyobjc 没有 CGPreflightScreenCaptureAccess）"
    try:
        return "已授权" if fn() else "未授权"
    except Exception as e:  # noqa: BLE001
        return f"未知（{e}）"


# ----------------------------------------------------------------------------- 截图

def capture_window(win: WinInfo, path: Path) -> Image.Image:
    """`screencapture -l` 截单个窗口（不带阴影；被遮挡也能截到）。像素尺寸 = 逻辑点 × 显示器缩放"""
    path.unlink(missing_ok=True)
    r = run(["screencapture", "-x", "-o", "-l", str(win.number), str(path)], timeout=30)
    if r.returncode != 0 or not path.exists() or path.stat().st_size == 0:
        raise AssertionError(f"screencapture -l {win.number} 失败（rc={r.returncode}）：{(r.stdout + r.stderr).strip()[:400]}")
    img = Image.open(path)
    img.load()
    return img.convert("RGBA")


def capture_screen(path: Path) -> Image.Image:
    path.unlink(missing_ok=True)
    r = run(["screencapture", "-x", str(path)], timeout=30)
    if r.returncode != 0 or not path.exists() or path.stat().st_size == 0:
        raise AssertionError(f"screencapture 整屏失败（rc={r.returncode}）：{(r.stdout + r.stderr).strip()[:400]}")
    img = Image.open(path)
    img.load()
    return img.convert("RGBA")


def content_stats(img: Image.Image) -> dict:
    """防"截图权限失效只截到壁纸 / 空白"的假成功：不透明像素占比、颜色数、非主色占比"""
    rgba = img.convert("RGBA")
    total = rgba.width * rgba.height
    colors = rgba.getcolors(1 << 24) or []
    opaque = [(n, c) for n, c in colors if c[3] > 0]
    n_opaque = sum(n for n, _ in opaque)
    rgb_counts: dict[tuple, int] = {}
    for n, c in opaque:
        rgb_counts[c[:3]] = rgb_counts.get(c[:3], 0) + n
    if rgb_counts:
        dominant, n_dom = max(rgb_counts.items(), key=lambda kv: kv[1])
    else:
        dominant, n_dom = (0, 0, 0), 0
    return {
        "size": f"{rgba.width}x{rgba.height}",
        "opaque_ratio": round(n_opaque / total, 4) if total else 0.0,
        "distinct_colors": len(rgb_counts),
        "dominant": "#%02X%02X%02X" % dominant,
        "non_dominant_ratio": round(1 - n_dom / n_opaque, 4) if n_opaque else 0.0,
    }


def has_content(stats: dict) -> bool:
    """纯色壁纸 / 空白截图：1~2 种颜色、非主色 0%。真实窗口哪怕大片白底，文字与图标的抗锯齿也有几十上百种颜色。
    界面上"有没有字"由 OCR 断言，这里只挡掉明显的假截图"""
    return stats["opaque_ratio"] >= 0.5 and stats["distinct_colors"] >= 16 and stats["non_dominant_ratio"] >= 0.001


def flatten(img: Image.Image, bg=(255, 255, 255)) -> Image.Image:
    base = Image.new("RGB", img.size, bg)
    rgba = img.convert("RGBA")
    base.paste(rgba, mask=rgba.split()[3])
    return base


def save_emit(img: Image.Image, out_dir: Path, name: str, max_w: int = 480, colors: int = 24) -> Path:
    """缩小 + 调色板量化后存到 <out>/emit/，由 e2e-windows/emit_annotations.py 作为注解发出（每个注解最多 3900 字符）"""
    d = Path(out_dir) / "emit"
    d.mkdir(parents=True, exist_ok=True)
    im = flatten(img) if img.mode == "RGBA" else img.convert("RGB")
    if im.width > max_w:
        im = im.resize((max_w, max(1, round(im.height * max_w / im.width))), Image.LANCZOS)
    n = len(list(d.glob("*.png")))
    path = d / f"{n:02d}-{name}.png"
    im.quantize(colors).save(path, "PNG", optimize=True)
    size = path.stat().st_size
    chunks = math.ceil(math.ceil(size / 3) * 4 / 3900)   # base64 长度 / 每个注解 3900 字符
    print(f"    emit {path.name}: {im.width}x{im.height} {size} B ≈ {chunks} 个注解分片")
    return path


# ----------------------------------------------------------------------------- OCR（Vision）

@dataclass
class OcrLine:
    text: str
    conf: float
    x: float   # 左上角（像素，图片坐标）
    y: float
    w: float
    h: float


def vision_languages() -> str:
    if Vision is None:
        return f"Vision 不可用：{VISION_ERR!r}"
    try:
        req = Vision.VNRecognizeTextRequest.alloc().init()
        req.setRecognitionLevel_(Vision.VNRequestTextRecognitionLevelAccurate)
        langs, err = req.supportedRecognitionLanguagesAndReturnError_(None)
        return ", ".join(str(x) for x in (langs or [])) or f"<空> {err}"
    except Exception as e:  # noqa: BLE001
        return f"<查询失败 {e}>"


def ocr(img: Image.Image, work_path: Path) -> list[OcrLine]:
    """Vision 文字识别（accurate、zh-Hans + en-US）。小图先放大 2 倍：1x 显示器上 13~14px 的中文识别率明显更高"""
    if Vision is None:
        raise AssertionError(f"Vision OCR 不可用：{VISION_ERR!r}")
    im = flatten(img)
    factor = 2 if im.width < 1600 else 1
    if factor != 1:
        im = im.resize((im.width * factor, im.height * factor), Image.LANCZOS)
    im.save(work_path)
    url = NSURL.fileURLWithPath_(str(work_path))
    handler = Vision.VNImageRequestHandler.alloc().initWithURL_options_(url, {})
    req = Vision.VNRecognizeTextRequest.alloc().init()
    req.setRecognitionLevel_(Vision.VNRequestTextRecognitionLevelAccurate)
    req.setRecognitionLanguages_(["zh-Hans", "en-US"])
    req.setUsesLanguageCorrection_(True)
    ok, err = handler.performRequests_error_([req], None)
    if not ok:
        raise AssertionError(f"Vision performRequests 失败：{err}")
    out: list[OcrLine] = []
    W, H = im.width, im.height
    for obs in req.results() or []:
        cands = obs.topCandidates_(1)
        if not cands or len(cands) == 0:
            continue
        c = cands[0]
        bb = obs.boundingBox()
        # Vision 的 boundingBox 是归一化坐标、左下为原点
        x = bb.origin.x * W / factor
        y = (1 - bb.origin.y - bb.size.height) * H / factor
        out.append(OcrLine(str(c.string()), float(c.confidence()), x, y,
                           bb.size.width * W / factor, bb.size.height * H / factor))
    out.sort(key=lambda l: (round(l.y / 6), l.x))
    return out


def print_ocr(lines: list[OcrLine], label: str, limit: int = 60) -> None:
    print(f"    OCR[{label}] {len(lines)} 行：")
    for l in lines[:limit]:
        print(f"      ({l.x:4.0f},{l.y:4.0f}) {l.conf:.2f} {l.text}")
    if len(lines) > limit:
        print(f"      … 还有 {len(lines) - limit} 行")


_PUNCT = str.maketrans({"：": ":", "，": ",", "。": ".", "（": "(", "）": ")", "；": ";", "！": "!", "？": "?"})


def norm(s: str) -> str:
    return re.sub(r"\s+", "", s.translate(_PUNCT)).lower()


def fuzzy_find(needle: str, lines: list[OcrLine], ratio: float = 0.6, min_len: int = 4) -> tuple[bool, str]:
    """OCR 文字里有没有 needle：去空白 / 统一全角标点后子串匹配，否则看最长公共子串是否 ≥ 60%（标题可能被截断）"""
    hay = norm("".join(l.text for l in lines))
    n = norm(needle)
    if not n:
        return False, "空字符串"
    if n in hay:
        return True, "完整匹配"
    m = SequenceMatcher(None, n, hay, autojunk=False).find_longest_match(0, len(n), 0, len(hay))
    need = min(len(n), max(min_len, math.ceil(len(n) * ratio)))
    return m.size >= need, f"最长公共子串 {m.size}/{len(n)}（需 ≥{need}）'{n[m.a:m.a + m.size]}'"


# ----------------------------------------------------------------------------- Accessibility（信息性）

def _ax(el, name):
    try:
        err, val = AS.AXUIElementCopyAttributeValue(el, name, None)
    except Exception as e:  # noqa: BLE001
        return None, f"exc {e}"
    if err != 0:
        return None, f"err {err}"
    return val, None


def _ax_xy(v, kind: str) -> tuple[float, float] | None:
    if v is None:
        return None
    try:
        t = AS.kAXValueCGPointType if kind == "point" else AS.kAXValueCGSizeType
        r = AS.AXValueGetValue(v, t, None)
        if isinstance(r, tuple) and r[0]:
            p = r[1]
            return (float(p.x), float(p.y)) if kind == "point" else (float(p.width), float(p.height))
    except Exception:  # noqa: BLE001
        pass
    pat = r"x:([-\d.]+)\s+y:([-\d.]+)" if kind == "point" else r"w:([-\d.]+)\s+h:([-\d.]+)"
    m = re.search(pat, str(v))
    return (float(m.group(1)), float(m.group(2))) if m else None


def ax_probe(pid: int) -> dict:
    """读本进程的 AX 树：每个窗口的红绿灯按钮、应用菜单栏的顶层菜单、菜单栏状态项。只打印，不判失败。
    返回 {"traffic": "可见" | "隐藏" | "未知", "menus": [...], "extras": n}"""
    res = {"traffic": "未知", "menus": [], "extras": None}
    if AS is None:
        print(f"    [AX] ApplicationServices 不可用：{AX_ERR!r}")
        return res
    try:
        trusted = bool(AS.AXIsProcessTrusted())
    except Exception as e:  # noqa: BLE001
        trusted = f"未知（{e}）"
    print(f"    [AX] AXIsProcessTrusted = {trusted}")
    app = AS.AXUIElementCreateApplication(pid)
    wins, err = _ax(app, "AXWindows")
    if wins is None:
        print(f"    [AX] 读 AXWindows 失败：{err}（-25211 = 没有辅助功能权限）")
        return res
    verdicts = []
    for i, w in enumerate(wins):
        title, _ = _ax(w, "AXTitle")
        pos = _ax_xy(_ax(w, "AXPosition")[0], "point")
        size = _ax_xy(_ax(w, "AXSize")[0], "size")
        print(f"    [AX] 窗口 {i}: title={title!r} pos={pos} size={size}")
        children, _ = _ax(w, "AXChildren")
        child_subroles = []
        for ch in children or []:
            sr, _ = _ax(ch, "AXSubrole")
            child_subroles.append(str(sr) if sr is not None else None)
        visible_any = False
        for attr in ("AXCloseButton", "AXMinimizeButton", "AXZoomButton"):
            b, berr = _ax(w, attr)
            if b is None:
                print(f"      {attr}: 无（{berr}）")
                continue
            bpos = _ax_xy(_ax(b, "AXPosition")[0], "point")
            bsize = _ax_xy(_ax(b, "AXSize")[0], "size")
            enabled, _ = _ax(b, "AXEnabled")
            hidden, _ = _ax(b, "AXHidden")
            in_children = attr in child_subroles
            print(f"      {attr}: pos={bpos} size={bsize} enabled={enabled} AXHidden={hidden} 在窗口子元素里={in_children}")
            if in_children and bsize and bsize[0] > 0 and bsize[1] > 0 and hidden is not True:
                visible_any = True
        print(f"      子元素 subrole：{[s for s in child_subroles if s]}")
        verdicts.append(visible_any)
    if verdicts:
        res["traffic"] = "可见" if any(verdicts) else "隐藏"
    menubar, err = _ax(app, "AXMenuBar")
    if menubar is not None:
        items, _ = _ax(menubar, "AXChildren")
        for it in items or []:
            t, _ = _ax(it, "AXTitle")
            res["menus"].append(str(t) if t is not None else "?")
        print(f"    [AX] 应用菜单栏顶层菜单：{res['menus']}")
    else:
        print(f"    [AX] 没有 AXMenuBar（{err}）")
    extras, err = _ax(app, "AXExtrasMenuBar")
    if extras is not None:
        items, _ = _ax(extras, "AXChildren")
        res["extras"] = len(items or [])
        descs = []
        for it in items or []:
            d, _ = _ax(it, "AXDescription")
            t, _ = _ax(it, "AXTitle")
            descs.append(f"{t!r}/{d!r}")
        print(f"    [AX] 菜单栏状态项（AXExtrasMenuBar）{res['extras']} 个：{descs}")
    else:
        print(f"    [AX] 没有 AXExtrasMenuBar（{err}）——没有菜单栏图标")
        res["extras"] = 0
    return res


# 系统红绿灯的颜色（活动窗口；非活动窗口是灰色，像素法识别不了）
TRAFFIC_COLORS = {"红": [(255, 95, 87), (236, 106, 94)], "黄": [(254, 188, 46), (245, 191, 79)],
                  "绿": [(40, 200, 64), (97, 197, 84)]}


def traffic_light_pixels(img: Image.Image, scale: float) -> dict:
    """窗口截图左上角（0..90 × 0..36 pt）里接近系统红绿灯颜色的像素数（信息性）"""
    rgba = img.convert("RGBA")
    x1, y1 = min(rgba.width, round(90 * scale)), min(rgba.height, round(36 * scale))
    counts = {k: 0 for k in TRAFFIC_COLORS}
    px = rgba.load()
    for y in range(y1):
        for x in range(x1):
            r, g, b, a = px[x, y]
            if a < 128:
                continue
            for k, refs in TRAFFIC_COLORS.items():
                if any(abs(r - c[0]) + abs(g - c[1]) + abs(b - c[2]) <= 60 for c in refs):
                    counts[k] += 1
    return counts


# ----------------------------------------------------------------------------- 键盘（CGEvent）

KEY_Q = 12  # kVK_ANSI_Q


def post_key_to_pid(pid: int, keycode: int, flags: int) -> str:
    """按下 + 抬起一个键，直接投递到该进程（CGEventPostToPid：不经前台应用，不会误按到别的程序上）"""
    if Quartz is None:
        raise AssertionError(f"pyobjc Quartz 不可用：{QUARTZ_ERR!r}")
    post = getattr(Quartz, "CGEventPostToPid", None)
    if post is None:
        raise AssertionError("pyobjc 没有 CGEventPostToPid")
    src = Quartz.CGEventSourceCreate(Quartz.kCGEventSourceStateHIDSystemState)
    for down in (True, False):
        ev = Quartz.CGEventCreateKeyboardEvent(src, keycode, down)
        Quartz.CGEventSetFlags(ev, flags)
        post(pid, ev)
        time.sleep(0.05)
    return "CGEventPostToPid"


def post_key_hid(keycode: int, flags: int) -> str:
    """按下 + 抬起一个键，投递到系统输入流（发给前台应用；调用前先确认前台是目标进程）"""
    src = Quartz.CGEventSourceCreate(Quartz.kCGEventSourceStateHIDSystemState)
    for down in (True, False):
        ev = Quartz.CGEventCreateKeyboardEvent(src, keycode, down)
        Quartz.CGEventSetFlags(ev, flags)
        Quartz.CGEventPost(Quartz.kCGHIDEventTap, ev)
        time.sleep(0.05)
    return "CGEventPost(HID)"


def frontmost_pid() -> int | None:
    if AppKit is None:
        return None
    app = AppKit.NSWorkspace.sharedWorkspace().frontmostApplication()
    return int(app.processIdentifier()) if app is not None else None


# ----------------------------------------------------------------------------- 应用进程

ALL_PROCS: list[subprocess.Popen] = []


def _killpg(p: subprocess.Popen, sig: int) -> None:
    try:
        os.killpg(p.pid, sig)
    except (ProcessLookupError, PermissionError):
        pass
    except Exception:  # noqa: BLE001
        try:
            p.send_signal(sig)
        except Exception:  # noqa: BLE001
            pass


def kill_all_procs() -> None:
    for p in ALL_PROCS:
        if p.poll() is None:
            _killpg(p, signal.SIGKILL)
            try:
                p.wait(5)
            except Exception:  # noqa: BLE001
                pass


class App:
    """一个 mini-todo 进程：直接执行 <.app>/Contents/MacOS/mini-todo（环境变量能传进去，NSBundle.mainBundle 仍是这个 .app）。
    `data_dir` = 开发专用数据隔离（MINI_TODO_DATA_DIR，仅 debug 构建生效；日志在 <dir>/logs/）"""

    def __init__(self, app_path: Path, *, data_dir: Path | None, debug_open: str | None = None,
                 tag: str = "app", out_dir: Path, work_dir: Path, env: dict | None = None):
        self.app_path = Path(app_path)
        self.exe = self.app_path / "Contents" / "MacOS" / "mini-todo"
        self.data_dir = Path(data_dir) if data_dir else None
        self.debug_open = debug_open
        self.tag = tag
        self.out_dir = out_dir
        self.work_dir = work_dir
        self.extra_env = env or {}
        self.proc: subprocess.Popen | None = None
        self.stdout_path = out_dir / "logs" / f"{tag}-stdout.txt"
        self._stdout = None
        self.exit_code: int | None = None

    @property
    def pid(self) -> int:
        return self.proc.pid if self.proc else 0

    @property
    def log_path(self) -> Path:
        if self.data_dir:
            return self.data_dir / "logs" / "mini-todo.log"
        return Path.home() / "Library" / "Logs" / "com.tauri-app.mini-todo" / "mini-todo.log"

    @property
    def db_path(self) -> Path:
        if self.data_dir:
            return self.data_dir / "data.db"
        return Path.home() / "Library" / "Application Support" / "mini-todo" / "data.db"

    def start(self) -> "App":
        env = {k: v for k, v in os.environ.items() if not k.startswith("MINI_TODO_")}
        if self.data_dir:
            env["MINI_TODO_DATA_DIR"] = str(self.data_dir)
            env["MINI_TODO_FENCES_ROOT"] = str(self.work_dir / f"{self.tag}-fences")
        if self.debug_open:
            env["MINI_TODO_DEBUG_OPEN"] = self.debug_open
        env.setdefault("RUST_BACKTRACE", "1")
        env.update(self.extra_env)
        self.stdout_path.parent.mkdir(parents=True, exist_ok=True)
        self._stdout = open(self.stdout_path, "ab")
        print(f"    启动 {self.exe}（tag={self.tag} MINI_TODO_DATA_DIR={self.data_dir} "
              f"MINI_TODO_DEBUG_OPEN={self.debug_open}）", flush=True)
        self.proc = subprocess.Popen([str(self.exe)], env=env, cwd=str(self.work_dir), stdout=self._stdout,
                                     stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, start_new_session=True)
        ALL_PROCS.append(self.proc)
        print(f"    pid={self.proc.pid}", flush=True)
        return self

    def alive(self) -> bool:
        return self.proc is not None and self.proc.poll() is None

    def wait_exit(self, timeout: float) -> bool:
        if not self.proc:
            return True
        try:
            self.exit_code = self.proc.wait(timeout)
            return True
        except subprocess.TimeoutExpired:
            return False

    def stop(self) -> None:
        """结束进程（SIGTERM → 5s → SIGKILL）；已退出时只收尾"""
        if self.proc and self.proc.poll() is None:
            _killpg(self.proc, signal.SIGTERM)
            if not self.wait_exit(5):
                _killpg(self.proc, signal.SIGKILL)
                self.wait_exit(5)
        if self.proc and self.exit_code is None:
            self.exit_code = self.proc.poll()
        if self._stdout:
            self._stdout.close()
            self._stdout = None

    # --- 窗口
    def windows(self, layer: int | None = 0) -> list[WinInfo]:
        if not self.alive():
            return []
        return windows_of(self.pid, layer)

    def wait_windows(self, pred: Callable[[list[WinInfo]], WinInfo | None], timeout: float, what: str) -> WinInfo:
        def found():
            if not self.alive():
                raise AssertionError(f"进程已退出（exit={self.proc.returncode}）；stdout 尾部：\n"
                                     + "\n".join(tail(self.stdout_text(), 30)))
            return pred(self.windows())
        return wait_until(found, timeout, what)

    # --- 日志
    def log_text(self) -> str:
        try:
            return self.log_path.read_text(encoding="utf-8", errors="replace")
        except FileNotFoundError:
            return ""

    def stdout_text(self) -> str:
        if self._stdout:
            self._stdout.flush()
        try:
            return self.stdout_path.read_text(encoding="utf-8", errors="replace")
        except FileNotFoundError:
            return ""

    def panics(self) -> list[str]:
        lines = self.stdout_text().splitlines() + self.log_text().splitlines()
        return [l for l in lines if "panicked" in l]

    def errors(self) -> list[str]:
        return [l for l in self.log_text().splitlines() if "[ERROR]" in l]

    def dump_logs(self, n: int = 30) -> None:
        """日志尾部打印到 stdout，完整日志复制到 <out>/logs/<tag>-mini-todo.log（随产物上传）"""
        if self.log_path.exists():
            try:
                (self.out_dir / "logs").mkdir(parents=True, exist_ok=True)
                (self.out_dir / "logs" / f"{self.tag}-mini-todo.log").write_bytes(self.log_path.read_bytes())
            except OSError as e:
                print(f"    (复制日志失败: {e})")
        log = self.log_text()
        print(f"    --- 应用日志最后 {min(n, len(log.splitlines()))} 行（{self.log_path}"
              f"{'' if self.log_path.exists() else '，不存在'}）---")
        for l in tail(log, n):
            print("    | " + l)
        out = self.stdout_text()
        print(f"    --- stdout/stderr 最后 {min(n, len(out.splitlines()))} 行（{self.stdout_path}）---")
        for l in tail(out, n):
            print("    | " + l)

    # --- 数据库
    def q(self, sql: str, params: tuple = ()) -> list[tuple]:
        con = sqlite3.connect(f"file:{self.db_path}?mode=ro", uri=True, timeout=10)
        try:
            return con.execute(sql, params).fetchall()
        finally:
            con.close()

    # --- 系统视角
    def running_app_info(self) -> str:
        if AppKit is None:
            return f"AppKit 不可用：{APPKIT_ERR!r}"
        ra = AppKit.NSRunningApplication.runningApplicationWithProcessIdentifier_(self.pid)
        if ra is None:
            return "NSRunningApplication 查不到（还没连上窗口服务器？）"
        policy = {0: "Regular", 1: "Accessory", 2: "Prohibited"}.get(int(ra.activationPolicy()), ra.activationPolicy())
        return (f"bundleIdentifier={ra.bundleIdentifier()!r} localizedName={ra.localizedName()!r} "
                f"activationPolicy={policy} active={bool(ra.isActive())} bundleURL={ra.bundleURL()}")

    def bundle_id(self) -> str | None:
        """系统认出的 bundle id（AppKit 不可用 / 查不到进程时 None）"""
        if AppKit is None:
            return None
        ra = AppKit.NSRunningApplication.runningApplicationWithProcessIdentifier_(self.pid)
        if ra is None:
            return None
        bid = ra.bundleIdentifier()
        return str(bid) if bid is not None else ""

    def activate(self) -> bool:
        """把本进程切到前台（macOS 14 起其它进程发起的激活可能被系统拒绝，返回后以 frontmost_pid() 为准）"""
        if AppKit is None:
            return False
        ra = AppKit.NSRunningApplication.runningApplicationWithProcessIdentifier_(self.pid)
        if ra is None:
            return False
        return bool(ra.activateWithOptions_(AppKit.NSApplicationActivateIgnoringOtherApps))

    def terminate_via_appkit(self) -> bool:
        if AppKit is None:
            return False
        ra = AppKit.NSRunningApplication.runningApplicationWithProcessIdentifier_(self.pid)
        return bool(ra.terminate()) if ra is not None else False


def seed(seed_exe: Path, data_dir: Path, sample: Path) -> None:
    if data_dir.exists():
        import shutil
        shutil.rmtree(data_dir, ignore_errors=True)
    r = run([str(seed_exe), str(data_dir), str(sample)], timeout=120)
    if r.returncode != 0:
        raise AssertionError(f"seed 失败（rc={r.returncode}）：{r.stdout}\n{r.stderr}")
    print(f"    seed: {r.stdout.strip()}")


# ----------------------------------------------------------------------------- 用例与汇总

class Skip(Exception):
    pass


class CaseTimeout(AssertionError):
    pass


class Checks:
    """一个用例里的多项软断言：全部检查完再统一判定"""

    def __init__(self):
        self.items: list[tuple[bool, str]] = []

    def ok(self, cond, what: str) -> bool:
        self.items.append((bool(cond), what))
        print(f"    {'ok ' if cond else 'BAD'} {what}", flush=True)
        return bool(cond)

    def near(self, actual: float, expected: float, tol: float, what: str) -> bool:
        return self.ok(abs(actual - expected) <= tol, f"{what}: {actual:g} ≈ {expected:g} (±{tol:g})")

    def done(self, summary: str = "") -> str:
        bad = [w for c, w in self.items if not c]
        if bad:
            if CURRENT_REPORT[0]:
                CURRENT_REPORT[0].snapshot()
            raise AssertionError("; ".join(bad))
        return summary or f"{len(self.items)} 项检查通过"


CURRENT_REPORT: list = [None]


class _Tee:
    def __init__(self, stream):
        self.stream = stream
        self.buf: list[str] = []

    def write(self, data):
        self.buf.append(data)
        return self.stream.write(data)

    def flush(self):
        self.stream.flush()


def _on_alarm(signum, frame):  # noqa: ARG001
    raise CaseTimeout(f"用例超时（{CURRENT_TIMEOUT[0]}s）")


CURRENT_TIMEOUT: list = [0]


class Report:
    def __init__(self, out_dir: Path, title: str, emit_budget: int = 2):
        self.out_dir = out_dir
        self.title = title
        self.emit_budget = emit_budget
        self.results: list[dict] = []
        self.current: str | None = None
        self.snapped: set[str] = set()
        out_dir.mkdir(parents=True, exist_ok=True)
        CURRENT_REPORT[0] = self
        signal.signal(signal.SIGALRM, _on_alarm)

    def snapshot(self) -> None:
        """失败当时的整屏截图（在用例清理之前调用）；前 emit_budget 张另存一份小图供注解输出"""
        cid = self.current
        if not cid or cid in self.snapped:
            return
        self.snapped.add(cid)
        try:
            img = capture_screen(self.out_dir / f"fail-{cid}.png")
            if len(self.snapped) <= self.emit_budget:
                save_emit(img, self.out_dir, f"fail-{cid}", max_w=480, colors=16)
        except Exception as e:  # noqa: BLE001
            print(f"  (失败截图出错: {e})")

    def case(self, cid: str, title: str, fn: Callable[[], str | None], timeout: int = 180) -> bool:
        print(f"\n::group::[{cid}] {title}", flush=True)
        self.current = cid
        tee = _Tee(sys.stdout)
        sys.stdout = tee
        t0 = time.time()
        status, detail = "PASS", ""
        procs_before = len(ALL_PROCS)
        try:
            try:
                CURRENT_TIMEOUT[0] = timeout
                signal.alarm(timeout)
                try:
                    detail = fn() or ""
                finally:
                    signal.alarm(0)
            except Skip as e:
                status, detail = "SKIP", str(e)
            except AssertionError as e:
                status, detail = "FAIL", str(e)
            except Exception as e:  # noqa: BLE001
                status, detail = "FAIL", f"{type(e).__name__}: {e}"
                traceback.print_exc(file=sys.stdout)
            if status == "FAIL":
                self.snapshot()
            # 本用例启动的进程一律结束（用例自己的 finally 被超时打断时兜底）
            for p in ALL_PROCS[procs_before:]:
                if p.poll() is None:
                    print(f"    (兜底结束残留进程 pid={p.pid})")
                    _killpg(p, signal.SIGKILL)
                    try:
                        p.wait(5)
                    except Exception:  # noqa: BLE001
                        pass
            secs = time.time() - t0
            print(f"[{status}] {cid} {title} ({secs:.1f}s) {detail}", flush=True)
        finally:
            sys.stdout = tee.stream
            self.current = None
        if status == "FAIL":
            (self.out_dir / f"fail-{cid}.txt").write_text("".join(tee.buf), encoding="utf-8")
        print("::endgroup::", flush=True)
        self.results.append({"id": cid, "title": title, "status": status, "detail": detail,
                             "seconds": round(time.time() - t0, 1)})
        return status == "PASS"

    def finish(self) -> int:
        (self.out_dir / "results.json").write_text(json.dumps(self.results, ensure_ascii=False, indent=2),
                                                   encoding="utf-8")
        counts = {s: sum(1 for r in self.results if r["status"] == s) for s in ("PASS", "FAIL", "SKIP")}
        lines = [f"## {self.title}", "",
                 f"PASS {counts['PASS']} / FAIL {counts['FAIL']} / SKIP {counts['SKIP']}", "",
                 "| 用例 | 结果 | 说明 |", "|---|---|---|"]
        for r in self.results:
            detail = r["detail"].replace("|", "\\|").replace("\n", " ")
            if len(detail) > 300:
                detail = detail[:300] + "…"
            lines.append(f"| {r['id']} {r['title']} | {r['status']} | {detail} |")
        md = "\n".join(lines) + "\n"
        (self.out_dir / "summary.md").write_text(md, encoding="utf-8")
        step_summary = os.environ.get("GITHUB_STEP_SUMMARY")
        if step_summary:
            with open(step_summary, "a", encoding="utf-8") as f:
                f.write(md + "\n")
        print("\n========== E2E SUMMARY ==========")
        w = max((len(r["id"]) for r in self.results), default=4)
        for r in self.results:
            detail = r["detail"].replace("\n", " ")
            if len(detail) > 220:
                detail = detail[:220] + "…"
            print(f"{r['status']:<4} {r['id']:<{w}} {r['title']} — {detail}")
        print(f"TOTAL: PASS {counts['PASS']}  FAIL {counts['FAIL']}  SKIP {counts['SKIP']}")
        print("=================================", flush=True)
        return 1 if counts["FAIL"] else 0
