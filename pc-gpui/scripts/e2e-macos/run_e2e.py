#!/usr/bin/env python3
"""GPUI 版 mini-todo 的 macOS 冒烟 e2e（GitHub macOS runner：已登录的图形会话，屏幕录制 / 辅助功能已预授权）。

    python run_e2e.py --app <mini-todo.app> --seed <seed 可执行文件> --out <输出目录> --work <工作目录>
                      [--only M01,M03] [--wallpaper 1F6F8B] [--strict-traffic-lights] [--case-timeout 180]

- `--app` 是 `pc-gpui/scripts/macos/bundle.sh` 包好的 .app（debug 构建：MINI_TODO_DATA_DIR / MINI_TODO_DEBUG_OPEN 只在 debug 生效）。
  直接执行 `<app>/Contents/MacOS/mini-todo`：环境变量能传进去，NSBundle.mainBundle 仍是这个 .app（bundle id 有效）
- 每个用例：seed 样例数据到 `<work>/<用例>/` → 隔离启动 → 按 pid 用 CGWindowListCopyWindowInfo 等窗口 →
  `screencapture -l` 截窗口（断言确有内容，防"没有屏幕录制权限只截到壁纸"的假成功）→ Vision OCR 断言界面文字 → 结束进程
- 用例之间互相独立，每个用例有超时（SIGALRM），结束时一定杀掉本用例启动的进程；单个用例失败不影响后面的用例
- 输出：`<out>/<用例>-*.png`（截图）、`<out>/<用例>-*-ocr.txt`、`<out>/logs/`（应用 stdout 与日志）、`results.json`、`summary.md`、
  `fail-<用例>.txt` / `fail-<用例>.png`、`emit/*.png`（小图，由 `../e2e-windows/emit_annotations.py images` 作为注解发出）；
  汇总表也写进 `$GITHUB_STEP_SUMMARY`。任一 FAIL 时退出码为 1
- 窗口列表（owner pid / layer / bounds / name）、OCR 识别到的文字、AX 探测结果、应用日志尾部都打印到 stdout（CI 日志就是主要反馈）

用例：

| 用例 | 内容 |
|---|---|
| M01 | 主窗口：约 380×600pt、截图有内容、OCR 认出 sample-data 的一个未完成待办标题；整屏截图；红绿灯 / 应用菜单 / 菜单栏状态项（信息性） |
| M02 | `MINI_TODO_DEBUG_OPEN=settings`：设置窗口约 680×560pt、OCR 认出左侧菜单 / 面板标题（常规 / 外观 …） |
| M03 | `MINI_TODO_DEBUG_OPEN=editor:1:view`：只读详情约 1080×600pt、OCR 认出待办 #1 的标题 |
| M04 | 退出：`tell application id "com.tauri-app.mini-todo" to quit`（不行再 NSRunningApplication.terminate），15s 内退出、退出码 0；日志 / stdout 无 panic |
| M05 | ⌘Q：AX 读得到 App 菜单时，`CGEventPostToPid` 发 ⌘Q（不行再切到前台经系统输入流发），10s 内退出、退出码 0；还没有 App 菜单时 SKIP |
"""
from __future__ import annotations

import argparse
import atexit
import json
import os
import platform
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from PIL import Image  # noqa: E402

import maclib as M  # noqa: E402
from maclib import App, Checks, Report, Skip, WinInfo  # noqa: E402

BUNDLE_ID = "com.tauri-app.mini-todo"
MAIN_TITLE, MAIN_SIZE = "Mini Todo", (380, 600)          # windows/main_window.rs DEFAULT_WIDTH / DEFAULT_HEIGHT
SETTINGS_TITLE, SETTINGS_SIZE = "设置", (680, 560)        # windows/settings/mod.rs WIDTH / HEIGHT
DETAIL_TITLE, DETAIL_SIZE = "待办详情", (1080, 600)        # windows/editor/mod.rs os_title / WIDTH / HEIGHT
SIZE_TOL = 40
SETTINGS_LABELS = ["常规", "外观", "数据与同步", "屏幕配置", "关于"]   # SettingsTab::label
DETAIL_TODO_ID = 1


