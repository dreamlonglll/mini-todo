"""Mini-Todo (GPUI 版) Linux 端到端测试支撑库。

黑盒驱动：真实的 debug 构建在 Xvfb + mesa lavapipe 上运行，用 xdotool 点击 / 输入 / 拖拽 / 快捷键操作界面，
用 tesseract OCR 读界面文字，断言数据库（sqlite3，只读打开）、WebDAV 上的 sync-data.json.gz、cloud REST、
Skill CLI 输出、应用日志（<data>/logs/mini-todo.log）与截图。GPUI 版没有 IPC，也没有无障碍树。

环境变量（都有默认值）：
  MT_E2E_WORK      日志 / 截图 / 隔离数据目录的根（默认 /tmp/mini-todo-gpui-e2e）
  MT_E2E_DAV_ROOT  setup_servers.sh 建的 WebDAV 根（默认 /srv/mtodo-e2e）
  MT_E2E_DISPLAYS  可用的 Xvfb 显示号，逗号分隔（默认 :78,:79；第一台"设备"用第一个）
  MT_E2E_SEED      core 的 seed 示例（默认 <app 同目录>/examples/seed）
"""
from __future__ import annotations

import difflib
import gzip
import json
import os
import re
import shutil
import signal
import socket
import sqlite3
import subprocess
import time
import traceback
import unicodedata
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

import requests

