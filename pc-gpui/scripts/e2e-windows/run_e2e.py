#!/usr/bin/env python3
"""GPUI 版 mini-todo 的 Windows 端到端测试（真实窗口 + 真实鼠标键盘输入 + Win32 只读断言）。

    python run_e2e.py --exe <mini-todo.exe> [--seed <seed.exe>] --out <输出目录> [--suite debug|release]
                      [--only W,S,...] [--allow-real-profile] [--emit-shots W01,I05]

- debug 套件：各窗口外观（W）、窗口模式与 Win32 状态（S）、应用内通知（N）、交互（I）、WebDAV（D）、
  收纳箱（F）、平台集成（P，需 --allow-real-profile）。除 P 外全部在开发专用数据隔离下运行
  （MINI_TODO_DATA_DIR / MINI_TODO_FENCES_ROOT，只在 debug 构建生效）。
- release 套件：release 冒烟 + 平台集成（P）。
- P 组不设 MINI_TODO_DATA_DIR，会读写 %LOCALAPPDATA%\\mini-todo 与开机自启注册表项，所以默认跳过；
  CI 的一次性虚拟机上用 --allow-real-profile 打开。**不要在装着正式版、有真实数据的机器上打开它。**

每个用例打印 PASS / FAIL / SKIP 与原因，最后打印汇总表（也写进 GITHUB_STEP_SUMMARY、<out>/summary.md、
<out>/results.json）；任一 FAIL 时退出码为 1。截图在 <out>/*.png。
"""
from __future__ import annotations

import argparse
import datetime as dt
import os
import re
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

import winlib as W
from winlib import App, Checks, Report, Skip, Win

HERE = Path(__file__).resolve().parent
MAIN = "Mini Todo"
EP_PRIMARY = (64, 158, 255)      # Element Plus 主色 #409EFF（编辑按钮文字、确认框按钮、开关开）
APP_PRIMARY = (59, 130, 246)     # 应用主色 #3B82F6（FAB、设置页主按钮）
SWITCH_OFF = (220, 223, 230)     # 开关关闭轨道 #DCDFE6
PROBE_TITLE = "__mini_todo_e2e_probe"
DRAG_TITLE = "__mini_todo_e2e_dragsrc"
# 任务栏按钮的名字前缀（Win11：应用名 "mini-todo - 1 个运行窗口"；经典任务栏：窗口标题）与 AppUserModelID（app lib.rs 的 APP_IDENTIFIER）
TASKBAR_PREFIXES = ("mini-todo", MAIN)
APP_ID = "com.tauri-app.mini-todo"
DAV_USER = "e2e"
DAV_PASS = "Pw-e2e-7f3a9c"


class Ctx:
    exe: Path
    seed_exe: Path | None
    sample: Path
    out: Path
    work: Path
    report: Report
    desktop_rgb: tuple = (0, 0, 0)
    explorer: bool = False
    emit_shots: set = set()
    suite: str = "debug"


C = Ctx()


# ----------------------------------------------------------------------------- 公共

def shot(win: Win, name: str, pad: int = 0):
    img = W.grab_client(win, pad)
    img.save(C.out / f"{name}.png")
    return img


def maybe_emit(cid: str, img, name: str | None = None) -> None:
    """--emit-shots 里列出的用例：截图另存一份小图，由 emit_annotations.py 作为注解发出"""
    if cid in C.emit_shots:
        W.save_emit(img, C.out, name or f"shot-{cid}")


def fresh_data(name: str, seed: bool = True) -> Path:
    d = C.work / name
    if seed:
        if not C.seed_exe:
            raise Skip("没有 seed.exe（--seed）")
        W.seed(C.seed_exe, d, C.sample)
    else:
        shutil.rmtree(d, ignore_errors=True)
        d.mkdir(parents=True, exist_ok=True)
    return d


def log_errors(app: App) -> list[str]:
    return [l for l in app.log_text().splitlines() if "[ERROR]" in l or "panicked" in l]


def dump_log_tail(app: App, n: int = 25) -> None:
    lines = app.log_text().splitlines()
    print(f"  --- {app.tag} 日志最后 {min(n, len(lines))} 行（{app.log_path}）---")
    for l in lines[-n:]:
        print("  | " + l)


def all_windows_of_class(cls: str) -> list[Win]:
    return [Win(h) for h in W.top_level_windows()
            if W.class_name(h) == cls and W.user32.IsWindowVisible(W.wt.HWND(h))]


def all_windows_titled(title: str) -> list[Win]:
    return [Win(h) for h in W.top_level_windows()
            if W.window_text(h) == title and W.user32.IsWindowVisible(W.wt.HWND(h))]


