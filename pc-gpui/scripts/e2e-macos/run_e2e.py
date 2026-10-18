#!/usr/bin/env python3
"""GPUI 版 mini-todo 的 macOS e2e（GitHub macOS runner：已登录的图形会话，屏幕录制 / 辅助功能已预授权）。

    python run_e2e.py --app <mini-todo.app> --out <输出目录> --work <工作目录>
                      [--suite smoke|release] [--seed <seed 可执行文件>] [--only M01,M09]
                      [--wallpaper 1F6F8B] [--strict-traffic-lights] [--allow-real-profile] [--case-timeout 180]

- `--app` 是 `pc-gpui/scripts/macos/bundle.sh` 包好的 .app。直接执行 `<app>/Contents/MacOS/mini-todo`：环境变量能传进去，
  NSBundle.mainBundle 仍是这个 .app（bundle id 有效）
- **smoke**（debug 构建 + seed）：每个用例 seed 样例数据到 `<work>/<用例>/`，以 MINI_TODO_DATA_DIR 隔离启动（debug 开关只在
  debug 构建生效）；用例之间互相独立。收纳箱用例（MF）另把收纳根目录、"桌面"（MINI_TODO_DESKTOP_DIR）、拖入 / 移回的原目录
  都放在 `<work>/<用例>-*` 临时目录里，绝不碰真实的 ~/Desktop / ~/Documents / ~/Downloads（隐私授权框没人点会一直卡住）
- **release**（release 构建，`--allow-real-profile` 才跑）：不隔离、用真实目录（只在一次性 VM 上跑）；P01~P04 共用一个实例
  （某个用例把它弄没了，后面的用例重新启动一个）
- 每个用例有超时（SIGALRM），结束时杀掉本用例启动的进程；单个用例失败不影响后面的用例；任一 FAIL 时退出码为 1
- 尺寸断言按"期望尺寸与屏幕可见区域取小"：AppKit 把放不下的窗口夹到屏幕内（runner 默认 1024×768 pt）
- 输出：`<out>/<用例>-*.png`、`*-ocr.txt`、`logs/`、`results.json`、`summary.md`（+ `$GITHUB_STEP_SUMMARY`）、`fail-<用例>.txt/.png`、
  `emit/*.png`（由 `../e2e-windows/emit_annotations.py images` 作为注解发出）；窗口列表、OCR 文字、AX 探测、日志尾部都打印到 stdout

| 用例 | 内容 |
|---|---|
| M01 | 主窗口：尺寸、截图有内容、OCR 认出未完成待办标题、bundle id；红绿灯 / 菜单 / 状态项（`--strict-traffic-lights` 时红绿灯须隐藏） |
| M02 | `MINI_TODO_DEBUG_OPEN=settings`：设置窗口尺寸、OCR 认出设置页文字 |
| M03 | `MINI_TODO_DEBUG_OPEN=editor:1:view`：只读详情尺寸、OCR 认出待办 #1 标题 |
| M04 | `tell application id … to quit`（不行再 NSRunningApplication.terminate）→ 15s 内退出、退出码 0、无 panic |
| M05 | 切到前台、确认前台是本应用 → ⌘Q（系统输入流）→ 退出码 0 |
| M06 | App 菜单（AX 菜单栏 1）：应用菜单（退出 / 设置… / 隐藏）、编辑（剪切 / 拷贝 / 粘贴 / 全选）、窗口（最小化 / 关闭窗口）；AX 点「设置…」→ 设置窗口 |
| M07 | 菜单栏图标：菜单项齐全（含收纳箱三项）；「打开设置」→ 设置窗口；「退出」→ 退出码 0 |
| M08 | ⌘W 关主窗口、进程仍在；`open <app>`（LaunchServices reopen）→ 主窗口重新出现、没有第二个进程 |
| M09 | 点 FAB → 新建窗口 → 输入"E2E 新建待办" → ⌘Enter → 落库（时间格式规范）→ 主窗口 OCR 认出 |
| M10 | 固定模式（菜单栏图标「固定模式」）：拖标题栏不动、AX 改尺寸无效、`is_fixed` 落库；取消后又能拖 |
| MF01 | 收纳箱 `fence:new`：kCGWindowLayer = 桌面图标层 + 1、排在普通窗口之后、工作区右上角 368×336、日志自报的 behavior / canHide / 面板、库与收纳文件夹、截图 OCR |
| MF02 | 往收纳文件夹写文件 → fence_items 补行（无原始路径）、收纳箱界面刷新 |
| MF03 | 拖放源进程（`drag_source.py`）把临时目录里的文件拖进收纳箱 → 移进收纳文件夹、记原始路径；同名再拖 → `a (2).txt`、不覆盖 |
| MF04 | 预置项目，右键「移回桌面」→ 回原目录；没有原始路径的回 `MINI_TODO_DESKTOP_DIR` |
| MF05 | 「更多」→「删除收纳箱」→「删除」→ 全部移回、`.DS_Store` 不挡删除、文件夹与记录删掉、窗口关闭 |
| MF06 | ⌘H / `NSRunningApplication.hide()` 后收纳箱仍在；应用隐藏中点收纳箱：前台不变、不取消隐藏 |
| MF07 | 有收纳箱可见时 ⌘W 关主窗口 → `open <app>` → 主窗口回来 |
| MF08 | Finder 在前台时双击收纳箱标题（改名会激活应用）→ 系统输入流输入新标题（逐字输入不行就 pbcopy + ⌘A + ⌘V）→ 回车 → 标题与收纳文件夹改名 |
| MF09 | 拖标题栏 → 收纳箱移动 → fences 与 fence_layouts 落库 |
| MF10 | 菜单栏图标「新增收纳箱」/「显示/隐藏所有收纳箱」×2 /「重置收纳箱位置」；「退出」后不带 DEBUG_OPEN 重启 → 原位恢复 |
| P01 | release：窗口有内容、真实数据库迁移到 LATEST_VERSION、日志在 ~/Library/Logs、单实例 socket |
| P02 | 单实例：再次执行 → 5s 内退出码 0、已运行的实例被唤起、窗口数不变；`--autostart` 再执行 → 也退出（不强断言唤起） |
| P03 | 开机自启：菜单栏图标「开机自启动」→ LaunchAgent plist（Label / ProgramArguments）→ 再点删除 |
| P04 | ⌘Q 退出：退出码 0、socket 被删、日志无 panic |
"""
from __future__ import annotations

import argparse
import atexit
import json
import os
import platform
import plistlib
import re
import shutil
import signal
import sqlite3
import stat
import subprocess
import sys
import threading
import time
from dataclasses import dataclass
from pathlib import Path

HERE = Path(__file__).resolve().parent
PC_GPUI = HERE.parents[1]
sys.path.insert(0, str(HERE))

from PIL import Image  # noqa: E402

import maclib as M  # noqa: E402
from maclib import App, Checks, Report, Skip, WinInfo  # noqa: E402