REPO = Path(__file__).resolve().parents[3]
WORK = Path(os.environ.get("MT_E2E_WORK", "/tmp/mini-todo-gpui-e2e"))
RUN = WORK / "run"
SHOTS = RUN / "shots"
DAV_ROOT = Path(os.environ.get("MT_E2E_DAV_ROOT", "/srv/mtodo-e2e"))
DAV_USER, DAV_PASS = "e2e", "e2e-pass"
DAV = {
    "apache": {"url": "http://127.0.0.1:8081/", "root": DAV_ROOT / "apache" / "root"},
    "nginx": {"url": "http://127.0.0.1:8082/", "root": DAV_ROOT / "nginx" / "root"},
}
TZ = "Asia/Shanghai"
DISPLAYS = [d.strip() for d in os.environ.get("MT_E2E_DISPLAYS", ":78,:79").split(",") if d.strip()]
SCREEN_W, SCREEN_H = 1600, 1000
LVP_ICD = "/usr/share/vulkan/icd.d/lvp_icd.json"
CANON = re.compile(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$")
OCR_SCALE = 3
OCR_MODES = [(6, "gray"), (11, "gray"), (6, "red")]


def log(*a: Any) -> None:
    print(time.strftime("%H:%M:%S"), *a, flush=True)


def check(cond: Any, msg: str) -> None:
    if not cond:
        raise AssertionError(msg)


def is_canon(s: str | None) -> bool:
    return s is None or bool(CANON.match(s))


def wait_until(fn: Callable[[], Any], timeout: float, interval: float = 0.5, what: str = "condition") -> Any:
    """反复调用 fn 直到返回真值；超时抛 TimeoutError（带最后一次的值）。"""
    deadline = time.time() + timeout
    last: Any = None
    while True:
        try:
            last = fn()
            if last:
                return last
        except AssertionError as e:  # 让 fn 里也能用 check
            last = e
        if time.time() >= deadline:
            raise TimeoutError(f"timed out after {timeout:.0f}s waiting for {what} (last={last!r})"[:600])
        time.sleep(interval)


def wait_port(port: int, timeout: float = 30.0) -> None:
    def ok() -> bool:
        with socket.socket() as s:
            s.settimeout(0.5)
            return s.connect_ex(("127.0.0.1", port)) == 0
    wait_until(ok, timeout, 0.2, f"port {port}")


# --------------------------------------------------------------------------- processes


class Procs:
    def __init__(self) -> None:
        self.items: list[tuple[str, subprocess.Popen]] = []

    def start(self, name: str, cmd: list[str], env: dict[str, str] | None = None,
              cwd: str | None = None) -> subprocess.Popen:
        RUN.mkdir(parents=True, exist_ok=True)
        out = open(RUN / f"{name}.log", "ab")
        p = subprocess.Popen(cmd, env=env, cwd=cwd, stdout=out, stderr=subprocess.STDOUT,
                             start_new_session=True)
        self.items.append((name, p))
        return p

    def stop(self, name: str, sig: int = signal.SIGTERM) -> None:
        for n, p in list(self.items):
            if n == name:
                kill(p, sig)
                self.items.remove((n, p))

    def stop_all(self) -> None:
        for _, p in reversed(self.items):
            kill(p)
        self.items.clear()


def kill(p: subprocess.Popen, sig: int = signal.SIGTERM, timeout: float = 8) -> None:
    if p.poll() is not None:
        return
    try:
        os.killpg(p.pid, sig)
        p.wait(timeout=timeout)
    except Exception:
        try:
            os.killpg(p.pid, signal.SIGKILL)
            p.wait(timeout=5)
        except Exception:
            pass


# --------------------------------------------------------------------------- X display / windows


@dataclass
class Win:
    wid: int
    title: str
    x: int
    y: int
    w: int
    h: int

    def center(self) -> tuple[int, int]:
        return self.w // 2, self.h // 2


class XDisplay:
    """一个 Xvfb（没有窗口管理器：焦点用 xdotool windowfocus 直接设置）。"""

    def __init__(self, procs: Procs, num: str) -> None:
        self.procs, self.num = procs, num
        self.name = f"xvfb{num.replace(':', '')}"
        self.env = dict(os.environ)
        self.env.update({"DISPLAY": num, "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8"})
        self.env.pop("WAYLAND_DISPLAY", None)

    def start(self) -> "XDisplay":
        lock = Path(f"/tmp/.X{self.num[1:]}-lock")
        if lock.exists():
            try:
                pid = int(lock.read_text().strip())
                os.kill(pid, signal.SIGTERM)
                time.sleep(0.5)
            except Exception:
                pass
            lock.unlink(missing_ok=True)
        self.procs.start(self.name, ["Xvfb", self.num, "-screen", "0", f"{SCREEN_W}x{SCREEN_H}x24",
                                     "-nolisten", "tcp"], env=self.env)
        wait_until(lambda: self.xdo("getdisplaygeometry", check_rc=False).strip(), 15, 0.1, f"Xvfb {self.num}")
        return self

    def stop(self) -> None:
        self.procs.stop(self.name)

    # -- xdotool ---------------------------------------------------------------
    def xdo(self, *args: str, check_rc: bool = True) -> str:
        r = subprocess.run(["xdotool", *map(str, args)], env=self.env, capture_output=True, text=True, timeout=30)
        if check_rc and r.returncode != 0:
            raise RuntimeError(f"xdotool {' '.join(map(str, args))} -> {r.returncode} {r.stderr.strip()}")
        return r.stdout

    def windows(self) -> list[Win]:
        out = self.xdo("search", "--onlyvisible", "--name", ".", check_rc=False)
        wins = []
        for line in out.split():
            try:
                wid = int(line)
            except ValueError:
                continue
            title = self.xdo("getwindowname", wid, check_rc=False).strip()
            geo = self.xdo("getwindowgeometry", "--shell", wid, check_rc=False)
            g = dict(kv.split("=", 1) for kv in geo.split() if "=" in kv)
            try:
                wins.append(Win(wid, title, int(g["X"]), int(g["Y"]), int(g["WIDTH"]), int(g["HEIGHT"])))
            except KeyError:
                continue
        return wins

    def find(self, title: str | Callable[[str], bool]) -> list[Win]:
        pred = title if callable(title) else (lambda t: t == title)
        return [w for w in self.windows() if pred(w.title)]

    def wait_window(self, title: str | Callable[[str], bool], timeout: float = 15) -> Win:
        return wait_until(lambda: (self.find(title) or [None])[-1], timeout, 0.25, f"window {title!r}")

    def wait_window_gone(self, title: str, timeout: float = 15) -> None:
        wait_until(lambda: not self.find(title), timeout, 0.25, f"window {title!r} to close")

    def refresh(self, win: Win) -> Win:
        for w in self.windows():
            if w.wid == win.wid:
                return w
        raise AssertionError(f"window {win.title!r} ({win.wid}) is gone")

    # -- input -----------------------------------------------------------------
    def move(self, win: Win, x: float, y: float) -> None:
        # 不用 --sync：光标已经在目标位置时 xdotool 会一直等到超时
        self.xdo("mousemove", int(win.x + x), int(win.y + y), check_rc=False)
        time.sleep(0.03)

    def click(self, win: Win, x: float, y: float, button: int = 1, settle: float = 0.35) -> None:
        self.move(win, x, y)
        time.sleep(0.08)
        self.xdo("click", button)
        time.sleep(settle)

    def dblclick(self, win: Win, x: float, y: float) -> None:
        self.move(win, x, y)
        time.sleep(0.08)
        self.xdo("click", "--repeat", 2, "--delay", 80, 1)
        time.sleep(0.35)

    def drag(self, win: Win, start: tuple[float, float], end: tuple[float, float], steps: int = 18,
             hold: float = 0.4) -> None:
        sx, sy = start
        ex, ey = end
        self.move(win, sx, sy)
        time.sleep(0.15)
        self.xdo("mousedown", 1)
        time.sleep(0.15)
        for i in range(1, steps + 1):
            t = i / steps
            self.move(win, sx + (ex - sx) * t, sy + (ey - sy) * t)
            time.sleep(0.04)
        time.sleep(hold)
        self.move(win, ex + 1, ey + 1)
        time.sleep(0.1)
        self.xdo("mouseup", 1)
        time.sleep(0.5)

    def scroll(self, win: Win, x: float, y: float, clicks: int) -> None:
        """clicks > 0 向下，< 0 向上"""
        self.move(win, x, y)
        button = 5 if clicks > 0 else 4
        for _ in range(abs(clicks)):
            self.xdo("click", button)
            time.sleep(0.03)
        time.sleep(0.4)

    def focus(self, win: Win) -> None:
        self.xdo("windowfocus", "--sync", win.wid, check_rc=False)
        time.sleep(0.1)

    def key(self, win: Win, *keys: str) -> None:
        self.focus(win)
        for k in keys:
            self.xdo("key", "--clearmodifiers", k)
            time.sleep(0.12)
        time.sleep(0.2)

    def type(self, win: Win, text: str) -> None:
        """ASCII 用 xdotool type；含非 ASCII 字符时走剪贴板粘贴（xdotool 临时改键位会丢第一个 CJK 字符）"""
        self.focus(win)
        if all(ord(c) < 128 for c in text):
            self.xdo("type", "--delay", 25, "--", text)
        else:
            self.paste(win, text)
        time.sleep(0.25)

    def paste(self, win: Win, text: str) -> None:
        p = subprocess.Popen(["xclip", "-selection", "clipboard"], stdin=subprocess.PIPE, env=self.env,
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        p.communicate(text.encode("utf-8"), timeout=10)
        time.sleep(0.2)
        self.focus(win)
        self.xdo("key", "--clearmodifiers", "ctrl+v")
        time.sleep(0.4)

    def wm_close(self, win: Win) -> None:
        """像窗口管理器的关闭按钮那样请求关闭窗口：发 WM_PROTOCOLS / WM_DELETE_WINDOW ClientMessage
        （xdotool windowclose 是 XDestroyWindow，不走应用的关闭流程）。"""
        import ctypes
        import ctypes.util
        x11 = ctypes.cdll.LoadLibrary(ctypes.util.find_library("X11") or "libX11.so.6")
        x11.XOpenDisplay.restype = ctypes.c_void_p
        x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        x11.XInternAtom.restype = ctypes.c_ulong
        x11.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
        x11.XSendEvent.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_long, ctypes.c_void_p]
        x11.XFlush.argtypes = [ctypes.c_void_p]
        x11.XCloseDisplay.argtypes = [ctypes.c_void_p]

        class ClientMessage(ctypes.Structure):
            _fields_ = [("type", ctypes.c_int), ("serial", ctypes.c_ulong), ("send_event", ctypes.c_int),
                        ("display", ctypes.c_void_p), ("window", ctypes.c_ulong), ("message_type", ctypes.c_ulong),
                        ("format", ctypes.c_int), ("data", ctypes.c_long * 5)]

        class XEvent(ctypes.Union):
            _fields_ = [("xclient", ClientMessage), ("pad", ctypes.c_long * 24)]

        dpy = x11.XOpenDisplay(self.num.encode())
        check(dpy, f"XOpenDisplay({self.num}) failed")
        try:
            ev = XEvent()
            ev.xclient.type = 33  # ClientMessage
            ev.xclient.window = win.wid
            ev.xclient.message_type = x11.XInternAtom(dpy, b"WM_PROTOCOLS", 0)
            ev.xclient.format = 32
            ev.xclient.data[0] = x11.XInternAtom(dpy, b"WM_DELETE_WINDOW", 0)
            ev.xclient.data[1] = 0  # CurrentTime
            x11.XSendEvent(dpy, win.wid, 0, 0, ctypes.byref(ev))
            x11.XFlush(dpy)
        finally:
            x11.XCloseDisplay(dpy)

    # -- screenshots -------------------------------------------------------------
    def grab(self, path: Path) -> Path:
        path.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["import", "-window", "root", str(path)], env=self.env, check=True,
                       capture_output=True, timeout=30)
        return path

    def shot(self, win: Win | None, path: Path) -> Path:
        """整屏截图后按窗口几何裁剪（win=None 时保存整屏）"""
        path.parent.mkdir(parents=True, exist_ok=True)
        full = path.with_suffix(".full.png")
        self.grab(full)
        if win is None:
            full.replace(path)
            return path
        subprocess.run(["convert", str(full), "-crop", f"{win.w}x{win.h}+{win.x}+{win.y}", "+repage", str(path)],
                       check=True, capture_output=True, timeout=30)
        full.unlink(missing_ok=True)
        return path

    def pixel(self, win: Win, x: int, y: int) -> tuple[int, int, int]:
        """窗口内某点的颜色"""
        tmp = RUN / "tmp" / f"px-{os.getpid()}-{self.num[1:]}.png"
        self.grab(tmp)
        out = subprocess.run(["convert", str(tmp), "-crop", f"1x1+{win.x + x}+{win.y + y}", "+repage", "-depth", "8",
                              "txt:-"], capture_output=True, text=True, timeout=30).stdout
        m = re.search(r"\((\d+),(\d+),(\d+)", out.splitlines()[-1] if out else "")
        return (int(m.group(1)), int(m.group(2)), int(m.group(3))) if m else (0, 0, 0)

    def find_color(self, win: Win, rgb: tuple[int, int, int], region: tuple[int, int, int, int] | None = None,
                   fuzz: int = 4) -> tuple[int, int, int, int] | None:
        """窗口（或 region）里颜色接近 rgb 的像素的外接矩形（窗口坐标 x, y, w, h）；没有则 None"""
        tmp = RUN / "tmp"
        tmp.mkdir(parents=True, exist_ok=True)
        full = self.grab(tmp / f"color-{os.getpid()}-{self.num[1:]}.png")
        rx, ry, rw, rh = region or (0, 0, win.w, win.h)
        r, g, b = rgb
        out = subprocess.run(
            ["convert", str(full), "-crop", f"{rw}x{rh}+{win.x + rx}+{win.y + ry}", "+repage",
             "-fuzz", f"{fuzz}%", "-fill", "#FF00FF", "-opaque", f"rgb({r},{g},{b})", "-fuzz", "0",
             "-fill", "black", "+opaque", "#FF00FF", "-fill", "white", "-opaque", "#FF00FF",
             "-bordercolor", "black", "-border", "1", "-format", "%@", "info:"],
            capture_output=True, text=True, timeout=30).stdout.strip()
        m = re.match(r"(\d+)x(\d+)\+(-?\d+)\+(-?\d+)", out)
        if not m:
            return None
        w, h, x, y = (int(v) for v in m.groups())
        if w <= 1 or h <= 1 or (w >= rw and h >= rh):
            return None
        return rx + x - 1, ry + y - 1, w, h

    # -- OCR -----------------------------------------------------------------------
    def ocr(self, win: Win, region: tuple[int, int, int, int] | None = None, psm: int = 6,
            variant: str = "gray") -> "OcrResult":
        """OCR 窗口（或窗口内 region=(x, y, w, h)）；坐标换算回窗口坐标。
        variant：gray = 灰度（深色字）；red = 只取红色通道（蓝底白字的主按钮在灰度下对比度太低）"""
        tmp = RUN / "tmp"
        tmp.mkdir(parents=True, exist_ok=True)
        stem = f"ocr-{os.getpid()}-{self.num[1:]}"
        full = self.grab(tmp / f"{stem}-full.png")
        rx, ry, rw, rh = region or (0, 0, win.w, win.h)
        big = tmp / f"{stem}.png"
        color = ["-channel", "R", "-separate"] if variant == "red" else ["-colorspace", "Gray"]
        subprocess.run(["convert", str(full), "-crop", f"{rw}x{rh}+{win.x + rx}+{win.y + ry}", "+repage",
                        *color, "-resize", f"{OCR_SCALE * 100}%", str(big)],
                       check=True, capture_output=True, timeout=30)
        out = subprocess.run(["tesseract", str(big), "-", "-l", "chi_sim+eng", "--psm", str(psm), "tsv"],
                             capture_output=True, text=True, timeout=60).stdout
        return OcrResult.parse(out, rx, ry)


