"""GPUI 版界面的操作封装（"页面对象"）。坐标以真实窗口布局为准（缩放 1，逻辑像素 = 物理像素），
能用 OCR 找到文字的地方都按文字点击，只有图标按钮与输入框按布局常量点击。

布局来源：`crates/app/src/windows/**`（主窗口 380×600、编辑窗口 1080×600、设置 680×560），
数值在 Xvfb 截图上核对过（见 README.md「实现要点」）。
"""
from __future__ import annotations

import time

from harness import (DAV, DAV_PASS, DAV_USER, OCR_MODES, App, LogMark, Win, box_center, check, log,
                     wait_until)

# ----------------------------------------------------------------------------- 主窗口

TITLE_BAR_H = 52
EDITOR_NEW = "新建待办"
EDITOR = "待办详情"
SETTINGS = "设置"
COMPLETED = "已完成"
NOTIFICATION = "通知"
SUBTASK_VIEW = "查看子任务"
SUBTASK_EDIT = "编辑子任务"

# 标题栏右区按钮（从右往左，28×28、间距 8、右内边距 16、描边 1）
_TITLE_BUTTONS = ["settings", "sync", "fixed", "theme", "view", "completed"]


def title_button(main: Win, name: str) -> tuple[int, int]:
    """标题栏右区按钮中心。`completed` 只在有已完成待办时出现（在 `view` 左边）。"""
    i = _TITLE_BUTTONS.index(name)
    return main.w - 1 - 16 - 14 - i * 36, 27