class Ctx:
    app: Path
    seed_exe: Path | None
    sample: Path
    out: Path
    work: Path
    strict_traffic: bool = False
    pending_titles: list[str] = []
    titles_by_id: dict[int, str] = {}


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


def size_match(w: WinInfo, size: tuple[int, int]) -> bool:
    return abs(w.w - size[0]) <= SIZE_TOL and abs(w.h - size[1]) <= SIZE_TOL


def pick(ws: list[WinInfo], title: str, size: tuple[int, int], exclude: tuple[str, ...] = ()) -> WinInfo | None:
    """按标题（有屏幕录制权限时 kCGWindowName 可读）找窗口，读不到标题时按尺寸"""
    for w in ws:
        if w.name == title:
            return w
    for w in ws:
        if w.name in exclude:
            continue
        if size_match(w, size):
            return w
    return None


def wait_for(app: App, title: str, size: tuple[int, int], timeout: float = 60,
             exclude: tuple[str, ...] = ()) -> WinInfo:
    t0 = time.time()
    try:
        w = app.wait_windows(lambda ws: pick(ws, title, size, exclude), timeout, f"窗口 '{title}'（约 {size[0]}×{size[1]}pt）")
    except M.Timeout:
        M.dump_windows("等窗口超时", app.pid)
        others = app.windows()
        if others:
            # 尺寸对不上（例如 Retina 换算错）时仍给出本进程的窗口，交给用例做软断言、继续截图与 OCR
            print(f"    按标题 / 尺寸没等到 '{title}'，本进程有 {len(others)} 个 layer 0 窗口，取最大的一个继续")
            return max(others, key=lambda x: x.w * x.h)
        raise
    print(f"    {time.time() - t0:.1f}s 后出现：{w.describe()}")
    return w


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


# ----------------------------------------------------------------------------- 用例