# --------------------------------------------------------------------------- OCR text matching

_FULLWIDTH = str.maketrans({"：": ":", "，": ",", "（": "(", "）": ")", "。": ".", "；": ";", "！": "!",
                            "？": "?", "【": "[", "】": "]", "“": '"', "”": '"', "‘": "'", "’": "'",
                            "|": "l", "—": "-", "－": "-", "／": "/"})


def norm(s: str) -> str:
    s = unicodedata.normalize("NFKC", s).translate(_FULLWIDTH)
    return re.sub(r"\s+", "", s).lower()


@dataclass
class OcrWord:
    text: str
    x: float
    y: float
    w: float
    h: float
    conf: float


@dataclass
class OcrLine:
    words: list[OcrWord]

    @property
    def text(self) -> str:
        return " ".join(w.text for w in self.words)

    def chars(self) -> tuple[str, list[OcrWord]]:
        s, owners = "", []
        for w in self.words:
            n = norm(w.text)
            s += n
            owners += [w] * len(n)
        return s, owners


@dataclass
class OcrResult:
    lines: list[OcrLine] = field(default_factory=list)

    @staticmethod
    def parse(tsv: str, dx: int, dy: int) -> "OcrResult":
        groups: dict[tuple, list[OcrWord]] = {}
        for row in tsv.splitlines()[1:]:
            cols = row.split("\t")
            if len(cols) < 12 or cols[0] != "5" or not cols[11].strip():
                continue
            key = (cols[2], cols[3], cols[4])
            x, y, w, h = (int(c) / OCR_SCALE for c in cols[6:10])
            groups.setdefault(key, []).append(
                OcrWord(cols[11].strip(), x + dx, y + dy, w, h, float(cols[10])))
        return OcrResult([OcrLine(ws) for ws in groups.values()])

    @property
    def text(self) -> str:
        return "\n".join(line.text for line in self.lines)

    def find(self, target: str, fuzzy: float = 0.8) -> tuple[float, float, float, float] | None:
        """在同一行里找 target（忽略空白 / 大小写 / 全半角）；返回窗口坐标 (x, y, w, h)。
        精确子串优先；否则用滑动窗口找相似度 >= fuzzy 的片段（OCR 偶有错字）。"""
        t = norm(target)
        if not t:
            return None
        best: tuple[float, list[OcrWord]] | None = None
        # 单行，再加上相邻两行拼接（窄列里标题会折行，如四象限视图）
        candidates = list(self.lines) + [OcrLine(a.words + b.words) for a, b in zip(self.lines, self.lines[1:])]
        for line in candidates:
            s, owners = line.chars()
            i = s.find(t)
            if i >= 0:
                return _span_box(owners[i:i + len(t)])
            if len(t) >= 3 and s:
                for n in {len(t) - 1, len(t), len(t) + 1}:
                    if n <= 0 or n > len(s):
                        continue
                    for j in range(0, len(s) - n + 1):
                        r = difflib.SequenceMatcher(None, s[j:j + n], t).ratio()
                        if r >= fuzzy and (best is None or r > best[0]):
                            best = (r, owners[j:j + n])
        return _span_box(best[1]) if best else None

    def find_all_exact(self, target: str) -> list[tuple[float, float, float, float]]:
        """所有精确（规范化后）出现的位置"""
        t = norm(target)
        out = []
        for line in self.lines:
            s, owners = line.chars()
            i = s.find(t)
            while t and i >= 0:
                out.append(_span_box(owners[i:i + len(t)]))
                i = s.find(t, i + 1)
        return out

    def has(self, target: str, fuzzy: float = 0.8) -> bool:
        return self.find(target, fuzzy) is not None