def wait_no_toast(app: App, main: Win, timeout: float = 8) -> None:
    """ElMessage 画在窗口顶部居中，会盖住标题栏右区按钮；光标停在提示上时它不会消失（与 Element Plus 相同：
    悬停暂停计时）。所以点标题栏按钮前先把光标移到列表左下角空白处，等提示消失：标题栏底部中点
    恢复成窗口底色（浅色主题纯白、深色主题透明 → Xvfb 下纯黑）。"""
    app.x.move(main, 24, main.h - 24)

    def clear() -> bool:
        p = app.x.pixel(main, main.w // 2, 47)
        return all(c >= 252 for c in p) or all(c <= 3 for c in p)
    try:
        wait_until(clear, timeout, 0.3, "toast to disappear")
    except TimeoutError:
        pass


def click_title(app: App, name: str) -> None:
    main = app.main_win()
    wait_no_toast(app, main)
    app.x.click(main, *title_button(main, name))


def fab(main: Win) -> tuple[int, int]:
    return main.w - 44, main.h - 44


def list_region(main: Win) -> tuple[int, int, int, int]:
    """主窗口内容区（标题栏以下、FAB 以上）"""
    return 0, TITLE_BAR_H, main.w, main.h - TITLE_BAR_H - 70


def find_row(app: App, title: str, timeout: float = 12) -> tuple[float, float, float, float]:
    main = app.main_win()
    b = app.wait_text(main, title, timeout, region=list_region(main))
    return b  # type: ignore[return-value]


def row_action(app: App, title: str, action: str) -> None:
    """悬停某行后点操作条：complete（✓）/ top（置顶）/ delete（删除）。操作条在行标题那一行的右端。"""
    main = app.main_win()
    x, y, w, h = find_row(app, title)
    cy = y + h / 2
    app.x.move(main, x + w / 2, cy)
    time.sleep(0.5)
    dx = {"complete": 97, "top": 69, "delete": 41}[action]
    app.x.move(main, main.w - dx, cy)
    time.sleep(0.3)
    app.x.click(main, main.w - dx, cy)


def row_handle(app: App, title: str) -> tuple[float, float]:
    """拖拽手柄（标题左侧的颜色圆点，标题文字左边 16px）"""
    x, y, w, h = find_row(app, title)
    return x - 16, y + h / 2


QUADRANT_NAMES = {1: "重要且紧急", 2: "重要不紧急", 3: "紧急不重要", 4: "不紧急不重要"}


def quadrant_drop_point(app: App, quadrant: int) -> tuple[float, float]:
    """四象限视图里某个象限的落点（2×2 网格，左右各半；上排从 y=61 开始，落点取标题下方）。
    下排的起始高度随上排内容变化：按第 3 / 4 象限标题的 OCR 位置定（四个标题互相只差一两个字，只认精确匹配；
    浅色底上的标题 OCR 偶尔认错，所以两个标题任一认出即可）。"""
    main = app.main_win()
    x = main.w * (0.28 if quadrant in (1, 3) else 0.72)
    if quadrant in (1, 2):
        return x, 61 + 60

    def row_y():
        for mode, variant in [(6, "gray"), (11, "gray"), (6, "red")]:
            r = app.ocr(main, list_region(main), mode, variant)
            for q in (3, 4):
                hits = [b for b in r.find_all_exact(QUADRANT_NAMES[q]) if b[1] > 200]
                if hits:
                    return hits[0][1]
        return None
    y = wait_until(row_y, 8, 0.3, "lower quadrant row")
    return x, y + 50


def confirm_dialog(app: App, win: Win, title_text: str, button: str, timeout: float = 8) -> None:
    """ElMessageBox.confirm：等标题出现后点按钮。按钮文字常常也出现在提示语里（「确定要删除…」），
    所以在标题下方找**最靠下**的一处精确匹配（按钮行在提示语下面）。"""
    tb = app.wait_text(win, title_text, timeout)
    _, ty, _, _ = tb  # type: ignore[misc]
    region = (0, int(ty) + 20, win.w, min(170, win.h - int(ty) - 20))

    def lowest():
        hits = []
        for psm, variant in ((11, "gray"), (6, "gray"), (11, "red")):
            hits += app.ocr(win, region, psm, variant).find_all_exact(button)
        # 按钮行在提示语下方（标题下 55px 以下）
        hits = [b for b in hits if b[1] > ty + 55]
        return max(hits, key=lambda b: b[1]) if hits else None
    try:
        b = wait_until(lowest, 4, 0.3, f"dialog button {button!r}")
    except TimeoutError:
        # tesseract 有时整行认不出按钮（短词被圆角边框包住）：退回到确认框的键盘约定 Enter = 确认
        log(f"[ui] dialog {title_text!r}: button {button!r} not found by OCR, pressing Enter")
        app.key(win, "Return")
        return
    app.x.click(win, *box_center(b))


def open_detail(app: App, title: str) -> Win:
    main = app.main_win()
    x, y, w, h = find_row(app, title)
    app.x.click(main, x + min(w / 2, 40), y + h / 2)
    ed = app.win(EDITOR, 15)
    time.sleep(0.8)
    return ed


def wait_toast(app: App, win: Win, text: str, timeout: float = 8) -> None:
    """ElMessage：窗口顶部居中 3 秒"""
    app.wait_text(win, text, timeout, region=(0, 0, win.w, 110), fuzzy=0.7)


def wait_main_shows(app: App, title: str, timeout: float = 12, present: bool = True) -> None:
    main = app.main_win()
    app.wait_text(main, title, timeout, present=present, region=list_region(main))


def close_completed(app: App, cw: Win) -> None:
    """已完成窗口（460×550）：右上角 ×（没有 Esc 快捷键）"""
    app.x.click(cw, cw.w - 26, 20)
    app.x.wait_window_gone(COMPLETED, 8)


# ----------------------------------------------------------------------------- 编辑窗口（1080×600）

ED_TITLE_INPUT = (350, 119)
# 描述框（y 183~307）：点框内任意位置（含空描述、内容下方的空白）都进入源码编辑（旧版行为），点框的中部
ED_DESC = (350, 245)
ED_DESC_MIDDLE = (350, 260)


def editor_click_edit(app: App, ed: Win) -> None:
    app.click_text(ed, "编辑", 8, region=(400, 0, 300, 56))
    time.sleep(0.8)
    app.wait_text(ed, "编辑待办", 8, region=(0, 0, 400, 56))


def editor_set_title(app: App, ed: Win, text: str) -> None:
    app.x.click(ed, *ED_TITLE_INPUT)
    app.key(ed, "ctrl+a")
    app.type(ed, text)


def editor_save(app: App, ed: Win, title: str = EDITOR) -> None:
    app.key(ed, "ctrl+Return")
    app.x.wait_window_gone(title, 10)


def subtask_panel_add(app: App, ed: Win, *titles: str, existing: int = 0) -> None:
    """子任务面板的「添加子任务...」输入框（浅灰占位文字 OCR 不可靠，按布局点）：没有子任务时在 y=97，
    有子任务时进度卡片把它推到 y=176。回车添加后焦点留在输入框里。"""
    app.x.click(ed, 860, 176 if existing else 97)
    for t in titles:
        app.type(ed, t)
        app.key(ed, "Return")
        time.sleep(0.8)


def subtask_toggle(app: App, ed: Win, title: str) -> None:
    b = app.wait_text(ed, title, 8, region=(700, 56, 380, 544))
    _, cy = box_center(b)  # type: ignore[arg-type]
    app.x.click(ed, 764, cy)


# ----------------------------------------------------------------------------- 设置窗口（680×560）

SETTINGS_MENU = {"常规": 85, "外观": 124, "数据与同步": 163, "屏幕配置": 202, "关于": 241}


def open_settings(app: App, tab: str = "常规") -> Win:
    click_title(app, "settings")
    st = app.win(SETTINGS, 15)
    time.sleep(0.8)
    app.x.click(st, 75, SETTINGS_MENU[tab])
    time.sleep(0.8)
    return st


def close_settings(app: App, st: Win) -> None:
    app.x.click(st, 636, 28)
    app.x.wait_window_gone(SETTINGS, 10)
    time.sleep(0.5)


def settings_content_top(app: App, st: Win) -> None:
    app.x.scroll(st, 420, 300, -15)


def settings_find(app: App, st: Win, text: str, max_scrolls: int = 6, step: int = 3) -> tuple[float, float]:
    """在设置内容区找文字（几种 OCR 方式轮流，蓝底白字的按钮要用红色通道），找不到就向下滚动再找"""
    region = (160, 56, 520, 504)
    texts = []
    for i in range(max_scrolls + 1):
        for psm, variant in OCR_MODES:
            r = app.ocr(st, region, psm, variant)
            b = r.find(text)
            if b and b[1] + b[3] < st.h - 8:
                return box_center(b)
            texts.append(r.text)
        if i < max_scrolls:
            app.x.scroll(st, 420, 300, step)
    raise AssertionError(f"settings: {text!r} not found; last OCR: {texts[-3][:400]!r}")


def settings_content_bottom(app: App, st: Win) -> None:
    app.x.scroll(st, 420, 300, 20)


def open_advanced(app: App, st: Win) -> None:
    """数据与同步 →「高级」折叠区（小号文字 OCR 不稳：按它与「立即同步」按钮的相对位置点，x=205、下方 93px）。
    结束时内容区滚到底（两个强制同步按钮在最下面）。"""
    settings_content_bottom(app, st)
    x, y = settings_find(app, st, "立即同步", 1)
    app.x.click(st, 205, y + 93)
    time.sleep(0.6)
    settings_content_bottom(app, st)


def fill_webdav(app: App, st: Win, kind: str, password: str = DAV_PASS) -> None:
    """设置 → 数据与同步：填写服务器 / 用户名 / 密码（不保存）"""
    settings_content_top(app, st)
    # 用户名 / 密码输入框在「测试连接」按钮行上方 48px；地址输入框在「服务器地址」标签下方 32px。
    # 先填用户名和密码：填了 http:// 地址后会多出一行明文警告，把下面的控件往下推。
    bx, by = settings_find(app, st, "测试连接", 0)
    app.x.click(st, 298, by - 48)
    app.key(st, "ctrl+a")
    app.type(st, DAV_USER)
    app.x.click(st, 530, by - 48)
    app.key(st, "ctrl+a")
    app.type(st, password)
    lx, ly = settings_find(app, st, "服务器地址", 0)
    app.x.click(st, 420, ly + 32)
    app.key(st, "ctrl+a")
    app.type(st, DAV[kind]["url"])
    time.sleep(0.5)


def test_connection(app: App, st: Win, expect: str = "连接成功") -> None:
    app.click_text(st, "测试连接", 6, region=(160, 56, 520, 504))
    wait_toast(app, st, expect, 10)


def configure_webdav(app: App, kind: str, check_connection: bool = True) -> Win:
    """设置 → 数据与同步：填写服务器 / 用户名 / 密码 →（测试连接）→ 保存配置。返回设置窗口（保持打开）"""
    st = open_settings(app, "数据与同步")
    fill_webdav(app, st, kind)
    if check_connection:
        test_connection(app, st)
    app.click_text(st, "保存配置", 6, region=(160, 56, 520, 504))
    wait_toast(app, st, "同步设置已保存", 10)
    return st


def settings_sync_now(app: App, st: Win, mark: LogMark, timeout: float = 60) -> str:
    """设置页「立即同步」→ 等日志里的同步结果行"""
    settings_content_bottom(app, st)
    x, y = settings_find(app, st, "立即同步", 1)
    app.x.click(st, x, y)
    return mark.wait(r"\[sync\] 同步(完成|失败)", timeout)


def title_sync(app: App, timeout: float = 60) -> str:
    """标题栏同步按钮 → 等日志里的同步结果行（失败直接报错）"""
    mark = LogMark(app.session)
    click_title(app, "sync")
    line = mark.wait(r"\[sync\] 同步(完成|失败)", timeout)
    check("同步完成" in line, f"title-bar sync failed: {line}")
    # 日志先于提示写出：等提示画出来，下一次点标题栏前 wait_no_toast 才看得见它
    time.sleep(0.6)
    return line
