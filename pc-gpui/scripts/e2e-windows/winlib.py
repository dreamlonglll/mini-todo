"""Windows e2e 工具库（只用 ctypes + Pillow）：Win32 窗口查询 / 真实鼠标键盘输入 / 截图与取色 /
应用进程与隔离数据目录 / SQLite 读写 / 用例结果汇总。

坐标约定：本进程设为 Per-Monitor-V2 DPI 感知，所有屏幕坐标都是**物理像素**；`Win.pt(x, y)` 把窗口客户区内的
**逻辑坐标**（GPUI 的 px）按窗口 DPI 换算成屏幕物理坐标。

在 Linux 上可以 import（Win32 部分不可用），用来本地检查图像分析逻辑。
"""
from __future__ import annotations

import base64
import ctypes
import io
import json
import os
import sqlite3
import subprocess
import sys
import time
import traceback
from pathlib import Path
from typing import Callable, Iterable

IS_WIN = sys.platform == "win32"

# ----------------------------------------------------------------------------- 常量

GWL_STYLE = -16
GWL_EXSTYLE = -20
GWLP_HWNDPARENT = -8
GW_HWNDLAST = 1
GW_HWNDPREV = 3
GW_OWNER = 4
GA_ROOT = 2

WS_MINIMIZEBOX = 0x00020000
WS_MAXIMIZEBOX = 0x00010000
WS_THICKFRAME = 0x00040000
WS_EX_TOPMOST = 0x00000008
WS_EX_TOOLWINDOW = 0x00000080
WS_EX_APPWINDOW = 0x00040000
WS_EX_NOACTIVATE = 0x08000000
WS_EX_LAYERED = 0x00080000
WS_EX_NOREDIRECTIONBITMAP = 0x00200000

WM_CLOSE = 0x0010
WM_NCHITTEST = 0x0084
SW_MINIMIZE = 6
SW_RESTORE = 9

HT_NAMES = {
    -2: "HTTRANSPARENT", -1: "HTERROR", 0: "HTNOWHERE", 1: "HTCLIENT", 2: "HTCAPTION", 3: "HTSYSMENU",
    8: "HTMINBUTTON", 9: "HTMAXBUTTON", 10: "HTLEFT", 11: "HTRIGHT", 12: "HTTOP", 13: "HTTOPLEFT",
    14: "HTTOPRIGHT", 15: "HTBOTTOM", 16: "HTBOTTOMLEFT", 17: "HTBOTTOMRIGHT", 18: "HTBORDER", 20: "HTCLOSE",
}

VK = {
    "backspace": 0x08, "tab": 0x09, "enter": 0x0D, "shift": 0x10, "ctrl": 0x11, "alt": 0x12, "esc": 0x1B,
    "space": 0x20, "end": 0x23, "home": 0x24, "left": 0x25, "up": 0x26, "right": 0x27, "down": 0x28,
    "delete": 0x2E, "lwin": 0x5B, "f2": 0x71,
}
EXTENDED_VKS = {0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x2E, 0x5B}

CREATE_NO_WINDOW = 0x08000000
LOCALAPPDATA = Path(os.environ.get("LOCALAPPDATA", str(Path.home() / "AppData" / "Local")))

# ----------------------------------------------------------------------------- ctypes 原型