def _span_box(words: list[OcrWord]) -> tuple[float, float, float, float]:
    uniq = list(dict.fromkeys(id(w) for w in words))
    ws = [w for w in words if id(w) in uniq]
    x0 = min(w.x for w in ws)
    y0 = min(w.y for w in ws)
    x1 = max(w.x + w.w for w in ws)
    y1 = max(w.y + w.h for w in ws)
    return x0, y0, x1 - x0, y1 - y0


def box_center(b: tuple[float, float, float, float]) -> tuple[float, float]:
    return b[0] + b[2] / 2, b[1] + b[3] / 2


# --------------------------------------------------------------------------- isolated "machines"


class Session:
    """一台隔离的"设备"：自己的 HOME、数据目录（MINI_TODO_DATA_DIR）与收纳根目录。"""

    def __init__(self, name: str, fresh: bool = True) -> None:
        self.name = name
        self.root = RUN / "sessions" / name
        if fresh and self.root.exists():
            shutil.rmtree(self.root)
        self.home = self.root / "home"
        self.data_dir = self.root / "data"
        for d in (self.home / ".local" / "share", self.home / ".config", self.home / ".cache", self.data_dir):
            d.mkdir(parents=True, exist_ok=True)

    @property
    def db_path(self) -> Path:
        return self.data_dir / "data.db"

    @property
    def images_dir(self) -> Path:
        return self.data_dir / "images"

    @property
    def log_path(self) -> Path:
        return self.data_dir / "logs" / "mini-todo.log"

    def env(self, base: dict[str, str]) -> dict[str, str]:
        e = dict(base)
        e.update({
            "HOME": str(self.home),
            "XDG_DATA_HOME": str(self.home / ".local" / "share"),
            "XDG_CONFIG_HOME": str(self.home / ".config"),
            "XDG_CACHE_HOME": str(self.home / ".cache"),
            "MINI_TODO_DATA_DIR": str(self.data_dir),
            "MINI_TODO_FENCES_ROOT": str(self.root / "fences"),
        })
        return e

    # -- database ------------------------------------------------------------------
    def query(self, sql: str, args: tuple = ()) -> list[dict]:
        c = sqlite3.connect(f"file:{self.db_path}?mode=ro", uri=True, timeout=10)
        c.row_factory = sqlite3.Row
        try:
            return [dict(r) for r in c.execute(sql, args).fetchall()]
        finally:
            c.close()

    def one(self, sql: str, args: tuple = ()) -> dict | None:
        rows = self.query(sql, args)
        return rows[0] if rows else None

    def execute(self, sql: str, args: tuple = ()) -> int:
        """外部直接写库（模拟其它进程 / 旧版本写入）；返回 lastrowid"""
        c = sqlite3.connect(str(self.db_path), timeout=10)
        try:
            cur = c.execute(sql, args)
            c.commit()
            return cur.lastrowid
        finally:
            c.close()

    def todos_by_title(self) -> dict[str, dict]:
        return {r["title"]: r for r in self.query("SELECT * FROM todos")}

    def setting(self, key: str) -> str | None:
        r = self.one("SELECT value FROM settings WHERE key=?", (key,))
        return r["value"] if r else None

    def tombstones(self) -> set[tuple[str, int]]:
        return {(r["entity_type"], r["entity_id"]) for r in self.query("SELECT * FROM tombstones")}

    # -- log ---------------------------------------------------------------------------
    def log_text(self) -> str:
        return self.log_path.read_text(encoding="utf-8", errors="replace") if self.log_path.exists() else ""