def m01() -> str:
    M.dump_windows("用例开始前")
    app = start("M01")
    try:
        ck = Checks()
        win = wait_for(app, MAIN_TITLE, MAIN_SIZE)
        M.dump_windows("主窗口出现后", app.pid)
        print(f"    NSRunningApplication：{app.running_app_info()}")
        ck.near(round(win.w), MAIN_SIZE[0], SIZE_TOL, "主窗口宽（pt）")
        ck.near(round(win.h), MAIN_SIZE[1], SIZE_TOL, "主窗口高（pt）")
        if win.name is not None:
            ck.ok(win.name == MAIN_TITLE, f"主窗口标题 {win.name!r} == {MAIN_TITLE!r}")
        else:
            print("    kCGWindowName 读不到（没有屏幕录制权限？）")
        time.sleep(2)
        img, scale, stats, lines, found, how = ocr_until(app, win, "M01-main", any_title(C.pending_titles), 40)
        ck.ok(M.has_content(stats), f"主窗口截图有内容 {stats}")
        ck.ok(found, f"OCR 认出未完成待办标题：{how}")
        # 红绿灯（信息性；--strict-traffic-lights 时判失败）
        emit_zoom_topleft(img, scale, "M01-topleft-x3")
        M.save_emit(img, C.out, "M01-main", max_w=380, colors=24)
        ax = M.ax_probe(app.pid)
        tl = traffic_info(img, scale, ax)
        if C.strict_traffic:
            ck.ok(ax["traffic"] == "隐藏", f"红绿灯已隐藏（{tl}）")
        screen = M.capture_screen(C.out / "M01-screen.png")
        print(f"    整屏截图 M01-screen.png: {screen.width}x{screen.height}，主显示器 {M.main_display_points()} pt")
        M.save_emit(screen, C.out, "M01-screen", max_w=560, colors=16)
        # bundle 身份：从 .app 里启动，NSRunningApplication 应认出 bundle id
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
        ck.near(round(win.w), SETTINGS_SIZE[0], SIZE_TOL, "设置窗口宽（pt）")
        ck.near(round(win.h), SETTINGS_SIZE[1], SIZE_TOL, "设置窗口高（pt）")
        time.sleep(1.5)

        def labels(lines):
            hits = [t for t in SETTINGS_LABELS if M.fuzzy_find(t, lines, ratio=1.0, min_len=2)[0]]
            return bool(hits), (f"认出 {hits}" if hits else f"{SETTINGS_LABELS} 一个都没认出")

        img, scale, stats, lines, found, how = ocr_until(app, win, "M02-settings", labels, 30)
        ck.ok(M.has_content(stats), f"设置窗口截图有内容 {stats}")
        ck.ok(found, f"OCR 认出设置页文字：{how}")
        M.save_emit(img, C.out, "M02-settings", max_w=480, colors=24)
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
        win = wait_for(app, DETAIL_TITLE, DETAIL_SIZE, exclude=(MAIN_TITLE,))
        M.dump_windows("详情窗口出现后", app.pid)
        ck.near(round(win.w), DETAIL_SIZE[0], SIZE_TOL, "详情窗口宽（pt）")
        ck.near(round(win.h), DETAIL_SIZE[1], SIZE_TOL, "详情窗口高（pt）")
        time.sleep(1.5)
        img, scale, stats, lines, found, how = ocr_until(app, win, "M03-detail", any_title([title]), 30)
        ck.ok(M.has_content(stats), f"详情窗口截图有内容 {stats}")
        ck.ok(found, f"OCR 认出待办 #{DETAIL_TODO_ID} 的标题 '{title}'：{how}")
        M.save_emit(img, C.out, "M03-detail", max_w=540, colors=24)
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
    """⌘Q：应用有 App 菜单（AX 读得到 Apple 菜单之外的顶层菜单）时才断言；还没有菜单时 SKIP 并说明"""
    M.dump_windows("用例开始前")
    app = start("M05")
    try:
        ck = Checks()
        wait_for(app, MAIN_TITLE, MAIN_SIZE)
        time.sleep(3)
        ax = M.ax_probe(app.pid)
        menus = [m for m in ax["menus"] if m not in ("Apple", "", "?")]
        if not menus:
            raise Skip(f"AX 读不到 App 菜单（顶层菜单 {ax['menus']}；没有菜单或没有辅助功能权限），⌘Q 无从生效")
        t0 = time.time()
        via = M.post_key_to_pid(app.pid, M.KEY_Q, M.Quartz.kCGEventFlagMaskCommand)
        exited = app.wait_exit(10)
        if not exited:
            print(f"    {via} 发 ⌘Q 后 10s 进程仍在；尝试切到前台后经系统输入流再发一次")
            print(f"    activate() → {app.activate()}，前台进程 {M.frontmost_pid()}（本进程 {app.pid}）")
            time.sleep(1)
            if M.frontmost_pid() == app.pid:
                via = M.post_key_hid(M.KEY_Q, M.Quartz.kCGEventFlagMaskCommand)
                exited = app.wait_exit(10)
            else:
                print("    本进程不在前台，不发系统级按键（避免按到别的程序上）")
        secs = time.time() - t0
        print(f"    ⌘Q（{via}）→ {'已退出' if exited else '未退出'}，{secs:.1f}s，exit code={app.exit_code}")
        ck.ok(exited, f"⌘Q 后进程退出（{via}，{secs:.1f}s）")
        if exited:
            ck.ok(app.exit_code == 0, f"退出码 {app.exit_code} == 0")
        app.stop()
        check_no_panic(ck, app)
        return ck.done(f"{via}，{secs:.1f}s，菜单 {ax['menus']}")
    finally:
        app.dump_logs(20)
        app.stop()
        M.dump_windows("用例结束后")