if IS_WIN:
    from ctypes import wintypes as wt

    user32 = ctypes.WinDLL("user32", use_last_error=True)
    shell32 = ctypes.WinDLL("shell32", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    LONG_PTR = ctypes.c_ssize_t
    ULONG_PTR = ctypes.c_size_t
    WNDENUMPROC = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)

    class MOUSEINPUT(ctypes.Structure):
        _fields_ = [("dx", wt.LONG), ("dy", wt.LONG), ("mouseData", wt.DWORD), ("dwFlags", wt.DWORD),
                    ("time", wt.DWORD), ("dwExtraInfo", ULONG_PTR)]

    class KEYBDINPUT(ctypes.Structure):
        _fields_ = [("wVk", wt.WORD), ("wScan", wt.WORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD),
                    ("dwExtraInfo", ULONG_PTR)]

    class HARDWAREINPUT(ctypes.Structure):
        _fields_ = [("uMsg", wt.DWORD), ("wParamL", wt.WORD), ("wParamH", wt.WORD)]

    class _INPUTU(ctypes.Union):
        _fields_ = [("mi", MOUSEINPUT), ("ki", KEYBDINPUT), ("hi", HARDWAREINPUT)]

    class INPUT(ctypes.Structure):
        _anonymous_ = ("u",)
        _fields_ = [("type", wt.DWORD), ("u", _INPUTU)]

    class MONITORINFO(ctypes.Structure):
        _fields_ = [("cbSize", wt.DWORD), ("rcMonitor", wt.RECT), ("rcWork", wt.RECT), ("dwFlags", wt.DWORD)]

    class GUID(ctypes.Structure):
        _fields_ = [("Data1", wt.DWORD), ("Data2", wt.WORD), ("Data3", wt.WORD), ("Data4", ctypes.c_ubyte * 8)]

    class NOTIFYICONIDENTIFIER(ctypes.Structure):
        _fields_ = [("cbSize", wt.DWORD), ("hWnd", wt.HWND), ("uID", wt.UINT), ("guidItem", GUID)]

    def _proto(fn, res, *args):
        fn.restype = res
        fn.argtypes = list(args)
        return fn

    _proto(user32.EnumWindows, wt.BOOL, WNDENUMPROC, wt.LPARAM)
    _proto(user32.GetWindowThreadProcessId, wt.DWORD, wt.HWND, ctypes.POINTER(wt.DWORD))
    _proto(user32.GetWindowTextLengthW, ctypes.c_int, wt.HWND)
    _proto(user32.GetWindowTextW, ctypes.c_int, wt.HWND, wt.LPWSTR, ctypes.c_int)
    _proto(user32.GetClassNameW, ctypes.c_int, wt.HWND, wt.LPWSTR, ctypes.c_int)
    _proto(user32.IsWindowVisible, wt.BOOL, wt.HWND)
    _proto(user32.IsIconic, wt.BOOL, wt.HWND)
    _proto(user32.IsWindow, wt.BOOL, wt.HWND)
    _proto(user32.GetWindow, wt.HWND, wt.HWND, wt.UINT)
    _proto(user32.GetWindowLongPtrW, LONG_PTR, wt.HWND, ctypes.c_int)
    _proto(user32.GetWindowRect, wt.BOOL, wt.HWND, ctypes.POINTER(wt.RECT))
    _proto(user32.GetClientRect, wt.BOOL, wt.HWND, ctypes.POINTER(wt.RECT))
    _proto(user32.ClientToScreen, wt.BOOL, wt.HWND, ctypes.POINTER(wt.POINT))
    _proto(user32.GetShellWindow, wt.HWND)
    _proto(user32.GetForegroundWindow, wt.HWND)
    _proto(user32.GetDpiForWindow, wt.UINT, wt.HWND)
    _proto(user32.WindowFromPoint, wt.HWND, wt.POINT)
    _proto(user32.GetAncestor, wt.HWND, wt.HWND, wt.UINT)
    _proto(user32.SetCursorPos, wt.BOOL, ctypes.c_int, ctypes.c_int)
    _proto(user32.GetCursorPos, wt.BOOL, ctypes.POINTER(wt.POINT))
    _proto(user32.SendInput, wt.UINT, wt.UINT, ctypes.POINTER(INPUT), ctypes.c_int)
    _proto(user32.SendMessageTimeoutW, LONG_PTR, wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM, wt.UINT, wt.UINT,
           ctypes.POINTER(ULONG_PTR))
    _proto(user32.PostMessageW, wt.BOOL, wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM)
    _proto(user32.ShowWindow, wt.BOOL, wt.HWND, ctypes.c_int)
    _proto(user32.VkKeyScanW, ctypes.c_short, wt.WCHAR)
    _proto(user32.MonitorFromPoint, wt.HMONITOR, wt.POINT, wt.DWORD)
    _proto(user32.MonitorFromWindow, wt.HMONITOR, wt.HWND, wt.DWORD)
    _proto(user32.GetMonitorInfoW, wt.BOOL, wt.HMONITOR, ctypes.POINTER(MONITORINFO))
    _proto(user32.GetSystemMetrics, ctypes.c_int, ctypes.c_int)
    _proto(user32.SetProcessDpiAwarenessContext, wt.BOOL, wt.HANDLE)
    _proto(user32.GetKeyboardLayout, wt.HKL, wt.DWORD)
    _proto(user32.SetForegroundWindow, wt.BOOL, wt.HWND)
    _proto(shell32.Shell_NotifyIconGetRect, ctypes.c_long, ctypes.POINTER(NOTIFYICONIDENTIFIER),
           ctypes.POINTER(wt.RECT))


def set_dpi_aware() -> bool:
    if not IS_WIN:
        return False
    return bool(user32.SetProcessDpiAwarenessContext(wt.HANDLE(-4)))


# ----------------------------------------------------------------------------- 窗口

def _h(hwnd: int):
    return wt.HWND(hwnd)


def top_level_windows() -> list[int]:
    out: list[int] = []

    @WNDENUMPROC
    def cb(h, _):
        out.append(h or 0)
        return True

    user32.EnumWindows(cb, 0)
    return out