class LogMark:
    """记下日志当前长度，之后只看新增的行"""

    def __init__(self, session: Session) -> None:
        self.session = session
        self.offset = len(session.log_text())

    def new_text(self) -> str:
        return self.session.log_text()[self.offset:]

    def wait(self, pattern: str, timeout: float = 30) -> str:
        rx = re.compile(pattern)

        def found() -> str | None:
            for line in self.new_text().splitlines():
                if rx.search(line):
                    return line
            return None
        return wait_until(found, timeout, 0.3, f"log line /{pattern}/")


# --------------------------------------------------------------------------- the app under test


class App:
    """一个运行中的应用实例（一个进程 + 它的窗口）。所有坐标都是窗口内的逻辑像素（缩放 1）。"""

    MAIN = "Mini Todo"

    def __init__(self, procs: Procs, binary: Path, session: Session, display: XDisplay,
                 extra_env: dict[str, str] | None = None, tag: str = "") -> None:
        self.procs, self.binary, self.session, self.x = procs, binary, session, display
        self.name = f"app-{session.name}{tag}"
        env = session.env(display.env)
        env.update({
            "VK_ICD_FILENAMES": LVP_ICD,
            "GPUI_X11_SCALE_FACTOR": "1",
            "TZ": TZ,
            "RUST_BACKTRACE": "1",
            # 版本检查不联网（当作已是最新）
            "MINI_TODO_DEBUG_LATEST_VERSION": "v0.0.1",
        })
        if extra_env:
            env.update(extra_env)
        self.env = env
        self.proc: subprocess.Popen | None = None

    def start(self, wait_loaded: float = 2.5) -> "App":
        check(not self.x.find(self.MAIN), f"display {self.x.num} already has a main window")
        self.proc = self.procs.start(self.name, [str(self.binary)], env=self.env)
        self.main = self.x.wait_window(self.MAIN, 40)
        time.sleep(wait_loaded)
        self.check_alive()
        # 光标先挪到标题栏空白处（GPUI 在光标离开窗口后不清除悬停）
        self.x.move(self.main, 100, 46)
        return self

    def check_alive(self) -> None:
        check(self.proc is not None and self.proc.poll() is None,
              f"app process exited rc={self.proc.returncode if self.proc else None}; see {RUN / (self.name + '.log')}")

    def alive(self) -> bool:
        return self.proc is not None and self.proc.poll() is None

    def stop(self, sig: int = signal.SIGTERM) -> None:
        self.procs.stop(self.name, sig)
        time.sleep(0.5)

    # -- windows -----------------------------------------------------------------------
    def win(self, title: str = MAIN, timeout: float = 15) -> Win:
        return self.x.wait_window(title, timeout)

    def main_win(self) -> Win:
        self.main = self.x.refresh(self.main)
        return self.main

    # -- convenience wrappers --------------------------------------------------------------
    def shot(self, name: str, win: Win | None = None) -> Path:
        return self.x.shot(win, SHOTS / f"{name}.png")

    def ocr(self, win: Win, region: tuple[int, int, int, int] | None = None, psm: int = 6,
            variant: str = "gray") -> OcrResult:
        return self.x.ocr(win, region, psm, variant)

    def wait_text(self, win: Win, text: str, timeout: float = 15, present: bool = True,
                  region: tuple[int, int, int, int] | None = None, fuzzy: float = 0.8,
                  psm: int | None = None) -> tuple[float, float, float, float] | bool:
        """OCR 轮询直到 text 出现（present=False：消失）。psm 不指定时轮流用 6（整块）/ 11（稀疏）/
        6 + 红色通道（蓝底白字），几种方式各有漏字的场景。"""
        last = ""
        modes = [(psm, "gray")] if psm is not None else OCR_MODES
        deadline = time.time() + timeout
        attempt = 0
        # 机器忙时一次 OCR 可能要好几秒：超时之前每种方式至少试过一遍才算失败
        while attempt < len(modes) or time.time() < deadline:
            if not present:  # 消失：同一时刻每种方式都找不到才算
                gone = True
                for mode, variant in modes:
                    r = self.ocr(win, region, mode, variant)
                    last = r.text
                    if r.find(text, fuzzy) is not None:
                        gone = False
                        break
                attempt += len(modes)
                if gone:
                    return True
            else:
                mode, variant = modes[attempt % len(modes)]
                attempt += 1
                r = self.ocr(win, region, mode, variant)
                last = r.text
                b = r.find(text, fuzzy)
                if b:
                    return b
            time.sleep(0.3)
        raise AssertionError(f"OCR: {text!r} {'not found' if present else 'still visible'} in "
                             f"{win.title!r} after {timeout:.0f}s ({attempt} tries); OCR text was: {last[:500]!r}")

    def click_text(self, win: Win, text: str, timeout: float = 10, dx: float = 0, dy: float = 0,
                   region: tuple[int, int, int, int] | None = None, fuzzy: float = 0.8,
                   psm: int | None = None) -> None:
        b = self.wait_text(win, text, timeout, region=region, fuzzy=fuzzy, psm=psm)
        cx, cy = box_center(b)  # type: ignore[arg-type]
        self.x.click(win, cx + dx, cy + dy)

    def click(self, win: Win, x: float, y: float, **kw: Any) -> None:
        self.x.click(win, x, y, **kw)

    def key(self, win: Win, *keys: str) -> None:
        self.x.key(win, *keys)

    def type(self, win: Win, text: str) -> None:
        self.x.type(win, text)


