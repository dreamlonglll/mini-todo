#!/usr/bin/env python3
"""e2e 用的文件拖放源（收纳箱 MF03 真实拖入）：一个只有一块橙色视图的小窗口，在它上面按下鼠标就把给定的文件当作拖放内容
开始拖动，之后由 run_e2e.py 用 CGEvent 把鼠标拖到收纳箱上松开——收纳箱收到的是与从访达拖进来同一条系统拖放路径
（NSDraggingSession → 目标窗口的 draggingEntered / performDragOperation）。

    python3 drag_source.py --x 20 --y 600 [--w 200 --h 150] [--mode items|legacy] [--timeout 90] <文件> [<文件> ...]

- `--x/--y/--w/--h`：窗口位置与尺寸，全局点坐标、主屏左上为原点（与 CGWindowList / CGEvent 同一套）
- `--mode items`（默认）：每个文件一个 `NSDraggingItem`（写入者是文件 NSURL，即 `public.file-url`；访达现在就这么拖）；
  `legacy`：旧式 `dragImage:at:offset:event:pasteboard:source:slideBack:` + 拖放剪贴板上直接声明 `NSFilenamesPboardType`
  （GPUI 读的就是这个类型；只作诊断：items 方式没生效时看是不是类型桥接的问题）
- 每个事件在 stdout 打一行 JSON：`ready`（窗口号、位置、pid）、`mouseDown`、`session`（拖放剪贴板上的类型）、`ended`
  （松手时目标接受的操作：0 = 没被接受，1 = Copy，16 = Move…）、`error`
- 窗口在浮动层级（盖在普通窗口之上），视图 `acceptsFirstMouse:` 返回 YES：本进程不在前台时第一下按下也直接开始拖动
- `--timeout` 秒后进程被 SIGALRM 默认动作结束（父进程也会在用完后结束它）

依赖：pyobjc（AppKit 随 pyobjc-framework-Quartz 的依赖 pyobjc-framework-Cocoa 装上）。
"""
from __future__ import annotations

import argparse
import json
import os
import signal
import sys
import traceback

import AppKit  # type: ignore
import objc  # type: ignore
from Foundation import NSURL, NSMakeRect, NSMakeSize  # type: ignore

PATHS: list[str] = []
MODE = "items"
OPS = int(AppKit.NSDragOperationCopy | AppKit.NSDragOperationMove | AppKit.NSDragOperationGeneric)


def out(**kw) -> None:
    print(json.dumps(kw, ensure_ascii=False), flush=True)


def drag_icon():
    icon = AppKit.NSWorkspace.sharedWorkspace().iconForFile_(PATHS[0])
    icon.setSize_(NSMakeSize(48, 48))
    return icon


def start_items(view, event) -> None:
    """现代方式：每个文件一个 NSDraggingItem（NSURL 自己写 public.file-url；旧类型由剪贴板服务桥接）"""
    icon = drag_icon()
    items = []
    for i, p in enumerate(PATHS):
        url = NSURL.fileURLWithPath_(p)
        item = AppKit.NSDraggingItem.alloc().initWithPasteboardWriter_(url)
        item.setDraggingFrame_contents_(NSMakeRect(20 + 6 * i, 20 + 6 * i, 48, 48), icon)
        items.append(item)
    session = view.beginDraggingSessionWithItems_event_source_(items, event, view)
    try:
        session.setAnimatesToStartingPositionsOnCancelOrFail_(False)
    except Exception:  # noqa: BLE001
        pass
    types = [str(t) for t in (session.draggingPasteboard().types() or [])]
    out(event="session", mode="items", types=types)