def window_pid(hwnd: int) -> int:
    pid = wt.DWORD(0)
    user32.GetWindowThreadProcessId(_h(hwnd), ctypes.byref(pid))
    return pid.value


def window_text(hwnd: int) -> str:
    n = user32.GetWindowTextLengthW(_h(hwnd))
    buf = ctypes.create_unicode_buffer(max(n + 1, 2))
    user32.GetWindowTextW(_h(hwnd), buf, len(buf))
    return buf.value


def class_name(hwnd: int) -> str:
    buf = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(_h(hwnd), buf, 256)
    return buf.value


def shell_window() -> int:
    return user32.GetShellWindow() or 0


def foreground_window() -> int:
    return user32.GetForegroundWindow() or 0


def z_index(target: int) -> int:
    """从 Z 序最底（桌面宿主）往上数可见顶层窗口，返回 target 的序号；-1 = 没找到 / 不可见"""
    shell = shell_window()
    h = user32.GetWindow(_h(shell), GW_HWNDLAST) if shell else None
    if not h:
        # 没有 Explorer：从任意顶层窗口找最底
        tops = top_level_windows()
        h = user32.GetWindow(_h(tops[0]), GW_HWNDLAST) if tops else None
    i = 0
    while h:
        if user32.IsWindowVisible(h):
            if h == target:
                return i
            i += 1
        h = user32.GetWindow(h, GW_HWNDPREV)
    return -1


def window_from_point(x: int, y: int) -> int:
    h = user32.WindowFromPoint(wt.POINT(int(x), int(y)))
    if not h:
        return 0
    return user32.GetAncestor(h, GA_ROOT) or 0


def describe_hwnd(h: int) -> str:
    if not h:
        return "0x0"
    return f"0x{h:X} [{class_name(h)}] '{window_text(h)}' pid={window_pid(h)}"