# --------------------------------------------------------------------------- seeding


def seed(seed_bin: Path, session: Session, doc: dict) -> None:
    """用 core 的 seed 示例把导出格式 4.0 的 JSON 导入隔离数据目录（恢复备份语义，保留 id）"""
    p = session.root / "seed.json"
    p.write_text(json.dumps(doc, ensure_ascii=False), encoding="utf-8")
    env = dict(os.environ)
    env["TZ"] = TZ
    r = subprocess.run([str(seed_bin), str(session.data_dir), str(p)], env=env, capture_output=True, text=True,
                       timeout=120)
    check(r.returncode == 0, f"seed failed: {r.stdout} {r.stderr}")


def mock_settings(**over: Any) -> dict:
    """seed 认识的"截图 harness"形状的设置（没有 isFixed 键时会被转换成 AppSettings）"""
    s = {"autoHideEnabled": False, "topOnWake": False, "fixedEmbedDesktop": False,
         "windowBackground": {"color": "#000000", "alpha": 0.45}, "todoFontFamily": "", "todoFontSize": 14,
         "notificationType": "app", "darkTheme": False, "showCalendar": False, "viewMode": "list"}
    s.update(over)
    return s


def todo_doc(id: int, title: str, ts: str, **over: Any) -> dict:
    t = {"id": id, "title": title, "description": None, "color": "#10B981", "quadrant": 4, "notifyAt": None,
         "notifyBefore": 0, "notified": False, "completed": False, "sortOrder": id, "startTime": None,
         "endTime": None, "createdAt": ts, "updatedAt": ts, "subtasks": [], "repeatEnabled": False,
         "repeatType": None, "repeatInterval": 1, "repeatWeekdays": None, "repeatMonthDay": None}
    t.update(over)
    return t