CASES = [
    ("M01", "主窗口：尺寸、截图有内容、OCR 认出待办标题", m01),
    ("M02", "设置窗口：尺寸、OCR 认出设置页文字", m02),
    ("M03", "只读详情窗口：尺寸、OCR 认出待办标题", m03),
    ("M04", "退出：quit 事件后进程退出、无 panic", m04),
    ("M05", "⌘Q 退出（有 App 菜单时）", m05),
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
    print(f"主显示器：{M.main_display_points()} pt")
    print(f"Vision 支持的识别语言：{M.vision_languages()}")
    print(f"app: {C.app}")
    print(sh_or("codesign", "-dv", "--verbose=2", str(C.app)))
    print(sh_or("plutil", "-p", str(C.app / "Contents" / "Info.plist")))
    print(sh_or("lipo", "-archs", str(C.app / "Contents" / "MacOS" / "mini-todo")))
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


def sweep_leftovers() -> None:
    """兜底：结束所有从这个 .app 启动、却不在本脚本记录里的 mini-todo 进程"""
    M.kill_all_procs()
    exe = str(C.app / "Contents" / "MacOS" / "mini-todo")
    r = subprocess.run(["pgrep", "-f", exe], capture_output=True, text=True)
    pids = [int(x) for x in r.stdout.split() if x.strip().isdigit() and int(x) != os.getpid()]
    for pid in pids:
        print(f"兜底结束残留进程 pid={pid}")
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--app", required=True, help="mini-todo.app（scripts/macos/bundle.sh 的产物）")
    ap.add_argument("--seed", help="seed 示例的可执行文件（target/debug/examples/seed）")
    ap.add_argument("--sample", default=str(HERE.parent / "sample-data.json"))
    ap.add_argument("--out", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--only", default="", help="逗号分隔的用例号前缀，如 M01,M03")
    ap.add_argument("--wallpaper", default="", help="换成这个颜色的纯色壁纸（RRGGBB，可选）")
    ap.add_argument("--strict-traffic-lights", action="store_true", help="红绿灯没隐藏时 M01 判失败")
    ap.add_argument("--case-timeout", type=int, default=180)
    a = ap.parse_args()

    C.app = Path(a.app).resolve()
    C.seed_exe = Path(a.seed).resolve() if a.seed else None
    C.sample = Path(a.sample).resolve()
    C.out = Path(a.out).resolve()
    C.work = Path(a.work).resolve()
    C.strict_traffic = a.strict_traffic_lights
    shutil.rmtree(C.out, ignore_errors=True)
    C.out.mkdir(parents=True, exist_ok=True)
    C.work.mkdir(parents=True, exist_ok=True)

    doc = json.loads(C.sample.read_text(encoding="utf-8"))
    C.pending_titles = [t["title"] for t in doc.get("todos", []) if not t.get("completed")]
    C.titles_by_id = {int(t["id"]): t["title"] for t in doc.get("todos", []) if "id" in t}

    def on_signal(signum, frame):  # noqa: ARG001
        print(f"\n收到信号 {signum}，结束所有应用进程", flush=True)
        sweep_leftovers()
        sys.exit(128 + signum)

    for s in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(s, on_signal)
    atexit.register(M.kill_all_procs)

    report = Report(C.out, "pc-gpui e2e（macOS）")
    if not (C.app / "Contents" / "MacOS" / "mini-todo").exists():
        print(f"::error::找不到 {C.app}/Contents/MacOS/mini-todo")
        report.results.append({"id": "M00", "title": "准备", "status": "FAIL",
                               "detail": f"找不到 {C.app}/Contents/MacOS/mini-todo", "seconds": 0})
        return report.finish()

    try:
        print_env()
    except Exception:  # noqa: BLE001
        import traceback
        print("打印环境信息出错（不影响用例）：")
        traceback.print_exc(file=sys.stdout)
        print("::endgroup::", flush=True)
    if a.wallpaper:
        set_wallpaper(a.wallpaper)

    only = [x.strip() for x in a.only.split(",") if x.strip()]
    try:
        for cid, title, fn in CASES:
            if only and not any(cid.startswith(o) for o in only):
                continue
            report.case(cid, title, fn, timeout=a.case_timeout)
            sweep_leftovers()
    finally:
        sweep_leftovers()
    return report.finish()


if __name__ == "__main__":
    sys.exit(main())