class Win:
    """一个顶层窗口（属性都是实时读取）"""

    def __init__(self, hwnd: int):
        self.hwnd = int(hwnd)

    def __repr__(self) -> str:
        return f"Win({describe_hwnd(self.hwnd)})"

    def __eq__(self, other) -> bool:
        return isinstance(other, Win) and other.hwnd == self.hwnd

    def __hash__(self) -> int:
        return self.hwnd

    @property
    def title(self) -> str:
        return window_text(self.hwnd)

    @property
    def cls(self) -> str:
        return class_name(self.hwnd)

    @property
    def pid(self) -> int:
        return window_pid(self.hwnd)

    @property
    def alive(self) -> bool:
        return bool(user32.IsWindow(_h(self.hwnd)))

    @property
    def visible(self) -> bool:
        return bool(user32.IsWindowVisible(_h(self.hwnd)))

    @property
    def iconic(self) -> bool:
        return bool(user32.IsIconic(_h(self.hwnd)))

    @property
    def style(self) -> int:
        return user32.GetWindowLongPtrW(_h(self.hwnd), GWL_STYLE) & 0xFFFFFFFF

    @property
    def exstyle(self) -> int:
        return user32.GetWindowLongPtrW(_h(self.hwnd), GWL_EXSTYLE) & 0xFFFFFFFF

    @property
    def owner(self) -> int:
        return user32.GetWindowLongPtrW(_h(self.hwnd), GWLP_HWNDPARENT) & 0xFFFFFFFFFFFFFFFF

    @property
    def dpi(self) -> int:
        return int(user32.GetDpiForWindow(_h(self.hwnd)) or 96)

    @property
    def scale(self) -> float:
        return self.dpi / 96.0

    def rect(self) -> tuple[int, int, int, int]:
        r = wt.RECT()
        user32.GetWindowRect(_h(self.hwnd), ctypes.byref(r))
        return r.left, r.top, r.right, r.bottom

    def client(self) -> tuple[int, int, int, int]:
        """客户区（屏幕物理坐标）：(x, y, w, h)"""
        r = wt.RECT()
        user32.GetClientRect(_h(self.hwnd), ctypes.byref(r))
        p = wt.POINT(0, 0)
        user32.ClientToScreen(_h(self.hwnd), ctypes.byref(p))
        return p.x, p.y, r.right - r.left, r.bottom - r.top

    def logical_size(self) -> tuple[float, float]:
        _, _, w, h = self.client()
        return w / self.scale, h / self.scale

    def pt(self, x: float, y: float) -> tuple[int, int]:
        cx, cy, _, _ = self.client()
        s = self.scale
        return cx + round(x * s), cy + round(y * s)

    def flags(self) -> dict:
        st, ex = self.style, self.exstyle
        return {
            "toolWindow": bool(ex & WS_EX_TOOLWINDOW),
            "appWindow": bool(ex & WS_EX_APPWINDOW),
            "topmost": bool(ex & WS_EX_TOPMOST),
            "noActivate": bool(ex & WS_EX_NOACTIVATE),
            "minimizeBox": bool(st & WS_MINIMIZEBOX),
            "thickFrame": bool(st & WS_THICKFRAME),
        }

    def state(self) -> dict:
        shell = shell_window()
        owner = self.owner
        d = {
            "hwnd": f"0x{self.hwnd:X}",
            "title": self.title,
            "class": self.cls,
            "rect": self.rect(),
            "client": self.client(),
            "visible": self.visible,
            "iconic": self.iconic,
            "owner": f"0x{owner:X}",
            "ownerIsShell": bool(owner) and owner == shell,
            "shell": f"0x{shell:X}",
            "zIndex": z_index(self.hwnd),
            "dpi": self.dpi,
            "style": f"0x{self.style:08X}",
            "exstyle": f"0x{self.exstyle:08X}",
        }
        d.update(self.flags())
        return d

    def nchittest(self, x: int, y: int, move_cursor: bool = True) -> int:
        """对屏幕点 (x, y) 发 WM_NCHITTEST；GPUI 的拖动区按最近一次鼠标位置判断，所以先把光标挪过去"""
        if move_cursor:
            mouse_move(x, y)
            time.sleep(0.25)
        lparam = ((y & 0xFFFF) << 16) | (x & 0xFFFF)
        res = ULONG_PTR(0)
        user32.SendMessageTimeoutW(_h(self.hwnd), WM_NCHITTEST, 0, lparam, 0x0002, 2000, ctypes.byref(res))
        v = res.value
        if v >= 2 ** 63:
            v -= 2 ** 64
        return v

    def post_close(self) -> bool:
        return bool(user32.PostMessageW(_h(self.hwnd), WM_CLOSE, 0, 0))

    def show(self, cmd: int) -> bool:
        return bool(user32.ShowWindow(_h(self.hwnd), cmd))

    def center_hit(self) -> int:
        x, y, w, h = self.client()
        return window_from_point(x + w // 2, y + h // 2)


def windows_of(pids: Iterable[int], title: str | None = None, cls: str | None = None,
               visible: bool = True) -> list[Win]:
    pids = set(pids)
    out = []
    for h in top_level_windows():
        if window_pid(h) not in pids:
            continue
        if visible and not user32.IsWindowVisible(_h(h)):
            continue
        if title is not None and window_text(h) != title:
            continue
        if cls is not None and class_name(h) != cls:
            continue
        out.append(Win(h))
    return out


def monitors() -> list[dict]:
    res = []
    MONITORENUMPROC = ctypes.WINFUNCTYPE(wt.BOOL, wt.HMONITOR, wt.HDC, ctypes.POINTER(wt.RECT), wt.LPARAM)

    @MONITORENUMPROC
    def cb(hmon, _hdc, _rc, _):
        mi = MONITORINFO()
        mi.cbSize = ctypes.sizeof(MONITORINFO)
        user32.GetMonitorInfoW(hmon, ctypes.byref(mi))
        m, w = mi.rcMonitor, mi.rcWork
        res.append({"rect": (m.left, m.top, m.right, m.bottom), "work": (w.left, w.top, w.right, w.bottom),
                    "primary": bool(mi.dwFlags & 1)})
        return True

    user32.EnumDisplayMonitors.argtypes = [wt.HDC, ctypes.c_void_p, MONITORENUMPROC, wt.LPARAM]
    user32.EnumDisplayMonitors(None, None, cb, 0)
    return res


def primary_monitor() -> dict:
    ms = monitors()
    for m in ms:
        if m["primary"]:
            return m
    return ms[0]


def tray_icon_rect(hwnd: int, uid: int) -> tuple[int, tuple | None]:
    """Shell_NotifyIconGetRect：(HRESULT, rect)"""
    ident = NOTIFYICONIDENTIFIER()
    ident.cbSize = ctypes.sizeof(NOTIFYICONIDENTIFIER)
    ident.hWnd = _h(hwnd)
    ident.uID = uid
    r = wt.RECT()
    hr = shell32.Shell_NotifyIconGetRect(ctypes.byref(ident), ctypes.byref(r))
    return hr & 0xFFFFFFFF, ((r.left, r.top, r.right, r.bottom) if hr == 0 else None)


def keyboard_layout() -> str:
    return f"0x{(user32.GetKeyboardLayout(0) or 0) & 0xFFFFFFFF:08X}"


# ----------------------------------------------------------------------------- 输入（真实输入事件：SetCursorPos + SendInput）

MOUSEEVENTF_MOVE = 0x0001
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004
MOUSEEVENTF_RIGHTDOWN = 0x0008
MOUSEEVENTF_RIGHTUP = 0x0010
MOUSEEVENTF_WHEEL = 0x0800
KEYEVENTF_EXTENDEDKEY = 0x0001
KEYEVENTF_KEYUP = 0x0002
KEYEVENTF_UNICODE = 0x0004


def _send(*inputs) -> int:
    arr = (INPUT * len(inputs))(*inputs)
    return user32.SendInput(len(inputs), arr, ctypes.sizeof(INPUT))


def _mouse_input(flags: int, data: int = 0, dx: int = 0, dy: int = 0):
    i = INPUT()
    i.type = 0
    i.mi = MOUSEINPUT(dx, dy, data & 0xFFFFFFFF, flags, 0, 0)
    return i


def _key_input(vk: int, up: bool, scan: int = 0, unicode: bool = False):
    i = INPUT()
    i.type = 1
    flags = KEYEVENTF_KEYUP if up else 0
    if unicode:
        flags |= KEYEVENTF_UNICODE
    elif vk in EXTENDED_VKS:
        flags |= KEYEVENTF_EXTENDEDKEY
    i.ki = KEYBDINPUT(vk, scan, flags, 0, 0)
    return i


def cursor_pos() -> tuple[int, int]:
    p = wt.POINT()
    user32.GetCursorPos(ctypes.byref(p))
    return p.x, p.y


def mouse_move(x: int, y: int) -> None:
    user32.SetCursorPos(int(x), int(y))
    # 零位移的相对移动：产生一条真实的鼠标移动输入（WM_MOUSEMOVE / OLE 拖放循环都能收到）
    _send(_mouse_input(MOUSEEVENTF_MOVE))
    time.sleep(0.03)


def click(x: int, y: int, settle: float = 0.35, right: bool = False) -> None:
    mouse_move(x, y)
    time.sleep(0.08)
    down, up = (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP) if right else (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)
    _send(_mouse_input(down))
    time.sleep(0.05)
    _send(_mouse_input(up))
    time.sleep(settle)


def dblclick(x: int, y: int, settle: float = 0.4) -> None:
    mouse_move(x, y)
    time.sleep(0.08)
    for _ in range(2):
        _send(_mouse_input(MOUSEEVENTF_LEFTDOWN))
        time.sleep(0.03)
        _send(_mouse_input(MOUSEEVENTF_LEFTUP))
        time.sleep(0.06)
    time.sleep(settle)


def drag(x1: int, y1: int, x2: int, y2: int, steps: int = 24, hold: float = 0.15, step_delay: float = 0.03,
         settle: float = 0.4) -> None:
    mouse_move(x1, y1)
    time.sleep(0.1)
    _send(_mouse_input(MOUSEEVENTF_LEFTDOWN))
    time.sleep(hold)
    for i in range(1, steps + 1):
        mouse_move(round(x1 + (x2 - x1) * i / steps), round(y1 + (y2 - y1) * i / steps))
        time.sleep(step_delay)
    time.sleep(hold)
    _send(_mouse_input(MOUSEEVENTF_LEFTUP))
    time.sleep(settle)


def wheel(x: int, y: int, notches: int, settle: float = 0.08) -> None:
    """notches > 0 向上，< 0 向下"""
    mouse_move(x, y)
    step = 120 if notches > 0 else -120
    for _ in range(abs(notches)):
        _send(_mouse_input(MOUSEEVENTF_WHEEL, data=step))
        time.sleep(settle)


def key_combo(*names: str, settle: float = 0.25) -> None:
    """key_combo('ctrl', 'a')；名字见 VK，单个字母 / 数字直接用"""
    vks = []
    for n in names:
        if n in VK:
            vks.append(VK[n])
        elif len(n) == 1:
            vks.append(ord(n.upper()))
        else:
            raise ValueError(n)
    for vk in vks:
        _send(_key_input(vk, False))
        time.sleep(0.02)
    for vk in reversed(vks):
        _send(_key_input(vk, True))
        time.sleep(0.02)
    time.sleep(settle)


def type_text(text: str, delay: float = 0.03, settle: float = 0.3) -> None:
    """逐字符按键：能映射到当前键盘布局的字符发虚拟键（含 Shift），其余字符走 KEYEVENTF_UNICODE"""
    for ch in text:
        code = user32.VkKeyScanW(ch)
        if code == -1:
            _send(_key_input(0, False, ord(ch), unicode=True), _key_input(0, True, ord(ch), unicode=True))
        else:
            vk = code & 0xFF
            shift = bool(code & 0x100)
            seq = []
            if shift:
                seq.append(_key_input(VK["shift"], False))
            seq += [_key_input(vk, False), _key_input(vk, True)]
            if shift:
                seq.append(_key_input(VK["shift"], True))
            _send(*seq)
        time.sleep(delay)
    time.sleep(settle)


def park_cursor() -> None:
    """把光标停到不会悬停到任何窗口的地方（主显示器左边缘中部）"""
    m = primary_monitor()
    l, t, r, b = m["rect"]
    mouse_move(l + 3, (t + b) // 2)


# ----------------------------------------------------------------------------- 截图与取色

def grab(bbox: tuple[int, int, int, int] | None = None):
    from PIL import ImageGrab
    return ImageGrab.grab(bbox=bbox, all_screens=True, include_layered_windows=True).convert("RGB")


def grab_client(win: Win, pad: int = 0):
    x, y, w, h = win.client()
    return grab((x - pad, y - pad, x + w + pad, y + h + pad))


def rgb_at(img, x: int, y: int) -> tuple[int, int, int]:
    x = max(0, min(img.width - 1, int(x)))
    y = max(0, min(img.height - 1, int(y)))
    return tuple(img.getpixel((x, y))[:3])


def color_dist(a, b) -> int:
    return max(abs(int(a[i]) - int(b[i])) for i in range(3))


def saturation(c) -> int:
    return max(c) - min(c)


def hex_rgb(s: str) -> tuple[int, int, int]:
    s = s.lstrip("#")
    return int(s[0:2], 16), int(s[2:4], 16), int(s[4:6], 16)


def runs(values: list[bool], min_len: int = 1) -> list[tuple[int, int]]:
    """连续为真的区间 [start, end]（含）"""
    out, start = [], None
    for i, v in enumerate(values + [False]):
        if v and start is None:
            start = i
        elif not v and start is not None:
            if i - start >= min_len:
                out.append((start, i - 1))
            start = None
    return out


def column_runs(img, x: int, y0: int, y1: int, pred: Callable[[tuple], bool], min_len: int = 1):
    vals = [pred(rgb_at(img, x, y)) for y in range(y0, y1)]
    return [(a + y0, b + y0) for a, b in runs(vals, min_len)]


def find_color_box(img, region: tuple[int, int, int, int], pred: Callable[[tuple], bool], min_count: int = 12):
    """region = (x0, y0, x1, y1)（图像坐标）里满足 pred 的像素外接框；不足 min_count 个返回 None"""
    x0, y0, x1, y1 = region
    xs, ys = [], []
    px = img.load()
    for y in range(max(0, y0), min(img.height, y1)):
        for x in range(max(0, x0), min(img.width, x1)):
            if pred(px[x, y][:3]):
                xs.append(x)
                ys.append(y)
    if len(xs) < min_count:
        return None
    return min(xs), min(ys), max(xs), max(ys), len(xs)


def diff_ratio(a, b, region: tuple[int, int, int, int] | None = None, tol: int = 24) -> float:
    """两张同尺寸图里差异像素的比例"""
    if region:
        a = a.crop(region)
        b = b.crop(region)
    if a.size != b.size:
        return 1.0
    pa, pb = a.load(), b.load()
    n = changed = 0
    for y in range(0, a.height, 2):
        for x in range(0, a.width, 2):
            n += 1
            if color_dist(pa[x, y], pb[x, y]) > tol:
                changed += 1
    return changed / max(1, n)


_EMITTED = [0]


def emit_image(img, name: str, budget: int | None, max_w: int = 360, quality: int = 40) -> bool:
    """把图片缩小后以 JPEG base64 打进日志（产物下载不可用时从日志还原）；budget 为失败截图的张数上限
    （None = 不计数）"""
    if budget is not None:
        if _EMITTED[0] >= budget:
            return False
        _EMITTED[0] += 1
    im = img.copy()
    if im.width > max_w:
        im = im.resize((max_w, round(im.height * max_w / im.width)))
    buf = io.BytesIO()
    im.save(buf, "JPEG", quality=quality)
    b64 = base64.b64encode(buf.getvalue()).decode()
    print(f"===IMG-BEGIN {name}===")
    for i in range(0, len(b64), 1000):
        print(b64[i:i + 1000])
    print(f"===IMG-END {name}===", flush=True)
    return True


# ----------------------------------------------------------------------------- 等待

class Timeout(AssertionError):
    pass


def wait_until(fn: Callable[[], object], timeout: float, what: str, interval: float = 0.25):
    deadline = time.time() + timeout
    last_exc = None
    while True:
        try:
            v = fn()
            if v:
                return v
        except Exception as e:  # noqa: BLE001
            last_exc = e
        if time.time() >= deadline:
            extra = f"（最后一次异常：{last_exc!r}）" if last_exc else ""
            raise Timeout(f"等待超时 {timeout:.0f}s：{what}{extra}")
        time.sleep(interval)


# ----------------------------------------------------------------------------- 应用进程

class App:
    """一个 mini-todo 进程。`data_dir` 非空 = 开发专用数据隔离（仅 debug 构建生效）"""

    def __init__(self, exe: Path, *, data_dir: Path | None = None, fences_root: Path | None = None,
                 debug_open: str | None = None, env: dict | None = None, tag: str = "app",
                 out_dir: Path | None = None):
        self.exe = Path(exe)
        self.data_dir = Path(data_dir) if data_dir else None
        self.fences_root = Path(fences_root) if fences_root else None
        self.debug_open = debug_open
        self.extra_env = env or {}
        self.tag = tag
        self.out_dir = out_dir
        self.proc: subprocess.Popen | None = None
        self.started_at = 0.0
        self._stdout = None

    # --- 路径
    @property
    def isolated(self) -> bool:
        return self.data_dir is not None

    @property
    def db_path(self) -> Path:
        return (self.data_dir / "data.db") if self.data_dir else (LOCALAPPDATA / "mini-todo" / "data.db")

    @property
    def log_path(self) -> Path:
        if self.data_dir:
            return self.data_dir / "logs" / "mini-todo.log"
        return LOCALAPPDATA / "com.tauri-app.mini-todo" / "logs" / "mini-todo.log"

    # --- 进程
    def start(self, args: list[str] | None = None) -> "App":
        env = {k: v for k, v in os.environ.items() if not k.startswith("MINI_TODO_")}
        if self.data_dir:
            env["MINI_TODO_DATA_DIR"] = str(self.data_dir)
        if self.fences_root:
            env["MINI_TODO_FENCES_ROOT"] = str(self.fences_root)
        if self.debug_open:
            env["MINI_TODO_DEBUG_OPEN"] = self.debug_open
        env.update(self.extra_env)
        stdout = subprocess.DEVNULL
        if self.out_dir:
            self.out_dir.mkdir(parents=True, exist_ok=True)
            self._stdout = open(self.out_dir / f"stdout-{self.tag}.txt", "ab")
            stdout = self._stdout
        self.proc = subprocess.Popen([str(self.exe)] + (args or []), env=env, cwd=str(self.exe.parent),
                                     stdout=stdout, stderr=subprocess.STDOUT, creationflags=CREATE_NO_WINDOW)
        self.started_at = time.time()
        return self

    @property
    def pid(self) -> int:
        return self.proc.pid if self.proc else 0

    def alive(self) -> bool:
        return self.proc is not None and self.proc.poll() is None

    def stop(self) -> None:
        if self.proc and self.proc.poll() is None:
            self.proc.kill()
            try:
                self.proc.wait(10)
            except subprocess.TimeoutExpired:
                pass
        if self._stdout:
            self._stdout.close()
            self._stdout = None
        time.sleep(0.3)

    # --- 窗口
    def windows(self, title: str | None = None, cls: str | None = None, visible: bool = True) -> list[Win]:
        if not self.alive():
            return []
        return windows_of([self.pid], title, cls, visible)

    def wait_window(self, title: str, timeout: float = 45, count: int = 1) -> Win:
        def found():
            if not self.alive():
                raise AssertionError(f"进程已退出（exit={self.proc.returncode}）")
            ws = self.windows(title)
            return ws if len(ws) >= count else None
        ws = wait_until(found, timeout, f"窗口 '{title}' ×{count}")
        return ws[0]

    def wait_gone(self, win: Win, timeout: float = 10) -> None:
        wait_until(lambda: not win.alive or not win.visible, timeout, f"窗口 {win.title or win.hwnd} 关闭")

    def main(self, timeout: float = 45) -> Win:
        return self.wait_window("Mini Todo", timeout)

    # --- 日志
    def log_text(self) -> str:
        try:
            return self.log_path.read_text(encoding="utf-8", errors="replace")
        except FileNotFoundError:
            return ""

    def log_size(self) -> int:
        try:
            return self.log_path.stat().st_size
        except FileNotFoundError:
            return 0

    def log_since(self, offset: int) -> str:
        try:
            with open(self.log_path, "rb") as f:
                f.seek(offset)
                return f.read().decode("utf-8", errors="replace")
        except FileNotFoundError:
            return ""

    # --- 数据库
    def q(self, sql: str, params: tuple = ()) -> list[tuple]:
        con = sqlite3.connect(f"file:{self.db_path}?mode=ro", uri=True, timeout=10)
        try:
            return con.execute(sql, params).fetchall()
        finally:
            con.close()

    def q1(self, sql: str, params: tuple = ()):
        rows = self.q(sql, params)
        return rows[0][0] if rows and rows[0] else None

    def setting(self, key: str):
        return self.q1("SELECT value FROM settings WHERE key = ?", (key,))

    def write(self, sql: str, params: tuple = ()) -> None:
        """只在应用没运行时写库（用于准备数据）"""
        con = sqlite3.connect(str(self.db_path), timeout=10)
        try:
            con.execute(sql, params)
            con.commit()
        finally:
            con.close()


def kill_all(exe: Path) -> None:
    """结束路径与 exe 相同的 mini-todo 进程（只碰本次构建出来的 exe）"""
    target = str(Path(exe).resolve()).lower()
    out = subprocess.run(["powershell", "-NoProfile", "-Command",
                          "Get-Process mini-todo -ErrorAction SilentlyContinue | ForEach-Object { \"$($_.Id)|$($_.Path)\" }"],
                         capture_output=True, text=True, timeout=60).stdout
    for line in out.splitlines():
        if "|" not in line:
            continue
        pid, path = line.split("|", 1)
        if path.strip().lower() == target:
            subprocess.run(["taskkill", "/F", "/PID", pid.strip()], capture_output=True)


def seed(seed_exe: Path, data_dir: Path, sample: Path) -> None:
    if data_dir.exists():
        import shutil
        shutil.rmtree(data_dir, ignore_errors=True)
    r = subprocess.run([str(seed_exe), str(data_dir), str(sample)], capture_output=True, text=True,
                       encoding="utf-8", errors="replace", timeout=120)
    if r.returncode != 0:
        raise RuntimeError(f"seed 失败: {r.stdout}\n{r.stderr}")


def run_ps(script: str, timeout: float = 60) -> str:
    r = subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script],
                       capture_output=True, text=True, timeout=timeout, encoding="utf-8", errors="replace")
    return (r.stdout + r.stderr).strip()


