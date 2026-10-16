#!/usr/bin/env python3
"""GPUI 版 mini-todo 的 macOS e2e（GitHub macOS runner：已登录的图形会话，屏幕录制 / 辅助功能已预授权）。

    python run_e2e.py --app <mini-todo.app> --out <输出目录> --work <工作目录>
                      [--suite smoke|release] [--seed <seed 可执行文件>] [--only M01,M09]
                      [--wallpaper 1F6F8B] [--strict-traffic-lights] [--allow-real-profile] [--case-timeout 180]

- `--app` 是 `pc-gpui/scripts/macos/bundle.sh` 包好的 .app。直接执行 `<app>/Contents/MacOS/mini-todo`：环境变量能传进去，
  NSBundle.mainBundle 仍是这个 .app（bundle id 有效）
- **smoke**（debug 构建 + seed）：每个用例 seed 样例数据到 `<work>/<用例>/`，以 MINI_TODO_DATA_DIR 隔离启动（debug 开关只在
  debug 构建生效）；用例之间互相独立
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
| M07 | 菜单栏图标：菜单项齐全、没有收纳箱项；「打开设置」→ 设置窗口；「退出」→ 退出码 0 |
| M08 | ⌘W 关主窗口、进程仍在；`open <app>`（LaunchServices reopen）→ 主窗口重新出现、没有第二个进程 |
| M09 | 点 FAB → 新建窗口 → 输入"E2E 新建待办" → ⌘Enter → 落库（时间格式规范）→ 主窗口 OCR 认出 |
| M10 | 固定模式（菜单栏图标「固定模式」）：拖标题栏不动、AX 改尺寸无效、`is_fixed` 落库；取消后又能拖 |
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
import stat
import subprocess
import sys
import time
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
# services/tray.rs imp::start（收纳箱三项只在 Windows 上有）
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


def ocr_until(app: App, win: WinInfo, name: str, check, timeout: float = 30) -> tuple[Image.Image, float, dict, list, bool, str]:
    """反复截图 + OCR 直到 check(lines) 成立（数据是异步加载的，第一帧可能还是空列表）"""
    deadline = time.time() + timeout
    attempt = 0
    while True:
        attempt += 1
        win = refresh(app, win)
        img, scale, stats = shoot(win, name)
        lines = M.ocr(img, C.work / f"{name}-ocr-input.png")
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
        for bad in TRAY_FENCE_ITEMS:
            ck.ok(bad not in titles, f"没有收纳箱项「{bad}」")
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
