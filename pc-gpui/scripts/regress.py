#!/usr/bin/env python3
"""pc-gpui 一键回归：Xvfb 下逐个场景启动真实应用（debug 构建、隔离数据目录、样例数据），用 xdotool 交互后截图，
再与参考截图（Vue 版）拼成并排对照图；`--smoke` 跑跨窗口端到端冒烟并按数据库断言给出 PASS / FAIL。

用法与场景说明见同目录 README.md（「regress.py」一节）。只依赖 Python 3 标准库 + Xvfb / xdotool / ImageMagick。
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import zipfile
from dataclasses import dataclass, field
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
WORKSPACE = SCRIPT_DIR.parent
APP = WORKSPACE / "target" / "debug" / "mini-todo"
SEED = WORKSPACE / "target" / "debug" / "examples" / "seed"
SAMPLE = SCRIPT_DIR / "sample-data.json"

MAIN = "Mini Todo"
EDITOR = "待办详情"
NEW_EDITOR = "新建待办"
SUBTASK_VIEW = "查看子任务"
SUBTASK_EDIT = "编辑子任务"
SETTINGS = "设置"
COMPLETED = "已完成"
NOTIFICATION = "通知"
FENCE = "收纳箱"

# 参考截图 harness 的节假日缓存（国庆 / 中秋，含调休上班日），日历场景用；节假日接口断网后走缓存
HOLIDAYS_2026 = (
    [
        {"name": "中秋节", "date": "2026-09-25", "isOffDay": True},
        {"name": "中秋节", "date": "2026-09-26", "isOffDay": True},
        {"name": "中秋节", "date": "2026-09-27", "isOffDay": True},
        {"name": "国庆节", "date": "2026-09-20", "isOffDay": False},
    ]
    + [{"name": "国庆节", "date": f"2026-10-0{d}", "isOffDay": True} for d in range(1, 8)]
    + [{"name": "国庆节", "date": "2026-10-10", "isOffDay": False}]
)

# 收纳箱样例项目（与参考截图的 6 项同名同顺序）
FENCE_ITEMS = [
    ("项目资料", None),
    ("Visual Studio Code.lnk", "x"),
    ("Q3 预算表.xlsx", "x"),
    ("会议纪要 1015.docx", "x"),
    ("截图 2026-10-14 153012.png", "x"),
    ("Mini Todo 官网.url", "[InternetShortcut]\nURL=https://example.com\n"),
]

DARK = ["UPDATE settings SET value='light' WHERE key='text_theme'"]
QUADRANT = ["INSERT OR REPLACE INTO settings (key, value) VALUES ('view_mode', 'quadrant')"]
FIXED = [
    "INSERT OR REPLACE INTO screen_configs (config_id, display_name, window_x, window_y, window_width, "
    "window_height, is_fixed) VALUES ('1_1920x1080@100', '1屏: 1920x1080 (100%)', 770, 240, 380, 600, 1)",
    "UPDATE settings SET value='true' WHERE key='is_fixed'",
]
EMPTY = ["DELETE FROM subtasks", "DELETE FROM todos"]


@dataclass
class Scene:
    """一个场景：一次全新的应用进程。

    steps 里的动作（坐标都相对窗口左上角，窗口按标题精确匹配）：
      ("move", 标题, x, y) / ("click", …) / ("rclick", …) / ("dbl", …)
      ("wheel", 标题, x, y, n)        n > 0 向下滚，n < 0 向上滚
      ("drag", 标题, x1, y1, x2, y2)   按下 → 分 12 步移动 → 停住（不松开，接着 ("up",) 松开）
      ("down",) / ("up",)              左键按下 / 松开
      ("key", 键) / ("type", 文本)      发给当前焦点窗口（xdotool 语法：ctrl+Return、Escape）。只输入 ASCII：
                                       xdotool 输入中文靠临时改键位映射，GPUI 的 X11 后端会丢字（测试工具的限制）
      ("sleep", 秒) / ("away",)        等待 / 光标移到屏幕左下角空白处
      ("wmove", 标题, x, y)             把窗口挪到屏幕坐标 (x, y)（Xvfb 没有窗口管理器，子窗口会盖住主窗口）
      ("shot", 标题, 文件名)            额外截一张（不含 .png）
      ("sql", 语句)                    在应用运行期间直接改数据库（冒烟用）
      ("check", 说明, 函数)            冒烟断言：函数(上下文) -> (bool, 详情)
    """

    name: str
    open: str = "main"
    window: str = MAIN
    sql: list[str] = field(default_factory=list)
    ui_state: dict | None = None
    env: dict = field(default_factory=dict)
    steps: list = field(default_factory=list)
    # 停光标："main" = 主窗口标题栏空白处（先经过第 2 行刷新悬停）；"away" = 屏幕左下角；None = 不动
    park: str | None = "main"
    webdav: bool = False  # 设置窗口场景：补 WebDAV 设置 + 两条屏幕配置（与参考截图 harness 一致）
    fences: str | None = None  # "items" = 收纳箱 #1 带 6 个样例项目 + 空的收纳箱 2；"empty" = 只有一个空收纳箱
    wait: float = 4.0
    shot: bool = True  # 结束时按 window 截一张 <name>.png
    ref: str | None = None  # 对照用的参考图名（缺省同名）
    step_delay: float = 0.5  # 每个交互动作之后的等待
    settle: float = 0.4  # 最后一步之后、截图之前的等待
    group: str = "scene"  # scene / interaction / smoke


def editor_scene(name, open_, steps=(), window=EDITOR, **kw):
    return Scene(name, open=open_, window=window, steps=list(steps), park=None, **kw)


def settings_scene(name, tab, steps=(), **kw):
    kw.setdefault("group", "scene")
    return Scene(
        name,
        open=f"main,settings:{tab}",
        window=SETTINGS,
        steps=[("away",)] + list(steps) + [("away",)],
        park=None,
        webdav=True,
        env={"MINI_TODO_DEBUG_WINDOWS_UI": "1"},
        wait=3.5,
        **kw,
    )


def fence_scene(name, steps=(), fences="items", **kw):
    return Scene(
        name,
        open="main,fence:1",
        window=FENCE,
        steps=[("away",)] + list(steps),
        park=None,
        fences=fences,
        wait=3.5,
        **kw,
    )


SCROLL_FORM = [("move", EDITOR, 600, 500), ("wheel", EDITOR, 600, 500, 16), ("sleep", 0.6)]

SCENES: list[Scene] = [
    # ---- 主窗口（与参考截图同名）----
    Scene("main-list"),
    Scene("main-list-hover", steps=[("move", MAIN, 120, 133)]),
    Scene("main-list-expanded", steps=[("click", MAIN, 68, 94), ("move", MAIN, 100, 45), ("sleep", 0.5)]),
    Scene("main-expand-all", ui_state={"subtaskExpandAll": True}),
    Scene("main-quadrant", sql=QUADRANT),
    Scene("main-calendar", open="main,calendar", steps=[("sleep", 2)]),
    Scene("main-dark", sql=DARK),
    Scene("main-dark-hover", sql=DARK, steps=[("move", MAIN, 120, 133)]),
    Scene("main-quadrant-dark", sql=DARK + QUADRANT),
    Scene("main-calendar-dark", open="main,calendar", sql=DARK, steps=[("sleep", 2)]),
    Scene("main-fixed", sql=FIXED),
    Scene("main-fixed-dark", sql=FIXED + DARK),
    Scene("main-empty", sql=EMPTY),
    Scene(
        "main-delete-confirm",
        steps=[("move", MAIN, 120, 133), ("sleep", 0.3), ("click", MAIN, 338, 137), ("sleep", 0.6)],
    ),
    Scene("main-update", env={"MINI_TODO_DEBUG_LATEST_VERSION": "v2.6.0"}),
    Scene(
        "main-update-dialog",
        env={"MINI_TODO_DEBUG_LATEST_VERSION": "v2.6.0"},
        steps=[("click", MAIN, 95, 26), ("sleep", 0.6)],
    ),
    # ---- 编辑类窗口 ----
    editor_scene("editor-view", "main,editor:1:view", [("sleep", 0.5)]),
    editor_scene("editor-view-repeat", "main,editor:5:view", [("sleep", 0.5)]),
    editor_scene("editor-edit", "main,editor:1:edit", [("sleep", 0.5)]),
    editor_scene(
        "editor-edit-bottom",
        "main,editor:5:edit",
        [("wheel", EDITOR, 300, 300, 30), ("sleep", 1), ("move", EDITOR, 1060, 590), ("sleep", 0.4)],
    ),
    editor_scene(
        "editor-desc-dialog",
        "main,editor:1:edit",
        [("click", EDITOR, 61, 164), ("sleep", 1.2), ("move", EDITOR, 1060, 590), ("sleep", 0.4)],
    ),
    editor_scene("editor-new", "main,editor:new", [("sleep", 0.5)], window=NEW_EDITOR),
    editor_scene("subtask-view", "main,subtask:101:view", [("sleep", 0.5)], window=SUBTASK_VIEW),
    editor_scene("subtask-edit", "main,subtask:101:edit", [("sleep", 0.5)], window=SUBTASK_EDIT),
    editor_scene("completed", "main,completed", [("sleep", 0.5)], window=COMPLETED),
    editor_scene(
        "completed-hover",
        "main,completed",
        [("move", COMPLETED, 136, 116), ("sleep", 0.3), ("move", COMPLETED, 140, 116), ("sleep", 0.6)],
        window=COMPLETED,
    ),
    editor_scene("notification", "main,notification:5", [("sleep", 0.5)], window=NOTIFICATION),
    # ---- 设置窗口 ----
    settings_scene("settings-general", "general"),
    settings_scene("settings-general-bottom", "general", [("wheel", SETTINGS, 420, 300, 15), ("sleep", 0.6)]),
    settings_scene(
        "settings-general-darktheme",
        "general",
        [("wheel", SETTINGS, 420, 300, 15), ("sleep", 0.6), ("click", SETTINGS, 636, 320), ("sleep", 0.8)],
    ),
    settings_scene("settings-appearance", "appearance"),
    settings_scene("settings-data", "data"),
    settings_scene(
        "settings-data-bottom",
        "data",
        [
            ("wheel", SETTINGS, 420, 300, 20),
            ("sleep", 0.6),
            ("click", SETTINGS, 205, 531),
            ("sleep", 0.6),
            ("wheel", SETTINGS, 420, 300, 20),
            ("sleep", 0.6),
            # 滚动后指针下的按钮悬停不会随后续移动清除（GPUI 的悬停失效判定），先经过别处再移开
            ("move", SETTINGS, 300, 100),
            ("sleep", 0.4),
            ("move", SETTINGS, 80, 500),
            ("sleep", 0.4),
        ],
    ),
    settings_scene("settings-screen", "screen"),
    settings_scene("settings-about", "about"),
    settings_scene(
        "settings-about-checked",
        "about",
        # 检查更新经断开的代理失败（debug 构建里 reqwest 初始化 TLS 要 1~2 秒），提示显示 3 秒
        [("click", SETTINGS, 420, 193), ("sleep", 0.3), ("away",), ("sleep", 2.2)],
    ),
    # ---- 收纳箱 ----
    fence_scene("fence"),
    # 悬停 500ms 后会出原生 title 提示（参考截图没有光标与提示）：移上去后尽快截图
    fence_scene("fence-hover", [("move", FENCE, 229, 80), ("sleep", 0.15)], step_delay=0.1, settle=0.05),
    fence_scene(
        "fence-selected",
        [("click", FENCE, 140, 80), ("sleep", 0.2), ("move", FENCE, 180, 300), ("sleep", 0.3)],
    ),
    fence_scene(
        "fence-context-menu",
        [("click", FENCE, 140, 80), ("sleep", 0.2), ("rclick", FENCE, 141, 88), ("sleep", 0.25)],
    ),
    fence_scene(
        "fence-title-menu",
        [("click", FENCE, 350, 16), ("sleep", 0.3), ("move", FENCE, 300, 250), ("sleep", 0.3)],
    ),
    fence_scene(
        "fence-appearance",
        [
            ("click", FENCE, 350, 16),
            ("sleep", 0.3),
            ("click", FENCE, 300, 92),
            ("sleep", 0.3),
            ("move", FENCE, 180, 300),
            ("sleep", 0.3),
        ],
    ),
    fence_scene(
        "fence-delete-confirm",
        [
            ("click", FENCE, 350, 16),
            ("sleep", 0.3),
            ("click", FENCE, 300, 133),
            ("sleep", 0.5),
            ("move", FENCE, 180, 200),
            ("sleep", 0.3),
        ],
    ),
    fence_scene("fence-locked", sql=["UPDATE fences SET locked = 1 WHERE id = 1"]),
    fence_scene("fence-collapsed", sql=["UPDATE fences SET collapsed = 1 WHERE id = 1"]),
    fence_scene("fence-drag-hover", env={"MINI_TODO_DEBUG_FENCE_STATE": "drag-hover"}),
    fence_scene("fence-onedrive-notice", env={"MINI_TODO_DEBUG_FENCE_STATE": "onedrive"}),
    fence_scene("fence-empty", fences="empty"),
    # ---- 关键交互（没有同名参考图的和参考图 harness 的 w2-* 对照）----
    Scene(
        "main-toast",
        open="main,toast:success:保存成功",
        steps=[("sleep", 0.2)],
        wait=1.0,
        group="interaction",
    ),
    Scene("main-quadrant-hover", sql=QUADRANT, steps=[("move", MAIN, 120, 133)], group="interaction"),
    editor_scene(
        "editor-color-panel",
        "main,editor:1:edit",
        [("click", EDITOR, 284, 523), ("sleep", 0.8)],
        ref="w2-editor-color-panel",
        group="interaction",
    ),
    editor_scene(
        "editor-date-panel",
        "main,editor:1:edit",
        SCROLL_FORM + [("click", EDITOR, 100, 225), ("sleep", 0.8)],
        ref="w2-editor-date-panel",
        group="interaction",
    ),
    editor_scene(
        "editor-time-panel",
        "main,editor:1:edit",
        SCROLL_FORM + [("click", EDITOR, 330, 225), ("sleep", 0.8)],
        ref="w2-editor-time-panel",
        group="interaction",
    ),
    editor_scene(
        "editor-notify-select",
        "main,editor:1:edit",
        SCROLL_FORM + [("click", EDITOR, 300, 425), ("sleep", 0.8)],
        ref="w2-editor-notify-select",
        group="interaction",
    ),
    editor_scene(
        "editor-notify-custom",
        "main,editor:1:edit",
        SCROLL_FORM
        + [("click", EDITOR, 300, 425), ("sleep", 0.8), ("click", EDITOR, 100, 375), ("sleep", 0.6)],
        group="interaction",
    ),
    editor_scene(
        "editor-import-menu",
        "main,editor:1:edit",
        [("click", EDITOR, 1037, 177), ("sleep", 0.8)],
        ref="w2-editor-import-menu",
        group="interaction",
    ),
    editor_scene(
        "editor-esc-confirm",
        "main,editor:1:edit",
        [("click", EDITOR, 600, 119), ("sleep", 0.3), ("type", "xyz"), ("sleep", 0.3), ("key", "Escape"), ("sleep", 0.8)],
        ref="w2-editor-esc-confirm",
        group="interaction",
    ),
    editor_scene(
        "editor-new-subtask",
        "main,editor:new",
        [
            ("click", NEW_EDITOR, 880, 92),
            ("sleep", 0.3),
            ("type", "Sub A"),
            ("key", "Return"),
            ("sleep", 0.4),
            ("type", "Sub B"),
            ("key", "Return"),
            ("sleep", 0.5),
            ("move", NEW_EDITOR, 900, 230),
            ("sleep", 0.5),
        ],
        window=NEW_EDITOR,
        ref="w2-editor-new-subtask",
        group="interaction",
    ),
    editor_scene(
        "editor-subtask-overlay",
        "main,editor:1:view",
        [
            ("move", EDITOR, 900, 226),
            ("sleep", 0.4),
            ("move", EDITOR, 902, 226),
            ("sleep", 0.5),
            ("click", EDITOR, 1037, 226),
            ("sleep", 1.5),
            ("shot", SUBTASK_VIEW, "editor-subtask-overlay-subtask"),
        ],
        group="interaction",
    ),
    editor_scene(
        "editor-drag-sort",
        "main,editor:1:view",
        [
            ("move", EDITOR, 900, 284),
            ("sleep", 0.3),
            ("drag", EDITOR, 737, 284, 737, 164),
            ("sleep", 0.3),
            ("shot", EDITOR, "editor-drag-sort-dragging"),
            ("up",),
            ("sleep", 0.8),
        ],
        group="interaction",
    ),
    settings_scene(
        "settings-bg-color-panel",
        "general",
        [("wheel", SETTINGS, 420, 300, 15), ("sleep", 0.6), ("click", SETTINGS, 644, 386), ("sleep", 0.8)],
        sql=DARK,
        group="interaction",
    ),
    settings_scene(
        "settings-font-select",
        "appearance",
        [("click", SETTINGS, 420, 160), ("sleep", 0.8), ("type", "x"), ("sleep", 0.5)],
        group="interaction",
    ),
    settings_scene(
        "settings-interval-select",
        "data",
        [("wheel", SETTINGS, 420, 300, 20), ("sleep", 0.6), ("click", SETTINGS, 590, 376), ("sleep", 0.8)],
        group="interaction",
    ),
    fence_scene(
        "fence-rename",
        [("dbl", FENCE, 60, 16), ("sleep", 0.5), ("type", "工作文件"), ("sleep", 0.4)],
        group="interaction",
    ),
    fence_scene(
        "fence-move-to",
        [
            ("click", FENCE, 140, 80),
            ("sleep", 0.2),
            ("rclick", FENCE, 141, 88),
            ("sleep", 0.3),
            ("click", FENCE, 190, 190),
            ("sleep", 0.6),
        ],
        group="interaction",
    ),
]


# ---------------------------------------------------------------------------
# 冒烟（跨窗口端到端）
# ---------------------------------------------------------------------------


def q(ctx, sql, args=()):
    con = sqlite3.connect(ctx.data / "data.db", timeout=5)
    try:
        return con.execute(sql, args).fetchall()
    finally:
        con.close()


def window_open(title):
    return lambda ctx: (ctx.window_id(title) is not None, f"窗口「{title}」")


def window_closed(title):
    return lambda ctx: (ctx.window_id(title) is None, f"窗口「{title}」已关闭")


SMOKE: list[Scene] = [
    Scene(
        "smoke-create-todo",
        steps=[
            # FAB → 新建窗口 → 标题 → Ctrl+Enter 保存 → 主窗口出现
            ("click", MAIN, 356, 576),
            ("sleep", 1.5),
            ("check", "FAB 打开新建窗口", window_open(NEW_EDITOR)),
            ("click", NEW_EDITOR, 300, 119),
            ("type", "Smoke todo"),
            ("sleep", 0.3),
            ("key", "ctrl+Return"),
            ("sleep", 1.5),
            ("check", "保存后新建窗口关闭", window_closed(NEW_EDITOR)),
            (
                "check",
                "数据库有新待办",
                lambda ctx: (
                    bool(q(ctx, "SELECT id FROM todos WHERE title = 'Smoke todo'")),
                    str(q(ctx, "SELECT id, title FROM todos ORDER BY id DESC LIMIT 1")),
                ),
            ),
            ("move", MAIN, 100, 45),
            ("sleep", 0.8),
            ("shot", MAIN, "smoke-create-todo-main"),
        ],
        shot=False,
        group="smoke",
    ),
    Scene(
        "smoke-edit-todo",
        steps=[
            # 点第 1 行 → 只读详情 → [编辑] → 改标题 → 保存 → 主窗口更新
            ("click", MAIN, 200, 94),
            ("sleep", 1.5),
            ("check", "点待办打开只读详情", window_open(EDITOR)),
            ("shot", EDITOR, "smoke-edit-todo-view"),
            ("click", EDITOR, 603, 28),
            ("sleep", 0.8),
            ("shot", EDITOR, "smoke-edit-todo-edit"),
            ("click", EDITOR, 600, 119),
            ("key", "End"),
            ("type", " edited"),
            ("sleep", 0.3),
            ("key", "ctrl+Return"),
            ("sleep", 1.5),
            (
                "check",
                "标题已保存",
                lambda ctx: (
                    q(ctx, "SELECT title FROM todos WHERE id = 1") == [("季度 OKR 复盘材料 edited",)],
                    str(q(ctx, "SELECT title FROM todos WHERE id = 1")),
                ),
            ),
            ("move", MAIN, 100, 45),
            ("sleep", 0.8),
            ("shot", MAIN, "smoke-edit-todo-main"),
        ],
        shot=False,
        group="smoke",
    ),
    Scene(
        "smoke-subtasks",
        # 只读详情里没有「添加子任务」输入框（与旧版一致），在编辑排版里加
        open="main,editor:1:edit",
        park=None,
        steps=[
            # 子任务新增 + 勾选 → 主窗口计数更新
            ("click", EDITOR, 880, 177),
            ("sleep", 0.3),
            ("type", "Smoke subtask"),
            ("key", "Return"),
            ("sleep", 1.0),
            ("click", EDITOR, 763, 281),
            ("sleep", 1.0),
            (
                "check",
                "子任务已新增",
                lambda ctx: (
                    bool(q(ctx, "SELECT id FROM subtasks WHERE parent_id = 1 AND title = 'Smoke subtask'")),
                    str(q(ctx, "SELECT id, title, completed FROM subtasks WHERE parent_id = 1")),
                ),
            ),
            (
                "check",
                "第 2 个子任务已勾选",
                lambda ctx: (
                    q(ctx, "SELECT completed FROM subtasks WHERE id = 102") == [(1,)],
                    str(q(ctx, "SELECT id, title, completed FROM subtasks WHERE parent_id = 1 ORDER BY sort_order")),
                ),
            ),
            ("shot", EDITOR, "smoke-subtasks-editor"),
            ("key", "Escape"),
            ("sleep", 1.2),
            ("move", MAIN, 100, 45),
            ("sleep", 0.8),
            ("shot", MAIN, "smoke-subtasks-main"),
        ],
        shot=False,
        group="smoke",
    ),
    Scene(
        "smoke-settings-live",
        open="main,settings:general",
        park=None,
        env={"MINI_TODO_DEBUG_WINDOWS_UI": "1"},
        steps=[
            # 深色主题开关 → 主窗口即时变深；外观里改字号 → 主窗口字号变化
            ("wmove", SETTINGS, 1200, 60),
            ("away",),
            ("wheel", SETTINGS, 420, 300, 15),
            ("sleep", 0.6),
            ("click", SETTINGS, 636, 320),
            ("sleep", 1.0),
            (
                "check",
                "深色主题已写库",
                lambda ctx: (
                    q(ctx, "SELECT value FROM settings WHERE key = 'text_theme'") == [("light",)],
                    str(q(ctx, "SELECT value FROM settings WHERE key = 'text_theme'")),
                ),
            ),
            ("shot", MAIN, "smoke-settings-live-dark"),
            ("click", SETTINGS, 80, 120),
            ("sleep", 0.6),
            ("shot", SETTINGS, "smoke-settings-live-appearance"),
            ("click", SETTINGS, 398, 232),
            ("sleep", 1.0),
            (
                "check",
                "字号已写库",
                lambda ctx: (
                    q(ctx, "SELECT value FROM settings WHERE key = 'todo_font_size'") != [("14",)],
                    str(q(ctx, "SELECT value FROM settings WHERE key = 'todo_font_size'")),
                ),
            ),
            ("shot", MAIN, "smoke-settings-live-font"),
        ],
        shot=False,
        group="smoke",
    ),
    Scene(
        "smoke-export-import",
        open="main,settings:data",
        park=None,
        env={"MINI_TODO_DEBUG_DIALOG_PATH": "{data}/smoke-export.zip"},
        steps=[
            # 导出 → 删库里一条待办 → 导入 → 主窗口刷新出被删的待办
            ("wmove", SETTINGS, 1200, 60),
            ("away",),
            ("call", "export"),
            ("sleep", 2.0),
            ("check", "导出文件已生成", lambda ctx: smoke_export_ok(ctx)),
            ("sql", "DELETE FROM todos WHERE id = 8"),
            ("call", "import"),
            ("sleep", 1.0),
            ("shot", SETTINGS, "smoke-export-import-confirm"),
            ("key", "Return"),
            ("sleep", 2.5),
            (
                "check",
                "导入后被删的待办回来了",
                lambda ctx: (bool(q(ctx, "SELECT id FROM todos WHERE id = 8")), str(q(ctx, "SELECT COUNT(*) FROM todos"))),
            ),
            ("shot", MAIN, "smoke-export-import-main"),
        ],
        shot=False,
        group="smoke",
    ),
    Scene(
        "smoke-restore-completed",
        open="main,completed",
        park=None,
        steps=[
            # 已完成窗口里取消完成第 1 条 → 主窗口出现
            ("wmove", COMPLETED, 1300, 100),
            ("move", COMPLETED, 136, 116),
            ("sleep", 0.3),
            ("move", COMPLETED, 140, 116),
            ("sleep", 0.4),
            ("call", "restore"),
            ("sleep", 1.5),
            (
                "check",
                "已完成的待办被恢复",
                lambda ctx: (
                    q(ctx, "SELECT COUNT(*) FROM todos WHERE completed = 1") == [(2,)],
                    str(q(ctx, "SELECT id, completed FROM todos WHERE id IN (21, 22, 23)")),
                ),
            ),
            ("shot", COMPLETED, "smoke-restore-completed-completed"),
            ("key", "Escape"),
            ("sleep", 1.5),
            ("move", MAIN, 100, 45),
            ("sleep", 0.6),
            ("shot", MAIN, "smoke-restore-completed-main"),
        ],
        shot=False,
        group="smoke",
    ),
    Scene(
        "smoke-reminder",
        # 应用内通知 + 一条 1 分钟内到期的待办；调度对齐整分钟，最多等 70 秒
        sql=[
            "UPDATE settings SET value = 'app' WHERE key = 'notification_type'",
            # 样例数据里已过点的提醒先标记掉，只让这一条到期
            "UPDATE todos SET notified = 1",
            "INSERT INTO todos (title, description, color, quadrant, notify_at, notify_before, notified, completed, "
            "sort_order, created_at, updated_at) VALUES ('冒烟提醒', '一分钟后到期', '#EF4444', 1, "
            "strftime('%Y-%m-%d %H:%M:00', 'now', 'localtime', '+1 minute'), 0, 0, 0, 99, "
            "datetime('now','localtime'), datetime('now','localtime'))",
        ],
        steps=[
            ("wait_window", NOTIFICATION, 75),
            ("check", "到期后弹出应用内通知", window_open(NOTIFICATION)),
            (
                "check",
                "提醒已标记",
                lambda ctx: (
                    q(ctx, "SELECT notified FROM todos WHERE title = '冒烟提醒'") == [(1,)],
                    str(q(ctx, "SELECT notified, notify_at FROM todos WHERE title = '冒烟提醒'")),
                ),
            ),
            ("shot", NOTIFICATION, "smoke-reminder-notification"),
        ],
        shot=False,
        group="smoke",
    ),
    Scene(
        "smoke-fence-new",
        open="main,fence:new",
        park=None,
        steps=[
            ("away",),
            ("check", "fence:new 建出收纳箱窗口", window_open(FENCE)),
            (
                "check",
                "收纳箱记录与文件夹",
                lambda ctx: (
                    q(ctx, "SELECT title FROM fences") == [("收纳箱",)] and (ctx.data / "fences" / "收纳箱").is_dir(),
                    str(q(ctx, "SELECT id, title, folder_name FROM fences")),
                ),
            ),
            ("dbl", FENCE, 60, 16),
            ("sleep", 0.5),
            ("key", "ctrl+a"),
            ("type", "Smoke Box"),
            ("key", "Return"),
            ("sleep", 1.0),
            (
                "check",
                "改名写库、文件夹同步改名",
                lambda ctx: (
                    q(ctx, "SELECT title, folder_name FROM fences") == [("Smoke Box", "Smoke Box")]
                    and (ctx.data / "fences" / "Smoke Box").is_dir(),
                    str(q(ctx, "SELECT title, folder_name FROM fences")),
                ),
            ),
            ("shot", "Smoke Box", "smoke-fence-new"),
        ],
        shot=False,
        group="smoke",
    ),
]


def smoke_export_ok(ctx):
    path = ctx.data / "smoke-export.zip"
    if not path.is_file():
        return False, f"{path} 不存在"
    with zipfile.ZipFile(path) as z:
        data = json.loads(z.read("data.json"))
    return len(data.get("todos", [])) == 11, f"导出 {len(data.get('todos', []))} 条待办"


# 冒烟里需要按控件位置找的点击（集中在这里，界面调整后只改这一处）
CALLS = {
    # 设置 → 数据与同步：导出 / 导入按钮
    "export": ("click", SETTINGS, 300, 136),
    "import": ("click", SETTINGS, 540, 136),
    # 已完成窗口第 1 行的「恢复」勾选圈
    "restore": ("click", COMPLETED, 376, 135),
}


# ---------------------------------------------------------------------------
# 运行
# ---------------------------------------------------------------------------


class Runner:
    def __init__(self, args):
        self.args = args
        self.display = args.display
        self.env = dict(os.environ)
        self.env.update(
            {
                "DISPLAY": self.display,
                "VK_ICD_FILENAMES": "/usr/share/vulkan/icd.d/lvp_icd.json",
                "GPUI_X11_SCALE_FACTOR": "1",
                "TZ": "Asia/Shanghai",
                # 断网：节假日走缓存、版本检查失败（结果确定）
                "HTTPS_PROXY": "http://127.0.0.1:9",
                "https_proxy": "http://127.0.0.1:9",
                "HTTP_PROXY": "http://127.0.0.1:9",
                "http_proxy": "http://127.0.0.1:9",
            }
        )
        self.env.pop("WAYLAND_DISPLAY", None)
        for key in list(self.env):
            if key.startswith("MINI_TODO_"):
                self.env.pop(key)
        self.out = Path(args.out).resolve()
        self.out.mkdir(parents=True, exist_ok=True)
        (self.out / "logs").mkdir(exist_ok=True)
        self.xvfb = None
        self.results: list[tuple[str, str, bool, str]] = []

    # ---- X ----
    def xdo(self, *args):
        return subprocess.run(["xdotool", *map(str, args)], capture_output=True, text=True, env=self.env).stdout.strip()

    def window_id(self, title):
        for wid in self.xdo("search", "--onlyvisible", "--name", ".").split():
            if self.xdo("getwindowname", wid) == title:
                return wid
        return None

    def geometry(self, wid):
        out = self.xdo("getwindowgeometry", "--shell", wid)
        vals = dict(line.split("=", 1) for line in out.splitlines() if "=" in line)
        return int(vals["X"]), int(vals["Y"]), int(vals["WIDTH"]), int(vals["HEIGHT"])

    def at(self, title, x, y):
        wid = self.window_id(title)
        if wid is None:
            raise RuntimeError(f"找不到窗口「{title}」")
        gx, gy, _, _ = self.geometry(wid)
        self.xdo("mousemove", gx + x, gy + y)
        return wid

    def start_xvfb(self):
        subprocess.run(["pkill", "-f", f"Xvfb {self.display} "], capture_output=True)
        self.xvfb = subprocess.Popen(
            ["Xvfb", self.display, "-screen", "0", "1920x1080x24", "-nolisten", "tcp"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        for _ in range(100):
            if subprocess.run(["xdotool", "getdisplaygeometry"], capture_output=True, env=self.env).returncode == 0:
                return
            time.sleep(0.1)
        raise RuntimeError("Xvfb 没有起来")

    def stop_xvfb(self):
        if self.xvfb:
            self.xvfb.terminate()
            self.xvfb.wait(timeout=5)

    def screenshot(self, title, path):
        full = path.with_suffix(".full.png")
        subprocess.run(["import", "-window", "root", str(full)], env=self.env, check=True)
        wid = self.window_id(title)
        if wid is None:
            print(f"    !! 找不到窗口「{title}」，保留整屏截图 {full}")
            return False
        x, y, w, h = self.geometry(wid)
        subprocess.run(["convert", str(full), "-crop", f"{w}x{h}+{x}+{y}", "+repage", str(path)], check=True)
        full.unlink()
        return True

    # ---- 数据 ----
    def prepare_data(self, scene: Scene) -> Path:
        data = Path(tempfile.mkdtemp(prefix=f"regress-{scene.name}-", dir=self.args.tmp))
        env = dict(self.env, MINI_TODO_DATA_DIR=str(data))
        subprocess.run([str(SEED), str(data), str(SAMPLE)], check=True, env=env, stdout=subprocess.DEVNULL)
        images = data / "images"
        images.mkdir(exist_ok=True)
        # 子任务 Markdown 引用的示例图（与参考截图 harness 的柱状图相同的数据）
        chart = ["-size", "480x240", "xc:#F8FAFC", "-stroke", "#CBD5E1", "-draw", "line 40,200 464,200", "-stroke", "none", "-fill", "#3B82F6"]
        x = 48
        for v in (42, 68, 55, 80, 73, 91):
            bh = v * 160 // 100
            chart += ["-draw", f"roundrectangle {x},{200 - bh} {x + 48},200 4,4"]
            x += 72
        subprocess.run(["convert", *chart, str(images / "sample-chart.png")], check=False)
        (data / "holidays").mkdir(exist_ok=True)
        (data / "holidays" / "2026.json").write_text(json.dumps(HOLIDAYS_2026, ensure_ascii=False))
        if scene.ui_state is not None:
            (data / "ui-state.json").write_text(json.dumps(scene.ui_state))
        con = sqlite3.connect(data / "data.db")
        if scene.webdav:
            self.prep_webdav(con)
        if scene.fences:
            self.prep_fences(con, data, scene.fences)
        for stmt in scene.sql:
            con.execute(stmt)
        con.commit()
        con.close()
        return data

    @staticmethod
    def prep_webdav(con):
        def setk(k, v):
            con.execute(
                "INSERT INTO settings(key,value,updated_at) VALUES(?,?,datetime('now','localtime')) "
                "ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                (k, v),
            )

        setk("webdav_url", "https://dav.example.com/dav/mini-todo")
        setk("webdav_username", "demo@example.com")
        setk("webdav_password", "secret")
        setk("webdav_auto_sync", "true")
        setk("webdav_sync_interval", "15")
        setk("webdav_last_sync_at", "2026-10-15T09:42:10+08:00")
        con.execute("DELETE FROM screen_configs")
        rows = [
            ("2_2560x1440@125_1920x1080@100", "2屏: 2560x1440 (125%) + 1920x1080 (100%)", 2950, 160, 1, "2026-10-10 10:00:00"),
            ("1_1920x1080@100", "1屏: 1920x1080 (100%)", 1500, 120, 0, "2026-10-15 09:00:00"),
        ]
        for cid, name, x, y, fixed, updated in rows:
            con.execute(
                "INSERT INTO screen_configs(config_id,display_name,window_x,window_y,window_width,window_height,"
                "is_fixed,created_at,updated_at) VALUES(?,?,?,?,380,600,?,'2026-10-10 10:00:00',?)",
                (cid, name, x, y, fixed, updated),
            )

    @staticmethod
    def prep_fences(con, data: Path, kind: str):
        now = "2026-10-15 10:00:00"
        root = data / "fences"
        fences = [(1, "收纳箱", 1528)] + ([(2, "收纳箱 2", 1144)] if kind == "items" else [])
        for fid, title, x in fences:
            (root / title).mkdir(parents=True, exist_ok=True)
            con.execute(
                "INSERT INTO fences (id, title, folder_name, x, y, width, height, sort_order, created_at, updated_at) "
                "VALUES (?, ?, ?, ?, 24, 368, 336, ?, ?, ?)",
                (fid, title, title, x, fid - 1, now, now),
            )
            con.execute(
                "INSERT INTO fence_layouts (fence_id, screen_config_id, x, y, width, height, updated_at) "
                "VALUES (?, '1_1920x1080@100', ?, 24, 368, 336, ?)",
                (fid, x, now),
            )
        if kind == "items":
            for order, (name, content) in enumerate(FENCE_ITEMS):
                path = root / "收纳箱" / name
                if content is None:
                    path.mkdir()
                else:
                    path.write_text(content)
                con.execute(
                    "INSERT INTO fence_items (fence_id, file_name, sort_order) VALUES (1, ?, ?)", (name, order)
                )

    # ---- 场景 ----
    def run_scene(self, scene: Scene):
        print(f"== {scene.name}")
        # 光标先放到所有窗口之外（窗口出现在光标下会带着悬停样式，且 GPUI 不会在光标离开后清除）
        self.xdo("mousemove", 5, 1075)
        data = self.prepare_data(scene)
        env = dict(self.env)
        env.update(
            {
                "MINI_TODO_DATA_DIR": str(data),
                "MINI_TODO_FENCES_ROOT": str(data / "fences"),
                "MINI_TODO_DEBUG_OPEN": scene.open,
            }
        )
        env.update({k: v.replace("{data}", str(data)) for k, v in scene.env.items()})
        log = open(self.out / "logs" / f"{scene.name}.log", "w")
        app = subprocess.Popen([str(APP)], env=env, stdout=log, stderr=log)
        ctx = Ctx(self, data)
        try:
            for _ in range(200):
                if self.window_id(MAIN):
                    break
                time.sleep(0.1)
            else:
                raise RuntimeError("主窗口没有出现")
            time.sleep(scene.wait)
            self.park(scene.park)
            for step in scene.steps:
                self.run_step(scene, step, ctx)
                if step[0] not in ("sleep", "shot", "check", "sql", "wait_window"):
                    time.sleep(scene.step_delay)
            time.sleep(scene.settle)
            if scene.shot:
                path = self.out / f"{scene.name}.png"
                if self.screenshot(scene.window, path):
                    print(f"    {path}")
        except Exception as e:  # noqa: BLE001
            print(f"    !! {e}")
            self.results.append((scene.name, "运行", False, str(e)))
        finally:
            app.terminate()
            try:
                app.wait(timeout=5)
            except subprocess.TimeoutExpired:
                app.kill()
            log.close()
            if self.args.keep_data:
                print(f"    数据目录保留在 {data}")
            else:
                shutil.rmtree(data, ignore_errors=True)

    def park(self, mode):
        if mode == "main":
            wid = self.window_id(MAIN)
            if wid:
                x, y, _, _ = self.geometry(wid)
                # 先移进一行（刷新悬停状态），再停到标题栏空白处（版本号下方）
                self.xdo("mousemove", x + 200, y + 135)
                time.sleep(0.3)
                self.xdo("mousemove", x + 100, y + 45)
            time.sleep(0.5)
        elif mode == "away":
            self.xdo("mousemove", 100, 1000)
            time.sleep(0.3)

    def run_step(self, scene: Scene, step, ctx):
        kind = step[0]
        if kind == "call":
            step = CALLS[step[1]]
            kind = step[0]
        if kind == "sleep":
            time.sleep(step[1])
        elif kind == "away":
            self.xdo("mousemove", 100, 1000)
            time.sleep(0.4)
        elif kind == "move":
            self.at(step[1], step[2], step[3])
        elif kind == "wmove":
            wid = self.window_id(step[1])
            if wid is None:
                raise RuntimeError(f"找不到窗口「{step[1]}」")
            self.xdo("windowmove", wid, step[2], step[3])
        elif kind in ("click", "rclick", "dbl"):
            self.at(step[1], step[2], step[3])
            time.sleep(0.15)
            if kind == "click":
                self.xdo("click", 1)
            elif kind == "rclick":
                self.xdo("click", 3)
            else:
                self.xdo("click", "--repeat", 2, "--delay", 80, 1)
        elif kind == "wheel":
            self.at(step[1], step[2], step[3])
            n = step[4]
            self.xdo("click", "--repeat", abs(n), "--delay", 30, 5 if n > 0 else 4)
        elif kind == "drag":
            _, title, x1, y1, x2, y2 = step
            wid = self.window_id(title)
            gx, gy, _, _ = self.geometry(wid)
            self.xdo("mousemove", gx + x1, gy + y1)
            time.sleep(0.3)
            self.xdo("mousedown", 1)
            for i in range(1, 13):
                self.xdo("mousemove", gx + x1 + (x2 - x1) * i // 12, gy + y1 + (y2 - y1) * i // 12)
                time.sleep(0.08)
        elif kind == "down":
            self.xdo("mousedown", 1)
        elif kind == "up":
            self.xdo("mouseup", 1)
        elif kind == "key":
            self.xdo("key", step[1])
        elif kind == "type":
            self.xdo("type", "--delay", 40, step[1])
        elif kind == "shot":
            path = self.out / f"{step[2]}.png"
            if self.screenshot(step[1], path):
                print(f"    {path}")
        elif kind == "sql":
            con = sqlite3.connect(ctx.data / "data.db", timeout=5)
            con.execute(step[1])
            con.commit()
            con.close()
        elif kind == "wait_window":
            deadline = time.time() + step[2]
            while time.time() < deadline and self.window_id(step[1]) is None:
                time.sleep(0.5)
            time.sleep(1.0)
        elif kind == "check":
            try:
                ok, detail = step[2](ctx)
            except Exception as e:  # noqa: BLE001
                ok, detail = False, f"异常：{e}"
            mark = "PASS" if ok else "FAIL"
            print(f"    [{mark}] {step[1]}：{detail}")
            self.results.append((scene.name, step[1], ok, detail))
        else:
            raise ValueError(f"未知动作 {step!r}")

    # ---- 对照图 ----
    def compare(self, scenes):
        refs = [Path(p) for p in self.args.ref]
        before = [Path(p) for p in self.args.before]
        cmp_dir = self.out / "compare"
        cmp_dir.mkdir(exist_ok=True)
        made = 0
        for shot in sorted(self.out.glob("*.png")):
            name = shot.stem
            scene = next((s for s in scenes if s.name == name), None)
            ref_name = (scene.ref if scene and scene.ref else name) + ".png"
            ref = next((d / ref_name for d in refs if (d / ref_name).is_file()), None)
            prev = next((d / shot.name for d in before if (d / shot.name).is_file()), None)
            if ref is None and prev is None:
                continue
            parts = []
            if ref is not None:
                flat = cmp_dir / f".ref-{name}.png"
                # 参考图的透明部分铺黑底（与 Xvfb 没有合成器时透明窗口的截图一致）
                subprocess.run(["convert", str(ref), "-background", "black", "-flatten", str(flat)], check=True)
                parts.append(str(flat))
            if prev is not None:
                parts.append(str(prev))
            parts.append(str(shot))
            subprocess.run(
                ["convert", *parts, "-background", "#FF00FF", "-splice", "4x0", "+append", str(cmp_dir / shot.name)],
                check=True,
            )
            for p in parts:
                if Path(p).name.startswith(".ref-"):
                    Path(p).unlink()
            made += 1
        print(f"对照图 {made} 张：{cmp_dir}（左：参考图{'，中：--before' if before else ''}，右：本次）")


class Ctx:
    def __init__(self, runner: Runner, data: Path):
        self.runner = runner
        self.data = data

    def window_id(self, title):
        return self.runner.window_id(title)


def main():
    # 场景里的 SQL（`datetime('now','localtime')`）与应用用同一个时区
    os.environ["TZ"] = "Asia/Shanghai"
    time.tzset()
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", default=str(WORKSPACE / "target" / "regress"), help="输出目录（默认 target/regress）")
    parser.add_argument("--display", default=":75", help="Xvfb 显示号（默认 :75）")
    parser.add_argument("--ref", action="append", default=[], help="参考截图目录（可多次；对照图左侧）")
    parser.add_argument("--before", action="append", default=[], help="上一次的截图目录（可多次；对照图中间）")
    parser.add_argument("--only", action="append", default=[], help="只跑名字匹配的场景（glob，可多次）")
    parser.add_argument("--group", choices=["scene", "interaction", "smoke", "all"], default=None,
                        help="scene = 与参考截图同名的 48 张；interaction = 关键交互；smoke = 端到端冒烟；"
                             "缺省 scene + interaction")
    parser.add_argument("--smoke", action="store_true", help="同 --group smoke")
    parser.add_argument("--no-build", action="store_true", help="跳过 cargo build")
    parser.add_argument("--keep-data", action="store_true", help="保留每个场景的数据目录")
    parser.add_argument("--list", action="store_true", help="列出场景")
    parser.add_argument("--tmp", default=None, help="临时数据目录的父目录")
    args = parser.parse_args()
    if not args.ref and os.environ.get("MINI_TODO_REF_DIR"):
        args.ref = os.environ["MINI_TODO_REF_DIR"].split(os.pathsep)

    group = "smoke" if args.smoke else args.group
    pool = SCENES + SMOKE
    if group == "all":
        scenes = pool
    elif group:
        scenes = [s for s in pool if s.group == group]
    else:
        scenes = [s for s in pool if s.group != "smoke"]
    if args.only:
        scenes = [s for s in pool if any(fnmatch.fnmatch(s.name, p) for p in args.only)]
    if args.list:
        for s in scenes:
            print(f"{s.group:12} {s.name:32} open={s.open}")
        return 0

    for tool in ("Xvfb", "xdotool", "import", "convert"):
        if shutil.which(tool) is None:
            print(f"缺少命令：{tool}", file=sys.stderr)
            return 1
    if not args.no_build:
        subprocess.run(["cargo", "build", "-p", "mini-todo"], cwd=WORKSPACE, check=True)
        subprocess.run(["cargo", "build", "-p", "mini-todo-core", "--example", "seed"], cwd=WORKSPACE, check=True)

    runner = Runner(args)
    runner.start_xvfb()
    try:
        for scene in scenes:
            runner.run_scene(scene)
    finally:
        runner.stop_xvfb()
    runner.compare(scenes)
    if runner.results:
        print("\n结果：")
        for name, what, ok, detail in runner.results:
            print(f"  {'PASS' if ok else 'FAIL'}  {name}  {what}  {detail}")
        failed = [r for r in runner.results if not r[2]]
        (runner.out / "results.json").write_text(
            json.dumps([dict(scene=n, check=w, ok=o, detail=d) for n, w, o, d in runner.results], ensure_ascii=False, indent=2)
        )
        return 1 if failed else 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