# ----------------------------------------------------------------------------- 用例与汇总

class Skip(Exception):
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
        return self.ok(abs(actual - expected) <= tol, f"{what}: {actual} ≈ {expected} (±{tol})")

    def eq(self, actual, expected, what: str) -> bool:
        return self.ok(actual == expected, f"{what}: {actual!r} == {expected!r}")

    def done(self, summary: str = "") -> str:
        bad = [w for c, w in self.items if not c]
        if bad:
            raise AssertionError("; ".join(bad))
        return summary or f"{len(self.items)} 项检查通过"


class Report:
    def __init__(self, out_dir: Path, title: str, emit_budget: int = 2):
        self.out_dir = out_dir
        self.title = title
        self.emit_budget = emit_budget
        self.results: list[dict] = []
        self.pending_images: list[tuple] = []
        out_dir.mkdir(parents=True, exist_ok=True)

    def case(self, cid: str, title: str, fn: Callable[[], str | None], on_fail: Callable[[str], None] | None = None):
        print(f"\n::group::[{cid}] {title}", flush=True)
        t0 = time.time()
        status, detail = "PASS", ""
        try:
            detail = fn() or ""
        except Skip as e:
            status, detail = "SKIP", str(e)
        except AssertionError as e:
            status, detail = "FAIL", str(e)
        except Exception as e:  # noqa: BLE001
            status, detail = "FAIL", f"{type(e).__name__}: {e}"
            traceback.print_exc()
        dt = time.time() - t0
        if status == "FAIL":
            try:
                img = grab()
                path = self.out_dir / f"fail-{cid}.png"
                img.save(path)
                self.pending_images.append((cid, img))
            except Exception as e:  # noqa: BLE001
                print(f"  (失败截图出错: {e})")
            if on_fail:
                try:
                    on_fail(cid)
                except Exception as e:  # noqa: BLE001
                    print(f"  (on_fail 出错: {e})")
        print(f"[{status}] {cid} {title} ({dt:.1f}s) {detail}", flush=True)
        print("::endgroup::", flush=True)
        self.results.append({"id": cid, "title": title, "status": status, "detail": detail, "seconds": round(dt, 1)})
        return status == "PASS"

    def add(self, cid: str, title: str, status: str, detail: str) -> None:
        print(f"[{status}] {cid} {title} {detail}", flush=True)
        self.results.append({"id": cid, "title": title, "status": status, "detail": detail, "seconds": 0})

    def failed(self) -> list[dict]:
        return [r for r in self.results if r["status"] == "FAIL"]

    def finish(self) -> int:
        # 失败截图（小图）放在汇总表之前，汇总表永远是日志最后一段
        for cid, img in self.pending_images:
            emit_image(img, f"fail-{cid}", self.emit_budget)
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