# --------------------------------------------------------------------------- WebDAV helpers


def dav_reset(kind: str) -> None:
    root = DAV[kind]["root"]
    for child in (root.iterdir() if root.exists() else []):
        if child.is_dir():
            shutil.rmtree(child)
        else:
            child.unlink()


def dav_get_sync(kind: str) -> dict | None:
    r = requests.get(DAV[kind]["url"] + "mini-todo/sync-data.json.gz", auth=(DAV_USER, DAV_PASS), timeout=15)
    if r.status_code == 404:
        return None
    r.raise_for_status()
    return json.loads(gzip.decompress(r.content).decode("utf-8"))


def dav_put_sync(kind: str, doc: dict) -> int:
    body = gzip.compress(json.dumps(doc, ensure_ascii=False).encode("utf-8"))
    r = requests.put(DAV[kind]["url"] + "mini-todo/sync-data.json.gz", data=body,
                     auth=(DAV_USER, DAV_PASS), timeout=15, headers={"Content-Type": "application/gzip"})
    return r.status_code


def dav_list(kind: str, sub: str) -> list[str]:
    root = DAV[kind]["root"] / sub
    return sorted(p.name for p in root.iterdir()) if root.exists() else []


# --------------------------------------------------------------------------- cloud helpers


class Cloud:
    def __init__(self, procs: Procs, binary: Path, dav_kind: str, name: str = "cloud",
                 port: int = 8787, pull_interval: int = 3600) -> None:
        self.procs, self.name, self.port = procs, name, port
        self.dir = RUN / name
        if self.dir.exists():
            shutil.rmtree(self.dir)
        (self.dir / "data").mkdir(parents=True)
        self.api_key = "e2e-" + "k" * 40
        cfg = self.dir / "config.toml"
        cfg.write_text(
            f'webdav_url = "{DAV[dav_kind]["url"]}"\n'
            f'webdav_username = "{DAV_USER}"\nwebdav_password = "{DAV_PASS}"\n'
            f'api_key = "{self.api_key}"\nbind = "127.0.0.1:{port}"\n'
            f'timezone = "{TZ}"\npull_interval = {pull_interval}\n'
            f'data_dir = "{self.dir / "data"}"\nimages_dir = "{self.dir / "data" / "images"}"\n')
        self.skill_cfg = self.dir / "skill.toml"
        self.skill_cfg.write_text(f'endpoint = "http://127.0.0.1:{port}"\napi_key = "{self.api_key}"\n'
                                  f'timeout = 60\ntimezone = "{TZ}"\n')
        env = dict(os.environ)
        env.update({"TZ": TZ, "RUST_LOG": "info"})
        procs.start(name, [str(binary), "--config", str(cfg)], env=env)
        wait_port(port, 60)
        self.base = f"http://127.0.0.1:{port}"

    def api(self, method: str, path: str, **kw: Any) -> requests.Response:
        h = kw.pop("headers", {})
        h["Authorization"] = f"Bearer {self.api_key}"
        return requests.request(method, self.base + path, headers=h, timeout=180, **kw)

    def call(self, path: str) -> dict:
        """POST /sync | /sync/pull | /sync/push，报告里有错误就失败"""
        r = self.api("POST", path)
        check(r.ok, f"cloud {path} -> {r.status_code} {r.text[:300]}")
        try:
            j = r.json()
        except Exception:
            j = {}
        bad = {k: v for k, v in j.items()
               if (k.endswith("Error") and v) or (isinstance(v, str) and v.lower().startswith("err"))}
        check(not bad, f"cloud {path} reported errors: {bad}")
        return j

    def skill(self, *args: str) -> subprocess.CompletedProcess:
        env = dict(os.environ)
        env["TZ"] = TZ
        return subprocess.run(["python3", str(REPO / "cloud/skill/minitodo/minitodo.py"), "--config",
                               str(self.skill_cfg), *args], capture_output=True, text=True, env=env, timeout=240)

    def todos(self, query: str = "") -> list[dict]:
        j = self.api("GET", "/todos" + query).json()
        return j if isinstance(j, list) else j.get("items", j.get("todos", []))

    def stop(self) -> None:
        self.procs.stop(self.name)