def has_content(img) -> int:
    """图里不同颜色的数量（缩小后统计）：GPUI 真的画了东西才会远大于个位数"""
    small = img.resize((max(1, img.width // 4), max(1, img.height // 4)))
    return len(small.getcolors(1 << 24) or [])


def title_button(main: Win, index_from_right: int) -> tuple[int, int]:
    """主窗口标题栏右区按钮（从右往左 0=设置 1=同步 2=固定 3=主题 4=视图 5=已完成）：28×28、间距 8、右内边距 16、描边 1"""
    w, _ = main.logical_size()
    return main.pt(w - 1 - 16 - 14 - 36 * index_from_right, 27)


def detect_rows(main: Win, img=None) -> list[float]:
    """主窗口列表里每行颜色圆点的中心 y（逻辑像素，自上而下）"""
    img = img or W.grab_client(main)
    s = main.scale
    _, h = main.logical_size()
    x = round(33 * s)
    rs = W.column_runs(img, x, round(54 * s), round((h - 60) * s),
                       lambda c: W.saturation(c) >= 60 and max(c) >= 80, min_len=max(3, round(4 * s)))
    return [((a + b) / 2) / s for a, b in rs if (b - a + 1) <= 12 * s]


def pending_ids(app: App) -> list[int]:
    return [r[0] for r in app.q("SELECT id FROM todos WHERE completed = 0 ORDER BY sort_order ASC, created_at DESC")]


def row_y(app: App, main: Win, todo_id: int) -> float:
    rows = detect_rows(main)
    ids = pending_ids(app)
    print(f"    rows(dot y)={[round(r) for r in rows]} pending ids={ids}")
    if todo_id not in ids:
        raise AssertionError(f"待办 #{todo_id} 不在未完成列表里")
    i = ids.index(todo_id)
    if i >= len(rows):
        raise AssertionError(f"界面上只识别到 {len(rows)} 行，找不到第 {i + 1} 行（#{todo_id}）")
    return rows[i]


def detect_switches(win: Win, img=None) -> list[tuple[float, bool]]:
    """设置窗口内容区右侧的 EP 开关：[(中心 y 逻辑像素, 是否打开)]"""
    img = img or W.grab_client(win)
    s = win.scale
    gx0, gx1 = round(636 * s), round(656 * s)
    bx0, bx1 = round(616 * s), round(632 * s)
    flags = []
    for y in range(round(56 * s), img.height):
        gray = sum(1 for x in range(gx0, gx1) if W.color_dist(W.rgb_at(img, x, y), SWITCH_OFF) <= 16)
        blue = sum(1 for x in range(bx0, bx1) if W.color_dist(W.rgb_at(img, x, y), EP_PRIMARY) <= 40)
        flags.append(2 if blue >= 6 else (1 if gray >= 8 else 0))
    y0 = round(56 * s)
    out = []
    for kind in (1, 2):
        for a, b in W.runs([f == kind for f in flags], min_len=max(6, round(10 * s))):
            out.append((((a + b) / 2 + y0) / s, kind == 2))
    return sorted(out)


def find_blue_box(win: Win, region_logical: tuple, rgb=EP_PRIMARY, tol: int = 45, img=None):
    """窗口内逻辑区域里某种蓝色像素的外接框中心（逻辑像素）；没找到 None"""
    img = img or W.grab_client(win)
    s = win.scale
    x0, y0, x1, y1 = (round(v * s) for v in region_logical)
    box = W.find_color_box(img, (x0, y0, x1, y1), lambda c: W.color_dist(c, rgb) <= tol, min_count=10)
    if not box:
        return None
    bx0, by0, bx1, by1, n = box
    print(f"    blue box {region_logical} → ({bx0 / s:.0f},{by0 / s:.0f})-({bx1 / s:.0f},{by1 / s:.0f}) n={n}")
    return (bx0 + bx1) / 2 / s, (by0 + by1) / 2 / s


EP_INPUT_BORDER = (220, 223, 230)   # EP 输入框边框 #DCDFE6
SETTINGS_BTN_BORDER = (226, 232, 240)  # 设置页白色按钮边框 #E2E8F0


def detect_boxes(win: Win, x_logical: float, y0: float, y1: float, border=EP_INPUT_BORDER, tol: int = 6,
                 min_h: float = 16, max_h: float = 36, img=None) -> list[tuple[float, float]]:
    """沿一列找"上下两条边框线、中间白底"的框（输入框 / 白色按钮）：[(上边, 下边)]（逻辑像素）"""
    img = img or W.grab_client(win)
    s = win.scale
    x = round(x_logical * s)
    lines = [a for a, _ in W.column_runs(img, x, round(y0 * s), min(img.height, round(y1 * s)),
                                         lambda c: W.color_dist(c, border) <= tol)]
    boxes = []
    for i in range(len(lines) - 1):
        top, bottom = lines[i], lines[i + 1]
        inside = (W.rgb_at(img, x, top + round(2 * s)), W.rgb_at(img, x, bottom - round(2 * s)))
        if min_h * s <= bottom - top <= max_h * s and all(min(c) >= 253 for c in inside):
            boxes.append((top / s, bottom / s))
    return boxes


def start_probe(x: int, y: int, w: int, h: int) -> tuple[subprocess.Popen, Win]:
    p = subprocess.Popen(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-File",
                          str(HERE / "probe-window.ps1"), "-Title", PROBE_TITLE, "-X", str(x), "-Y", str(y),
                          "-W", str(w), "-H", str(h)], creationflags=W.CREATE_NO_WINDOW)
    win = W.wait_until(lambda: (all_windows_titled(PROBE_TITLE) or [None])[0], 30, "探针窗口出现")
    time.sleep(0.5)
    return p, win


def stop_proc(p: subprocess.Popen | None) -> None:
    if p and p.poll() is None:
        p.kill()
        try:
            p.wait(5)
        except subprocess.TimeoutExpired:
            pass


def watch_shell_action(action, targets: list[Win], ms: int = 1800) -> dict:
    track = {t.hwnd: {"iconic": False, "hidden": False} for t in targets}
    action()
    t_end = time.time() + ms / 1000
    while time.time() < t_end:
        for t in targets:
            if t.iconic:
                track[t.hwnd]["iconic"] = True
            if not t.visible:
                track[t.hwnd]["hidden"] = True
        time.sleep(0.02)
    return track


def win_d() -> None:
    W.key_combo("lwin", "d", settle=0.1)


def restart_explorer() -> tuple[int, int, float]:
    old = W.shell_window()
    subprocess.run(["taskkill", "/F", "/IM", "explorer.exe"], capture_output=True)
    time.sleep(2)
    if "explorer.exe" not in subprocess.run(["tasklist", "/FI", "IMAGENAME eq explorer.exe"], capture_output=True,
                                            text=True).stdout:
        subprocess.Popen(["explorer.exe"])
    t0 = time.time()
    W.wait_until(lambda: W.shell_window() not in (0, old), 40, "Explorer 重启后桌面宿主窗口出现")
    return old, W.shell_window(), time.time() - t0


def run_value() -> str | None:
    import winreg
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Run") as k:
            return winreg.QueryValueEx(k, "mini-todo")[0]
    except FileNotFoundError:
        return None


def now_str(delta_minutes: float = 0) -> str:
    return (dt.datetime.now() + dt.timedelta(minutes=delta_minutes)).strftime("%Y-%m-%d %H:%M:%S")


def is_transparent_blend(c, desktop) -> bool:
    """深色主题半透明底：像素应是桌面色按比例变暗（0.15~0.9），而不是纯黑 / 纯白"""
    if max(desktop) < 60:
        return False
    ratios = [c[i] / desktop[i] for i in range(3) if desktop[i] >= 60]
    return all(0.12 <= r <= 0.92 for r in ratios) and max(ratios) - min(ratios) <= 0.25


def check_no_caption_buttons(win: Win, c: Checks, what: str = "") -> None:
    """open_window 建窗后去掉了 WS_SYSMENU：DWM 不再布置系统标题栏按钮（否则系统 × 会从半透明客户区透出来，激活时带红底）"""
    label = f"{what}：" if what else ""
    st = win.style
    c.ok(not (st & W.WS_SYSMENU), f"{label}没有 WS_SYSMENU（style=0x{st:08X}）")
    b = win.caption_buttons()
    c.ok(b is not None and b[2] - b[0] <= 0, f"{label}DWM 标题栏按钮区宽度为 0（DWMWA_CAPTION_BUTTON_BOUNDS={b}）")


def find_taskbar_buttons(timeout: int = 10) -> tuple[list[tuple[tuple[int, int, int, int], str, str]], str]:
    """UIA 找本应用的任务栏按钮（taskbar-button.ps1）：([(屏幕矩形 x,y,w,h, 名字, 任务栏窗口类名)], 脚本原始输出)"""
    prefixes = ",".join(f"'{p}'" for p in TASKBAR_PREFIXES)
    out = W.run_ps(f"& '{HERE / 'taskbar-button.ps1'}' -Prefix {prefixes} -AppId '{APP_ID}' -TimeoutSec {timeout}",
                   timeout=timeout + 90)
    hits = []
    for line in out.splitlines():
        parts = line.split("|")
        if len(parts) < 4 or parts[0].count(",") != 3:
            continue
        try:
            rect = tuple(int(v) for v in parts[0].split(","))
        except ValueError:
            continue
        hits.append((rect, parts[1], parts[3]))
    return hits, out


# ----------------------------------------------------------------------------- 环境

def setup_env() -> None:
    W.set_dpi_aware()
    W.kill_all(C.exe)
    m = W.primary_monitor()
    C.explorer = bool(W.shell_window())
    # 屏幕上其它程序的窗口（例如 GitHub 托管 runner 的 hosted-compute-agent 终端）会挡住桌面、干扰取色与 Z 序断言：最小化
    keep = {"Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "tray_icon_app"}
    minimized = []
    for h in W.top_level_windows():
        win = Win(h)
        if not win.visible or win.cls in keep or not win.title or win.flags()["toolWindow"]:
            continue
        x0, y0, x1, y1 = win.rect()
        if x1 - x0 < 50 or y1 - y0 < 50:
            continue
        win.show(W.SW_MINIMIZE)
        minimized.append(W.describe_hwnd(h))
    print(f"  已最小化其它程序的窗口: {minimized}")
    W.park_cursor()
    W.redraw_desktop()
    time.sleep(2)
    # 桌面颜色：主显示器工作区 5×5 个点取众数（启动应用之前，屏幕上应只有桌面）
    l, t, r, b = m["work"]
    img = W.grab((l, t, r, b))
    pts = [(img.width * fx // 10, img.height * fy // 10) for fx in (1, 3, 5, 7, 9) for fy in (1, 3, 5, 7, 9)]
    colors = [W.rgb_at(img, x, y) for x, y in pts]
    C.desktop_rgb = max(set(colors), key=colors.count)
    img.save(C.out / "desktop-before.png")
    hist = {str(cc): colors.count(cc) for cc in set(colors)}
    others = [W.describe_hwnd(h) for h in W.top_level_windows() if W.user32.IsWindowVisible(W.wt.HWND(h))][:12]
    print(f"  desktop colors={hist}\n  visible windows={others}")
    info = (f"primary={m['rect']} work={m['work']} monitors={len(W.monitors())} shell={W.describe_hwnd(W.shell_window())} "
            f"explorer={C.explorer} desktop_rgb={C.desktop_rgb}（{colors.count(C.desktop_rgb)}/25）kbd={W.keyboard_layout()}")
    C.report.add("ENV", "运行环境", "PASS", info)


# ----------------------------------------------------------------------------- W：窗口启动与外观

WINDOW_CASES = [
    # id, DEBUG_OPEN, 说明, 窗口标题, 逻辑尺寸
    ("W01", "main", "主窗口", MAIN, (380, 600)),
    ("W02", "main,editor:1:view", "待办详情（只读）", "待办详情", (1080, 600)),
    ("W03", "main,editor:1:edit", "待办详情（编辑）", "待办详情", (1080, 600)),
    ("W04", "main,editor:new", "新建待办", "新建待办", (1080, 600)),
    ("W05", "main,subtask:101:view", "子任务（查看）", "查看子任务", (800, 750)),
    ("W06", "main,settings:general", "设置-常规", "设置", (680, 560)),
    ("W07", "main,settings:appearance", "设置-外观", "设置", (680, 560)),
    ("W08", "main,settings:data", "设置-数据与同步", "设置", (680, 560)),
    ("W09", "main,settings:screen", "设置-屏幕配置", "设置", (680, 560)),
    ("W10", "main,settings:about", "设置-关于", "设置", (680, 560)),
    ("W11", "main,completed", "已完成", "已完成", (460, 550)),
    ("W12", "main,notification:1", "应用内通知", "通知", (320, 120)),
]


def group_windows(uia: bool) -> None:
    data = None

    def run_one(cid, spec, title, size):
        nonlocal data
        if data is None:
            data = fresh_data("w-data")
        app = App(C.exe, data_dir=data, fences_root=C.work / "w-fences", debug_open=spec, tag=cid, out_dir=C.out)
        app.start()
        try:
            win = app.wait_window(title, 60)
            W.park_cursor()
            time.sleep(2.5)
            c = Checks()
            st = win.state()
            print(f"    state={st}")
            lw, lh = win.logical_size()
            c.ok(win.visible and not win.iconic, "窗口可见且未最小化")
            c.eq(win.cls, "Zed::Window", "窗口类名")
            check_no_caption_buttons(win, c)
            c.near(round(lw), size[0], 2, "逻辑宽")
            c.near(round(lh), size[1], 2, "逻辑高")
            img = shot(win, f"{cid}-{title}")
            n = has_content(img)
            c.ok(n >= 12, f"界面已渲染（不同颜色数 {n}）")
            maybe_emit(cid, W.grab_client(win, 12) if cid == "W01" else img)
            if cid == "W01":
                check_main_transparency(win, img, c)
            if uia and cid == "W06":
                print(W.run_ps(f"& '{HERE / 'uia-dump.ps1'}' -Hwnd {win.hwnd} -Max 80", timeout=60))
            errs = [e for e in log_errors(app) if "panicked" in e]
            c.ok(not errs, f"日志无 panic（{errs[:2]}）")
            return c.done(f"{round(lw)}×{round(lh)} @dpi{win.dpi}, colors={n}")
        except Exception:
            dump_log_tail(app)
            raise
        finally:
            app.stop()

    for cid, spec, desc, title, size in WINDOW_CASES:
        C.report.case(cid, f"窗口：{desc}（{spec}）", lambda: run_one(cid, spec, title, size))


def check_main_transparency(win: Win, img, c: Checks) -> None:
    """透明窗口：客户区四角（圆角之外）应透出背后的桌面；中心是白色不透明底。
    对照色取窗口外框（含不可见缩放边框）之外 25px 的三个点，多数一致时用它，否则用启动前采的桌面色"""
    w, h = img.size
    x, y, _, _ = win.client()
    l, t, r, b = win.rect()
    pts = {"left": (l - 25, y + h // 2), "right": (r + 25, y + h // 2), "top": (x + w // 2, max(0, t - 25))}
    outs = {k: W.rgb_at(W.grab((px, py, px + 1, py + 1)), 0, 0) for k, (px, py) in pts.items()}
    vals = list(outs.values())
    ref = next((v for v in vals if sum(W.color_dist(v, o) <= 20 for o in vals) >= 2), C.desktop_rgb)
    corners = [W.rgb_at(img, 0, 0), W.rgb_at(img, w - 1, 0), W.rgb_at(img, 0, h - 1), W.rgb_at(img, w - 1, h - 1)]
    center = W.rgb_at(img, w // 2, h // 2)
    info = f"窗外={outs} 对照={ref} 四角={corners} 中心={center} 桌面(启动前)={C.desktop_rgb}"
    print(f"    {info}")
    ok = [W.color_dist(cc, ref) <= 45 for cc in corners]
    c.ok(all(ok), f"四角透出背后的桌面（{sum(ok)}/4；{info}）")
    c.ok(W.color_dist(ref, (255, 255, 255)) > 40 and W.color_dist(ref, (0, 0, 0)) > 40,
         f"对照色可分辨（不是纯白 / 纯黑：{ref}）")


# ----------------------------------------------------------------------------- S：窗口模式与 Win32 状态

def group_states() -> None:
    data = fresh_data("s-data")
    app = App(C.exe, data_dir=data, debug_open="main", tag="S", out_dir=C.out)
    probe = None
    st: dict = {"main": None}

    def need_main() -> Win:
        if not st["main"]:
            raise Skip("前置用例没有拿到主窗口")
        return st["main"]

    def s01():
        app.start()
        main = app.main(60)
        st["main"] = main
        W.park_cursor()
        time.sleep(2.5)
        c = Checks()
        f = main.flags()
        print(f"    state={main.state()}")
        c.eq(main.owner, 0, "owner")
        c.ok(not f["toolWindow"] and f["appWindow"], f"APPWINDOW 且非 TOOLWINDOW（{f}）")
        c.ok(f["minimizeBox"] and f["thickFrame"], "有最小化按钮与可缩放边框")
        c.ok(not f["topmost"], "不置顶")
        check_no_caption_buttons(main, c, "普通模式")
        title_pt = main.pt(60, 47)
        ht = main.nchittest(*title_pt)
        c.eq(W.HT_NAMES.get(ht, ht), "HTCAPTION", "标题栏 WM_NCHITTEST")
        rect, (cx, cy, cw, ch) = main.rect(), main.client()
        if rect[0] < cx - 2:
            ht = main.nchittest(cx - 3, cy + ch // 2)
            c.eq(W.HT_NAMES.get(ht, ht), "HTLEFT", "左边框 WM_NCHITTEST")
        before = main.client()
        W.drag(title_pt[0], title_pt[1], title_pt[0] + 120, title_pt[1] + 80)
        after = main.client()
        c.ok(abs(after[0] - before[0] - 120) <= 3 and abs(after[1] - before[1] - 80) <= 3,
             f"拖动标题栏移动了窗口 {before[:2]} → {after[:2]}")
        W.park_cursor()
        try:
            saved = W.wait_until(lambda: (lambda r: r if r and abs(r[0] - after[0]) <= 2 and abs(r[1] - after[1]) <= 2 else None)(
                (app.q("SELECT window_x, window_y, is_fixed FROM screen_configs ORDER BY updated_at DESC LIMIT 1") or [None])[0]),
                6, "screen_configs 记下新位置")
            c.ok(True, f"screen_configs 已保存位置 {saved}")
        except W.Timeout as e:
            c.ok(False, f"{e}；screen_configs={app.q('SELECT config_id, window_x, window_y, window_width, window_height, is_fixed FROM screen_configs')}")
        shot(main, "S01-normal")
        # 真实双击标题栏 → 最大化 → 再双击 → 还原（去掉 WS_SYSMENU 后系统的 HTCAPTION 双击仍然生效）。放在拖动落库之后，
        # 还原后再等一次落库，后面的用例（S03 重启恢复位置）不受影响
        restored = main.client()
        W.dblclick(*main.pt(60, 47))
        W.park_cursor()
        try:
            W.wait_until(lambda: main.zoomed, 5, "双击标题栏后最大化（IsZoomed）")
            c.ok(True, f"双击标题栏最大化（客户区 {main.client()}）")
        except W.Timeout as e:
            c.ok(False, f"{e}；state={main.state()}")
        if main.zoomed:
            time.sleep(0.5)
            W.dblclick(*main.pt(60, 47))
            W.park_cursor()
            try:
                W.wait_until(lambda: not main.zoomed, 5, "再双击标题栏后还原")
                c.ok(True, "再双击标题栏还原")
            except W.Timeout as e:
                c.ok(False, str(e))
                main.show(W.SW_RESTORE)  # 不让后面的用例在最大化状态下跑
        time.sleep(0.8)
        now = main.client()
        c.ok(all(abs(a - b) <= 2 for a, b in zip(now, restored)), f"还原后几何复原 {restored} → {now}")
        try:
            W.wait_until(lambda: (lambda r: r and abs(r[0] - now[0]) <= 2 and abs(r[1] - now[1]) <= 2)(
                (app.q("SELECT window_x, window_y FROM screen_configs ORDER BY updated_at DESC LIMIT 1") or [None])[0]),
                6, "还原后 screen_configs 仍是还原后的位置")
            c.ok(True, "还原后 screen_configs 仍是还原后的位置")
        except W.Timeout as e:
            c.ok(False, f"{e}；screen_configs={app.q('SELECT window_x, window_y, window_width, window_height FROM screen_configs')}")
        return c.done(f"normal: {f}")

    def s02():
        main = need_main()
        lock = title_button(main, 2)
        W.click(*lock)
        W.park_cursor()
        c = Checks()
        try:
            W.wait_until(lambda: main.flags()["toolWindow"], 6, "切到经典固定（TOOLWINDOW）")
        except W.Timeout as e:
            c.ok(False, str(e))
        time.sleep(1.5)
        f = main.flags()
        print(f"    state={main.state()}")
        c.ok(f["toolWindow"] and not f["appWindow"], f"TOOLWINDOW 且非 APPWINDOW（{f}）")
        c.eq(main.owner, 0, "owner")
        check_no_caption_buttons(main, c, "经典固定")
        c.eq(app.q1("SELECT is_fixed FROM screen_configs ORDER BY updated_at DESC LIMIT 1"), 1, "screen_configs.is_fixed")
        c.eq(app.setting("is_fixed"), "true", "settings.is_fixed")
        title_pt = main.pt(60, 47)
        ht = main.nchittest(*title_pt)
        c.ok(ht != 2, f"标题栏不再是拖动区（{W.HT_NAMES.get(ht, ht)}）")
        rect, (cx, cy, cw, ch) = main.rect(), main.client()
        if rect[0] < cx - 2:
            ht = main.nchittest(cx - 3, cy + ch // 2)
            c.ok(ht not in (10, 11, 12, 13, 14, 15, 16, 17), f"左边框不再是缩放区（{W.HT_NAMES.get(ht, ht)}）")
        ht = main.nchittest(cx + cw - 2, cy + ch - 2)
        c.ok(ht not in (10, 11, 12, 13, 14, 15, 16, 17), f"右下角不是缩放区（{W.HT_NAMES.get(ht, ht)}）")
        before = main.client()
        W.drag(title_pt[0], title_pt[1], title_pt[0] + 120, title_pt[1] + 80)
        after = main.client()
        c.eq(after, before, "拖动标题栏后窗口不动")
        W.park_cursor()
        shot(main, "S02-classic-fixed")
        return c.done(f"classic fixed: {f}")

    def s03():
        need_main()
        app.stop()
        app.write("INSERT INTO settings (key, value) VALUES ('fixed_embed_desktop', 'true') "
                  "ON CONFLICT(key) DO UPDATE SET value = 'true'")
        app.start()
        main = app.main(60)
        st["main"] = main
        c = Checks()
        try:
            W.wait_until(lambda: main.owner and main.owner == W.shell_window(), 10, "owner = 桌面宿主")
        except W.Timeout as e:
            c.ok(False, str(e))
        time.sleep(1)
        f = main.flags()
        state = main.state()
        print(f"    state={state} shell={W.describe_hwnd(W.shell_window())}")
        c.ok(state["ownerIsShell"], f"owner 是桌面宿主（owner={state['owner']} shell={state['shell']} {W.class_name(W.shell_window())}）")
        c.ok(f["toolWindow"] and not f["appWindow"], f"TOOLWINDOW 且非 APPWINDOW（{f}）")
        c.ok(not f["minimizeBox"], "没有 WS_MINIMIZEBOX")
        check_no_caption_buttons(main, c, "嵌入桌面")
        c.ok("已嵌入桌面" in app.log_text(), "日志记录了嵌入桌面")
        shot(main, "S03-embedded")
        return c.done(f"embedded: owner={state['owner']} z={state['zIndex']}")

    def s04():
        nonlocal probe
        main = need_main()
        x, y, w, h = main.client()
        probe, pw = start_probe(x - 150, y + h // 2, 420, 260)
        # 点主窗口（标题栏空白处）：嵌入桌面的窗口被激活也不能盖到探针之上
        W.click(*main.pt(60, 47))
        W.park_cursor()
        time.sleep(0.8)
        c = Checks()
        ov = main.pt(40, (h / main.scale) * 0.5 + 60)
        hit = W.window_from_point(*ov)
        c.ok(hit == pw.hwnd, f"重叠处命中探针窗口（命中 {W.describe_hwnd(hit)}）")
        zm, zp = W.z_index(main.hwnd), W.z_index(pw.hwnd)
        c.ok(0 <= zm < zp, f"Z 序：主窗口 {zm} 在探针 {zp} 之下")
        img = W.grab((x - 160, y - 10, x + w + 10, y + h + 10))
        img.save(C.out / "S04-embedded-under-probe.png")
        return c.done(f"z main={zm} probe={zp}")

    def s05():
        main = need_main()
        if not C.explorer:
            raise Skip("没有 Explorer（无桌面宿主），Win+D 不适用")
        targets = [main]
        pw = (all_windows_titled(PROBE_TITLE) or [None])[0]
        if pw:
            targets.append(pw)
        track = watch_shell_action(win_d, targets)
        time.sleep(0.5)
        c = Checks()
        c.ok(not track[main.hwnd]["iconic"] and not track[main.hwnd]["hidden"], f"Win+D 期间主窗口未最小化 / 隐藏（{track[main.hwnd]}）")
        c.ok(main.visible and not main.iconic, "Win+D 后主窗口可见")
        hit = main.center_hit()
        c.ok(hit == main.hwnd, f"Win+D 后主窗口中心命中自己（{W.describe_hwnd(hit)}）")
        if pw:
            print(f"    probe during Win+D: {track[pw.hwnd]}")
        x, y, w, h = main.client()
        W.grab((x - 160, y - 10, x + w + 10, y + h + 10)).save(C.out / "S05-after-win-d.png")
        win_d()
        time.sleep(1)
        return c.done()

    def s06():
        main = need_main()
        if not C.explorer:
            raise Skip("没有 Explorer")
        stop_proc(probe)
        mark = app.log_size()
        old, new, secs = restart_explorer()
        c = Checks()
        c.ok(main.alive, "Explorer 重启后主窗口还在")
        try:
            t0 = time.time()
            W.wait_until(lambda: main.owner == W.shell_window() != 0, 10, "重新挂到新的桌面宿主")
            c.ok(True, f"{time.time() - t0:.1f}s 内重挂（宿主 0x{old:X} → 0x{new:X}）")
        except W.Timeout as e:
            c.ok(False, f"{e}；owner=0x{main.owner:X} shell=0x{W.shell_window():X}")
        f = main.flags()
        c.ok(f["toolWindow"] and not f["minimizeBox"], f"样式保持（{f}）")
        c.ok(main.visible, "主窗口可见")
        print("    log: " + " / ".join(l for l in app.log_since(mark).splitlines() if "[embed]" in l or "[window]" in l)[:600])
        return c.done(f"explorer back in {secs:.1f}s")

    def s07():
        main = need_main()
        stop_proc(probe)
        W.click(*title_button(main, 2))
        W.park_cursor()
        c = Checks()
        try:
            W.wait_until(lambda: main.owner == 0 and not main.flags()["toolWindow"], 6, "回到普通模式")
        except W.Timeout as e:
            c.ok(False, str(e))
        time.sleep(1.2)
        f = main.flags()
        print(f"    state={main.state()}")
        c.eq(main.owner, 0, "owner 清除")
        c.ok(f["appWindow"] and not f["toolWindow"], f"APPWINDOW（{f}）")
        c.ok(f["minimizeBox"], "最小化按钮恢复")
        check_no_caption_buttons(main, c, "回到普通模式")
        c.eq(app.q1("SELECT is_fixed FROM screen_configs ORDER BY updated_at DESC LIMIT 1"), 0, "screen_configs.is_fixed")
        ht = main.nchittest(*main.pt(60, 47))
        c.eq(W.HT_NAMES.get(ht, ht), "HTCAPTION", "标题栏又能拖动")
        W.park_cursor()
        return c.done()

    def s08():
        main = need_main()
        if not C.explorer:
            raise Skip("没有 Explorer（没有任务栏）")
        f = main.flags()
        if main.owner or f["toolWindow"] or not f["appWindow"]:
            raise Skip(f"主窗口不在普通模式（owner=0x{main.owner:X} {f}），没有任务栏按钮")
        if main.iconic:
            main.show(W.SW_RESTORE)
            time.sleep(0.5)
        hits, out = find_taskbar_buttons()
        print(f"    taskbar-button.ps1: {' / '.join(out.splitlines())[:600]}")
        if not hits:
            if "notfound" not in out.splitlines():
                # 脚本本身出错（不是"没找到"）：记 FAIL，不要用 SKIP 盖住
                raise AssertionError(f"taskbar-button.ps1 运行失败：{' / '.join(out.splitlines()[-4:])[:600]}")
            raise Skip(f"任务栏上找不到主窗口的按钮（前缀 {TASKBAR_PREFIXES} / Appid {APP_ID}；UIA：{' / '.join(out.splitlines()[-2:])[:400]}）")
        # 多显示器且"在所有任务栏上显示按钮"时每条任务栏各有一个：用主任务栏上的
        primary = [h for h in hits if h[2] == "Shell_TrayWnd"] or hits
        if len(primary) > 1:
            raise Skip(f"同一条任务栏上有 {len(primary)} 个匹配的按钮，无法确定哪个是被测窗口（{[h[1] for h in primary]}）")
        rect, name, bar = primary[0]
        grouped = re.search(r"\s-\s(\d+)\s", name)
        if grouped and int(grouped.group(1)) > 1:
            raise Skip(f"任务栏按钮合并了 {grouped.group(1)} 个窗口（{name}：有安装版或其它实例的窗口在任务栏上？单击会弹出缩略图而不是最小化）")
        bx, by, bw, bh = rect
        px, py = bx + bw // 2, by + bh // 2
        if not any(l <= px < r and t <= py < b for l, t, r, b in (m["rect"] for m in W.monitors())):
            raise Skip(f"任务栏按钮不在屏幕上（{rect}；任务栏自动隐藏？）")
        # 任务栏只在按钮对应的窗口处于前台时把单击当作最小化：找到按钮之后、紧挨着单击之前点标题栏空白处激活主窗口
        if W.foreground_window() != main.hwnd:
            W.click(*main.pt(60, 47))
            W.park_cursor()
        try:
            W.wait_until(lambda: W.foreground_window() == main.hwnd, 5, "主窗口成为前台窗口")
        except W.Timeout as e:
            raise AssertionError(f"{e}（前台={W.describe_hwnd(W.foreground_window())}）")
        before = main.client()
        c = Checks()
        W.click(px, py)
        W.park_cursor()
        try:
            W.wait_until(lambda: main.iconic, 5, "单击任务栏按钮后主窗口最小化（IsIconic）")
            c.ok(True, "单击任务栏按钮：最小化")
        except W.Timeout as e:
            c.ok(False, f"{e}（前台={W.describe_hwnd(W.foreground_window())}；state={main.state()}）")
        if main.iconic:
            time.sleep(0.6)
            W.click(px, py)
            W.park_cursor()
            try:
                W.wait_until(lambda: not main.iconic and main.visible, 5, "再单击任务栏按钮后还原")
                c.ok(True, "再单击任务栏按钮：还原")
            except W.Timeout as e:
                c.ok(False, str(e))
            try:
                W.wait_until(lambda: W.foreground_window() == main.hwnd, 3, "还原后在前台")
                c.ok(True, "还原后是前台窗口")
            except W.Timeout as e:
                c.ok(False, f"{e}（前台={W.describe_hwnd(W.foreground_window())}）")
            time.sleep(0.5)
            after = main.client()
            c.ok(all(abs(a - b) <= 2 for a, b in zip(after, before)), f"还原后几何不变 {before} → {after}")
        if main.iconic:
            main.show(W.SW_RESTORE)
        check_no_caption_buttons(main, c, "最小化 / 还原之后")
        return c.done(f"{name}（{bar} {rect}）")

    try:
        C.report.case("S01", "普通模式：APPWINDOW / 无 owner / 无 WS_SYSMENU / 可拖动可缩放 / 位置落库 / 双击标题栏最大化还原", s01)
        C.report.case("S02", "经典固定：点锁按钮 → TOOLWINDOW / 不可拖动缩放 / 落库", s02)
        C.report.case("S03", "固定 + 嵌入桌面（重启恢复）：owner=桌面宿主 / TOOLWINDOW / 无最小化", s03)
        C.report.case("S04", "嵌入桌面的 Z 序：在普通应用窗口之下（探针窗口）", s04)
        C.report.case("S05", "嵌入桌面：Win+D 后仍可见", s05)
        C.report.case("S06", "嵌入桌面：Explorer 重启后自动重挂 owner", s06)
        C.report.case("S07", "嵌入桌面 → 点锁按钮回到普通模式", s07)
        C.report.case("S08", "普通模式：单击任务栏按钮最小化 → 再单击还原到前台", s08)
    finally:
        stop_proc(probe)
        app.stop()


# ----------------------------------------------------------------------------- N：应用内通知

def group_notifications() -> None:
    geo: dict = {}

    def n01():
        data = fresh_data("n-data")
        app = App(C.exe, data_dir=data, debug_open="main,notification:1,notification:2,notification:3", tag="N",
                  out_dir=C.out)
        app.start()
        try:
            app.main(60)
            W.wait_until(lambda: len(app.windows("通知")) >= 3, 30, "3 个通知窗口")
            time.sleep(2)
            wins = app.windows("通知")
            fg = W.foreground_window()
            m = W.primary_monitor()
            wl, wt_, wr, wb = m["work"]
            c = Checks()
            ys = []
            for n in wins:
                f = n.flags()
                x, y, w, h = n.client()
                ys.append((y, x, w, h))
                c.ok(f["toolWindow"] and f["topmost"], f"通知 0x{n.hwnd:X} TOOLWINDOW|TOPMOST（{f}）")
                c.ok(n.hwnd != fg, f"通知 0x{n.hwnd:X} 不抢焦点")
                check_no_caption_buttons(n, c, f"通知 0x{n.hwnd:X}")
            ys.sort(reverse=True)
            s = wins[0].scale
            y0, x0, w0, h0 = ys[0]
            print(f"    work={m['work']} clients={[(x, y, w, h) for y, x, w, h in ys]} rects={[n.rect() for n in wins]}")
            desc = f"work={m['work']} client=({x0},{y0},{w0}×{h0}) rects={[n.rect() for n in wins]}"
            geo.update(desc=desc, right=x0 + w0, bottom=y0 + h0, wr=wr, wb=wb, s=s)
            c.near(x0 + w0, wr - round(20 * s), 3, f"最下方通知客户区右边缘 = 工作区右 - 20（{desc}）")
            # 下边距单独在 N02 精确检查；这里只要求贴着工作区右下角（在工作区内、离底边不超过 30）
            c.ok(wb - 30 * s <= y0 + h0 <= wb, f"最下方通知贴近工作区底边（下边缘 {y0 + h0}，工作区底 {wb}）")
            c.near(round(w0 / s), 320, 2, "通知逻辑宽")
            c.near(round(h0 / s), 120, 2, "通知逻辑高")
            for i in range(1, len(ys)):
                c.near(ys[i - 1][0] - ys[i][0], h0 + round(10 * s), 3, f"第 {i + 1} 条叠在上一条之上（间距 10）")
                c.near(ys[i][1] + ys[i][2], x0 + w0, 2, f"第 {i + 1} 条右对齐")
            top = ys[-1][0]
            nimg = W.grab((x0 - 30, top - 10, wr, wb))
            nimg.save(C.out / "N01-notifications.png")
            maybe_emit("N01", nimg)
            return c.done(f"{len(wins)} 个通知，work={m['work']}，rects={[(x, y, w, h) for y, x, w, h in ys]}")
        except Exception:
            dump_log_tail(app)
            raise
        finally:
            app.stop()

    def n02():
        if not geo:
            raise Skip("N01 没有拿到通知窗口几何")
        expected = geo["wb"] - round(20 * geo["s"])
        off = expected - geo["bottom"]
        if abs(off) > 1:
            raise AssertionError(
                f"通知客户区下边缘 {geo['bottom']}，规格为工作区底 - 20 = {expected}（偏差 {off}px；{geo['desc']}）。"
                "GPUI(gpui-pre-windows 0.3.9) calculate_window_rect 把只在底部的不可见边框高度对半分到上下，"
                "按逻辑坐标建的窗口客户区整体比请求位置高 height_offset/2（96 DPI 下 4px）")
        return f"下边距 20px（{geo['desc']}）"

    C.report.case("N01", "应用内通知：TOOLWINDOW|TOPMOST、无 WS_SYSMENU、不抢焦点、右对齐、自下而上叠放（间距 10）", n01)
    C.report.case("N02", "应用内通知：客户区距工作区底边正好 20px", n02)


# ----------------------------------------------------------------------------- I：交互

def group_interactions() -> None:
    data = fresh_data("i-data")
    probe_app = App(C.exe, data_dir=data)
    # 系统通知：#4 提醒时间设为 2 分钟前、未通知；通知方式 system
    probe_app.write("UPDATE todos SET notify_at = ?, notified = 0, notify_before = 0 WHERE id = 4", (now_str(-2),))
    probe_app.write("INSERT INTO settings (key, value) VALUES ('notification_type', 'system') "
                    "ON CONFLICT(key) DO UPDATE SET value = 'system'")
    app = App(C.exe, data_dir=data, debug_open="main", tag="I", out_dir=C.out)
    st: dict = {"main": None}

    def main_win() -> Win:
        if not st["main"]:
            raise Skip("主窗口没有起来")
        return st["main"]

    def i00():
        app.start()
        st["main"] = app.main(60)
        W.park_cursor()
        time.sleep(3)
        rows = detect_rows(st["main"])
        ids = pending_ids(app)
        shot(st["main"], "I00-main")
        if len(rows) < min(len(ids), 6):
            raise AssertionError(f"列表行识别不足：rows={rows} ids={ids}")
        return f"rows={[round(r) for r in rows]} ids={ids}"

    def i01():
        main = main_win()
        before = W.grab_client(main)
        w, h = main.logical_size()
        W.click(*main.pt(w - 44, h - 44))
        ed = app.wait_window("新建待办", 15)
        time.sleep(1.2)
        shot(ed, "I01-new-editor")
        W.type_text("E2E created todo")
        W.key_combo("ctrl", "enter")
        c = Checks()
        try:
            app.wait_gone(ed, 10)
            c.ok(True, "保存后新建窗口关闭")
        except W.Timeout as e:
            c.ok(False, str(e))
            shot(ed, "I01-editor-after-save")
            W.key_combo("esc")
        new_id = app.q1("SELECT id FROM todos WHERE title = 'E2E created todo'")
        c.ok(new_id is not None, f"数据库里有新待办（id={new_id}）")
        W.park_cursor()
        time.sleep(1.5)
        after = shot(main, "I01-main-after")
        s = main.scale
        ratio = W.diff_ratio(before, after, (0, round(54 * s), after.width, after.height))
        c.ok(ratio > 0.002, f"主窗口列表刷新（差异像素 {ratio:.2%}）")
        return c.done(f"id={new_id}, diff={ratio:.2%}")

    def i02():
        main = main_win()
        y = row_y(app, main, 2)
        W.click(*main.pt(150, y + 3))
        ed = app.wait_window("待办详情", 15)
        time.sleep(1.5)
        img = shot(ed, "I02-detail-view")
        pos = find_blue_box(ed, (420, 6, 690, 52), EP_PRIMARY, 45, img)
        c = Checks()
        c.ok(pos is not None, f"只读详情标题栏有蓝色「编辑」按钮（{pos}）")
        W.click(*ed.pt(*(pos or (595, 28))))
        time.sleep(1.0)
        img2 = shot(ed, "I02-detail-edit")
        c.ok(find_blue_box(ed, (420, 6, 690, 52), EP_PRIMARY, 45, img2) is None, "切到编辑排版（「编辑」按钮消失）")
        W.click(*ed.pt(350, 119))
        W.key_combo("ctrl", "a")
        W.type_text("E2E renamed todo")
        W.key_combo("ctrl", "enter")
        try:
            app.wait_gone(ed, 10)
            c.ok(True, "保存后窗口关闭")
        except W.Timeout as e:
            c.ok(False, str(e))
            shot(ed, "I02-editor-after-save")
            W.key_combo("esc")
            time.sleep(0.5)
        c.eq(app.q1("SELECT title FROM todos WHERE id = 2"), "E2E renamed todo", "数据库标题")
        W.park_cursor()
        return c.done()

    def i03():
        main = main_win()
        time.sleep(1)
        y = row_y(app, main, 5)
        w, _ = main.logical_size()
        W.mouse_move(*main.pt(150, y + 3))
        time.sleep(0.7)
        img = shot(main, "I03-hover")
        W.click(*main.pt(w - 97, y + 3))
        c = Checks()
        try:
            W.wait_until(lambda: app.q1("SELECT completed FROM todos WHERE id = 5") == 1, 5, "#5 completed = 1")
            c.ok(True, "悬停操作条「完成」写库")
        except W.Timeout as e:
            c.ok(False, str(e))
            maybe_emit("I03", img)
        W.park_cursor()
        return c.done()

    def i04():
        main = main_win()
        time.sleep(1)
        y = row_y(app, main, 7)
        w, _ = main.logical_size()
        W.mouse_move(*main.pt(150, y + 3))
        time.sleep(0.7)
        W.click(*main.pt(w - 41, y + 3))
        time.sleep(1.0)
        img = shot(main, "I04-confirm")
        pos = find_blue_box(main, (220, 280, 378, 400), EP_PRIMARY, 45, img)
        c = Checks()
        c.ok(pos is not None, f"出现确认框（蓝色确认按钮 {pos}）")
        W.click(*main.pt(*(pos or (322, 336))))
        try:
            W.wait_until(lambda: app.q1("SELECT COUNT(*) FROM todos WHERE id = 7") == 0, 5, "#7 已删除")
            c.ok(True, "确认后删除写库")
        except W.Timeout as e:
            c.ok(False, str(e))
        W.park_cursor()
        return c.done()

    def open_settings(main: Win) -> Win:
        W.click(*title_button(main, 0))
        stw = app.wait_window("设置", 15)
        W.park_cursor()
        time.sleep(1.5)
        return stw

    def close_settings(stw: Win) -> None:
        W.click(*stw.pt(636, 28))
        try:
            app.wait_gone(stw, 6)
        except W.Timeout:
            W.key_combo("esc")
        time.sleep(1.0)

    def i05():
        main = main_win()
        light = W.grab_client(main)
        stw = open_settings(main)
        img = shot(stw, "I05-settings-general")
        sw = detect_switches(stw, img)
        print(f"    switches={[(round(y), on) for y, on in sw]}")
        c = Checks()
        if len(sw) < 6:
            maybe_emit("I05", img)
            raise AssertionError(f"常规页只识别到 {len(sw)} 个开关：{sw}")
        dark_y = sw[5][0]
        W.click(*stw.pt(646, dark_y))
        try:
            W.wait_until(lambda: app.setting("text_theme") == "light", 5, "settings.text_theme = light（深色）")
            c.ok(True, "深色主题写库（text_theme=light）")
        except W.Timeout as e:
            c.ok(False, f"{e}；text_theme={app.setting('text_theme')}")
        close_settings(stw)
        W.park_cursor()
        time.sleep(1.0)
        dark = shot(main, "I05-main-dark")
        s = main.scale
        p_light = W.rgb_at(light, round(60 * s), round(585 * s))
        p_dark = W.rgb_at(dark, round(60 * s), round(585 * s))
        print(f"    main bg light={p_light} dark={p_dark} desktop={C.desktop_rgb}")
        c.ok(W.color_dist(p_light, (255, 255, 255)) <= 12, f"浅色主题底为白色（{p_light}）")
        c.ok(W.color_dist(p_dark, (255, 255, 255)) > 80, f"切换后主窗口底色变深（{p_dark}）")
        c.ok(is_transparent_blend(p_dark, C.desktop_rgb),
             f"深色半透明底透出桌面色（{p_dark} vs 桌面 {C.desktop_rgb}；DirectComposition 透明）")
        maybe_emit("I05", dark)
        return c.done(f"light={p_light} dark={p_dark}")

    def i06():
        main = main_win()
        stw = open_settings(main)
        img = shot(stw, "I06-settings-general")
        sw = detect_switches(stw, img)
        if not sw:
            raise AssertionError("常规页没有识别到开关")
        auto_y = sw[0][0]
        c = Checks()
        before = run_value()
        print(f"    Run\\mini-todo before={before!r}")
        W.click(*stw.pt(646, auto_y))
        try:
            v = W.wait_until(run_value, 6, "HKCU\\...\\Run\\mini-todo 出现")
            exe = str(C.exe.resolve()).lower()
            c.ok(exe in v.lower() and "--autostart" in v, f"注册表值指向本 exe 且带 --autostart（{v}）")
        except W.Timeout as e:
            c.ok(False, str(e))
        time.sleep(1.5)
        W.click(*stw.pt(646, auto_y))
        try:
            W.wait_until(lambda: run_value() is None, 6, "Run\\mini-todo 删除")
            c.ok(True, "再次点击后注册表值删除")
        except W.Timeout as e:
            c.ok(False, str(e))
        close_settings(stw)
        return c.done()

    def i07():
        main_win()
        c = Checks()
        c.ok("托盘已创建" in app.log_text(), "日志：托盘已创建")
        trays = app.windows(cls="tray_icon_app", visible=False)
        c.ok(bool(trays), f"进程有 tray_icon_app 窗口（{trays}）")
        res = []
        for t in trays:
            for uid in (1, 2, 3):
                hr, rect = W.tray_icon_rect(t.hwnd, uid)
                res.append((uid, f"0x{hr:08X}", rect))
                if hr == 0:
                    break
        print(f"    Shell_NotifyIconGetRect: {res}")
        c.ok(any(r[1] == "0x00000000" for r in res), f"Shell_NotifyIconGetRect 找到图标（{res}）")
        return c.done(str(res))

    def i08():
        main_win()
        remain = max(5, 95 - (time.time() - app.started_at))
        c = Checks()
        try:
            W.wait_until(lambda: app.q1("SELECT notified FROM todos WHERE id = 4") == 1, remain, "#4 notified = 1")
            c.ok(True, "到期提醒已发出（notified=1）")
        except W.Timeout as e:
            c.ok(False, f"{e}；notify_at={app.q1('SELECT notify_at FROM todos WHERE id = 4')}")
        bad = [l for l in app.log_text().splitlines() if "提醒发送失败" in l or "通知检查失败" in l]
        c.ok(not bad, f"日志无发送失败（{bad[:2]}）")
        return c.done()

    # HKCU Run\mini-todo 与安装版共用：先记下原值，收尾时恢复成原样（原来没有就删、原来有就写回）。
    # 不能见值就删 —— 已开启开机自启的机器上（哪怕 --skip I06）会把安装版的自启项删掉
    run_before = run_value()
    try:
        C.report.case("I00", "交互准备：主窗口列表识别", i00)
        C.report.case("I01", "FAB → 新建窗口 → 输入标题 → Ctrl+Enter → 落库且主窗口刷新", i01)
        C.report.case("I02", "点击待办 → 只读详情 → 编辑 → 改标题 → Ctrl+Enter 保存", i02)
        C.report.case("I03", "悬停操作条：完成", i03)
        C.report.case("I04", "悬停操作条：删除 → 确认框 → 删除", i04)
        C.report.case("I05", "设置 → 深色主题：settings 落库、主窗口变为半透明深色", i05)
        C.report.case("I06", "设置 → 开机自启开 / 关：HKCU Run\\mini-todo 出现 / 消失", i06)
        C.report.case("I07", "托盘图标已注册（Shell_NotifyIconGetRect）", i07)
        C.report.case("I08", "系统通知：到期提醒发出、notified=1、无发送失败", i08)
    finally:
        if app.alive():
            errs = log_errors(app)
            if errs:
                print("  I 组日志 ERROR：\n  " + "\n  ".join(errs[:10]))
        app.stop()
        run_key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run"
        now = run_value()
        if run_before is None and now is not None:
            subprocess.run(["reg", "delete", run_key, "/v", "mini-todo", "/f"], capture_output=True)
        elif run_before is not None and now != run_before:
            subprocess.run(["reg", "add", run_key, "/v", "mini-todo", "/t", "REG_SZ", "/d", run_before, "/f"],
                           capture_output=True)
            print(f"  已恢复 Run\\mini-todo = {run_before!r}")


# ----------------------------------------------------------------------------- D：WebDAV

def port_open(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.5):
            return True
    except OSError:
        return False


def group_webdav() -> None:
    data = fresh_data("d-data")
    root = C.work / "dav-root"
    shutil.rmtree(root, ignore_errors=True)
    root.mkdir(parents=True)
    access = C.out / "dav-access.jsonl"
    access.unlink(missing_ok=True)
    port = 8765
    server = None
    app = App(C.exe, data_dir=data, debug_open="main", tag="D", out_dir=C.out)
    st: dict = {}

    def access_log() -> list[dict]:
        import json
        if not access.exists():
            return []
        return [json.loads(l) for l in access.read_text(encoding="utf-8").splitlines() if l.strip()]

    def d00():
        nonlocal server
        try:
            import wsgidav  # noqa: F401
            import cheroot  # noqa: F401
        except ImportError:
            raise Skip("没有安装 wsgidav / cheroot（pip install wsgidav cheroot）")
        server = subprocess.Popen([sys.executable, str(HERE / "dav_server.py"), "--root", str(root), "--port", str(port),
                                   "--user", DAV_USER, "--password", DAV_PASS, "--access-log", str(access)],
                                  stdout=open(C.out / "dav-server.txt", "wb"), stderr=subprocess.STDOUT)
        W.wait_until(lambda: port_open(port), 20, "WebDAV 端口就绪")
        app.start()
        main = app.main(60)
        st["main"] = main
        W.park_cursor()
        time.sleep(2.5)
        W.click(*title_button(main, 0))
        stw = app.wait_window("设置", 15)
        time.sleep(1.2)
        W.click(*stw.pt(75, 163))      # 左侧菜单「数据与同步」
        W.park_cursor()
        time.sleep(1.2)
        st["settings"] = stw
        maybe_emit("D00", shot(stw, "D00-settings-data"))
        return "WebDAV 服务与设置页就绪"

    def need_settings() -> Win:
        if "settings" not in st:
            raise Skip("设置窗口没有打开")
        return st["settings"]

    def d01():
        stw = need_settings()
        mid = lambda box: (box[0] + box[1]) / 2  # noqa: E731

        def fill(x: float, column: float, index: int, fallback_y: float, text: str) -> None:
            # 每次都重新识别：输入 http:// 地址后表单会多出一行明文提示，下面的控件整体下移
            boxes = detect_boxes(stw, column, 240, 540)
            y = mid(boxes[index]) if len(boxes) > index else fallback_y
            print(f"    输入框 x={column}: {boxes} → 点 ({x}, {y})")
            W.click(*stw.pt(x, y))
            W.key_combo("ctrl", "a")
            W.type_text(text)
            W.park_cursor()
            time.sleep(0.5)

        fill(298, 300, 1, 375, DAV_USER)
        fill(530, 540, 1, 375, DAV_PASS)
        fill(420, 300, 0, 307, f"http://127.0.0.1:{port}")
        buttons = detect_boxes(stw, 300, 240, 560, border=SETTINGS_BTN_BORDER, tol=6, min_h=30, max_h=50)
        test_y = mid(buttons[0]) if buttons else 423
        print(f"    白色按钮 x=300: {buttons} → 测试连接 y={test_y}")
        st["save_y"] = test_y
        maybe_emit("D01", shot(stw, "D01-filled"))
        n0 = len(access_log())
        W.click(*stw.pt(300, test_y))       # 测试连接
        W.park_cursor()
        try:
            hits = W.wait_until(lambda: [r for r in access_log()[n0:] if r.get("user") == DAV_USER
                                         and r.get("status") in (200, 207)], 10, "服务器收到带认证的请求")
        except W.Timeout as e:
            shot(stw, "D01-fail")
            raise AssertionError(f"{e}；访问日志={access_log()[n0:][:5]}")
        shot(stw, "D01-test-connection")
        return f"{hits[0]['method']} {hits[0]['path']} → {hits[0]['status']}"

    def d02():
        stw = need_settings()
        W.click(*stw.pt(540, st.get("save_y", 423)))       # 保存配置（与「测试连接」同一行）
        W.park_cursor()
        c = Checks()
        try:
            W.wait_until(lambda: (app.setting("webdav_password") or "").startswith("dpapi:"), 8,
                         "webdav_password 以 dpapi: 开头")
            c.ok(True, "密码以 DPAPI 密文保存")
        except W.Timeout as e:
            c.ok(False, f"{e}；值前缀={str(app.setting('webdav_password'))[:12]!r}")
        url = app.setting("webdav_url") or ""
        c.ok(url.rstrip("/") == f"http://127.0.0.1:{port}", f"webdav_url 已保存（{url!r}）")
        c.eq(app.setting("webdav_username"), DAV_USER, "webdav_username")
        leaks = []
        for p in data.glob("data.db*"):
            raw = p.read_bytes()
            if DAV_PASS.encode() in raw or DAV_PASS.encode("utf-16-le") in raw:
                leaks.append(p.name)
        c.ok(not leaks, f"数据库文件里没有明文密码（{leaks}）")
        return c.done()

    def d03():
        stw = need_settings()
        mark = app.log_size()
        W.wheel(*stw.pt(420, 300), -12)
        W.park_cursor()
        time.sleep(1.0)
        img = shot(stw, "D03-scrolled")
        s = stw.scale
        bands = W.column_runs(img, round(300 * s), round(300 * s), img.height,
                              lambda c: W.color_dist(c, APP_PRIMARY) <= 40, min_len=round(20 * s))
        print(f"    blue bands at x=300: {[(round(a / s), round(b / s)) for a, b in bands]}")
        # 只认下半部的蓝条：没滚动时上方的蓝条是「导出数据」（会弹系统文件对话框）
        bands = [(a, b) for a, b in bands if (a + b) / 2 / s > 330]
        if not bands:
            maybe_emit("D03", img)
            raise AssertionError("滚到底后没找到「立即同步」按钮（蓝色条）")
        a, b = bands[-1]
        W.click(*stw.pt(300, (a + b) / 2 / s))
        W.park_cursor()
        c = Checks()
        try:
            f = W.wait_until(lambda: list(root.rglob("sync-data.json.gz")), 30, "服务器上出现 sync-data.json.gz")
            c.ok(True, f"服务器文件 {f[0].relative_to(root)}（{f[0].stat().st_size} 字节）")
        except W.Timeout as e:
            c.ok(False, str(e))
        try:
            W.wait_until(lambda: app.setting("webdav_last_sync_at"), 10, "webdav_last_sync_at 更新")
            c.ok(True, f"last_sync_at={app.setting('webdav_last_sync_at')}")
        except W.Timeout as e:
            c.ok(False, str(e))
        lines = [l for l in app.log_since(mark).splitlines() if "[sync]" in l]
        c.ok(any("同步完成" in l for l in lines), f"日志：同步完成（{lines[-2:]}）")
        return c.done()

    def d04():
        main = st.get("main")
        if not main:
            raise Skip("主窗口没有起来")
        stw = st.get("settings")
        if stw and stw.alive:
            W.click(*stw.pt(636, 28))
            time.sleep(1.2)
        mark = app.log_size()
        W.click(*title_button(main, 1))
        W.park_cursor()
        line = W.wait_until(lambda: next((l for l in app.log_since(mark).splitlines() if "[sync] 同步" in l), None), 30,
                            "标题栏同步按钮触发同步")
        if "同步完成" not in line:
            raise AssertionError(line)
        return line.split("]")[-1].strip()

    try:
        C.report.case("D00", "WebDAV 准备：本地 wsgidav（Basic 认证）+ 设置 → 数据与同步", d00)
        C.report.case("D01", "测试连接：服务器收到带认证的 PROPFIND", d01)
        C.report.case("D02", "保存配置：密码以 DPAPI 密文落库、库文件无明文", d02)
        C.report.case("D03", "立即同步：服务器出现 sync-data.json.gz、last_sync_at 更新", d03)
        C.report.case("D04", "标题栏同步按钮：同步完成", d04)
    finally:
        app.stop()
        stop_proc(server)


# ----------------------------------------------------------------------------- F：桌面收纳箱

def group_fences() -> None:
    data = fresh_data("f-data")
    froot = C.work / "fences"
    shutil.rmtree(froot, ignore_errors=True)
    froot.mkdir(parents=True)
    app = App(C.exe, data_dir=data, fences_root=froot, debug_open="main,fence:new", tag="F", out_dir=C.out)
    st: dict = {}
    drag_proc = None

    def folder() -> Path:
        name = app.q1("SELECT folder_name FROM fences ORDER BY id LIMIT 1")
        if not name:
            raise AssertionError("fences 表里没有记录")
        return froot / name

    def need_fence() -> Win:
        if "fence" not in st:
            raise Skip("收纳箱窗口没有起来")
        return st["fence"]

    def f01():
        app.start()
        app.main(60)
        fence = W.wait_until(lambda: next((w for w in app.windows() if w.title not in (MAIN, "")), None), 30,
                             "收纳箱窗口出现")
        st["fence"] = fence
        c = Checks()
        try:
            W.wait_until(lambda: fence.owner and fence.owner == W.shell_window(), 8, "收纳箱 owner = 桌面宿主")
        except W.Timeout as e:
            c.ok(False, str(e))
        W.park_cursor()
        time.sleep(1.5)
        f = fence.flags()
        state = fence.state()
        print(f"    state={state}")
        c.ok(state["ownerIsShell"], f"owner 是桌面宿主（{state['owner']} / {state['shell']}）")
        c.ok(f["toolWindow"] and not f["appWindow"], f"TOOLWINDOW 且非 APPWINDOW（{f}）")
        c.ok(not f["minimizeBox"], "没有最小化按钮")
        check_no_caption_buttons(fence, c)
        rows = app.q("SELECT id, title, folder_name FROM fences")
        c.ok(len(rows) == 1, f"fences 表 1 条（{rows}）")
        if rows:
            c.ok((froot / rows[0][2]).is_dir(), f"收纳文件夹已建在 MINI_TODO_FENCES_ROOT 下（{rows[0][2]}）")
            c.eq(fence.title, rows[0][1], "窗口标题 = 收纳箱标题")
        lw, lh = fence.logical_size()
        c.ok(lw >= 150 and lh >= 100, f"展开尺寸合理（{lw:.0f}×{lh:.0f}）")
        m = W.primary_monitor()
        x, y, w, h = fence.client()
        print(f"    fence client={fence.client()} monitor work={m['work']}")
        img = shot(fence, "F01-fence-new")
        maybe_emit("F01", img)
        return c.done(f"{fence.title} {lw:.0f}×{lh:.0f} at ({x},{y})")

    def f02():
        fence = need_fence()
        d = folder()
        before = W.grab_client(fence)
        (d / "e2e-note.txt").write_text("hello from e2e\n", encoding="utf-8")
        c = Checks()
        try:
            W.wait_until(lambda: W.diff_ratio(before, W.grab_client(fence)) > 0.01, 10, "收纳箱界面出现新图标")
            c.ok(True, "目录监听刷新了收纳箱窗口")
        except W.Timeout as e:
            c.ok(False, str(e))
        time.sleep(1)
        img = shot(fence, "F02-fence-with-file")
        items = app.q("SELECT file_name FROM fence_items")
        print(f"    fence_items={items}")
        maybe_emit("F02", img)
        return c.done(f"fence_items={items}")

    def f03():
        fence = need_fence()
        old = folder()
        W.dblclick(*fence.pt(30, 16))
        time.sleep(0.6)
        W.type_text("E2E Box")
        W.key_combo("enter")
        W.park_cursor()
        c = Checks()
        try:
            W.wait_until(lambda: app.q1("SELECT title FROM fences ORDER BY id LIMIT 1") == "E2E Box", 6,
                         "fences.title = E2E Box")
            c.ok(True, "标题落库")
        except W.Timeout as e:
            c.ok(False, f"{e}；title={app.q1('SELECT title FROM fences')}")
        try:
            W.wait_until(lambda: folder().name == "E2E Box" and folder().is_dir() and not old.exists(), 6,
                         "收纳文件夹改名为 E2E Box")
            c.ok(True, f"文件夹 {old.name} → {folder().name}")
        except (W.Timeout, AssertionError) as e:
            c.ok(False, f"{e}；目录={[p.name for p in froot.iterdir()]}")
        c.eq(fence.title, "E2E Box", "窗口标题")
        shot(fence, "F03-renamed")
        return c.done()

    def drag_file_into(fence: Win, mode: str, name: str) -> tuple[bool, str]:
        """用拖放源窗口（mode=shell / winforms）把一个新文件真实拖到收纳箱上；返回（是否移入, 诊断）"""
        nonlocal drag_proc
        stop_proc(drag_proc)
        src_dir = C.work / f"drag-src-{mode}"
        shutil.rmtree(src_dir, ignore_errors=True)
        src_dir.mkdir(parents=True)
        src = src_dir / name
        src.write_text(f"dragged into a fence ({mode})\n", encoding="utf-8")
        result = C.out / f"drag-result-{mode}.txt"
        result.unlink(missing_ok=True)
        m = W.primary_monitor()
        # 放在左上角：拖动路径从主窗口上方经过、不进入主窗口（主窗口居中、顶边约 236）
        sx, sy = m["work"][0] + 60, m["work"][1] + 40
        drag_proc = subprocess.Popen(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-STA", "-File",
                                      str(HERE / "drag-source.ps1"), "-Path", str(src), "-Mode", mode,
                                      "-Title", DRAG_TITLE, "-X", str(sx), "-Y", str(sy), "-W", "220", "-H", "120",
                                      "-ResultFile", str(result)],
                                     creationflags=W.CREATE_NO_WINDOW)
        dsrc = W.wait_until(lambda: (all_windows_titled(DRAG_TITLE) or [None])[0], 30, "拖放源窗口出现")
        time.sleep(0.8)
        mark = app.log_size()
        x, y, w, h = dsrc.client()
        fx, fy, fw, fh = fence.client()
        W.drag(x + w // 2, y + h // 2, fx + fw // 2, fy + fh // 2, steps=40, hold=0.4, step_delay=0.04, settle=1.0)
        W.park_cursor()
        moved = False
        try:
            W.wait_until(lambda: (folder() / name).exists() and not src.exists(), 10, "文件被移入收纳文件夹")
            moved = True
        except (W.Timeout, AssertionError):
            pass
        res = result.read_text(encoding="utf-8", errors="replace").strip() if result.exists() else "(无结果)"
        logs = [l.split("]", 3)[-1].strip() for l in app.log_since(mark).splitlines()][-6:]
        diag = f"{mode}: 移入={moved} 源存在={src.exists()} 拖放源={res} 收纳夹={[p.name for p in folder().iterdir()]} 日志={logs}"
        print(f"    {diag}")
        stop_proc(drag_proc)
        return moved, diag

    def f04():
        # 新实例里的第一次外部拖入；不成功时立刻原样再拖一次，区分"拖入整体不可用"与"首次拖入丢失"
        fence = need_fence()
        moved, diag = drag_file_into(fence, "winforms", "e2e-drop-1.txt")
        img = shot(fence, "F04-after-first-drop")
        if moved:
            return f"第一次拖入成功：{diag}"
        maybe_emit("F04", img)
        moved2, diag2 = drag_file_into(fence, "winforms", "e2e-drop-2.txt")
        moved3, diag3 = drag_file_into(fence, "winforms", "e2e-drop-3.txt")
        raise AssertionError(f"新实例里第一次拖入收纳箱没有生效（第二次 {'成功' if moved2 else '也失败'}，第三次 "
                             f"{'成功' if moved3 else '也失败'}）。第一次：{diag}；第二次：{diag2}；第三次：{diag3}")

    def f09():
        fence = need_fence()
        moved, diag = drag_file_into(fence, "shell", "e2e-drop-shell.txt")
        img = shot(fence, "F09-after-drop")
        maybe_emit("F09", img)
        if not moved:
            raise AssertionError(diag)
        return diag

    def f08():
        """资源管理器作拖放源：打开文件夹窗口，用 UI Automation 找到列表项，真实鼠标拖到收纳箱"""
        fence = need_fence()
        src_dir = C.work / "explorer-src"
        shutil.rmtree(src_dir, ignore_errors=True)
        name = "e2e-explorer-folder"
        (src_dir / name).mkdir(parents=True)
        (src_dir / name / "inside.txt").write_text("from explorer\n", encoding="utf-8")
        before = {w.hwnd for w in all_windows_of_class("CabinetWClass")}
        subprocess.Popen(["explorer.exe", str(src_dir)])
        ex = W.wait_until(lambda: next((w for w in all_windows_of_class("CabinetWClass") if w.hwnd not in before), None),
                          30, "资源管理器窗口出现")
        try:
            m = W.primary_monitor()
            ex.set_rect(m["work"][0] + 30, m["work"][1] + 30, 760, 480)
            time.sleep(1.0)
            ex_x, ex_y, ex_w, _ = ex.client()
            W.click(ex_x + ex_w // 2, ex_y + 12)      # 点标题栏，把资源管理器带到前台
            time.sleep(1.5)
            out = W.run_ps(f"& '{HERE / 'uia-find.ps1'}' -Name '{name}' -Hwnd {ex.hwnd} -TimeoutSec 15",
                           timeout=120).splitlines()
            found = next((l for l in reversed(out) if l.count(",") == 3 and "items" not in l), None)
            print(f"    资源管理器 {ex}，UIA: {' / '.join(out)[:300]}")
            if not found:
                raise AssertionError(f"资源管理器窗口里找不到 {name}（UIA：{out[-3:]}）")
            ix, iy, iw, ih = (int(v) for v in found.split(","))
            fx, fy, fw, fh = fence.client()
            mark = app.log_size()
            W.drag(ix + min(40, iw // 2), iy + ih // 2, fx + fw // 2, fy + fh // 2, steps=40, hold=0.5,
                   step_delay=0.04, settle=1.0)
            W.park_cursor()
            c = Checks()
            try:
                W.wait_until(lambda: (folder() / name).is_dir() and not (src_dir / name).exists(), 12,
                             "资源管理器里的文件夹被移进收纳文件夹")
                c.ok(True, f"已移入 {folder().name}")
            except (W.Timeout, AssertionError) as e:
                logs = [l.split("]", 3)[-1].strip() for l in app.log_since(mark).splitlines()][-6:]
                c.ok(False, f"{e}；源仍在={(src_dir / name).exists()} 收纳夹={[p.name for p in folder().iterdir()]} 日志={logs}")
            maybe_emit("F08", shot(fence, "F08-after-explorer-drop"))
            return c.done()
        finally:
            ex.post_close()
            time.sleep(1)

    def drag_after(action, key: str, label: str) -> str:
        """先做 action（改名 / 单击标题），紧接着拖入一次；没生效再原样拖一次，区分「只丢第一次」与「一直不行」"""
        fence = need_fence()
        action(fence)
        moved, diag = drag_file_into(fence, "winforms", f"e2e-after-{key}-1.txt")
        if moved:
            return f"{label}后第一次拖入成功：{diag}"
        W.save_emit(shot(fence, f"F-after-{key}"), C.out, f"F-after-{key}")
        moved2, diag2 = drag_file_into(fence, "winforms", f"e2e-after-{key}-2.txt")
        raise AssertionError(f"{label}后的第一次外部拖入没有生效（文件没被移入、虚线悬停框残留），紧接着再拖一次"
                             f"{'成功' if moved2 else '也失败'}。第一次：{diag}；第二次：{diag2}")

    def rename_again(fence: Win) -> None:
        W.dblclick(*fence.pt(30, 16))
        time.sleep(0.6)
        W.type_text("E2E Box 2")
        W.key_combo("enter")
        W.park_cursor()
        W.wait_until(lambda: app.q1("SELECT title FROM fences ORDER BY id LIMIT 1") == "E2E Box 2", 6, "改名落库")
        time.sleep(1)

    def click_title(fence: Win) -> None:
        W.click(*fence.pt(120, 16))
        W.park_cursor()
        time.sleep(1)

    def f10():
        return drag_after(rename_again, "rename", "双击标题改名")

    def f11():
        return drag_after(click_title, "click", "单击标题栏")

    def f07():
        fence = need_fence()
        row = app.q("SELECT x, y, width, height FROM fences ORDER BY id LIMIT 1")
        if not row:
            raise AssertionError("fences 表里没有记录")
        x, y, w, h = row[0]
        cx, cy, cw, ch = fence.client()
        rect = fence.rect()
        desc = f"数据库几何=({x},{y},{w}×{h}) 客户区=({cx},{cy},{cw}×{ch}) 窗口外框={rect}"
        c = Checks()
        c.ok(abs(cw - w) <= 1 and abs(ch - h) <= 1, f"可见（客户区）尺寸 = 记录的尺寸（{desc}）")
        c.ok(abs(cx - x) <= 1 and abs(cy - y) <= 1, f"可见（客户区）位置 = 记录的位置（{desc}）")
        return c.done(desc)

    def f05():
        fence = need_fence()
        stop_proc(drag_proc)
        if not C.explorer:
            raise Skip("没有 Explorer")
        track = watch_shell_action(win_d, [fence])
        time.sleep(0.5)
        c = Checks()
        c.ok(not track[fence.hwnd]["iconic"] and not track[fence.hwnd]["hidden"], f"Win+D 期间未最小化 / 隐藏（{track[fence.hwnd]}）")
        c.ok(fence.visible and not fence.iconic, "Win+D 后可见")
        hit = fence.center_hit()
        c.ok(hit == fence.hwnd, f"Win+D 后中心命中收纳箱（{W.describe_hwnd(hit)}）")
        win_d()
        time.sleep(1)
        return c.done()

    def f06():
        fence = need_fence()
        if not C.explorer:
            raise Skip("没有 Explorer")
        old, new, secs = restart_explorer()
        c = Checks()
        try:
            W.wait_until(lambda: fence.owner == W.shell_window() != 0, 10, "收纳箱重挂到新的桌面宿主")
            c.ok(True, "已重挂")
        except W.Timeout as e:
            c.ok(False, f"{e}；owner=0x{fence.owner:X} shell=0x{W.shell_window():X}")
        c.ok(fence.visible and fence.flags()["toolWindow"], "仍可见且 TOOLWINDOW")
        return c.done(f"explorer back in {secs:.1f}s")

    try:
        C.report.case("F01", "新增收纳箱：窗口嵌入桌面（owner=桌面宿主 / TOOLWINDOW / 无 WS_SYSMENU）、收纳文件夹已创建", f01)
        C.report.case("F02", "往收纳文件夹放文件 → 监听刷新出图标", f02)
        C.report.case("F04", "真实 OLE 拖放（.NET DataObject）：新实例里第一次拖入收纳箱即生效", f04)
        C.report.case("F03", "双击标题改名 → 收纳文件夹同步改名", f03)
        C.report.case("F10", "双击标题改名之后，紧接着的第一次外部拖入生效", f10)
        C.report.case("F11", "单击收纳箱标题栏之后，紧接着的第一次外部拖入生效", f11)
        C.report.case("F09", "真实 OLE 拖放（Shell 文件夹的数据对象）：文件被移进收纳文件夹", f09)
        C.report.case("F08", "从资源管理器窗口把文件夹真实拖进收纳箱（资源管理器作拖放源）", f08)
        C.report.case("F07", "收纳箱可见区域与数据库记录的物理像素几何一致", f07)
        C.report.case("F05", "收纳箱：Win+D 后仍可见", f05)
        C.report.case("F06", "收纳箱：Explorer 重启后自动重挂", f06)
    finally:
        stop_proc(drag_proc)
        if app.alive():
            dump_log_tail(app, 15)
        app.stop()


def group_fence_geometry() -> None:
    """收纳箱移动 / 缩放（自绘拖动）与吸附对齐：库里预置两个收纳箱 A（目标）、B（被拖），重启恢复出两个窗口。
    几何一律是客户区的物理像素；阈值 = round(10 × 缩放)"""
    data = fresh_data("fg-data")
    froot = C.work / "fences-geometry"
    shutil.rmtree(froot, ignore_errors=True)
    froot.mkdir(parents=True)
    app = App(C.exe, data_dir=data, fences_root=froot, tag="FG", out_dir=C.out)
    st: dict = {}
    # 主显示器工作区右侧：A 在右上，B 在 A 左下方（两个方向上都离 A 的边很远，不会误吸）
    l, t, r, b = W.primary_monitor()["work"]
    a_rect = (r - 40 - 420, t + 80, 420, 320)
    b_rect = (a_rect[0] - 400, a_rect[1] + 400, 320, 260)
    for title, (x, y, w, h), order in (("Snap A", a_rect, 0), ("Snap B", b_rect, 1)):
        app.write("INSERT INTO fences (title, folder_name, x, y, width, height, sort_order, created_at, updated_at) "
                  "VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now', 'localtime'), datetime('now', 'localtime'))",
                  (title, title, x, y, w, h, order))
        (froot / title).mkdir(parents=True, exist_ok=True)

    def need() -> tuple[Win, Win]:
        if "a" not in st:
            raise Skip("收纳箱窗口没有起来")
        return st["a"], st["b"]

    def b_row() -> tuple:
        return app.q("SELECT x, y, width, height FROM fences WHERE title = 'Snap B'")[0]

    def b_layouts() -> list[tuple]:
        return app.q("SELECT l.x, l.y, l.width, l.height FROM fence_layouts l JOIN fences f ON f.id = l.fence_id "
                     "WHERE f.title = 'Snap B'")

    def wait_committed(fence: Win, c: Checks, what: str) -> None:
        """松手后 fences 与当前组合的 fence_layouts 都 = 窗口客户区"""
        want = fence.client()
        try:
            W.wait_until(lambda: b_row() == want and want in b_layouts(), 6, f"{what}落库 = 客户区 {want}")
            c.ok(True, f"{what}落库：fences = fence_layouts = {want}")
        except W.Timeout as e:
            c.ok(False, f"{e}；fences={b_row()} fence_layouts={b_layouts()}")

    def held_drag(p0: tuple[int, int], p1: tuple[int, int], *, alt_midway: bool = False, while_held=None,
                  steps: int = 24) -> object:
        """左键按下 → 分步拖到 p1（alt_midway：过半时按下 Alt，松开鼠标后再松 Alt）→ 停住时调 while_held → 抬起"""
        W.mouse_move(*p0)
        time.sleep(0.15)
        W._send(W._mouse_input(W.MOUSEEVENTF_LEFTDOWN))
        time.sleep(0.15)
        alt_down = False
        held = None
        try:
            for i in range(1, steps + 1):
                if alt_midway and not alt_down and i > steps // 2:
                    W._send(W._key_input(W.VK["alt"], False))
                    alt_down = True
                    time.sleep(0.05)
                W.mouse_move(round(p0[0] + (p1[0] - p0[0]) * i / steps), round(p0[1] + (p1[1] - p0[1]) * i / steps))
                time.sleep(0.03)
            time.sleep(0.4)
            if while_held:
                held = while_held()
        finally:
            W._send(W._mouse_input(W.MOUSEEVENTF_LEFTUP))
            time.sleep(0.1)
            if alt_down:
                W._send(W._key_input(W.VK["alt"], True))
            time.sleep(0.5)
        return held

    def edge_blue(fence: Win, edge: str) -> bool:
        """收纳箱窗口内某条边上（中点附近、往里 3 个逻辑像素以内）有没有吸附高亮色 #409EFF"""
        img = W.grab_client(fence)
        depth = max(2, round(3 * fence.scale)) + 1
        if edge in ("top", "bottom"):
            x = img.width // 2
            rows = range(depth) if edge == "top" else range(img.height - depth, img.height)
            px = [W.rgb_at(img, x, y) for y in rows]
        else:
            y = img.height // 2
            cols = range(depth) if edge == "left" else range(img.width - depth, img.width)
            px = [W.rgb_at(img, x, y) for x in cols]
        return any(W.color_dist(p, EP_PRIMARY) <= 48 for p in px)

    def f12():
        app.start()
        main = app.main(60)
        a = app.wait_window("Snap A", 30)
        b = app.wait_window("Snap B", 30)
        st["a"], st["b"] = a, b
        # 主窗口（普通窗口）盖在桌面层的收纳箱之上，真实输入可能点到它：先最小化
        main.show(W.SW_MINIMIZE)
        W.wait_until(lambda: a.owner and a.owner == W.shell_window() and b.owner == a.owner, 10, "两个收纳箱都已嵌入桌面")
        W.park_cursor()
        time.sleep(1.5)
        c = Checks()
        for name, win, rect in (("A", a, a_rect), ("B", b, b_rect)):
            got = win.client()
            c.ok(all(abs(g - w) <= 1 for g, w in zip(got, rect)), f"{name} 按库里的几何恢复（{got}，库 {rect}）")
        before = b.client()
        p0 = b.pt(40, 16)
        dx, dy = -100, -60
        held_drag(p0, (p0[0] + dx, p0[1] + dy))
        W.park_cursor()
        after = b.client()
        # 离 A 的边都远：不吸附，位移 = 光标位移，尺寸不变
        c.eq(after, (before[0] + dx, before[1] + dy, before[2], before[3]), f"拖标题栏：窗口跟手移动 {before[:2]} → {after[:2]}")
        wait_committed(b, c, "拖动结束")
        shot(b, "F12-moved")
        return c.done(f"B {before} → {after}")

    def f13():
        a, b = need()
        c = Checks()
        ax, ay, aw, ah = a.client()
        bx, by, _, _ = b.client()
        s = b.scale
        threshold = int(10 * s + 0.5)
        off = round(6 * s)
        # 只改纵向：让 B.top 落在 A.top + 6（逻辑像素）——在阈值内；横向离 A 的左右边都远
        p0 = b.pt(40, 16)
        target_y = ay + off
        held = held_drag(p0, (p0[0], p0[1] + (target_y - by)),
                         while_held=lambda: (edge_blue(a, "top"), edge_blue(a, "left"), b.client()))
        top_lit, left_lit, during = held
        after = b.client()
        print(f"    阈值 {threshold}px；A={a.client()} B 拖动中={during} 松手后={after}")
        c.eq(during[1], ay, f"拖动中 B.top 吸到 A.top（跟手应为 {target_y}）")
        c.eq(after[1], ay, "松手后 B.top = A.top")
        c.eq(after[0], bx, "横向没有吸附（x 不变）")
        c.ok(top_lit, "拖动中 A 的上边画出高亮线 #409EFF")
        c.ok(not left_lit, "A 没对齐的边（左边）不高亮")
        time.sleep(0.3)
        c.ok(not edge_blue(a, "top"), "松手后高亮消失")
        wait_committed(b, c, "吸附后的位置")
        shot(a, "F13-target")
        return c.done(f"B.top {by} → {after[1]}（A.top {ay}）")

    def f14():
        a, b = need()
        c = Checks()
        _, ay, _, _ = a.client()
        bx, by, _, _ = b.client()
        off = round(8 * b.scale)
        # 拖到一半按住 Alt：B.top 落在 A.top + 8（逻辑像素），本该吸附的位置不吸附、不高亮。顺带横向挪 40px（离 A 的
        # 左右边仍远），手势够长：GPUI 的拖动在移动超过 2px 后才开始、窗口超过 3px 才动
        p0 = b.pt(40, 16)
        lit = held_drag(p0, (p0[0] - 40, p0[1] + (ay + off - by)), alt_midway=True,
                        while_held=lambda: edge_blue(a, "top"))
        after = b.client()
        c.eq(after[:2], (bx - 40, ay + off), f"按住 Alt 不吸附：B.top = A.top + {off}")
        c.ok(not lit, "按住 Alt 时 A 不高亮")
        wait_committed(b, c, "按住 Alt 拖动的位置")
        # 松开 Alt 后（单按 Alt 不会进系统菜单模式）照常能拖、照常吸附：往上 3 个逻辑像素，仍在阈值内
        p0 = b.pt(40, 16)
        held_drag(p0, (p0[0] + 40, p0[1] - round(3 * b.scale)))
        c.eq(b.client()[:2], (bx, ay), "松开 Alt 后再拖：照常吸到 A.top")
        wait_committed(b, c, "再次吸附的位置")
        return c.done()

    def f15():
        a, b = need()
        c = Checks()
        ax, ay, aw, ah = a.client()
        # B 先挪开到 A.top 下方 8 个逻辑像素（按住 Alt，不吸；横向 +40 让手势够长），再拖下边缩放
        off = round(8 * b.scale)
        y0 = b.client()[1]
        p0 = b.pt(40, 16)
        held_drag(p0, (p0[0] + 40, p0[1] + (ay + off - y0)), alt_midway=True)
        bx, by, bw, bh = b.client()
        c.eq(by, ay + off, "准备：B.top = A.top + 8（逻辑像素）")
        # 下边手柄（底部 5 个逻辑像素）：高度拖到 A.height + 4（逻辑像素）→ 等高吸附成 A.height；
        # 下边对齐 A.bottom 的候选差 12 个逻辑像素，超出阈值
        lw, lh = b.logical_size()
        p0 = b.pt(lw / 2, lh - 2.5)
        grow = (ah + round(4 * b.scale)) - bh
        lit = held_drag(p0, (p0[0], p0[1] + grow),
                        while_held=lambda: (edge_blue(a, "left"), edge_blue(a, "right"), edge_blue(a, "top")))
        after = b.client()
        print(f"    A={a.client()} B 缩放前=({bx}, {by}, {bw}, {bh}) 后={after} 高亮(左, 右, 上)={lit}")
        c.eq(after, (bx, by, bw, ah), "拖下边：高度吸成 A 的高度，上边与宽度不动")
        c.ok(lit[0] and lit[1], "等高：A 的左、右两边高亮")
        c.ok(not lit[2], "A 的上边不高亮")
        wait_committed(b, c, "缩放结束")
        shot(b, "F15-resized")
        return c.done(f"B 高 {bh} → {after[3]}（A 高 {ah}）")

    try:
        C.report.case("F12", "拖标题栏移动收纳箱（自绘拖动）：窗口跟手、松手后 fences 与 fence_layouts 落库", f12)
        C.report.case("F13", "移动吸附：B.top 落在 A.top ± 阈值内 → 吸到 A.top；拖动中 A 上边高亮、松手后消失", f13)
        C.report.case("F14", "按住 Alt 拖动不吸附、不高亮；松开后照常吸附", f14)
        C.report.case("F15", "缩放吸附：拖下边让高度接近 A → 等高、上边不动；A 的左右两边高亮", f15)
    finally:
        if app.alive():
            dump_log_tail(app, 15)
        app.stop()


# ----------------------------------------------------------------------------- P：平台集成（不隔离）

def group_platform(allow_real: bool) -> None:
    if not allow_real:
        for cid, t in [("P01", "非隔离启动：主窗口 + 日志位置"), ("P02", "单实例"), ("P03", "关闭主窗口退出进程")]:
            C.report.add(cid, t, "SKIP", "需要 --allow-real-profile（会使用真实的 %LOCALAPPDATA%\\mini-todo）")
        return
    log_path = W.LOCALAPPDATA / "com.tauri-app.mini-todo" / "logs" / "mini-todo.log"
    ignored_dir = C.work / "release-ignored-data"
    shutil.rmtree(ignored_dir, ignore_errors=True)
    env = {}
    if C.suite == "release":
        # release 必须忽略 debug 专用变量
        env = {"MINI_TODO_DATA_DIR": str(ignored_dir), "MINI_TODO_DEBUG_OPEN": "settings"}
    app = App(C.exe, env=env, tag="P", out_dir=C.out)
    st: dict = {}

    def p01():
        mark = log_path.stat().st_size if log_path.exists() else 0
        app.start()
        main = app.main(90)
        st["main"] = main
        W.park_cursor()
        time.sleep(3)
        c = Checks()
        c.ok(log_path.exists(), f"日志文件在 {log_path}")
        new = app.log_since(mark)
        c.ok("启动" in new, "日志有本次启动记录")
        c.ok("开发专用数据隔离" not in new, "未启用数据隔离")
        c.ok((W.LOCALAPPDATA / "mini-todo" / "data.db").exists(), "数据库在 %LOCALAPPDATA%\\mini-todo\\data.db")
        lw, lh = main.logical_size()
        c.near(round(lw), 380, 2, "主窗口逻辑宽")
        c.near(round(lh), 600, 2, "主窗口逻辑高")
        img = shot(main, f"P01-{C.suite}-main")
        c.ok(has_content(img) >= 12, "界面已渲染")
        check_main_transparency(main, img, c)
        if C.suite == "release":
            c.ok(not ignored_dir.exists(), "release 忽略 MINI_TODO_DATA_DIR")
            time.sleep(1)
            c.ok(not app.windows("设置"), "release 忽略 MINI_TODO_DEBUG_OPEN")
        maybe_emit("P01", img)
        return c.done(f"log={log_path}")

    def p02():
        main = st.get("main")
        if not main:
            raise Skip("主窗口没有起来")
        mark = app.log_size()
        main.show(W.SW_MINIMIZE)
        W.wait_until(lambda: main.iconic, 5, "主窗口最小化")
        second = App(C.exe, env=env, tag="P-second", out_dir=C.out)
        second.start()
        c = Checks()
        try:
            second.proc.wait(15)
            c.ok(True, f"第二个实例很快退出（exit={second.proc.returncode}）")
        except subprocess.TimeoutExpired:
            c.ok(False, "第二个实例 15s 内没有退出")
            second.stop()
        try:
            W.wait_until(lambda: not main.iconic and main.visible, 6, "第一个实例的主窗口被唤起")
            c.ok(True, "主窗口已还原")
        except W.Timeout as e:
            c.ok(False, str(e))
        time.sleep(1)
        fg = W.foreground_window()
        c.ok(fg == main.hwnd, f"主窗口成为前台窗口（前台={W.describe_hwnd(fg)}）")
        new = app.log_since(mark)
        c.ok("检测到已运行的实例" in new, "第二实例日志：检测到已运行的实例")
        c.ok("检测到重复启动" in new, "第一实例日志：检测到重复启动，唤起窗口")
        procs = [w for w in W.run_ps("Get-Process mini-todo -ErrorAction SilentlyContinue | ForEach-Object { $_.Id }").split()
                 if w.strip().isdigit()]
        c.eq(len(procs), 1, "只剩一个 mini-todo 进程")
        return c.done()

    def p03():
        main = st.get("main")
        if not main:
            raise Skip("主窗口没有起来")
        main.post_close()
        try:
            app.proc.wait(15)
            return f"WM_CLOSE 后进程退出（exit={app.proc.returncode}）"
        except subprocess.TimeoutExpired:
            raise AssertionError(f"关闭主窗口 15s 后进程仍在运行（窗口：{[w.title for w in app.windows()]}）")

    try:
        C.report.case("P01", f"非隔离启动（{C.suite}）：主窗口、%LOCALAPPDATA% 日志与数据库位置", p01)
        C.report.case("P02", "单实例：再次启动 → 第二个进程退出、已运行实例的主窗口被唤起", p02)
        C.report.case("P03", "关闭主窗口（WM_CLOSE）→ 进程退出", p03)
    finally:
        if app.alive():
            dump_log_tail(app, 20)
        app.stop()
        W.kill_all(C.exe)


# ----------------------------------------------------------------------------- 入口

def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", required=True)
    ap.add_argument("--seed")
    ap.add_argument("--sample", default=str(HERE.parent / "sample-data.json"))
    ap.add_argument("--out", required=True)
    ap.add_argument("--work")
    ap.add_argument("--suite", choices=["debug", "release"], default="debug")
    ap.add_argument("--only", default="")
    # 按用例 ID 跳过（逗号分隔，如 I06 —— 它会写入再删除与安装版共用的 HKCU Run\mini-todo）
    ap.add_argument("--skip", default="")
    ap.add_argument("--allow-real-profile", action="store_true")
    ap.add_argument("--emit-shots", default="")
    ap.add_argument("--emit-failures", type=int, default=1)
    ap.add_argument("--uia-dump", action="store_true")
    args = ap.parse_args()

    C.exe = Path(args.exe).resolve()
    C.seed_exe = Path(args.seed).resolve() if args.seed else None
    C.sample = Path(args.sample).resolve()
    C.out = Path(args.out).resolve()
    C.work = Path(args.work).resolve() if args.work else C.out.parent / "e2e-work"
    C.suite = args.suite
    C.emit_shots = {s.strip() for s in args.emit_shots.split(",") if s.strip()}
    C.out.mkdir(parents=True, exist_ok=True)
    C.work.mkdir(parents=True, exist_ok=True)
    C.report = Report(C.out, f"mini-todo GPUI Windows e2e（{args.suite}）", emit_budget=args.emit_failures)
    C.report.skip = {s.strip().upper() for s in args.skip.split(",") if s.strip()}
    only = {s.strip().upper() for s in args.only.split(",") if s.strip()}

    def want(g: str) -> bool:
        return not only or g in only

    if not C.exe.exists():
        C.report.add("BUILD", "exe 存在", "FAIL", f"找不到 {C.exe}")
        return C.report.finish()
    C.report.add("BUILD", f"{args.suite} 构建产物", "PASS", f"{C.exe.name} {C.exe.stat().st_size / 1e6:.1f} MB")
    setup_env()
    try:
        if args.suite == "debug":
            if want("W"):
                group_windows(args.uia_dump)
            if want("S"):
                group_states()
            if want("N"):
                group_notifications()
            if want("I"):
                group_interactions()
            if want("D"):
                group_webdav()
            if want("F"):
                group_fences()
                group_fence_geometry()
        if want("P"):
            group_platform(args.allow_real_profile)
    finally:
        W.kill_all(C.exe)
    return C.report.finish()


if __name__ == "__main__":
    sys.exit(main())