BUNDLE_ID = "com.tauri-app.mini-todo"
MAIN_TITLE, MAIN_SIZE = "Mini Todo", (380, 600)          # windows/main_window.rs DEFAULT_WIDTH / DEFAULT_HEIGHT
SETTINGS_TITLE, SETTINGS_SIZE = "设置", (680, 560)        # windows/settings/mod.rs WIDTH / HEIGHT
DETAIL_TITLE, NEW_TITLE, EDITOR_SIZE = "待办详情", "新建待办", (1080, 600)   # windows/editor/mod.rs os_title / WIDTH / HEIGHT
SIZE_TOL = 40
FAB_OFFSET = 44            # theme.rs FAB_INSET 20 + FAB_SIZE 48 / 2：FAB 中心距窗口右 / 下边
SETTINGS_LABELS = ["常规", "外观", "数据与同步", "屏幕配置", "关于"]   # SettingsTab::label
DETAIL_TODO_ID = 1
NEW_TODO_TITLE = "E2E 新建待办"
# services/mac_menu.rs menus()
APP_MENU_ITEMS = ["退出", "设置…", "隐藏"]                   # 前缀匹配（"退出 Mini Todo"、"隐藏 Mini Todo"）
EDIT_MENU, EDIT_ITEMS = "编辑", ["剪切", "拷贝", "粘贴", "全选"]
WINDOW_MENU, WINDOW_ITEMS = "窗口", ["最小化", "关闭窗口"]
# services/tray.rs imp::start（收纳箱三项：Windows 托盘与 macOS 菜单栏图标都有）
TRAY_ITEMS = ["添加待办项", "固定模式", "重置位置", "打开设置", "开机自启动", "退出"]
TRAY_FENCE_ITEMS = ["新增收纳箱", "显示/隐藏所有收纳箱", "重置收纳箱位置"]
# 真实目录（release 组）：core db/paths.rs、app services/logging.rs、services/single_instance/unix_socket.rs、services/autostart.rs
REAL_DB = Path.home() / "Library" / "Application Support" / "mini-todo" / "data.db"
REAL_LOG = Path.home() / "Library" / "Logs" / "com.tauri-app.mini-todo" / "mini-todo.log"
SOCKET = Path("/tmp/com_tauri_app_mini_todo_si.sock")
LAUNCH_AGENT = Path.home() / "Library" / "LaunchAgents" / "mini-todo.plist"
CANONICAL_TIME = re.compile(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$")


class Ctx:
    app: Path
    exe: Path
    seed_exe: Path | None
    sample: Path
    out: Path
    work: Path
    screen: M.ScreenInfo
    strict_traffic: bool = False
    allow_real: bool = False
    pending_titles: list[str] = []
    titles_by_id: dict[int, str] = {}
    rel_app: App | None = None


C = Ctx()


# ----------------------------------------------------------------------------- 公共

def fresh_data(name: str) -> Path:
    if not C.seed_exe:
        raise Skip("没有 seed 可执行文件（--seed）")
    d = C.work / name
    M.seed(C.seed_exe, d, C.sample)
    return d


def start(tag: str, debug_open: str | None = None) -> App:
    d = fresh_data(tag)
    return App(C.app, data_dir=d, debug_open=debug_open, tag=tag, out_dir=C.out, work_dir=C.work).start()


def exp(spec: tuple[float, float]) -> tuple[float, float]:
    return M.expected_size(spec, C.screen)


def size_match(w: WinInfo, spec: tuple[float, float]) -> bool:
    e = exp(spec)
    return abs(w.w - e[0]) <= SIZE_TOL and abs(w.h - e[1]) <= SIZE_TOL


def check_size(ck: Checks, w: WinInfo, spec: tuple[float, float], what: str) -> None:
    e = exp(spec)
    clamp = "" if e == tuple(spec) else f"，建窗 {spec[0]:g}×{spec[1]:g} 夹到屏幕可见区域"
    ck.near(round(w.w), e[0], SIZE_TOL, f"{what}宽（pt{clamp}）")
    ck.near(round(w.h), e[1], SIZE_TOL, f"{what}高（pt）")


def pick(ws: list[WinInfo], title: str, spec: tuple[float, float], exclude: tuple[str, ...] = ()) -> WinInfo | None:
    """按标题（有屏幕录制权限时 kCGWindowName 可读）找窗口，读不到标题时按尺寸"""
    for w in ws:
        if w.name == title:
            return w
    for w in ws:
        if w.name in exclude:
            continue
        if size_match(w, spec):
            return w
    return None


def wait_for(app: App, title: str, spec: tuple[float, float], timeout: float = 60,
             exclude: tuple[str, ...] = (), strict: bool = False) -> WinInfo:
    t0 = time.time()
    e = exp(spec)
    try:
        w = app.wait_windows(lambda ws: pick(ws, title, spec, exclude), timeout,
                             f"窗口 '{title}'（约 {e[0]:g}×{e[1]:g}pt）")
    except M.Timeout:
        M.dump_windows("等窗口超时", app.pid)
        others = [w for w in app.windows() if w.name not in exclude]
        if others and not strict:
            # 尺寸对不上时仍给出本进程的窗口，交给用例做软断言、继续截图与 OCR
            print(f"    按标题 / 尺寸没等到 '{title}'，本进程有 {len(others)} 个 layer 0 窗口，取最大的一个继续")
            return max(others, key=lambda x: x.w * x.h)
        raise
    print(f"    {time.time() - t0:.1f}s 后出现：{w.describe()}")
    return w


def main_win(app: App) -> WinInfo | None:
    return pick(app.windows(), MAIN_TITLE, MAIN_SIZE)


def refresh(app: App, win: WinInfo) -> WinInfo:
    for w in app.windows():
        if w.number == win.number:
            return w
    raise AssertionError(f"窗口 #{win.number} 已不在屏幕上")


def shoot(win: WinInfo, name: str) -> tuple[Image.Image, float, dict]:
    path = C.out / f"{name}.png"
    img = M.capture_window(win, path)
    scale = img.width / win.w if win.w else 1.0
    stats = M.content_stats(img)
    print(f"    截图 {path.name}: {stats}（缩放 {scale:.2f}）")
    return img, scale, stats


def ocr_until(app: App, win: WinInfo, name: str, check, timeout: float = 30,
              bg: tuple[int, int, int] = (255, 255, 255),
              refresh_fn=None) -> tuple[Image.Image, float, dict, list, bool, str]:
    """反复截图 + OCR 直到 check(lines) 成立（数据是异步加载的，第一帧可能还是空列表）。
    `bg`：透明像素铺的底色（收纳箱铺黑色）；`refresh_fn(app, win)`：重新取窗口（默认只找 layer 0 的窗口）"""
    deadline = time.time() + timeout
    attempt = 0
    while True:
        attempt += 1
        win = (refresh_fn or refresh)(app, win)
        img, scale, stats = shoot(win, name)
        lines = M.ocr(img, C.work / f"{name}-ocr-input.png", bg=bg)
        ok, detail = check(lines)
        last = ok or time.time() >= deadline
        if last:
            M.print_ocr(lines, f"{name} 第 {attempt} 次")
        else:
            print(f"    OCR[{name} 第 {attempt} 次] {len(lines)} 行，未满足：{detail}；"
                  f"前几行 {[l.text for l in lines[:6]]}")
        (C.out / f"{name}-ocr.txt").write_text(
            "\n".join(f"({l.x:.0f},{l.y:.0f}) {l.conf:.2f} {l.text}" for l in lines) + "\n", encoding="utf-8")
        if last:
            return img, scale, stats, lines, ok, detail
        time.sleep(2)


def any_title(titles: list[str]):
    def check(lines):
        hits = []
        notes = []
        for t in titles:
            ok, how = M.fuzzy_find(t, lines)
            notes.append(f"'{t}': {how}")
            if ok:
                hits.append(t)
        return bool(hits), (f"认出 {hits}" if hits else "一个都没认出：" + "; ".join(notes))
    return check


def labels_check(labels: list[str]):
    def check(lines):
        hits = [t for t in labels if M.fuzzy_find(t, lines, ratio=1.0, min_len=2)[0]]
        return bool(hits), (f"认出 {hits}" if hits else f"{labels} 一个都没认出")
    return check


def check_no_panic(ck: Checks, app: App) -> None:
    panics = app.panics()
    for l in panics[:10]:
        print(f"    PANIC: {l}")
    ck.ok(not panics, f"stdout / 日志里没有 panicked（{len(panics)} 处）")
    errs = app.errors()
    if errs:
        print(f"    日志里有 {len(errs)} 条 [ERROR]（只打印，不判失败）：")
        for l in errs[:15]:
            print(f"      {l}")


def emit_zoom_topleft(img: Image.Image, scale: float, name: str) -> Image.Image:
    """窗口左上角 160×48pt 放大 3 倍（看红绿灯）"""
    crop = img.crop((0, 0, min(img.width, round(160 * scale)), min(img.height, round(48 * scale))))
    zoom = crop.resize((crop.width * 3 // max(1, round(scale)), crop.height * 3 // max(1, round(scale))), Image.NEAREST)
    zoom.save(C.out / f"{name}.png")
    M.save_emit(zoom, C.out, name, max_w=480, colors=32)
    return zoom


def traffic_info(img: Image.Image, scale: float, ax: dict) -> str:
    px = M.traffic_light_pixels(img, scale)
    pixel_verdict = "有" if sum(1 for v in px.values() if v >= 6 * scale * scale) >= 2 else "没有"
    s = f"AX：{ax['traffic']}；像素：左上角{pixel_verdict}红黄绿色块 {px}（窗口不活动时红绿灯是灰色，像素法看不出）"
    print(f"    红绿灯：{s}")
    return s


def bring_to_front(app: App, win: WinInfo | None = None, timeout: float = 12) -> bool:
    """把被测应用切到前台并确认：NSRunningApplication.activate → System Events `set frontmost` → 点一下窗口顶部空白处"""
    deadline = time.time() + timeout
    step = 0
    while True:
        fp = M.frontmost_pid()
        if fp == app.pid:
            print(f"    前台：{M.describe_pid(fp)}（本应用）")
            return True
        if time.time() >= deadline:
            print(f"    没能切到前台；当前前台：{M.describe_pid(fp)}")
            return False
        step += 1
        if step % 3 == 1:
            print(f"    前台是 {M.describe_pid(fp)}，activateWithOptions → {app.activate()}")
        elif step % 3 == 2:
            script = (f'tell application "System Events" to set frontmost of '
                      f'(first process whose unix id is {app.pid}) to true')
            try:
                r = M.run(["osascript", "-e", script], timeout=10)
                print(f"    System Events set frontmost → rc={r.returncode} {(r.stdout + r.stderr).strip()[:160]!r}")
            except (subprocess.TimeoutExpired, OSError) as e:
                print(f"    System Events set frontmost 失败：{e}")
        else:
            w = win or main_win(app) or next(iter(app.windows()), None)
            if w is not None:
                w = next((x for x in app.windows() if x.number == w.number), w)
                M.click(w.x + w.w / 2, w.y + 6)
        time.sleep(0.8)


def send_cmd_key(app: App, keycode: int, front: bool) -> str:
    """⌘+键：应用在前台时走系统输入流（真实按键路径），否则退到 CGEventPostToPid"""
    if front:
        return M.post_key_hid(keycode, M.cmd_flag())
    print("    应用不在前台，退到 CGEventPostToPid")
    return M.post_key_to_pid(app.pid, keycode, M.cmd_flag())


def setting(app: App, key: str):
    rows = app.q("SELECT value FROM settings WHERE key = ?", (key,))
    return rows[0][0] if rows else None


def pids_of_exe() -> list[int]:
    r = subprocess.run(["pgrep", "-f", str(C.exe)], capture_output=True, text=True)
    return [int(x) for x in r.stdout.split() if x.strip().isdigit() and int(x) != os.getpid()]


def clean_env() -> dict:
    return {k: v for k, v in os.environ.items() if not k.startswith("MINI_TODO_")}


def menu_has(items: list[M.AxMenuItem], want: str, prefix: bool = False) -> bool:
    titles = [i.title for i in items]
    return any(t.startswith(want) for t in titles) if prefix else want in titles


# ----------------------------------------------------------------------------- smoke：窗口与退出

def m01() -> str:
    M.dump_windows("用例开始前")
    app = start("M01")
    try:
        ck = Checks()
        win = wait_for(app, MAIN_TITLE, MAIN_SIZE)
        M.dump_windows("主窗口出现后", app.pid)
        print(f"    NSRunningApplication：{app.running_app_info()}")
        check_size(ck, win, MAIN_SIZE, "主窗口")
        if win.name is not None:
            ck.ok(win.name == MAIN_TITLE, f"主窗口标题 {win.name!r} == {MAIN_TITLE!r}")
        else:
            print("    kCGWindowName 读不到（没有屏幕录制权限？）")
        time.sleep(2)
        M.park_cursor()
        img, scale, stats, lines, found, how = ocr_until(app, win, "M01-main", any_title(C.pending_titles), 40)
        ck.ok(M.has_content(stats), f"主窗口截图有内容 {stats}")
        ck.ok(found, f"OCR 认出未完成待办标题：{how}")
        emit_zoom_topleft(img, scale, "M01-topleft-x3")
        M.save_emit(img, C.out, "M01-main", max_w=380, colors=24)
        ax = M.ax_probe(app.pid)
        tl = traffic_info(img, scale, ax)
        if C.strict_traffic:
            ck.ok(ax["traffic"] == "隐藏", f"红绿灯已隐藏（{tl}）")
        screen = M.capture_screen(C.out / "M01-screen.png")
        print(f"    整屏截图 M01-screen.png: {screen.width}x{screen.height}；{C.screen.describe()}")
        M.save_emit(screen, C.out, "M01-screen", max_w=480, colors=16)
        bid = app.bundle_id()
        if bid is not None:
            ck.ok(bid == BUNDLE_ID, f"进程的 bundle id {bid!r} == {BUNDLE_ID!r}")
        else:
            print(f"    bundle id 查不到（{app.running_app_info()}），跳过该项")
        status_items = [w for w in M.windows_of(app.pid, layer=None) if w.layer != 0]
        print(f"    本进程非 layer 0 的窗口（layer 25 = 菜单栏状态项）：{[w.describe() for w in status_items]}")
        check_no_panic(ck, app)
        return ck.done(f"{how}；红绿灯 {ax['traffic']}；菜单 {ax['menus']}；状态项 {ax['extras']}")
    finally:
        app.dump_logs(20)
        app.stop()
        M.dump_windows("用例结束后")


def m02() -> str:
    M.dump_windows("用例开始前")
    app = start("M02", "settings")
    try:
        ck = Checks()
        win = wait_for(app, SETTINGS_TITLE, SETTINGS_SIZE, exclude=(MAIN_TITLE,))
        M.dump_windows("设置窗口出现后", app.pid)
        check_size(ck, win, SETTINGS_SIZE, "设置窗口")
        time.sleep(1.5)
        M.park_cursor()
        img, scale, stats, lines, found, how = ocr_until(app, win, "M02-settings", labels_check(SETTINGS_LABELS), 30)
        ck.ok(M.has_content(stats), f"设置窗口截图有内容 {stats}")
        ck.ok(found, f"OCR 认出设置页文字：{how}")
        M.save_emit(img, C.out, "M02-settings", max_w=400, colors=16)
        traffic_info(img, scale, {"traffic": "（M02 未探测 AX）"})
        check_no_panic(ck, app)
        return ck.done(how)
    finally:
        app.dump_logs(15)
        app.stop()
        M.dump_windows("用例结束后")


def m03() -> str:
    M.dump_windows("用例开始前")
    title = C.titles_by_id.get(DETAIL_TODO_ID)
    if not title:
        raise Skip(f"sample-data 里没有待办 #{DETAIL_TODO_ID}")
    app = start("M03", f"editor:{DETAIL_TODO_ID}:view")
    try:
        ck = Checks()
        win = wait_for(app, DETAIL_TITLE, EDITOR_SIZE, exclude=(MAIN_TITLE,))
        M.dump_windows("详情窗口出现后", app.pid)
        check_size(ck, win, EDITOR_SIZE, "详情窗口")
        time.sleep(1.5)
        M.park_cursor()
        img, scale, stats, lines, found, how = ocr_until(app, win, "M03-detail", any_title([title]), 30)
        ck.ok(M.has_content(stats), f"详情窗口截图有内容 {stats}")
        ck.ok(found, f"OCR 认出待办 #{DETAIL_TODO_ID} 的标题 '{title}'：{how}")
        M.save_emit(img, C.out, "M03-detail", max_w=480, colors=16)
        check_no_panic(ck, app)
        return ck.done(how)
    finally:
        app.dump_logs(15)
        app.stop()
        M.dump_windows("用例结束后")


def m04() -> str:
    M.dump_windows("用例开始前")
    app = start("M04")
    try:
        ck = Checks()
        wait_for(app, MAIN_TITLE, MAIN_SIZE)
        M.dump_windows("主窗口出现后", app.pid)
        print(f"    NSRunningApplication：{app.running_app_info()}")
        time.sleep(3)
        t0 = time.time()
        via = None
        script = f'tell application id "{BUNDLE_ID}" to quit'
        try:
            r = M.run(["osascript", "-e", script], timeout=10)
            print(f"    osascript '{script}' → rc={r.returncode} {(r.stdout + r.stderr).strip()!r}")
        except subprocess.TimeoutExpired:
            print("    osascript 10s 没返回（可能卡在自动化授权弹框上）")
        except OSError as e:
            print(f"    osascript 执行失败：{e}")
        if app.wait_exit(max(0.5, 15 - (time.time() - t0))):
            via = "AppleScript quit"
        else:
            print(f"    AppleScript quit 之后 {time.time() - t0:.1f}s 进程仍在，改用 NSRunningApplication.terminate()")
            print(f"    terminate() 返回 {app.terminate_via_appkit()}")
            if app.wait_exit(10):
                via = "NSRunningApplication.terminate"
        secs = time.time() - t0
        if via is None:
            print(f"    {secs:.1f}s 内没有响应退出请求，SIGTERM")
            app.stop()
            via = f"SIGTERM/SIGKILL（exit={app.exit_code}）"
        print(f"    退出方式：{via}，用时 {secs:.1f}s，exit code={app.exit_code}")
        ck.ok(via in ("AppleScript quit", "NSRunningApplication.terminate"),
              f"应用响应 quit 请求退出（{via}，{secs:.1f}s）")
        ck.ok(secs <= 25, f"从发出 quit 到进程退出 {secs:.1f}s")
        if via in ("AppleScript quit", "NSRunningApplication.terminate"):
            # NSApp terminate: 走完 applicationWillTerminate 后 exit(0)；负数 = 退出过程中被信号结束（崩溃）
            ck.ok(app.exit_code == 0, f"正常退出的退出码 {app.exit_code} == 0")
        M.dump_windows("退出后")
        ck.ok(not M.windows_of(app.pid, layer=None), "退出后本进程没有残留窗口")
        app.stop()
        check_no_panic(ck, app)
        return ck.done(f"{via}，{secs:.1f}s，exit={app.exit_code}")
    finally:
        app.dump_logs(40)
        app.stop()
        M.dump_windows("用例结束后")


def m05() -> str:
    """⌘Q：先切到前台并确认前台是本应用，再经系统输入流发 ⌘Q（真实按键路径：菜单 key equivalent / GPUI 绑定 cmd-q）"""
    M.dump_windows("用例开始前")
    app = start("M05")
    try:
        ck = Checks()
        win = wait_for(app, MAIN_TITLE, MAIN_SIZE)
        time.sleep(2)
        front = bring_to_front(app, win)
        ck.ok(front, f"发 ⌘Q 前本应用在前台（前台 {M.describe_pid(M.frontmost_pid())}）")
        t0 = time.time()
        via = send_cmd_key(app, M.KEY_Q, front)
        exited = app.wait_exit(10)
        if not exited:
            print(f"    {via} 发 ⌘Q 后 10s 进程仍在；前台 {M.describe_pid(M.frontmost_pid())}；再用 CGEventPostToPid 发一次")
            via += " + " + M.post_key_to_pid(app.pid, M.KEY_Q, M.cmd_flag())
            exited = app.wait_exit(10)
        secs = time.time() - t0
        print(f"    ⌘Q（{via}）→ {'已退出' if exited else '未退出'}，{secs:.1f}s，exit code={app.exit_code}")
        ck.ok(exited, f"⌘Q 后进程退出（{via}，{secs:.1f}s）")
        if exited:
            ck.ok(app.exit_code == 0, f"退出码 {app.exit_code} == 0")
        app.stop()
        check_no_panic(ck, app)
        return ck.done(f"{via}，{secs:.1f}s")
    finally:
        app.dump_logs(20)
        app.stop()
        M.dump_windows("用例结束后")


# ----------------------------------------------------------------------------- smoke：菜单、菜单栏图标、Dock 唤起

def m06() -> str:
    M.dump_windows("用例开始前")
    app = start("M06")
    try:
        ck = Checks()
        wait_for(app, MAIN_TITLE, MAIN_SIZE)
        time.sleep(2)
        last: list[str] = []

        def read_menus():
            try:
                ms = M.app_menus(app.pid)
            except AssertionError as e:      # 刚启动时 AX 可能还读不到：记下原因，继续等
                last[:] = [f"错误 {e}"]
                return None
            last[:] = [f"读到 {[(t, len(items)) for t, items in ms]}"]
            return ms if any(t not in ("Apple", "") and items for t, items in ms) else None

        try:
            menus = M.wait_until(read_menus, 15, "AX 读到带子项的应用菜单")
        except M.Timeout as e:
            raise AssertionError(f"{e}；最后一次：{last}") from None
        for title, items in menus:
            print(f"    菜单「{title}」：{[i.describe() for i in items]}")
        by_title = {t: items for t, items in menus}
        # 应用菜单的标题由系统换成应用名（bundle 名 mini-todo）：取 Apple / 编辑 / 窗口之外的第一个
        app_entry = next((m for m in menus if m[0] not in ("Apple", "", EDIT_MENU, WINDOW_MENU)), ("?", []))
        app_menu = app_entry[1]
        print(f"    应用菜单「{app_entry[0]}」")
        for want in APP_MENU_ITEMS:
            ck.ok(menu_has(app_menu, want, prefix=True), f"应用菜单有「{want}…」")
        ck.ok(EDIT_MENU in by_title, f"有「{EDIT_MENU}」菜单（顶层：{list(by_title)}）")
        for want in EDIT_ITEMS:
            ck.ok(menu_has(by_title.get(EDIT_MENU, []), want), f"「{EDIT_MENU}」菜单有「{want}」")
        ck.ok(WINDOW_MENU in by_title, f"有「{WINDOW_MENU}」菜单")
        for want in WINDOW_ITEMS:
            ck.ok(menu_has(by_title.get(WINDOW_MENU, []), want), f"「{WINDOW_MENU}」菜单有「{want}」")
        settings_item = next((i for i in app_menu if i.title.startswith("设置")), None)
        if settings_item is None:
            return ck.done()
        print(f"    AXPress(「{settings_item.title}」) → {M.ax_press(settings_item.el)}")
        try:
            win = wait_for(app, SETTINGS_TITLE, SETTINGS_SIZE, timeout=15, exclude=(MAIN_TITLE,), strict=True)
        except M.Timeout as e:
            ck.ok(False, f"点「设置…」后设置窗口出现：{e}")
            return ck.done()
        check_size(ck, win, SETTINGS_SIZE, "设置窗口")
        time.sleep(1.5)
        M.park_cursor()
        img, scale, stats, lines, found, how = ocr_until(app, win, "M06-settings", labels_check(["常规"]), 20)
        ck.ok(found, f"设置窗口 OCR 认出「常规」：{how}")
        check_no_panic(ck, app)
        return ck.done(f"顶层菜单 {[t for t, _ in menus]}；{how}")
    finally:
        app.dump_logs(15)
        app.stop()
        M.dump_windows("用例结束后")


def m07() -> str:
    M.dump_windows("用例开始前")
    app = start("M07")
    try:
        ck = Checks()
        wait_for(app, MAIN_TITLE, MAIN_SIZE)
        time.sleep(2)
        item, items, how = M.open_status_menu(app.pid)
        titles = [i.title for i in items if i.title]
        print(f"    菜单栏图标菜单（{how}）：{[i.describe() for i in items]}")
        for want in TRAY_ITEMS:
            ck.ok(want in titles, f"菜单栏图标菜单有「{want}」")
        for want in TRAY_FENCE_ITEMS:
            ck.ok(want in titles, f"菜单栏图标菜单有收纳箱项「{want}」")
        target = next((i for i in items if i.title == "打开设置"), None)
        if target is None:
            M.close_open_menu(item)
            return ck.done()
        print(f"    AXPress(「打开设置」) → {M.ax_press(target.el)}")
        try:
            win = wait_for(app, SETTINGS_TITLE, SETTINGS_SIZE, timeout=15, exclude=(MAIN_TITLE,), strict=True)
            check_size(ck, win, SETTINGS_SIZE, "设置窗口")
            print(f"    前台：{M.describe_pid(M.frontmost_pid())}（「打开设置」会先激活应用）")
        except M.Timeout as e:
            ck.ok(False, f"「打开设置」后设置窗口出现：{e}")
        time.sleep(1)
        t0 = time.time()
        M.press_status_menu_item(app.pid, "退出")
        exited = app.wait_exit(10)
        secs = time.time() - t0
        ck.ok(exited, f"「退出」后进程退出（{secs:.1f}s）")
        if exited:
            ck.ok(app.exit_code == 0, f"退出码 {app.exit_code} == 0")
        app.stop()
        check_no_panic(ck, app)
        return ck.done(f"{how}；{titles}")
    finally:
        app.dump_logs(15)
        app.stop()
        M.dump_windows("用例结束后")


def m08() -> str:
    M.dump_windows("用例开始前")
    app = start("M08")
    try:
        ck = Checks()
        win = wait_for(app, MAIN_TITLE, MAIN_SIZE)
        time.sleep(2)
        front = bring_to_front(app, win)
        ck.ok(front, "⌘W 之前本应用在前台")
        via = send_cmd_key(app, M.KEY_W, front)
        try:
            M.wait_until(lambda: main_win(app) is None, 8, "主窗口关闭")
            ck.ok(True, f"⌘W（{via}）后主窗口关闭")
        except M.Timeout as e:
            ck.ok(False, f"⌘W（{via}）后主窗口关闭：{e}")
            M.dump_windows("⌘W 之后", app.pid)
            return ck.done()
        M.dump_windows("⌘W 之后", app.pid)
        time.sleep(3)
        ck.ok(app.alive(), "关掉主窗口后进程仍在（菜单栏图标常驻）")
        r = M.run(["open", str(C.app)], timeout=30)
        print(f"    open {C.app} → rc={r.returncode} {(r.stdout + r.stderr).strip()!r}")
        try:
            w2 = wait_for(app, MAIN_TITLE, MAIN_SIZE, timeout=20, strict=True)
            ck.ok(True, f"open（Dock / Finder 唤起）后主窗口重新出现：{w2.describe()}")
        except M.Timeout as e:
            ck.ok(False, f"open 后主窗口重新出现：{e}")
        time.sleep(1)
        pids = pids_of_exe()
        print(f"    mini-todo 进程：{pids}（本用例 pid {app.pid}）")
        ck.ok(pids == [app.pid], f"open 没有另起一个进程（{pids}）")
        check_no_panic(ck, app)
        return ck.done(via)
    finally:
        app.dump_logs(20)
        app.stop()
        M.dump_windows("用例结束后")


# ----------------------------------------------------------------------------- smoke：交互

def m09() -> str:
    M.dump_windows("用例开始前")
    app = start("M09")
    try:
        ck = Checks()
        win = wait_for(app, MAIN_TITLE, MAIN_SIZE)
        time.sleep(2.5)
        before = app.q("SELECT COUNT(*) FROM todos")[0][0]
        front = bring_to_front(app, win)
        ck.ok(front, "点 FAB 前本应用在前台")
        win = refresh(app, win)
        M.click(win.x + win.w - FAB_OFFSET, win.y + win.h - FAB_OFFSET)
        try:
            ed = wait_for(app, NEW_TITLE, EDITOR_SIZE, timeout=15, exclude=(MAIN_TITLE,), strict=True)
        except M.Timeout as e:
            ck.ok(False, f"点 FAB 后新建窗口出现：{e}")
            return ck.done()
        check_size(ck, ed, EDITOR_SIZE, "新建窗口")
        time.sleep(1.5)
        print(f"    前台：{M.describe_pid(M.frontmost_pid())}")
        M.type_text(NEW_TODO_TITLE)
        time.sleep(1.0)
        img, _, _ = shoot(refresh(app, ed), "M09-editor-typed")
        lines = M.ocr(img, C.work / "M09-editor-ocr.png")
        typed, how = M.fuzzy_find(NEW_TODO_TITLE, lines)
        print(f"    逐字输入后 OCR：{how}；{[l.text for l in lines[:10]]}")
        method = "CGEventKeyboardSetUnicodeString"
        if not typed:
            print("    逐字输入没认出来，改用剪贴板：⌘A → ⌘V")
            M.set_clipboard(NEW_TODO_TITLE)
            M.post_key_hid(M.KEY_A, M.cmd_flag())
            M.post_key_hid(M.KEY_V, M.cmd_flag())
            time.sleep(1.0)
            img, _, _ = shoot(refresh(app, ed), "M09-editor-pasted")
            lines = M.ocr(img, C.work / "M09-editor-ocr.png")
            typed, how = M.fuzzy_find(NEW_TODO_TITLE, lines)
            print(f"    粘贴后 OCR：{how}")
            method = "pbcopy + ⌘V"
        ck.ok(typed, f"新建窗口里认出输入的标题（{method}：{how}）")
        t0 = time.time()
        M.post_key_hid(M.KEY_RETURN, M.cmd_flag())
        try:
            M.wait_until(lambda: not [w for w in app.windows() if w.number == ed.number], 10, "新建窗口关闭")
            ck.ok(True, f"⌘Enter 后新建窗口关闭（{time.time() - t0:.1f}s）")
        except M.Timeout as e:
            ck.ok(False, f"⌘Enter 后新建窗口关闭：{e}")
            shoot(refresh(app, ed), "M09-editor-after-save")
        try:
            row = M.wait_until(lambda: app.q("SELECT id, title, created_at, updated_at FROM todos WHERE title = ?",
                                              (NEW_TODO_TITLE,)), 10, "新待办落库")
            print(f"    库里的新待办：{row}；待办总数 {before} → {app.q('SELECT COUNT(*) FROM todos')[0][0]}")
            _, _, created, updated = row[0]
            ck.ok(True, f"库里有新待办「{NEW_TODO_TITLE}」（id={row[0][0]}）")
            ck.ok(bool(CANONICAL_TIME.match(str(created))), f"created_at {created!r} 是 YYYY-MM-DD HH:MM:SS")
            ck.ok(bool(CANONICAL_TIME.match(str(updated))), f"updated_at {updated!r} 是 YYYY-MM-DD HH:MM:SS")
        except M.Timeout as e:
            titles = [r[0] for r in app.q("SELECT title FROM todos ORDER BY id DESC LIMIT 5")]
            ck.ok(False, f"库里有新待办「{NEW_TODO_TITLE}」：{e}；最近的标题 {titles}")
        main = main_win(app)
        if main is not None:
            M.park_cursor()
            img, _, _, _, found, how = ocr_until(app, main, "M09-main-after", any_title([NEW_TODO_TITLE]), 20)
            ck.ok(found, f"主窗口 OCR 认出新待办：{how}")
            M.save_emit(img, C.out, "M09-main-after", max_w=300, colors=16)
        else:
            ck.ok(False, "保存后找不到主窗口")
        check_no_panic(ck, app)
        return ck.done(method)
    finally:
        app.dump_logs(20)
        app.stop()
        M.dump_windows("用例结束后")


def m10() -> str:
    """固定模式：拖标题栏是否移动窗口（真实拖动）为主断言；AX 改尺寸是否生效为辅；`is_fixed` 落库"""
    M.dump_windows("用例开始前")
    app = start("M10")
    try:
        ck = Checks()
        win = wait_for(app, MAIN_TITLE, MAIN_SIZE)
        time.sleep(2.5)
        front = bring_to_front(app, win)
        ck.ok(front, "拖动前本应用在前台")

        def current() -> WinInfo:
            w = main_win(app)
            if w is None:
                raise AssertionError("找不到主窗口")
            return w

        def drag_title(dx: float, dy: float) -> tuple[float, float]:
            w = current()
            x0, y0 = w.x + w.w / 2, w.y + 6          # 标题栏上沿的留白（按钮之上），是拖动区
            M.drag(x0, y0, x0 + dx, y0 + dy)
            time.sleep(0.8)
            w2 = current()
            d = (w2.x - w.x, w2.y - w.y)
            print(f"    拖动 ({dx:+.0f},{dy:+.0f}) → 窗口 {w.x:.0f},{w.y:.0f} → {w2.x:.0f},{w2.y:.0f}，Δ=({d[0]:+.0f},{d[1]:+.0f})")
            return d

        def moved(d, dx, dy) -> bool:
            return abs(d[0] - dx) <= 25 and abs(d[1] - dy) <= 25

        def still(d) -> bool:
            return abs(d[0]) <= 3 and abs(d[1]) <= 3

        def ax_resize_probe(label: str) -> bool | None:
            """AX 把主窗口加大 60pt：返回尺寸是否变了（读不到 AX 窗口时 None）"""
            el = M.ax_window(app.pid, MAIN_TITLE, exp(MAIN_SIZE))
            if el is None:
                print(f"    [{label}] AX 找不到主窗口")
                return None
            f0 = M.ax_frame(el)
            print(f"    [{label}] AX frame={f0} AXPosition 可设={M.ax_settable(el, 'AXPosition')} "
                  f"AXSize 可设={M.ax_settable(el, 'AXSize')}")
            if not f0:
                return None
            def set_xy(attr: str, prop: str, a: float, b: float) -> str:
                r = M.ax_set_xy(el, attr, a, b)
                if "err=0" not in r:   # pyobjc 的 AXValueCreate 不一定能用：退到 System Events
                    r += "；" + M.system_events_set(app.pid, prop, a, b)
                return r

            print(f"    [{label}] 设 AXSize +60 → {set_xy('AXSize', 'size', f0[2] + 60, f0[3] + 60)}")
            time.sleep(0.8)
            f1 = M.ax_frame(el)
            changed = bool(f1) and (abs(f1[2] - f0[2]) > 3 or abs(f1[3] - f0[3]) > 3)
            print(f"    [{label}] 之后 AX frame={f1} → 尺寸{'变了' if changed else '没变'}")
            if changed:
                set_xy("AXSize", "size", f0[2], f0[3])      # 恢复
                time.sleep(0.5)
            print(f"    [{label}] 设 AXPosition +40（只打印）→ {set_xy('AXPosition', 'position', f0[0] + 40, f0[1] + 40)}")
            time.sleep(0.5)
            f2 = M.ax_frame(el)
            print(f"    [{label}] 之后 AX frame={f2}（位置{'变了' if f2 and abs(f2[0] - f0[0]) > 3 else '没变'}）")
            return changed

        d0 = drag_title(100, 60)
        ck.ok(moved(d0, 100, 60), f"普通模式：拖标题栏移动窗口 Δ={d0}")
        base_resize = ax_resize_probe("普通模式")
        print(f"    is_fixed（切换前）= {setting(app, 'is_fixed')!r}")

        M.press_status_menu_item(app.pid, "固定模式")
        try:
            M.wait_until(lambda: setting(app, "is_fixed") == "true", 8, "is_fixed = 'true'")
            ck.ok(True, "菜单栏图标「固定模式」后 settings.is_fixed = 'true'")
        except M.Timeout as e:
            ck.ok(False, f"固定后 settings.is_fixed = 'true'：{e}（现值 {setting(app, 'is_fixed')!r}）")
        print(f"    screen_configs：{app.q('SELECT config_id, is_fixed FROM screen_configs')}")
        time.sleep(1.0)
        bring_to_front(app)
        d1 = drag_title(100, 60)
        ck.ok(still(d1), f"固定模式：拖标题栏不移动窗口 Δ={d1}")
        fixed_resize = ax_resize_probe("固定模式")
        if base_resize:
            ck.ok(fixed_resize is False, "固定模式：AX 改尺寸无效（普通模式下可以改）")
        else:
            print(f"    普通模式下 AX 改尺寸就没生效（{base_resize}），不据此判断固定模式的可缩放性")
        w = current()
        img = M.capture_window(w, C.out / "M10-fixed.png")
        print(f"    截图 M10-fixed.png: {M.content_stats(img)}")
        try:
            item, items, _ = M.open_status_menu(app.pid)
            print(f"    固定后的菜单：{[i.describe() for i in items]}")
            M.close_open_menu(item)
        except AssertionError as e:
            print(f"    (重新读菜单失败: {e})")

        M.press_status_menu_item(app.pid, "固定模式")
        try:
            M.wait_until(lambda: setting(app, "is_fixed") == "false", 8, "is_fixed = 'false'")
            ck.ok(True, "再点「固定模式」后 settings.is_fixed = 'false'")
        except M.Timeout as e:
            ck.ok(False, f"取消固定后 settings.is_fixed = 'false'：{e}")
        time.sleep(1.0)
        bring_to_front(app)
        d2 = drag_title(-100, -60)
        ck.ok(moved(d2, -100, -60), f"取消固定后：拖标题栏又能移动窗口 Δ={d2}")
        check_no_panic(ck, app)
        return ck.done(f"普通 Δ{d0} / 固定 Δ{d1} / 取消 Δ{d2}；AX 改尺寸 普通={base_resize} 固定={fixed_resize}")
    finally:
        app.dump_logs(20)
        app.stop()
        M.dump_windows("用例结束后")


# ----------------------------------------------------------------------------- release（不隔离）

def need_real() -> None:
    if not C.allow_real:
        raise Skip("release 组会读写真实数据目录 / LaunchAgent，只在一次性 VM 上用 --allow-real-profile 跑")


def rel_app() -> App:
    """release 组共用的实例；已退出就重新启动一个"""
    if C.rel_app is not None and C.rel_app.alive():
        return C.rel_app
    if C.rel_app is not None:
        print(f"    上一个实例已退出（exit={C.rel_app.proc.returncode if C.rel_app.proc else None}），重新启动")
        M.KEEP_PIDS.discard(C.rel_app.pid)
    app = App(C.app, data_dir=None, tag=f"P-{len(M.ALL_PROCS)}", out_dir=C.out, work_dir=C.work).start()
    M.KEEP_PIDS.add(app.pid)
    C.rel_app = app
    wait_for(app, MAIN_TITLE, MAIN_SIZE, timeout=60)
    time.sleep(2)
    return app


def latest_version() -> int | None:
    src = PC_GPUI / "crates" / "core" / "src" / "db" / "migrations.rs"
    m = re.search(r"pub const LATEST_VERSION:\s*\w+\s*=\s*(\d+)", src.read_text(encoding="utf-8"))
    return int(m.group(1)) if m else None


def p01() -> str:
    need_real()
    M.dump_windows("用例开始前")
    print(f"    启动前：data.db 存在={REAL_DB.exists()}，日志存在={REAL_LOG.exists()}，socket 存在={SOCKET.exists()}")
    ck = Checks()
    app = rel_app()
    win = main_win(app) or wait_for(app, MAIN_TITLE, MAIN_SIZE)
    M.dump_windows("主窗口出现后", app.pid)
    check_size(ck, win, MAIN_SIZE, "主窗口")
    M.park_cursor()
    img, scale, stats = shoot(win, "P01-main")
    ck.ok(M.has_content(stats), f"主窗口截图有内容 {stats}")
    M.save_emit(img, C.out, "P01-main", max_w=380, colors=24)
    bid = app.bundle_id()
    if bid is not None:
        ck.ok(bid == BUNDLE_ID, f"bundle id {bid!r} == {BUNDLE_ID!r}")
    ck.ok(REAL_DB.exists(), f"数据库在 {REAL_DB}")
    want = latest_version()
    if REAL_DB.exists():
        ver = app.q("SELECT COALESCE(MAX(version), 0) FROM migrations")[0][0]
        ck.ok(want is not None and ver == want, f"迁移版本 {ver} == LATEST_VERSION {want}")
    try:
        M.wait_until(lambda: "启动" in app.log_text(), 10, f"{REAL_LOG} 里有启动记录")
        ck.ok(True, f"日志 {REAL_LOG} 有启动记录")
    except M.Timeout as e:
        ck.ok(False, f"日志在 {REAL_LOG} 且有启动记录：{e}（存在={REAL_LOG.exists()}）")
    for l in [l for l in app.log_text().splitlines() if "启动" in l][:3]:
        print(f"    | {l}")
    is_sock = SOCKET.exists() and stat.S_ISSOCK(SOCKET.stat().st_mode)
    ck.ok(is_sock, f"单实例 socket {SOCKET} 存在（存在={SOCKET.exists()}）")
    print(f"    菜单栏状态项：{M.status_item(app.pid) is not None}；{app.running_app_info()}")
    check_no_panic(ck, app)
    return ck.done(f"迁移 v{want}")


def run_second(args: list[str]) -> tuple[int | None, float, str]:
    t0 = time.time()
    try:
        r = subprocess.run([str(C.exe)] + args, env=clean_env(), cwd=str(C.work), capture_output=True, text=True,
                           encoding="utf-8", errors="replace", timeout=20, stdin=subprocess.DEVNULL)
        return r.returncode, time.time() - t0, (r.stdout + r.stderr).strip()
    except subprocess.TimeoutExpired as e:
        out = ((e.stdout or b"") if isinstance(e.stdout, bytes) else (e.stdout or "").encode()).decode("utf-8", "replace")
        return None, time.time() - t0, out


def p02() -> str:
    need_real()
    ck = Checks()
    app = rel_app()
    before = len(app.windows())
    deactivated = M.activate_other("Finder") != app.pid
    rc, secs, out = run_second([])
    print(f"    第二个实例：rc={rc} 用时 {secs:.1f}s 输出 {out[-400:]!r}")
    ck.ok(rc == 0 and secs <= 5, f"第二次启动 5s 内退出、退出码 0（rc={rc}，{secs:.1f}s）")
    time.sleep(2.5)
    fp = M.frontmost_pid()
    print(f"    之后前台：{M.describe_pid(fp)}")
    after = len(app.windows())
    ck.ok(app.alive(), "第一个实例仍在运行")
    ck.ok(after == before, f"第一个实例的窗口数不变（{before} → {after}）")
    ck.ok(main_win(app) is not None, "主窗口可见")
    if deactivated:
        ck.ok(fp == app.pid, "第二次启动唤起了已运行的实例（切到前台）")
    else:
        print("    没能先把 Finder 切到前台，不判断是否被唤起")
    deactivated = M.activate_other("Finder") != app.pid
    rc2, secs2, out2 = run_second(["--autostart"])
    print(f"    --autostart 第二个实例：rc={rc2} 用时 {secs2:.1f}s 输出 {out2[-400:]!r}")
    ck.ok(rc2 == 0 and secs2 <= 5, f"--autostart 再次启动 5s 内退出、退出码 0（rc={rc2}，{secs2:.1f}s）")
    time.sleep(2.5)
    fp2 = M.frontmost_pid()
    print(f"    --autostart 之后前台：{M.describe_pid(fp2)}（期望不被唤起；只打印）"
          + ("" if deactivated else "（Finder 没切到前台，参考意义不大）"))
    ck.ok(len(app.windows()) == before, "--autostart 之后窗口数不变")
    for l in [l for l in app.log_text().splitlines() if "single" in l.lower() or "单实例" in l][-6:]:
        print(f"    | {l}")
    check_no_panic(ck, app)
    return ck.done(f"二次启动 {secs:.1f}s / --autostart {secs2:.1f}s")


def read_plist() -> dict | None:
    try:
        with open(LAUNCH_AGENT, "rb") as f:
            return plistlib.load(f)
    except (OSError, plistlib.InvalidFileException) as e:
        print(f"    读 plist 失败：{e}")
        return None


def p03() -> str:
    need_real()
    ck = Checks()
    app = rel_app()
    initial = LAUNCH_AGENT.exists()
    print(f"    切换前 {LAUNCH_AGENT} 存在={initial}")
    if initial:
        print(f"    （已存在，先按一次关掉）内容：{read_plist()}")
        M.press_status_menu_item(app.pid, "开机自启动")
        M.wait_until(lambda: not LAUNCH_AGENT.exists(), 10, "plist 删除")
    M.press_status_menu_item(app.pid, "开机自启动")
    try:
        M.wait_until(LAUNCH_AGENT.exists, 10, f"{LAUNCH_AGENT} 出现")
        ck.ok(True, f"「开机自启动」后 {LAUNCH_AGENT} 出现")
    except M.Timeout as e:
        ck.ok(False, f"「开机自启动」后 plist 出现：{e}")
        for l in [l for l in app.log_text().splitlines() if "autostart" in l.lower() or "自启" in l][-6:]:
            print(f"    | {l}")
        return ck.done()
    time.sleep(0.5)
    lint = M.run(["plutil", "-lint", str(LAUNCH_AGENT)], timeout=15)
    print(f"    plutil -lint → rc={lint.returncode} {(lint.stdout + lint.stderr).strip()!r}")
    ck.ok(lint.returncode == 0, "plist 通过 plutil -lint")
    data = read_plist() or {}
    print(f"    plist：{data}")
    exe = os.path.realpath(C.exe)
    ck.ok(data.get("Label") == "mini-todo", f"Label {data.get('Label')!r} == 'mini-todo'")
    ck.ok(data.get("ProgramArguments") == [exe, "--autostart"],
          f"ProgramArguments {data.get('ProgramArguments')!r} == {[exe, '--autostart']!r}")
    print(f"    RunAtLoad = {data.get('RunAtLoad')!r}")
    try:
        item, items, _ = M.open_status_menu(app.pid)
        print(f"    开启后的菜单：{[i.describe() for i in items]}")
        M.close_open_menu(item)
    except AssertionError as e:
        print(f"    (重新读菜单失败: {e})")
    M.press_status_menu_item(app.pid, "开机自启动")
    try:
        M.wait_until(lambda: not LAUNCH_AGENT.exists(), 10, f"{LAUNCH_AGENT} 删除")
        ck.ok(True, "再点「开机自启动」后 plist 删除")
    except M.Timeout as e:
        ck.ok(False, f"再点「开机自启动」后 plist 删除：{e}")
    check_no_panic(ck, app)
    return ck.done(f"ProgramArguments {data.get('ProgramArguments')}")


def p04() -> str:
    need_real()
    ck = Checks()
    app = rel_app()
    front = bring_to_front(app)
    ck.ok(front, "⌘Q 前本应用在前台")
    t0 = time.time()
    via = send_cmd_key(app, M.KEY_Q, front)
    exited = app.wait_exit(10)
    if not exited:
        via += " + " + M.post_key_to_pid(app.pid, M.KEY_Q, M.cmd_flag())
        exited = app.wait_exit(10)
    secs = time.time() - t0
    ck.ok(exited, f"⌘Q（{via}）后进程退出（{secs:.1f}s）")
    if exited:
        ck.ok(app.exit_code == 0, f"退出码 {app.exit_code} == 0")
        try:
            M.wait_until(lambda: not SOCKET.exists(), 5, "socket 删除")
            ck.ok(True, f"退出后 {SOCKET} 已删除")
        except M.Timeout as e:
            ck.ok(False, f"退出后 socket 删除：{e}")
    M.KEEP_PIDS.discard(app.pid)
    app.stop()
    app.dump_logs(30)
    check_no_panic(ck, app)
    return ck.done(f"{via}，{secs:.1f}s，exit={app.exit_code}")


# ----------------------------------------------------------------------------- smoke：桌面收纳箱（MF 组）
#
# macOS 的收纳箱是桌面图标层之上一层、不激活应用的面板（GPUI PopUp；kCGWindowLayer =
# CGWindowLevelForKey(kCGDesktopIconWindowLevelKey) + 1），在所有普通窗口之下：主窗口（layer 0）盖住的地方点不到收纳箱，
# 交互前先把主窗口挪开（clear_main_off）。窗口枚举一律不带 kCGWindowListExcludeDesktopElements（include_desktop=True）。
# 每个用例隔离：数据目录、收纳根目录（MINI_TODO_FENCES_ROOT）、"桌面"（MINI_TODO_DESKTOP_DIR，"移回桌面"的兜底目标）、
# 拖入 / 移回用的原目录都是 <work> 下的临时目录；绝不碰真实的 ~/Desktop / ~/Documents / ~/Downloads（隐私授权框在 runner
# 上没人点，后台移动线程会一直卡住）。collectionBehavior / canHide 在 CGWindowList / AX 里读不到：核对应用自报的日志
# 「[fence] 收纳箱窗口 #<窗口号> 已放到桌面层：level=… behavior=0x… canHide=false …」（services/fences.rs embed_after_open）。

FENCE_TITLE = "收纳箱"                   # 第一个收纳箱的默认标题（core fence::create）
FENCE_TITLE_2 = "收纳箱 2"
FENCE_SIZE = (368.0, 336.0)             # core default_fence_size：(88×4 + 8×2) × (32 + 96×3 + 8×2) DIP
FENCE_MARGIN = 24.0                     # core SCREEN_MARGIN_DIP：新建 / 重置位置离工作区右边、上边
FENCE_GAP = 16.0                        # core LAYOUT_GAP_DIP：重置位置时并排的间距
FENCE_TITLE_BAR = 32.0                  # core FENCE_TITLE_BAR_DIP
FENCE_EMPTY_HINT = "把桌面图标拖到这里"   # windows/fence/logic.rs empty_hint(0)
FENCE_POS_TOL = 8
# ns::make_desktop_layer 整体替换的 collectionBehavior：CanJoinAllSpaces | Stationary | IgnoresCycle | FullScreenNone；
# 不该再有 GPUI PopUp 建窗时的 FullScreenAuxiliary，也不该有 MoveToActiveSpace / Managed / Transient
FENCE_BEHAVIOR = 0x1 | 0x10 | 0x40 | 0x200
FENCE_BEHAVIOR_FORBIDDEN = 0x2 | 0x4 | 0x8 | 0x100
FENCE_LOG = re.compile(r"收纳箱窗口 #(\d+) 已放到桌面层：(.*)$")
# 收纳箱窗口内的坐标（逻辑点，相对窗口左上角；windows/fence/{mod,grid,panels}.rs，与 regress.py 的 fence 场景同一套）
FENCE_TITLE_TEXT = (60, 16)             # 标题文字（双击改名）
FENCE_TITLE_DRAG = (120, 16)            # 标题栏（按下后移动超过 3pt 才拖窗口）
FENCE_MORE_BTN = (350, 16)              # 标题栏右侧「更多」（右起：更多 / 固定 / 折叠，各 24pt）
FENCE_MENU_DELETE = (300, 171)          # 「更多」菜单：重命名 46-78、外观 78-110、显示隐藏的图标 110-142、分隔线、删除收纳箱 155-187
# 删除确认面板的「删除」（small 按钮约 48×24，面板内边距 12 / 10）：按钮行贴面板底边；万一没贴底就紧跟在说明文字下面
FENCE_DELETE_BTN = [(331, 313), (331, 109)]
FENCE_CELL0 = (53, 80)                  # 第一个项目的图标：内容区从 (9, 41) 起，单元 88 宽、上内边距 6、图标 48
# 右键菜单开在指针处：边框 1 + 内边距 4 + 每行 28（打开 / 在访达中显示 / 隐藏 / 移回桌面 / 移动到…）
FENCE_ITEM_MENU_RESTORE = (60, 1 + 4 + 3 * 28 + 14)   # 「移回桌面」的中心，相对指针
FENCE_RENAMED = "E2E 收纳"
DRAG_SOURCE = HERE / "drag_source.py"


@dataclass
class FenceEnv:
    tag: str
    data: Path
    root: Path          # MINI_TODO_FENCES_ROOT
    desktop: Path       # MINI_TODO_DESKTOP_DIR："移回桌面"的兜底目标（原目录不在 / 没有原始路径时）
    src: Path           # 拖入的来源 / 移回的原目录
    app: App | None = None


def fence_app(env: FenceEnv, tag: str, debug_open: str | None) -> App:
    return App(C.app, data_dir=env.data, debug_open=debug_open, tag=tag, out_dir=C.out, work_dir=C.work,
               fences_root=env.root, env={"MINI_TODO_DESKTOP_DIR": str(env.desktop)}).start()


def fence_start(tag: str, debug_open: str | None = "main,fence:new", prepare=None) -> FenceEnv:
    """seed 样例数据 → 准备隔离目录 →（prepare(env) 预置收纳箱，返回它的 id，debug_open 里可写 {fence_id}）→ 启动"""
    data = fresh_data(tag)
    root, desktop, src = (C.work / f"{tag}-{k}" for k in ("fences", "desktop", "src"))
    for d in (root, desktop, src):
        shutil.rmtree(d, ignore_errors=True)
    desktop.mkdir(parents=True)
    src.mkdir(parents=True)
    env = FenceEnv(tag, data, root, desktop, src)
    fence_id = prepare(env) if prepare else None
    if debug_open and fence_id is not None:
        debug_open = debug_open.format(fence_id=fence_id)
    env.app = fence_app(env, tag, debug_open)
    return env


def seed_fence(env: FenceEnv, title: str, rect_pts: tuple[float, float, float, float],
               items: list[tuple[str, str, Path | None]], extra_files: tuple[str, ...] = ()) -> int:
    """启动前直接写库：一个收纳箱（几何 = 全局点坐标 × 统一系数，没有 fence_layouts 记录 → 按"最近一次位置"恢复）+ 项目
    （(文件名, 内容, 原始路径)，文件建在收纳文件夹里）+ 不登记的文件（.DS_Store 等）。返回收纳箱 id"""
    unit = M.unit_scale()
    x, y, w, h = (round(v * unit) for v in rect_pts)
    folder = env.root / title
    folder.mkdir(parents=True, exist_ok=True)
    now = time.strftime("%Y-%m-%d %H:%M:%S")
    con = sqlite3.connect(env.data / "data.db")
    try:
        cur = con.execute(
            "INSERT INTO fences (title, folder_name, x, y, width, height, sort_order, created_at, updated_at) "
            "VALUES (?, ?, ?, ?, ?, ?, (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM fences), ?, ?)",
            (title, title, x, y, w, h, now, now))
        fid = int(cur.lastrowid)
        for order, (name, content, original) in enumerate(items):
            (folder / name).write_text(content, encoding="utf-8")
            con.execute("INSERT INTO fence_items (fence_id, file_name, original_path, sort_order) VALUES (?, ?, ?, ?)",
                        (fid, name, str(original) if original else None, order))
        con.commit()
    finally:
        con.close()
    for name in extra_files:
        (folder / name).write_bytes(b"\x00\x00\x00\x01Bud1")
    print(f"    预置收纳箱 #{fid}「{title}」物理 ({x},{y} {w}×{h})，项目 {[i[0] for i in items]}，另有 {list(extra_files)}")
    return fid


def expected_fence_rect(screen: M.ScreenInfo | None = None) -> tuple[float, float, float, float]:
    """新建 / 重置位置的第一个收纳箱（全局点坐标）：主屏工作区右上角，离右、上边各 24pt（工作区放不下时尺寸夹小）"""
    s = screen or C.screen
    w = min(FENCE_SIZE[0], s.vis_w - 2 * FENCE_MARGIN)
    h = min(FENCE_SIZE[1], s.vis_h - 2 * FENCE_MARGIN)
    return (s.vis_x + s.vis_w - FENCE_MARGIN - w, s.vis_y + FENCE_MARGIN, w, h)


def fence_states(app: App) -> list[tuple[int, dict]]:
    """日志里的桌面层自报：[(窗口号, {level, behavior, canHide, …})]，按出现顺序"""
    out = []
    for line in app.log_text().splitlines():
        m = FENCE_LOG.search(line)
        if m:
            kv = dict(part.split("=", 1) for part in m.group(2).split() if "=" in part)
            out.append((int(m.group(1)), kv))
    return out


def fence_windows(app: App) -> list[WinInfo]:
    """本进程屏幕上的收纳箱窗口：负层级的窗口，或日志自报过的窗口号（层级不对时也找得到，交给层级断言判失败）"""
    if not app.alive():
        return []
    logged = {n for n, _ in fence_states(app)}
    return [w for w in M.windows_of(app.pid, layer=None, include_desktop=True) if w.layer < 0 or w.number in logged]


def find_fence(app: App, number: int) -> WinInfo | None:
    return next((w for w in M.windows_of(app.pid, layer=None, include_desktop=True) if w.number == number), None)


def refresh_fence(app: App, win: WinInfo) -> WinInfo:
    w = find_fence(app, win.number)
    if w is None:
        raise AssertionError(f"收纳箱窗口 #{win.number} 已不在屏幕上")
    return w


def on_screen(app: App, number: int) -> bool:
    return find_fence(app, number) is not None


def wait_fences(app: App, count: int = 1, timeout: float = 45) -> list[WinInfo]:
    t0 = time.time()

    def found():
        if not app.alive():
            raise AssertionError(f"进程已退出（exit={app.proc.returncode if app.proc else None}）；stdout 尾部：\n"
                                 + "\n".join(M.tail(app.stdout_text(), 30)))
        ws = fence_windows(app)
        return ws if len(ws) >= count else None

    try:
        ws = M.wait_until(found, timeout, f"{count} 个收纳箱窗口（本进程负层级的窗口）")
    except M.Timeout as e:
        M.dump_windows("等收纳箱窗口超时", app.pid, include_desktop=True)
        mine = [w.describe() for w in M.windows_of(app.pid, layer=None, include_desktop=True)]
        logs = [l for l in app.log_text().splitlines() if "[fence]" in l][-12:]
        raise AssertionError(f"{e}；本进程的窗口 {mine}；[fence] 日志 {logs}") from None
    print(f"    {time.time() - t0:.1f}s 后有 {len(ws)} 个收纳箱窗口：{[w.describe() for w in ws]}")
    return ws


def at(win: WinInfo, rel: tuple[float, float]) -> tuple[float, float]:
    return (win.x + rel[0], win.y + rel[1])


def fence_body_point(win: WinInfo, dy_from_bottom: float | None = None) -> tuple[float, float]:
    """内容区里的一点：默认内容区中心（拖入的落点），给了 dy_from_bottom 时取离底边这么远的一点（空白处）"""
    if dy_from_bottom is not None:
        return (win.x + win.w / 2, win.y + win.h - dy_from_bottom)
    return (win.x + win.w / 2, win.y + FENCE_TITLE_BAR + (win.h - FENCE_TITLE_BAR) / 2)


def clear_main_off(app: App, fence: WinInfo) -> WinInfo:
    """主窗口（layer 0）盖着收纳箱（桌面层）时，收纳箱被盖住的部分点不到、拖不进：把主窗口拖到收纳箱左边（放不下再往下挪）。
    1024×768 的 runner 上居中的主窗口与右上角的收纳箱有重叠；1920×1080 时不重叠、什么都不做。返回收纳箱的最新位置"""
    main = main_win(app)
    if main is None or not M.overlaps(main, fence, 6):
        return find_fence(app, fence.number) or fence
    s = C.screen
    dx = max(s.vis_x + 2, fence.x - main.w - 30) - main.x
    dy = 0.0
    if main.x + dx + main.w > fence.x - 6:
        dy = fence.y + fence.h + 20 - main.y
    print(f"    主窗口 {main.describe()} 盖着收纳箱，拖开 Δ=({dx:+.0f},{dy:+.0f})")
    bring_to_front(app, main)
    x0, y0 = main.x + main.w / 2, main.y + 6
    M.drag(x0, y0, x0 + dx, y0 + dy)
    time.sleep(0.8)
    m2 = main_win(app)
    print(f"    主窗口现在 {m2.describe() if m2 else '不在屏幕上'}")
    if m2 is not None and M.overlaps(m2, fence, 6):
        print("    !! 主窗口仍盖着收纳箱：后面点收纳箱 / 拖入可能落到主窗口上")
    return find_fence(app, fence.number) or fence


def check_fence_state(ck: Checks, app: App, win: WinInfo, level: int, what: str = "收纳箱") -> dict | None:
    """核对应用日志里自报的窗口属性（collectionBehavior / canHide 等在 CGWindowList 里读不到）"""
    try:
        st = M.wait_until(lambda: next((kv for n, kv in reversed(fence_states(app)) if n == win.number), None), 10,
                          f"日志里窗口 #{win.number} 的桌面层自报")
    except M.Timeout:
        ck.ok(False, f"{what}：日志里有窗口 #{win.number} 的桌面层自报「[fence] 收纳箱窗口 #… 已放到桌面层」"
                     f"（日志里有的窗口号：{[n for n, _ in fence_states(app)]}）")
        return None
    print(f"    {what}自报：{st}")
    try:
        lv = int(st.get("level", ""))
    except ValueError:
        lv = None
    try:
        b = int(st.get("behavior", ""), 16)
    except ValueError:
        b = -1
    ck.ok(lv == level + 1, f"{what}：自报层级 level={st.get('level')} == 桌面图标层 + 1（{level + 1}）")
    ck.ok(b >= 0 and (b & FENCE_BEHAVIOR) == FENCE_BEHAVIOR,
          f"{what}：behavior={st.get('behavior')} 含 CanJoinAllSpaces | Stationary | IgnoresCycle | FullScreenNone"
          f"（{FENCE_BEHAVIOR:#x}）")
    ck.ok(b >= 0 and (b & FENCE_BEHAVIOR_FORBIDDEN) == 0,
          f"{what}：behavior={st.get('behavior')} 不含 MoveToActiveSpace / Managed / Transient / FullScreenAuxiliary")
    ck.ok(st.get("canHide") == "false", f"{what}：canHide={st.get('canHide')}（期望 false：⌘H 不隐藏）")
    ck.ok(st.get("panel") == "true" and st.get("nonactivating") == "true",
          f"{what}：不激活应用的面板（panel={st.get('panel')} nonactivating={st.get('nonactivating')}）")
    ck.ok(st.get("hidesOnDeactivate") == "false", f"{what}：hidesOnDeactivate={st.get('hidesOnDeactivate')}（期望 false）")
    ck.ok(st.get("excludedFromWindowsMenu") == "true",
          f"{what}：excludedFromWindowsMenu={st.get('excludedFromWindowsMenu')}（期望 true）")
    return st


def fence_rows(app: App) -> list[tuple]:
    return app.q("SELECT id, title, folder_name, x, y, width, height FROM fences ORDER BY sort_order, id")


def first_fence(app: App, env: FenceEnv) -> tuple[int, Path]:
    rows = M.wait_until(lambda: fence_rows(app), 10, "fences 表里有收纳箱")
    return int(rows[0][0]), env.root / rows[0][2]


def fence_item_rows(app: App, fid: int) -> list[tuple]:
    return app.q("SELECT file_name, original_path, hidden FROM fence_items WHERE fence_id = ? ORDER BY sort_order",
                 (fid,))


def same_file(a, b) -> bool:
    return a is not None and b is not None and os.path.realpath(str(a)) == os.path.realpath(str(b))


def fence_ocr(win: WinInfo, name: str) -> tuple[Image.Image, float, list]:
    img, scale, _ = shoot(win, name)
    lines = M.ocr(img, C.work / f"{name}-ocr-input.png", bg=(0, 0, 0))
    (C.out / f"{name}-ocr.txt").write_text(
        "\n".join(f"({l.x:.0f},{l.y:.0f}) {l.conf:.2f} {l.text}" for l in lines) + "\n", encoding="utf-8")
    return img, scale, lines


def ocr_point(win: WinInfo, name: str, label: str, exact: bool = False) -> tuple[float, float] | None:
    """收纳箱截图里 OCR 找文字（exact：整行就是它），返回该行中心的全局坐标；找不到 None"""
    try:
        _, scale, lines = fence_ocr(win, name)
    except AssertionError as e:
        print(f"    OCR 找「{label}」失败：{e}")
        return None
    want = M.norm(label)
    hit = next((l for l in lines if (M.norm(l.text) == want if exact else want in M.norm(l.text))), None)
    texts = [l.text for l in lines[:16]]
    if hit is None:
        print(f"    OCR 没找到「{label}」：{texts}")
        return None
    pt = (win.x + (hit.x + hit.w / 2) / scale, win.y + (hit.y + hit.h / 2) / scale)
    print(f"    OCR 找到「{label}」（{hit.text!r}）@ ({pt[0]:.0f},{pt[1]:.0f})")
    return pt


def click_label(win: WinInfo, name: str, label: str, fallback: tuple[float, float], exact: bool = False) -> str:
    """点收纳箱里的一项文字：OCR 找得到就点它（且在窗口内），否则点按布局常量算出的位置（全局坐标 fallback）"""
    pt = ocr_point(win, name, label, exact)
    if pt is not None and win.x <= pt[0] <= win.x + win.w and win.y <= pt[1] <= win.y + win.h:
        M.click(*pt)
        return f"OCR ({pt[0]:.0f},{pt[1]:.0f})"
    M.click(*fallback)
    return f"布局常量 ({fallback[0]:.0f},{fallback[1]:.0f})"


class DragSource:
    """drag_source.py 子进程：一个可以按下拖动的小窗口（拖放内容是给定的文件），stdout 每行一个 JSON 事件"""

    def __init__(self, paths: list[Path], x: float, y: float, w: float, h: float, mode: str, tag: str):
        cmd = [sys.executable, "-u", str(DRAG_SOURCE), "--x", f"{x:.0f}", "--y", f"{y:.0f}", "--w", f"{w:.0f}",
               "--h", f"{h:.0f}", "--mode", mode, "--timeout", "90"] + [str(p) for p in paths]
        print(f"    拖放源（{mode}）：{' '.join(cmd[2:])}")
        self.proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                                     text=True, encoding="utf-8", errors="replace", start_new_session=True)
        M.ALL_PROCS.append(self.proc)
        self.tag = tag
        self.lines: list[str] = []
        self.events: list[dict] = []
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self) -> None:
        assert self.proc.stdout is not None
        for line in self.proc.stdout:
            line = line.rstrip("\n")
            self.lines.append(line)
            try:
                ev = json.loads(line)
                if isinstance(ev, dict):
                    self.events.append(ev)
            except ValueError:
                pass

    def event(self, name: str, timeout: float) -> dict | None:
        deadline = time.time() + timeout
        while time.time() < deadline:
            ev = next((e for e in self.events if e.get("event") == name), None)
            if ev is not None:
                return ev
            if self.proc.poll() is not None and not any(e.get("event") == name for e in self.events):
                time.sleep(0.2)
                return next((e for e in self.events if e.get("event") == name), None)
            time.sleep(0.1)
        return None

    def window(self) -> WinInfo | None:
        ws = M.windows_of(self.proc.pid, layer=None, include_desktop=True)
        return max(ws, key=lambda w: w.w * w.h) if ws else None

    def stop(self) -> None:
        if self.proc.poll() is None:
            M._killpg(self.proc, signal.SIGTERM)
            try:
                self.proc.wait(3)
            except subprocess.TimeoutExpired:
                M._killpg(self.proc, signal.SIGKILL)
                try:
                    self.proc.wait(3)
                except subprocess.TimeoutExpired:
                    pass
        brief = [{k: v for k, v in e.items() if k != "trace"} for e in self.events]
        print(f"    拖放源事件：{brief}")
        for e in self.events:
            if e.get("trace"):
                print(f"    拖放源异常：\n{e['trace']}")
        junk = [l for l in self.lines if not l.startswith("{")]
        if junk:
            print(f"    拖放源其它输出：{junk[-12:]}")


def helper_spot(fence: WinInfo, w: float, h: float) -> tuple[float, float]:
    """拖放源窗口的位置：不能盖住收纳箱（落点）。优先屏幕左下，其次左上，再次收纳箱下方"""
    s = C.screen
    cands = [(s.vis_x + 20, s.vis_y + s.vis_h - h - 20), (s.vis_x + 20, s.vis_y + 20),
             (fence.x, min(fence.y + fence.h + 20, s.vis_y + s.vis_h - h - 5))]
    for x, y in cands:
        if not (x - 10 < fence.x + fence.w and fence.x - 10 < x + w and y - 10 < fence.y + fence.h
                and fence.y - 10 < y + h):
            return x, y
    return cands[0]


def drag_into_fence(app: App, fence: WinInfo, paths: list[Path], mode: str, tag: str) -> dict:
    """起一个拖放源窗口，CGEvent 从它上面按下、拖到收纳箱内容区中心松开。返回拖放源的 ended 事件（没有时 None）"""
    fence = find_fence(app, fence.number) or fence
    w, h = 160.0, 120.0
    x, y = helper_spot(fence, w, h)
    src = DragSource(paths, x, y, w, h, mode, tag)
    try:
        ready = src.event("ready", 25)
        if ready is None:
            raise AssertionError(f"拖放源没有就绪（exit={src.proc.poll()}）：{src.lines[-12:]}")
        try:
            win = M.wait_until(src.window, 10, "拖放源窗口出现在屏幕上")
        except M.Timeout as e:
            raise AssertionError(f"{e}；拖放源输出 {src.lines[-12:]}") from None
        print(f"    拖放源窗口：{win.describe()}")
        M.park_cursor()
        tx, ty = fence_body_point(fence)
        M.drag_and_drop(win.x + win.w / 2, win.y + win.h / 2, tx, ty)
        ended = src.event("ended", 6)
        print(f"    拖放结束：{ended}（operation 0 = 没被接受，1 = Copy；GPUI 对外部拖入回 Copy，移动由应用自己做）")
        return {"ended": ended, "mouse_down": src.event("mouseDown", 0.1) is not None}
    finally:
        src.stop()


def wait_moved(src: Path, dest: Path, timeout: float) -> bool:
    try:
        M.wait_until(lambda: dest.exists() and not src.exists(), timeout, f"{src} → {dest}")
        return True
    except M.Timeout:
        return False


def fence_case_end(env: FenceEnv | None, *apps: App | None) -> None:
    for a in apps:
        if a is not None:
            a.dump_logs(25)
            a.stop()
    if env is not None:
        for d in (env.root, env.desktop, env.src):
            if d.exists():
                listing = sorted(str(p.relative_to(d)) for p in d.rglob("*"))
                print(f"    {d.name}/：{listing[:30]}")
    M.dump_windows("用例结束后")


def mf01() -> str:
    """新建（fence:new，与菜单栏图标「新增收纳箱」同一实现）：桌面层窗口、位置、行为自报、库与文件夹、截图 OCR"""
    M.dump_windows("用例开始前", include_desktop=True)
    env = fence_start("MF01")
    app = env.app
    try:
        ck = Checks()
        wait_for(app, MAIN_TITLE, MAIN_SIZE)
        fence = wait_fences(app)[0]
        time.sleep(1.5)
        fence = find_fence(app, fence.number) or fence
        M.dump_windows("收纳箱出现后（含桌面元素）", app.pid, include_desktop=True)
        level = M.desktop_icon_level()
        print(f"    桌面图标层 CGWindowLevelForKey(kCGDesktopIconWindowLevelKey) = {level}；收纳箱期望 {level + 1}")
        ck.ok(fence.layer == level + 1, f"收纳箱窗口 kCGWindowLayer {fence.layer} == 桌面图标层 + 1（{level + 1}）")
        negs = fence_windows(app)
        ck.ok(len(negs) == 1, f"本进程只有一个收纳箱窗口（{[w.describe() for w in negs]}）")
        ws = M.list_windows(include_desktop=True)
        idx = next((i for i, w in enumerate(ws) if w.number == fence.number), -1)
        behind_normal = [w.describe() for w in ws[idx + 1:] if w.layer >= 0] if idx >= 0 else ["（列表里没有收纳箱）"]
        ck.ok(not behind_normal, f"收纳箱排在所有 layer ≥ 0 的窗口之后（之后的普通窗口：{behind_normal[:5]}）")
        finder_icons = [w.describe() for w in ws if w.layer == level]
        print(f"    桌面图标层的窗口（Finder，只打印）：{finder_icons}")
        if idx >= 0:
            below_icons = [w for w in ws[:idx] if w.layer == level]
            print(f"    排在收纳箱之前的桌面图标层窗口（应为空，只打印）：{[w.describe() for w in below_icons]}")
        ex, ey, ew, eh = expected_fence_rect()
        print(f"    期望（工作区右上角）：({ex:.0f},{ey:.0f} {ew:.0f}×{eh:.0f})；{C.screen.describe()}")
        ck.near(round(fence.w), ew, FENCE_POS_TOL, "收纳箱宽（pt）")
        ck.near(round(fence.h), eh, FENCE_POS_TOL, "收纳箱高（pt）")
        ck.near(round(fence.x + fence.w), ex + ew, FENCE_POS_TOL, "收纳箱右边（pt，= 工作区右边 - 24）")
        ck.near(round(fence.y), ey, FENCE_POS_TOL, "收纳箱顶边（pt，= 工作区顶边 + 24）")
        s = C.screen
        ck.ok(fence.x >= s.vis_x - 1 and fence.y >= s.vis_y - 1 and fence.x + fence.w <= s.vis_x + s.vis_w + 1
              and fence.y + fence.h <= s.vis_y + s.vis_h + 1, "收纳箱整个在屏幕可见区域（visibleFrame）内")
        print(f"    kCGWindowName = {fence.name!r}（有屏幕录制权限时是窗口标题）")
        rows = fence_rows(app)
        print(f"    fences：{rows}")
        ck.ok(len(rows) == 1 and rows[0][1] == FENCE_TITLE, f"fences 表一行、标题「{FENCE_TITLE}」（{rows}）")
        if rows:
            fid, _, folder_name, x, y, w, h = rows[0]
            folder = env.root / folder_name
            ck.ok(folder.is_dir(), f"收纳文件夹建在 MINI_TODO_FENCES_ROOT 下：{folder}")
            unit = M.unit_scale()
            ck.ok(abs(x - fence.x * unit) <= 2 * unit and abs(y - fence.y * unit) <= 2 * unit
                  and abs(w - fence.w * unit) <= 2 * unit and abs(h - fence.h * unit) <= 2 * unit,
                  f"库里的几何 ({x},{y} {w}×{h}) = 窗口 × 统一系数 {unit:g}")
            layouts = app.q("SELECT screen_config_id, x, y, width, height FROM fence_layouts WHERE fence_id = ?", (fid,))
            print(f"    fence_layouts：{layouts}")
            ck.ok(len(layouts) == 1 and tuple(layouts[0][1:]) == (x, y, w, h), "记下了当前屏幕组合的布局（与 fences 相同）")
        check_fence_state(ck, app, fence, level)
        M.park_cursor()
        time.sleep(0.8)
        img, _, stats, _, found, how = ocr_until(
            app, fence, "MF01-fence", any_title([FENCE_TITLE, FENCE_EMPTY_HINT]), 30, bg=(0, 0, 0),
            refresh_fn=refresh_fence)
        ck.ok(M.has_content(stats), f"收纳箱截图有内容 {stats}")
        ck.ok(found, f"OCR 认出收纳箱的标题 / 空状态提示：{how}")
        M.save_emit(M.flatten(img, (0x1F, 0x6F, 0x8B)), C.out, "MF01-fence", max_w=300, colors=16)
        check_no_panic(ck, app)
        return ck.done(f"layer {fence.layer}；({fence.x:.0f},{fence.y:.0f} {fence.w:.0f}×{fence.h:.0f})；{how}")
    finally:
        fence_case_end(env, app)


def mf02() -> str:
    """目录监听：往收纳文件夹里写文件 → FSEvents → 对账 → fence_items 补行、收纳箱界面刷新"""
    M.dump_windows("用例开始前")
    env = fence_start("MF02")
    app = env.app
    try:
        ck = Checks()
        wait_for(app, MAIN_TITLE, MAIN_SIZE)
        fence = wait_fences(app)[0]
        fid, folder = first_fence(app, env)
        time.sleep(2.5)
        M.park_cursor()
        time.sleep(0.5)
        fence = refresh_fence(app, fence)
        before, _, _ = shoot(fence, "MF02-before")
        target = folder / "watch.txt"
        target.write_text("written into the fence folder by e2e\n", encoding="utf-8")
        t0 = time.time()
        try:
            rows = M.wait_until(lambda: [r for r in fence_item_rows(app, fid) if r[0] == "watch.txt"], 15,
                                "fence_items 里有 watch.txt")
            ck.ok(True, f"{time.time() - t0:.1f}s 后 fence_items 有 watch.txt：{rows}")
            ck.ok(rows[0][1] is None, f"直接放进收纳文件夹的项没有原始路径（original_path={rows[0][1]!r}）")
        except M.Timeout as e:
            ck.ok(False, f"写进收纳文件夹后 fence_items 补行：{e}（现有 {fence_item_rows(app, fid)}）")
        last = [0.0]

        def changed():
            img, _, _ = shoot(refresh_fence(app, fence), "MF02-after")
            last[0] = M.diff_ratio(before, img)
            return last[0] > 0.005

        try:
            M.wait_until(changed, 10, "收纳箱界面变化", interval=1.0)
            ck.ok(True, f"收纳箱界面随之刷新（{last[0]:.1%} 的像素变了）")
        except M.Timeout as e:
            ck.ok(False, f"收纳箱界面随之刷新：{e}（变化 {last[0]:.2%}）")
        _, _, lines = fence_ocr(refresh_fence(app, fence), "MF02-after")
        hit, how = M.fuzzy_find("watch.txt", lines, min_len=5)
        print(f"    OCR 认出 watch.txt（只打印）：{hit}，{how}；{[l.text for l in lines[:10]]}")
        check_no_panic(ck, app)
        return ck.done(f"监听 {time.time() - t0:.1f}s")
    finally:
        fence_case_end(env, app)


def mf03() -> str:
    """真实拖入：拖放源进程（NSDraggingItem / 文件 NSURL）→ CGEvent 拖到收纳箱松开 → 文件移进收纳文件夹、记下原始路径；
    同名再拖一个 → a (2).txt，原有的 a.txt 不被覆盖"""
    M.dump_windows("用例开始前")
    env = fence_start("MF03")
    app = env.app
    try:
        ck = Checks()
        wait_for(app, MAIN_TITLE, MAIN_SIZE)
        fence = wait_fences(app)[0]
        fid, folder = first_fence(app, env)
        time.sleep(2)
        fence = clear_main_off(app, fence)
        a1 = env.src / "a.txt"
        a1.write_text("drag 1\n", encoding="utf-8")
        src2 = C.work / f"{env.tag}-src2"
        shutil.rmtree(src2, ignore_errors=True)
        src2.mkdir(parents=True)
        a2 = src2 / "a.txt"
        a2.write_text("drag 2\n", encoding="utf-8")

        r1 = drag_into_fence(app, fence, [a1], "items", "MF03-1")
        moved1 = wait_moved(a1, folder / "a.txt", 12)
        img, _, lines = fence_ocr(refresh_fence(app, fence), "MF03-after-drop")
        print(f"    拖入后收纳箱 OCR：{[l.text for l in lines[:12]]}")
        if not moved1:
            print(f"    第一次拖入没生效（拖放源按下={r1['mouse_down']}、ended={r1['ended']}）；换旧式 NSFilenamesPboardType 诊断一次")
            b1 = env.src / "b.txt"
            b1.write_text("drag legacy\n", encoding="utf-8")
            drag_into_fence(app, refresh_fence(app, fence), [b1], "legacy", "MF03-legacy")
            moved_b = wait_moved(b1, folder / "b.txt", 12)
            ck.ok(False, f"真实拖入（NSDraggingItem / public.file-url）：{a1} 没有移进 {folder}（拖放源按下="
                         f"{r1['mouse_down']}，ended={r1['ended']}；旧式 NSFilenamesPboardType 拖放"
                         f"{'生效了' if moved_b else '也没生效'}）")
            M.save_emit(M.flatten(img, (0x1F, 0x6F, 0x8B)), C.out, "MF03-after-drop", max_w=300, colors=12)
            return ck.done()
        ck.ok(True, f"拖入后 {a1.name} 离开原目录、进了收纳文件夹")
        ck.ok((folder / "a.txt").read_text(encoding="utf-8") == "drag 1\n", "内容不变")
        try:
            row = M.wait_until(lambda: [r for r in fence_item_rows(app, fid) if r[0] == "a.txt" and r[1]], 8,
                               "fence_items 记下 a.txt 的原始路径")
            ck.ok(same_file(row[0][1], a1), f"fence_items.original_path {row[0][1]!r} = 拖入前的路径 {a1}")
        except M.Timeout as e:
            ck.ok(False, f"fence_items 记下原始路径：{e}（{fence_item_rows(app, fid)}）")

        time.sleep(1.5)
        r2 = drag_into_fence(app, refresh_fence(app, fence), [a2], "items", "MF03-2")
        moved2 = wait_moved(a2, folder / "a (2).txt", 12)
        ck.ok(moved2, f"同名再拖入一个 → a (2).txt（ended={r2['ended']}；收纳文件夹 {sorted(p.name for p in folder.iterdir())}）")
        ck.ok((folder / "a.txt").read_text(encoding="utf-8") == "drag 1\n", "原有的 a.txt 没被覆盖")
        if moved2:
            ck.ok((folder / "a (2).txt").read_text(encoding="utf-8") == "drag 2\n", "a (2).txt 是第二个文件的内容")
            try:
                row2 = M.wait_until(lambda: [r for r in fence_item_rows(app, fid) if r[0] == "a (2).txt" and r[1]], 8,
                                    "fence_items 记下 a (2).txt 的原始路径")
                ck.ok(same_file(row2[0][1], a2), f"a (2).txt 的原始路径 {row2[0][1]!r} = {a2}")
            except M.Timeout as e:
                ck.ok(False, f"fence_items 记下 a (2).txt：{e}（{fence_item_rows(app, fid)}）")
        M.park_cursor()
        time.sleep(1)
        shoot(refresh_fence(app, fence), "MF03-final")
        print(f"    fence_items：{fence_item_rows(app, fid)}")
        check_no_panic(ck, app)
        return ck.done(f"{sorted(p.name for p in folder.iterdir())}")
    finally:
        fence_case_end(env, app)


def restore_cell0(app: App, fence: WinInfo, name: str) -> str:
    """右键第一个项目 → 右键菜单「移回桌面」"""
    fence = refresh_fence(app, fence)
    px, py = at(fence, FENCE_CELL0)
    M.click(px, py, "right")
    time.sleep(1.0)
    fallback = (px + FENCE_ITEM_MENU_RESTORE[0], py + FENCE_ITEM_MENU_RESTORE[1])
    return click_label(refresh_fence(app, fence), f"{name}-menu", "移回桌面", fallback)


def mf04() -> str:
    """右键「移回桌面」：有原始路径的回原目录；没有原始路径的回"桌面"（MINI_TODO_DESKTOP_DIR）；fence_items 行删掉"""
    M.dump_windows("用例开始前")

    def prepare(env: FenceEnv) -> int:
        return seed_fence(env, FENCE_TITLE, expected_fence_rect(),
                          [("r1.txt", "back to its folder\n", env.src / "r1.txt"), ("d1.txt", "to the desktop\n", None)])

    env = fence_start("MF04", "main,fence:{fence_id}", prepare)
    app = env.app
    try:
        ck = Checks()
        wait_for(app, MAIN_TITLE, MAIN_SIZE)
        fence = wait_fences(app)[0]
        fid, folder = first_fence(app, env)
        time.sleep(2.5)
        fence = clear_main_off(app, fence)
        M.park_cursor()
        _, _, lines = fence_ocr(refresh_fence(app, fence), "MF04-before")
        print(f"    收纳箱 OCR：{[l.text for l in lines[:12]]}；fence_items {fence_item_rows(app, fid)}")

        how = restore_cell0(app, fence, "MF04-r1")
        ok1 = wait_moved(folder / "r1.txt", env.src / "r1.txt", 10)
        ck.ok(ok1, f"「移回桌面」（{how}）→ r1.txt 回到原目录 {env.src}")
        if ok1:
            ck.ok((env.src / "r1.txt").read_text(encoding="utf-8") == "back to its folder\n", "内容不变")
            try:
                M.wait_until(lambda: not [r for r in fence_item_rows(app, fid) if r[0] == "r1.txt"], 6, "r1.txt 的行删掉")
                ck.ok(True, "fence_items 里 r1.txt 的行删掉")
            except M.Timeout as e:
                ck.ok(False, f"fence_items 里 r1.txt 的行删掉：{e}（{fence_item_rows(app, fid)}）")
        else:
            shoot(refresh_fence(app, fence), "MF04-after-r1")
        time.sleep(3.5)         # 等「已移回 …」提示消失，第二个项目挪到第一格
        M.park_cursor()
        how2 = restore_cell0(app, fence, "MF04-d1")
        ok2 = wait_moved(folder / "d1.txt", env.desktop / "d1.txt", 10)
        ck.ok(ok2, f"「移回桌面」（{how2}）→ 没有原始路径的 d1.txt 回到桌面（MINI_TODO_DESKTOP_DIR={env.desktop}）")
        if ok2:
            try:
                M.wait_until(lambda: not fence_item_rows(app, fid), 6, "fence_items 清空")
                ck.ok(True, "fence_items 清空")
            except M.Timeout as e:
                ck.ok(False, f"fence_items 清空：{e}（{fence_item_rows(app, fid)}）")
        ck.ok(folder.is_dir() and not [p for p in folder.iterdir() if p.name != ".DS_Store"],
              f"收纳文件夹还在、已经空了（{sorted(p.name for p in folder.iterdir()) if folder.is_dir() else '不存在'}）")
        check_no_panic(ck, app)
        return ck.done(f"r1 {how} / d1 {how2}")
    finally:
        fence_case_end(env, app)


def mf05() -> str:
    """「更多」→「删除收纳箱」→「删除」：项目全部移回（原目录 / 桌面）、.DS_Store 不挡删除、收纳文件夹删掉、记录删掉、窗口关闭"""
    M.dump_windows("用例开始前")

    def prepare(env: FenceEnv) -> int:
        return seed_fence(env, FENCE_TITLE, expected_fence_rect(),
                          [("k1.txt", "keep 1\n", env.src / "k1.txt"), ("k2.txt", "keep 2\n", None)],
                          extra_files=(".DS_Store",))

    env = fence_start("MF05", "main,fence:{fence_id}", prepare)
    app = env.app
    try:
        ck = Checks()
        wait_for(app, MAIN_TITLE, MAIN_SIZE)
        fence = wait_fences(app)[0]
        _, folder = first_fence(app, env)
        time.sleep(2.5)
        fence = clear_main_off(app, fence)
        M.click(*at(fence, FENCE_MORE_BTN))
        time.sleep(0.8)
        how = click_label(refresh_fence(app, fence), "MF05-more-menu", "删除收纳箱", at(fence, FENCE_MENU_DELETE))
        print(f"    点「删除收纳箱」：{how}")
        time.sleep(1.5)          # 删除确认面板统计项目数（统计完「删除」才可点）
        M.park_cursor()
        fence = refresh_fence(app, fence)
        _, _, lines = fence_ocr(fence, "MF05-confirm")
        texts = [l.text for l in lines]
        print(f"    确认面板 OCR：{texts[:14]}")
        hint, hint_how = M.fuzzy_find("其中的 2 个项目会移回桌面", lines)
        print(f"    说明「其中的 2 个项目会移回桌面」（.DS_Store 不计入；只打印）：{hint}，{hint_how}")
        cands = []
        pt = ocr_point(fence, "MF05-confirm-btn", "删除", exact=True)
        if pt is not None:
            cands.append(("OCR", pt))
        cands += [(f"布局常量 {rel}", at(fence, rel)) for rel in FENCE_DELETE_BTN]
        gone = False
        tried = []
        for label, (x, y) in cands:
            tried.append(label)
            M.click(x, y)
            try:
                M.wait_until(lambda: not on_screen(app, fence.number), 4, "收纳箱窗口关闭")
                gone = True
                break
            except M.Timeout:
                continue
        if not gone:
            try:
                M.wait_until(lambda: not on_screen(app, fence.number), 10, "收纳箱窗口关闭")
                gone = True
            except M.Timeout:
                shoot(refresh_fence(app, fence), "MF05-not-deleted")
        ck.ok(gone, f"点「删除」（试过 {tried}）后收纳箱窗口关闭")
        try:
            M.wait_until(lambda: not fence_rows(app), 6, "fences 表清空")
            ck.ok(True, "fences 表里的记录删掉")
        except M.Timeout as e:
            ck.ok(False, f"fences 表里的记录删掉：{e}（{fence_rows(app)}）")
        ck.ok((env.src / "k1.txt").is_file(), f"k1.txt 移回原目录 {env.src}")
        ck.ok((env.desktop / "k2.txt").is_file(), f"没有原始路径的 k2.txt 移回桌面（{env.desktop}）")
        ck.ok(not folder.exists(), f"收纳文件夹删掉了（.DS_Store 不算剩余；{folder}）")
        strays = [p.name for p in env.desktop.iterdir() if p.name.startswith(".DS_Store")]
        ck.ok(not strays, f".DS_Store 没有被搬到桌面（{strays}）")
        ck.ok(not fence_windows(app), "本进程没有收纳箱窗口了")
        check_no_panic(ck, app)
        return ck.done(f"删除按钮 {tried[-1] if tried else '?'}")
    finally:
        fence_case_end(env, app)


def mf06() -> str:
    """⌘H / 别的应用发起的隐藏：主窗口藏起来、收纳箱留在桌面（canHide = NO）；应用隐藏中点收纳箱：不激活应用、不取消隐藏"""
    M.dump_windows("用例开始前")
    env = fence_start("MF06")
    app = env.app
    try:
        ck = Checks()
        wait_for(app, MAIN_TITLE, MAIN_SIZE)
        fence = wait_fences(app)[0]
        time.sleep(2)
        fence = clear_main_off(app, fence)
        front = bring_to_front(app, main_win(app))
        ck.ok(front, "⌘H 之前本应用在前台")
        via = send_cmd_key(app, M.KEY_H, front)
        try:
            M.wait_until(lambda: main_win(app) is None, 8, "主窗口藏起来")
            ck.ok(True, f"⌘H（{via}）后主窗口不在屏幕上")
        except M.Timeout as e:
            ck.ok(False, f"⌘H（{via}）后主窗口藏起来：{e}")
        time.sleep(1)
        print(f"    NSRunningApplication.isHidden = {app.is_hidden()}；前台 {M.describe_pid(M.frontmost_pid())}")
        M.dump_windows("⌘H 之后（含桌面元素）", app.pid, include_desktop=True)
        ck.ok(on_screen(app, fence.number), "⌘H 之后收纳箱仍在屏幕上（canHide = NO）")

        fp = M.activate_other("Finder")
        if fp == app.pid or fp is None:
            print(f"    没能把 Finder 切到前台（前台 {M.describe_pid(fp)}），跳过「点收纳箱不激活应用」")
        else:
            fence = refresh_fence(app, fence)
            M.click(*fence_body_point(fence, dy_from_bottom=40))
            time.sleep(1.5)
            fp2 = M.frontmost_pid()
            ck.ok(fp2 != app.pid, f"应用隐藏中点收纳箱：前台没有变成本应用（前台 {M.describe_pid(fp2)}；不激活应用的面板）")
            ck.ok(main_win(app) is None, "点收纳箱没有把隐藏中的应用取消隐藏（主窗口仍不在屏幕上）")
            ck.ok(on_screen(app, fence.number), "点过之后收纳箱仍在")

        print(f"    NSRunningApplication.unhide → {app.unhide_via_appkit()}")
        try:
            M.wait_until(lambda: main_win(app) is not None, 8, "取消隐藏后主窗口回来")
            ck.ok(True, "取消隐藏后主窗口回到屏幕上")
        except M.Timeout as e:
            ck.ok(False, f"取消隐藏后主窗口回来：{e}")
        time.sleep(1)
        print(f"    NSRunningApplication.hide（别的应用发起的隐藏）→ {app.hide_via_appkit()}")
        try:
            M.wait_until(lambda: main_win(app) is None, 8, "主窗口藏起来")
            ck.ok(True, "NSRunningApplication.hide 后主窗口不在屏幕上")
        except M.Timeout as e:
            ck.ok(False, f"NSRunningApplication.hide 后主窗口藏起来：{e}")
        time.sleep(1)
        ck.ok(on_screen(app, fence.number), "别的应用发起的隐藏之后收纳箱仍在屏幕上")
        app.unhide_via_appkit()
        check_no_panic(ck, app)
        return ck.done(via)
    finally:
        fence_case_end(env, app)


def mf07() -> str:
    """有收纳箱可见时 ⌘W 关主窗口 → open <app>（Dock / 访达唤起）→ 主窗口回来（收纳箱是 NSPanel，不算"有可见窗口"）"""
    M.dump_windows("用例开始前")
    env = fence_start("MF07")
    app = env.app
    try:
        ck = Checks()
        win = wait_for(app, MAIN_TITLE, MAIN_SIZE)
        fence = wait_fences(app)[0]
        time.sleep(2)
        front = bring_to_front(app, win)
        ck.ok(front, "⌘W 之前本应用在前台")
        via = send_cmd_key(app, M.KEY_W, front)
        try:
            M.wait_until(lambda: main_win(app) is None, 8, "主窗口关闭")
            ck.ok(True, f"⌘W（{via}）后主窗口关闭")
        except M.Timeout as e:
            ck.ok(False, f"⌘W（{via}）后主窗口关闭：{e}")
            return ck.done()
        time.sleep(2)
        ck.ok(on_screen(app, fence.number), "⌘W 不关收纳箱（收纳箱仍在屏幕上）")
        r = M.run(["open", str(C.app)], timeout=30)
        print(f"    open {C.app} → rc={r.returncode} {(r.stdout + r.stderr).strip()!r}")
        try:
            w2 = wait_for(app, MAIN_TITLE, MAIN_SIZE, timeout=20, strict=True)
            ck.ok(True, f"有收纳箱可见时 open（Dock / 访达唤起）后主窗口重新出现：{w2.describe()}")
        except M.Timeout as e:
            ck.ok(False, f"有收纳箱可见时 open 后主窗口重新出现：{e}（收纳箱若被 AppKit 算作可见窗口，on_reopen 不会回调）")
        time.sleep(1)
        pids = pids_of_exe()
        ck.ok(pids == [app.pid], f"open 没有另起一个进程（{pids}）")
        ck.ok(len(fence_windows(app)) == 1 and on_screen(app, fence.number), "收纳箱仍是同一个窗口、没有多出来")
        check_no_panic(ck, app)
        return ck.done(via)
    finally:
        fence_case_end(env, app)


def mf08() -> str:
    """应用不在前台时双击收纳箱标题改名 → 键盘（系统输入流）进得了收纳箱面板 → 标题与收纳文件夹改名。
    改名开始时应用会被激活（`start_rename`，菜单栏切到 mini-todo），面板成为 key。输入同 M09：先逐字输入
    （CGEventKeyboardSetUnicodeString，runner 上进不了 GPUI 输入框——普通窗口也一样），OCR 认不出就 pbcopy + ⌘A + ⌘V；
    回车同样走系统输入流（HID），证明系统键盘事件到得了面板"""
    M.dump_windows("用例开始前")
    env = fence_start("MF08")
    app = env.app
    try:
        ck = Checks()
        wait_for(app, MAIN_TITLE, MAIN_SIZE)
        fence = wait_fences(app)[0]
        _, folder = first_fence(app, env)
        time.sleep(2)
        fence = clear_main_off(app, fence)
        fp = M.activate_other("Finder")
        print(f"    改名前前台：{M.describe_pid(fp)}（改名从本应用不在前台时开始）")
        M.double_click(*at(refresh_fence(app, fence), FENCE_TITLE_TEXT))
        time.sleep(0.8)
        print(f"    双击标题后前台（只打印；改名会激活应用，预期是 mini-todo）：{M.describe_pid(M.frontmost_pid())}")
        fence = refresh_fence(app, fence)
        shoot(fence, "MF08-editing")
        M.type_text(FENCE_RENAMED)
        time.sleep(0.5)
        img_typed, _, lines = fence_ocr(refresh_fence(app, fence), "MF08-typed")
        typed, how = M.fuzzy_find(FENCE_RENAMED, lines)
        print(f"    逐字输入后收纳箱 OCR：{how}；{[l.text for l in lines[:8]]}")
        method = "CGEventKeyboardSetUnicodeString"
        if not typed:
            print("    逐字输入没认出来，改用剪贴板：⌘A → ⌘V（系统输入流）")
            M.set_clipboard(FENCE_RENAMED)
            M.post_key_hid(M.KEY_A, M.cmd_flag())
            M.post_key_hid(M.KEY_V, M.cmd_flag())
            time.sleep(0.8)
            img_typed, _, lines = fence_ocr(refresh_fence(app, fence), "MF08-pasted")
            pasted, how = M.fuzzy_find(FENCE_RENAMED, lines)
            print(f"    粘贴后收纳箱 OCR（只打印，以落库为准）：{pasted}，{how}；{[l.text for l in lines[:8]]}")
            method = "pbcopy + ⌘A + ⌘V"
        M.post_key_hid(M.KEY_RETURN)
        renamed = False
        try:
            M.wait_until(lambda: [r for r in fence_rows(app) if r[1] == FENCE_RENAMED], 8, f"fences.title = {FENCE_RENAMED}")
            renamed = True
        except M.Timeout:
            pass
        fp_after = M.frontmost_pid()
        print(f"    改名后前台（只打印；改名时应用被激活，预期是 mini-todo）：{M.describe_pid(fp_after)}")
        if not renamed:
            rows = fence_rows(app)
            M.save_emit(M.flatten(img_typed, (0x1F, 0x6F, 0x8B)), C.out, "MF08-typed", max_w=300, colors=12)
            # 诊断：键盘事件直接投递给进程（CGEventPostToPid）能不能改名——区分"面板没成为 key"与"按键没路由到本进程"
            M.double_click(*at(refresh_fence(app, fence), FENCE_TITLE_TEXT))
            time.sleep(0.8)
            M.type_text_to_pid(app.pid, FENCE_RENAMED)
            time.sleep(0.3)
            M.post_key_to_pid(app.pid, M.KEY_RETURN, 0)
            try:
                M.wait_until(lambda: [r for r in fence_rows(app) if r[1] == FENCE_RENAMED], 6, "PostToPid 改名")
                diag = "直接投递给进程（CGEventPostToPid）可以改名：系统输入流的按键（粘贴 / 回车）没路由到收纳箱面板"
            except M.Timeout:
                diag = "直接投递给进程（CGEventPostToPid）也改不了：面板没进入改名 / 没成为 key"
            ck.ok(False, f"系统输入流（{method} + 回车）输入的新标题进了收纳箱面板并落库：fences {rows}；"
                         f"前台 {M.describe_pid(fp_after)}；{diag}")
            return ck.done()
        ck.ok(True, f"双击标题 → {method} 输入「{FENCE_RENAMED}」→ 回车（系统输入流）→ fences.title 落库")
        rows = fence_rows(app)
        new_folder = env.root / FENCE_RENAMED
        ck.ok(rows and rows[0][2] == FENCE_RENAMED, f"folder_name 跟着改名（{rows}）")
        ck.ok(new_folder.is_dir() and not folder.exists(), f"收纳文件夹改名：{folder.name} → {new_folder.name}")
        time.sleep(0.5)
        fence = refresh_fence(app, fence)
        print(f"    kCGWindowName = {fence.name!r}（只打印：有屏幕录制权限时应是新标题）")
        ck.ok(fence.layer == M.desktop_icon_level() + 1, f"改名（面板成为 key）之后层级不变：{fence.layer}")
        M.park_cursor()
        _, _, _, _, found, how = ocr_until(app, fence, "MF08-renamed", any_title([FENCE_RENAMED]), 15, bg=(0, 0, 0),
                                            refresh_fn=refresh_fence)
        print(f"    OCR 认出新标题（只打印）：{found}，{how}")
        check_no_panic(ck, app)
        return ck.done(f"{FENCE_RENAMED}；输入方式 {method}；前台 {M.describe_pid(fp_after)}")
    finally:
        fence_case_end(env, app)


def drag_fence_title(app: App, fence: WinInfo, dx: float, dy: float) -> tuple[WinInfo, tuple[float, float]]:
    fence = refresh_fence(app, fence)
    x0, y0 = at(fence, FENCE_TITLE_DRAG)
    M.drag(x0, y0, x0 + dx, y0 + dy, steps=20)
    time.sleep(1.2)          # 松手后等最后一帧落地（约 16ms）再落库
    f2 = refresh_fence(app, fence)
    d = (f2.x - fence.x, f2.y - fence.y)
    print(f"    拖标题栏 ({dx:+.0f},{dy:+.0f}) → 收纳箱 {fence.x:.0f},{fence.y:.0f} → {f2.x:.0f},{f2.y:.0f}，"
          f"Δ=({d[0]:+.0f},{d[1]:+.0f})")
    return f2, d


def mf09() -> str:
    """拖标题栏（自绘拖动：按下后移动超过 3pt 窗口才跟手，每帧 setFrameOrigin:；只有一个收纳箱，没有吸附目标）→ 窗口移动 →
    松手后 fences 与当前屏幕组合的 fence_layouts 同时落库（= 窗口位置 × 统一系数）"""
    M.dump_windows("用例开始前")
    env = fence_start("MF09")
    app = env.app
    try:
        ck = Checks()
        wait_for(app, MAIN_TITLE, MAIN_SIZE)
        fence = wait_fences(app)[0]
        fid, _ = first_fence(app, env)
        time.sleep(2)
        fence = clear_main_off(app, fence)
        before = app.q("SELECT x, y FROM fences WHERE id = ?", (fid,))[0]
        layouts_before = app.q("SELECT screen_config_id, x, y FROM fence_layouts WHERE fence_id = ?", (fid,))
        print(f"    拖动前 fences ({before[0]},{before[1]})；fence_layouts {layouts_before}")
        dx, dy = -160.0, 90.0
        f2, d = drag_fence_title(app, fence, dx, dy)
        ck.ok(abs(d[0] - dx) <= 30 and abs(d[1] - dy) <= 30, f"拖标题栏移动了收纳箱 Δ=({d[0]:+.0f},{d[1]:+.0f})（期望约 ({dx:+.0f},{dy:+.0f})）")
        ck.ok(f2.layer == fence.layer, f"拖动后层级不变（{f2.layer}）")
        unit = M.unit_scale()
        want = (f2.x * unit, f2.y * unit)

        def committed():
            r = app.q("SELECT x, y FROM fences WHERE id = ?", (fid,))[0]
            moved = (r[0], r[1]) != (before[0], before[1])
            return r if moved and abs(r[0] - want[0]) <= 2 * unit and abs(r[1] - want[1]) <= 2 * unit else None

        try:
            r = M.wait_until(committed, 6, "fences.x/y = 拖动后的位置")
            ck.ok(True, f"fences.x/y 落库 ({r[0]},{r[1]}) = 窗口 ({f2.x:.0f},{f2.y:.0f}) × {unit:g}")
            layouts = app.q("SELECT screen_config_id, x, y FROM fence_layouts WHERE fence_id = ?", (fid,))
            print(f"    fence_layouts：{layouts}")
            ck.ok(any((x, y) == (r[0], r[1]) for _, x, y in layouts), "当前屏幕组合的 fence_layouts 同时更新")
        except M.Timeout as e:
            now = app.q("SELECT x, y FROM fences WHERE id = ?", (fid,))[0]
            ck.ok(False, f"拖动后几何落库：{e}（现在 ({now[0]},{now[1]})，拖动前 ({before[0]},{before[1]})，"
                         f"期望约 ({want[0]:.0f},{want[1]:.0f})）")
        check_no_panic(ck, app)
        return ck.done(f"Δ=({d[0]:+.0f},{d[1]:+.0f})")
    finally:
        fence_case_end(env, app)


def mf10() -> str:
    """菜单栏图标三项：「新增收纳箱」→ 第二个收纳箱；「显示/隐藏所有收纳箱」两次；「重置收纳箱位置」→ 工作区右上角并排；
    「退出」后不带 MINI_TODO_DEBUG_OPEN 重启 → 两个收纳箱在原位恢复（restore_all）"""
    M.dump_windows("用例开始前")
    s = C.screen
    seeded_rect = (s.vis_x + 40, s.vis_y + 60, FENCE_SIZE[0], FENCE_SIZE[1])

    def prepare(env: FenceEnv) -> int:
        return seed_fence(env, FENCE_TITLE, seeded_rect, [])

    env = fence_start("MF10", "main,fence:{fence_id}", prepare)
    app = env.app
    app2 = None
    try:
        ck = Checks()
        level = M.desktop_icon_level()
        wait_for(app, MAIN_TITLE, MAIN_SIZE)
        fa = wait_fences(app)[0]
        ck.near(round(fa.x), seeded_rect[0], FENCE_POS_TOL, "预置的收纳箱按库里的位置打开：x（pt）")
        ck.near(round(fa.y), seeded_rect[1], FENCE_POS_TOL, "预置的收纳箱按库里的位置打开：y（pt）")
        time.sleep(2)

        M.press_status_menu_item(app.pid, "新增收纳箱")
        try:
            ws = wait_fences(app, 2, 20)
        except AssertionError as e:
            ck.ok(False, f"菜单栏图标「新增收纳箱」后出现第二个收纳箱：{e}")
            return ck.done()
        fb = next(w for w in ws if w.number != fa.number)
        rows = fence_rows(app)
        print(f"    fences：{rows}")
        ck.ok(sorted(r[1] for r in rows) == sorted([FENCE_TITLE, FENCE_TITLE_2]),
              f"第二个收纳箱标题「{FENCE_TITLE_2}」（{[r[1] for r in rows]}）")
        ck.ok((env.root / FENCE_TITLE_2).is_dir(), f"建了收纳文件夹「{FENCE_TITLE_2}」")
        ex, ey, ew, _ = expected_fence_rect()
        ck.near(round(fb.x + fb.w), ex + ew, FENCE_POS_TOL, "新收纳箱右边（pt，工作区右边 - 24）")
        ck.near(round(fb.y), ey, FENCE_POS_TOL, "新收纳箱顶边（pt，工作区顶边 + 24）")
        ck.ok(fb.layer == level + 1, f"新收纳箱也在桌面层（{fb.layer}）")
        check_fence_state(ck, app, fb, level, "新收纳箱")
        numbers = [fa.number, fb.number]

        M.press_status_menu_item(app.pid, "显示/隐藏所有收纳箱")
        try:
            M.wait_until(lambda: not any(on_screen(app, n) for n in numbers), 8, "收纳箱全部隐藏")
            ck.ok(True, "「显示/隐藏所有收纳箱」→ 两个收纳箱都不在屏幕上")
        except M.Timeout as e:
            ck.ok(False, f"「显示/隐藏所有收纳箱」后全部隐藏：{e}（{[w.describe() for w in fence_windows(app)]}）")
        time.sleep(0.8)
        M.press_status_menu_item(app.pid, "显示/隐藏所有收纳箱")
        try:
            M.wait_until(lambda: all(on_screen(app, n) for n in numbers), 8, "收纳箱全部显示")
            ck.ok(True, "再点一次 → 两个收纳箱回到屏幕上")
            back = [refresh_fence(app, w) for w in (fa, fb)]
            ck.ok(all(w.layer == level + 1 for w in back), f"重新显示后仍在桌面层（{[w.layer for w in back]}）")
        except M.Timeout as e:
            ck.ok(False, f"再点「显示/隐藏所有收纳箱」后全部显示：{e}")

        M.press_status_menu_item(app.pid, "重置收纳箱位置")
        reset: list[WinInfo] = []

        def arranged():
            cur = [find_fence(app, n) for n in numbers]
            if any(w is None for w in cur):
                return None
            cur.sort(key=lambda w: w.x)
            left, right = cur
            ok = (abs(right.x + right.w - (ex + ew)) <= FENCE_POS_TOL and all(abs(w.y - ey) <= FENCE_POS_TOL for w in cur)
                  and abs(left.x + left.w - (right.x - FENCE_GAP)) <= FENCE_POS_TOL)
            reset[:] = cur
            return cur if ok else None

        try:
            M.wait_until(arranged, 8, "重置后两个收纳箱在工作区右上角并排")
            ck.ok(True, f"「重置收纳箱位置」→ 右上角并排：{[w.describe() for w in reset]}")
        except M.Timeout as e:
            ck.ok(False, f"「重置收纳箱位置」后的位置：{e}（{[w.describe() for w in reset]}；期望最右一个右边 {ex + ew:.0f}、"
                         f"顶边 {ey:.0f}、间距 {FENCE_GAP:g}）")
        unit = M.unit_scale()

        def db_matches():
            rs = fence_rows(app)
            return rs if len(rs) == 2 and all(
                any(abs(r[3] - w.x * unit) <= 2 * unit and abs(r[4] - w.y * unit) <= 2 * unit for w in reset)
                for r in rs) else None

        try:
            M.wait_until(db_matches, 6, "重置后的位置落库")
            ck.ok(True, "重置后的位置落库（fences = 窗口位置 × 统一系数）")
        except M.Timeout as e:
            ck.ok(False, f"重置后的位置落库：{e}（{fence_rows(app)}）")
        reset_rects = [(w.x, w.y, w.w, w.h) for w in reset]

        t0 = time.time()
        M.press_status_menu_item(app.pid, "退出")
        if not app.wait_exit(10):
            print("    「退出」后 10s 进程仍在，SIGTERM")
        app.dump_logs(15)
        app.stop()
        print(f"    已退出（{time.time() - t0:.1f}s，exit={app.exit_code}）；不带 MINI_TODO_DEBUG_OPEN 重启")
        app2 = fence_app(env, f"{env.tag}-restart", None)
        wait_for(app2, MAIN_TITLE, MAIN_SIZE)
        try:
            ws2 = wait_fences(app2, 2, 45)
        except AssertionError as e:
            ck.ok(False, f"重启后恢复两个收纳箱（restore_all）：{e}")
            return ck.done()
        time.sleep(1)
        ws2 = [refresh_fence(app2, w) for w in ws2]
        for w in ws2:
            hit = any(abs(w.x - r[0]) <= FENCE_POS_TOL and abs(w.y - r[1]) <= FENCE_POS_TOL
                      and abs(w.w - r[2]) <= FENCE_POS_TOL and abs(w.h - r[3]) <= FENCE_POS_TOL for r in reset_rects)
            ck.ok(hit, f"重启后收纳箱 {w.describe()} 在退出前的位置（{[tuple(round(v) for v in r) for r in reset_rects]}）")
            ck.ok(w.layer == level + 1, f"重启后在桌面层（{w.layer}）")
            check_fence_state(ck, app2, w, level, "重启后的收纳箱")
        ck.ok(len(fence_rows(app2)) == 2, f"库里仍是两个收纳箱（{fence_rows(app2)}）")
        check_no_panic(ck, app2)
        return ck.done(f"重置后 {[tuple(round(v) for v in r[:2]) for r in reset_rects]}")
    finally:
        fence_case_end(env, app, app2)


SMOKE = [
    ("M01", "主窗口：尺寸、截图有内容、OCR 认出待办标题", m01, 180),
    ("M02", "设置窗口：尺寸、OCR 认出设置页文字", m02, 180),
    ("M03", "只读详情窗口：尺寸、OCR 认出待办标题", m03, 180),
    ("M04", "退出：quit 事件后进程退出、无 panic", m04, 180),
    ("M05", "⌘Q 退出（先切到前台）", m05, 180),
    ("M06", "App 菜单：应用 / 编辑 / 窗口菜单项，点「设置…」打开设置", m06, 180),
    ("M07", "菜单栏图标：菜单项、打开设置、退出", m07, 180),
    ("M08", "⌘W 关主窗口、open 唤起（on_reopen）", m08, 180),
    ("M09", "FAB 新建待办：输入、⌘Enter、落库、主窗口显示", m09, 240),
    ("M10", "固定模式：拖不动、改不了尺寸、is_fixed 落库；取消后恢复", m10, 240),
    ("MF01", "收纳箱新建：桌面图标层 + 1、右上角、行为自报、库与文件夹、截图 OCR", mf01, 180),
    ("MF02", "收纳箱目录监听：写进收纳文件夹 → fence_items 补行、界面刷新", mf02, 180),
    ("MF03", "收纳箱真实拖入（拖放源进程）：移进收纳文件夹、记原始路径、同名不覆盖", mf03, 300),
    ("MF04", "收纳箱右键「移回桌面」：回原目录 / 回桌面（MINI_TODO_DESKTOP_DIR）", mf04, 240),
    ("MF05", "删除收纳箱：全部移回、.DS_Store 不挡删除、文件夹与记录删掉、窗口关闭", mf05, 240),
    ("MF06", "⌘H / 隐藏后收纳箱仍在；隐藏中点收纳箱不激活应用", mf06, 240),
    ("MF07", "有收纳箱可见时 ⌘W 关主窗口、open 唤起主窗口", mf07, 180),
    ("MF08", "应用不在前台时双击标题改名（激活应用、系统输入流粘贴 + 回车）", mf08, 240),
    ("MF09", "拖标题栏移动收纳箱、几何落库（fences + fence_layouts）", mf09, 180),
    ("MF10", "菜单栏图标：新增 / 显示隐藏 / 重置收纳箱位置；退出后重启恢复", mf10, 300),
]
RELEASE = [
    ("P01", "release：窗口、真实数据库迁移、日志位置、单实例 socket", p01, 180),
    ("P02", "单实例：二次启动转交并退出、唤起已运行实例", p02, 180),
    ("P03", "开机自启：LaunchAgent plist 写入与删除", p03, 180),
    ("P04", "⌘Q 退出：socket 删除、无 panic", p04, 180),
]


# ----------------------------------------------------------------------------- 环境与入口

def print_env() -> None:
    print("::group::环境")
    print(f"python {sys.version.split()[0]} on {platform.platform()}")
    print(sh_or("sw_vers"))
    try:
        import objc  # type: ignore
        print(f"pyobjc {getattr(objc, '__version__', '?')}")
    except Exception as e:  # noqa: BLE001
        print(f"pyobjc 不可用：{e}")
    print(f"Quartz: {'OK' if M.Quartz else M.QUARTZ_ERR!r}；Vision: {'OK' if M.Vision else M.VISION_ERR!r}；"
          f"ApplicationServices: {'OK' if M.AS else M.AX_ERR!r}；AppKit: {'OK' if M.AppKit else M.APPKIT_ERR!r}")
    print(f"屏幕录制（CGPreflightScreenCaptureAccess）：{M.screen_capture_preflight()}")
    if M.AS is not None:
        try:
            print(f"辅助功能（AXIsProcessTrusted）：{bool(M.AS.AXIsProcessTrusted())}")
        except Exception as e:  # noqa: BLE001
            print(f"辅助功能：未知（{e}）")
    print(f"屏幕：{C.screen.describe()}；CGDisplayBounds {M.main_display_points()}")
    print(f"窗口期望尺寸（与可见区域取小）：主窗口 {exp(MAIN_SIZE)}、设置 {exp(SETTINGS_SIZE)}、编辑 / 详情 {exp(EDITOR_SIZE)}")
    print(f"桌面图标层 {M.desktop_icon_level()}（收纳箱期望 +1）；统一系数 {M.unit_scale():g}；"
          f"收纳箱默认位置（pt）{tuple(round(v) for v in expected_fence_rect())}")
    print(f"前台：{M.describe_pid(M.frontmost_pid())}")
    print(f"Vision 支持的识别语言：{M.vision_languages()}")
    print(f"app: {C.app}")
    print(sh_or("codesign", "-dv", "--verbose=2", str(C.app)))
    print(sh_or("plutil", "-p", str(C.app / "Contents" / "Info.plist")))
    print(sh_or("lipo", "-archs", str(C.exe)))
    print("::endgroup::", flush=True)


def sh_or(*cmd: str) -> str:
    return M.sh_out(list(cmd))


def set_wallpaper(hexcolor: str) -> None:
    """换纯色壁纸（透明窗口圆角外透出来的颜色可辨认）；失败只打印"""
    try:
        rgb = tuple(int(hexcolor[i:i + 2], 16) for i in (0, 2, 4))
        p = C.work / f"wallpaper-{hexcolor}.png"
        Image.new("RGB", (64, 64), rgb).save(p)
        script = f'tell application "System Events" to tell every desktop to set picture to "{p}"'
        r = M.run(["osascript", "-e", script], timeout=30)
        print(f"壁纸 #{hexcolor}: rc={r.returncode} {(r.stdout + r.stderr).strip()!r}")
    except Exception as e:  # noqa: BLE001
        print(f"换壁纸失败（不影响测试）：{e}")


def sweep_leftovers(include_kept: bool = False) -> None:
    """兜底：结束这个 .app 的 mini-todo 进程（release 组共用的实例除外，除非 include_kept）"""
    M.kill_all_procs(include_kept=include_kept)
    for pid in pids_of_exe():
        if not include_kept and pid in M.KEEP_PIDS:
            continue
        print(f"兜底结束残留进程 pid={pid}")
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--app", required=True, help="mini-todo.app（scripts/macos/bundle.sh 的产物）")
    ap.add_argument("--suite", choices=["smoke", "release"], default="smoke")
    ap.add_argument("--seed", help="seed 示例的可执行文件（target/debug/examples/seed；smoke 组需要）")
    ap.add_argument("--sample", default=str(HERE.parent / "sample-data.json"))
    ap.add_argument("--out", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--only", default="", help="逗号分隔的用例号前缀，如 M01,M09")
    ap.add_argument("--wallpaper", default="", help="换成这个颜色的纯色壁纸（RRGGBB，可选）")
    ap.add_argument("--strict-traffic-lights", action="store_true", help="红绿灯没隐藏时 M01 判失败")
    ap.add_argument("--allow-real-profile", action="store_true",
                    help="release 组：读写真实数据目录 / 日志 / LaunchAgent（只在一次性 VM 上用）")
    ap.add_argument("--case-timeout", type=int, default=0, help="覆盖每个用例的超时（秒）")
    a = ap.parse_args()

    C.app = Path(a.app).resolve()
    C.exe = C.app / "Contents" / "MacOS" / "mini-todo"
    C.seed_exe = Path(a.seed).resolve() if a.seed else None
    C.sample = Path(a.sample).resolve()
    C.out = Path(a.out).resolve()
    C.work = Path(a.work).resolve()
    C.strict_traffic = a.strict_traffic_lights
    C.allow_real = a.allow_real_profile
    shutil.rmtree(C.out, ignore_errors=True)
    C.out.mkdir(parents=True, exist_ok=True)
    C.work.mkdir(parents=True, exist_ok=True)

    doc = json.loads(C.sample.read_text(encoding="utf-8"))
    C.pending_titles = [t["title"] for t in doc.get("todos", []) if not t.get("completed")]
    C.titles_by_id = {int(t["id"]): t["title"] for t in doc.get("todos", []) if "id" in t}

    def on_signal(signum, frame):  # noqa: ARG001
        print(f"\n收到信号 {signum}，结束所有应用进程", flush=True)
        sweep_leftovers(include_kept=True)
        sys.exit(128 + signum)

    for s in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(s, on_signal)
    atexit.register(M.kill_all_procs)

    report = Report(C.out, f"pc-gpui e2e（macOS，{a.suite}）")
    if not C.exe.exists():
        print(f"::error::找不到 {C.exe}")
        report.results.append({"id": "M00", "title": "准备", "status": "FAIL",
                               "detail": f"找不到 {C.exe}", "seconds": 0})
        return report.finish()

    try:
        C.screen = M.screen_info()
    except Exception as e:  # noqa: BLE001
        print(f"读屏幕尺寸失败：{e}")
        C.screen = M.ScreenInfo(0, 0, 0, 0, 0, 0, 1.0)
    try:
        print_env()
    except Exception:  # noqa: BLE001
        import traceback
        print("打印环境信息出错（不影响用例）：")
        traceback.print_exc(file=sys.stdout)
        print("::endgroup::", flush=True)
    if a.wallpaper:
        set_wallpaper(a.wallpaper)

    cases = SMOKE if a.suite == "smoke" else RELEASE
    only = [x.strip() for x in a.only.split(",") if x.strip()]
    try:
        for cid, title, fn, timeout in cases:
            if only and not any(cid.startswith(o) for o in only):
                continue
            report.case(cid, title, fn, timeout=a.case_timeout or timeout)
            sweep_leftovers()
    finally:
        sweep_leftovers(include_kept=True)
    return report.finish()


if __name__ == "__main__":
    sys.exit(main())