def start_legacy(view, event) -> None:
    """旧方式：拖放剪贴板直接声明 NSFilenamesPboardType（路径数组）；dragImage 同步跑完整个拖放"""
    name = getattr(AppKit, "NSPasteboardNameDrag", None) or getattr(AppKit, "NSDragPboard", "Apple CFPasteboard drag")
    pb = AppKit.NSPasteboard.pasteboardWithName_(name)
    kind = getattr(AppKit, "NSFilenamesPboardType", "NSFilenamesPboardType")
    pb.declareTypes_owner_([kind], None)
    pb.setPropertyList_forType_(PATHS, kind)
    out(event="session", mode="legacy", types=[str(t) for t in (pb.types() or [])])
    loc = view.convertPoint_fromView_(event.locationInWindow(), None)
    view.dragImage_at_offset_event_pasteboard_source_slideBack_(
        drag_icon(), loc, NSMakeSize(0, 0), event, pb, view, False)


class DragSourceView(AppKit.NSView):
    def acceptsFirstMouse_(self, event):
        return True

    def drawRect_(self, rect):
        AppKit.NSColor.orangeColor().set()
        AppKit.NSRectFill(self.bounds())

    def mouseDown_(self, event):
        p = event.locationInWindow()
        out(event="mouseDown", x=float(p.x), y=float(p.y), mode=MODE)
        try:
            if MODE == "legacy":
                start_legacy(self, event)
            else:
                start_items(self, event)
        except Exception as e:  # noqa: BLE001
            out(event="error", where="mouseDown", error=f"{type(e).__name__}: {e}",
                trace=traceback.format_exc()[-1500:])

    # NSDraggingSource（旧式 dragImage 在 10.7 之后也走拖放会话，同样问这两个方法）
    @objc.typedSelector(b"Q@:@q")
    def draggingSession_sourceOperationMaskForDraggingContext_(self, session, context):
        return OPS

    @objc.typedSelector(b"v@:@{CGPoint=dd}Q")
    def draggingSession_endedAtPoint_operation_(self, session, point, operation):
        out(event="ended", operation=int(operation), x=float(point.x), y=float(point.y))


def main() -> int:
    global PATHS, MODE
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--x", type=float, required=True)
    ap.add_argument("--y", type=float, required=True)
    ap.add_argument("--w", type=float, default=200)
    ap.add_argument("--h", type=float, default=150)
    ap.add_argument("--mode", choices=["items", "legacy"], default="items")
    ap.add_argument("--timeout", type=int, default=90)
    ap.add_argument("paths", nargs="+")
    a = ap.parse_args()
    PATHS = [os.path.realpath(p) for p in a.paths]
    MODE = a.mode
    missing = [p for p in PATHS if not os.path.exists(p)]
    if missing:
        out(event="error", where="args", error=f"文件不存在：{missing}")
        return 2
    # 默认动作（结束进程）：NSApp.run() 期间 Python 层的信号处理函数不一定有机会执行
    signal.signal(signal.SIGALRM, signal.SIG_DFL)
    signal.alarm(max(5, a.timeout))

    app = AppKit.NSApplication.sharedApplication()
    app.setActivationPolicy_(AppKit.NSApplicationActivationPolicyAccessory)
    screens = AppKit.NSScreen.screens()
    primary_h = float(screens[0].frame().size.height) if screens else 0.0
    # AppKit 坐标左下为原点：窗口底边 = 主屏高 - (顶边 + 高)
    rect = NSMakeRect(a.x, primary_h - a.y - a.h, a.w, a.h)
    style = getattr(AppKit, "NSWindowStyleMaskBorderless", 0)
    win = AppKit.NSWindow.alloc().initWithContentRect_styleMask_backing_defer_(
        rect, style, AppKit.NSBackingStoreBuffered, False)
    win.setLevel_(AppKit.NSFloatingWindowLevel)
    win.setTitle_("mini-todo e2e drag source")
    win.setReleasedWhenClosed_(False)
    view = DragSourceView.alloc().initWithFrame_(NSMakeRect(0, 0, a.w, a.h))
    win.setContentView_(view)
    win.orderFrontRegardless()
    try:
        app.activateIgnoringOtherApps_(True)
    except Exception:  # noqa: BLE001
        pass
    out(event="ready", window=int(win.windowNumber()), pid=os.getpid(), mode=MODE,
        frame=[a.x, a.y, a.w, a.h], paths=PATHS)
    app.run()
    return 0


if __name__ == "__main__":
    sys.exit(main())