# --------------------------------------------------------------------------- tiny test runner


class NotApplicable(Exception):
    """用例在本平台 / 本构建下不适用（结果记 N/A，不计入失败）"""


class Results:
    def __init__(self, only: list[str] | None = None) -> None:
        self.rows: list[tuple[str, str, str]] = []
        self.only = [p for p in (only or []) if p]

    def wanted(self, name: str) -> bool:
        """前缀匹配；带方括号的前缀如 `S[nginx]` / `S03[apache]` = 编号前缀 + 方括号部分"""
        def match(p: str) -> bool:
            if "[" in p:
                head, _, tail = p.partition("[")
                return name.startswith(head) and f"[{tail}" in name
            return name.startswith(p)
        return not self.only or any(match(p) for p in self.only)

    def run(self, name: str, fn: Callable[[], Any], on_fail: Callable[[str], None] | None = None) -> bool:
        if not self.wanted(name):
            return True
        log(f"=== {name}")
        t0 = time.time()
        try:
            detail = fn()
            self.rows.append((name, "PASS", f"{time.time() - t0:.1f}s {detail or ''}".strip()))
            log(f"--- PASS {name} ({time.time() - t0:.1f}s) {detail or ''}")
            return True
        except NotApplicable as e:
            self.rows.append((name, "N/A", str(e)))
            log(f"--- N/A {name}: {e}")
            return True
        except Exception as e:
            tb = traceback.format_exc(limit=8)
            self.rows.append((name, "FAIL", f"{time.time() - t0:.1f}s {e}"))
            log(f"--- FAIL {name}: {e}\n{tb}")
            if on_fail:
                try:
                    on_fail(name.split()[0])
                except Exception:
                    pass
            return False

    def summary(self, elapsed: float) -> str:
        lines = [f"{'RESULT':6}  {'TEST':72}  DETAIL"]
        for n, s, d in self.rows:
            lines.append(f"{s:6}  {n:72}  {d[:300]}")
        passed = sum(1 for _, s, _ in self.rows if s == "PASS")
        failed = sum(1 for _, s, _ in self.rows if s == "FAIL")
        na = sum(1 for _, s, _ in self.rows if s == "N/A")
        lines.append(f"\n{passed} passed, {failed} failed, {na} not applicable; elapsed {elapsed / 60:.1f} min")
        return "\n".join(lines)

