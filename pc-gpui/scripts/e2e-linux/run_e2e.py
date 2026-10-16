"""GPUI 版 Linux 端到端测试：真实应用（xdotool 黑盒驱动）+ Apache / nginx WebDAV + cloud + Skill CLI。

usage: python3 run_e2e.py <app> <cloud> [prefix ...]
  <app>    pc-gpui 的 debug 构建（target/debug/mini-todo）；同目录 examples/seed 用来造样例数据
           （或用 MT_E2E_SEED 指定）
  <cloud>  cloud/target/debug/minitodo-cloud
  prefix   只跑名字以它开头的用例，如 F（本地）、S（同步）、L（生命周期）、F03、S04[nginx]
结果：$MT_E2E_WORK/run/results.txt；截图 $MT_E2E_WORK/run/shots/；各进程输出 $MT_E2E_WORK/run/*.log
"""
from __future__ import annotations

import datetime as dt
import json
import os
import signal
import subprocess
import sys
import threading
import time
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import ui  # noqa: E402
from harness import (DAV, DISPLAYS, REPO, RUN, App, Cloud, LogMark, NotApplicable,  # noqa: E402
                     Procs, Results, Session, XDisplay, box_center, check, dav_get_sync, dav_list,
                     dav_put_sync, dav_reset, is_canon, log, mock_settings, seed, todo_doc, wait_until)

APP_BIN, CLOUD_BIN = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
SEED_BIN = Path(os.environ.get("MT_E2E_SEED") or APP_BIN.parent / "examples" / "seed").resolve()
ONLY = sys.argv[3:]
SAMPLE = REPO / "pc-gpui" / "scripts" / "sample-data.json"
REF_SHOTS = Path(os.environ.get("MT_E2E_REF_SHOTS", ""))  # 可选：参考截图目录（只用于报告里的差异度量）

procs = Procs()
res = Results(ONLY)
X: list[XDisplay] = []
CURRENT: list[App] = []  # 失败时截图用


def now_local() -> dt.datetime:
    return dt.datetime.now(dt.timezone(dt.timedelta(hours=8))).replace(tzinfo=None)


def fmt(d: dt.datetime) -> str:
    return d.strftime("%Y-%m-%d %H:%M:%S")


def want(prefix: str) -> bool:
    return not ONLY or any(p.startswith(prefix) or prefix.startswith(p.partition("[")[0]) for p in ONLY)


def on_fail(case: str) -> None:
    """失败现场截图，然后关掉主窗口以外的窗口（不让一个用例的残留窗口挡住后面的用例）"""
    for app in CURRENT:
        if app.alive():
            app.shot(f"FAIL-{case}-{app.x.num[1:]}")
            dismiss_children(app)


def dismiss_children(app: App) -> None:
    try:
        app.key(app.main_win(), "Escape")  # 主窗口里的确认框：Esc = 取消
    except Exception:
        pass
    for _ in range(3):
        others = [w for w in app.x.windows() if w.title not in (App.MAIN, ui.NOTIFICATION)]
        if not others:
            return
        for w in others:
            app.key(w, "Escape")      # 编辑类窗口：Esc 关闭；有未保存修改时弹「放弃修改」
            time.sleep(0.6)
            if app.x.find(w.title):
                app.key(w, "Return")  # 确认框：Enter = 确认（放弃 / 删除…）
                time.sleep(0.6)
            if app.x.find(w.title) and w.title == ui.SETTINGS:
                app.x.click(w, 636, 28)
            if app.x.find(w.title) and w.title == ui.COMPLETED:
                app.x.click(w, w.w - 26, 20)
            time.sleep(0.4)


def display(i: int = 0) -> XDisplay:
    while len(X) <= i:
        X.append(XDisplay(procs, DISPLAYS[len(X)]).start())
    return X[i]


def launch(session: Session, i: int = 0, extra_env: dict[str, str] | None = None, tag: str = "") -> App:
    app = App(procs, APP_BIN, session, display(i), extra_env, tag).start()
    CURRENT.append(app)
    return app


def shutdown(app: App, sig: int = signal.SIGTERM) -> None:
    app.stop(sig)
    if app in CURRENT:
        CURRENT.remove(app)


def png(path: Path, color: str, size: str = "160x100") -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["convert", "-size", size, f"xc:{color}", str(path)], check=True, capture_output=True)
    return path


def count_color(img: Path, rgb: tuple[int, int, int], tol: int = 12) -> int:
    """图片里接近 rgb 的像素数"""
    r, g, b = rgb
    out = subprocess.run(["convert", str(img), "-fuzz", f"{tol * 100 // 255}%", "-fill", "black", "+opaque",
                          f"rgb({r},{g},{b})", "-fill", "white", "-opaque", f"rgb({r},{g},{b})",
                          "-format", "%[fx:mean*w*h]", "info:"], capture_output=True, text=True)
    try:
        return int(float(out.stdout.strip() or 0))
    except ValueError:
        return 0


def compare_ref(shot: Path, ref_name: str) -> str:
    """与参考截图的差异度量（RMSE，0~1，越小越接近；只写进报告，不作断言）"""
    ref = REF_SHOTS / ref_name if str(REF_SHOTS) else None
    if not ref or not ref.exists():
        return ""
    r = subprocess.run(["compare", "-metric", "RMSE", str(shot), str(ref), "null:"], capture_output=True, text=True)
    txt = (r.stderr or r.stdout).strip()
    return f"; RMSE vs {ref_name} = {txt.split('(')[-1].rstrip(')') if '(' in txt else txt}"


# ============================================================================ F：本地


def suite_sample() -> None:
    """F00：官方样例数据（scripts/sample-data.json）启动与显示"""
    if not res.wanted("F00"):
        return
    s = Session("sample")
    seed(SEED_BIN, s, json.loads(SAMPLE.read_text(encoding="utf-8")))
    app = launch(s)
    try:
        def f00():
            main = app.main_win()
            check((main.w, main.h) == (380, 600), f"main window size {main.w}x{main.h}")
            p = app.shot("F00-main-list", main)
            for t in ["季度 OKR 复盘材料", "准备技术分享", "回复供应商的报价邮件", "英语口语练习", "缴纳房租"]:
                app.wait_text(main, t, 8, region=ui.list_region(main), fuzzy=0.7)
            app.wait_text(main, "1/3", 6, region=ui.list_region(main), fuzzy=0.9)
            n = s.one("SELECT COUNT(*) c FROM todos")["c"]
            check(n == 11, f"todos in db: {n}")
            # 四象限截图（对照 ref-ui/screens/main-quadrant.png）
            ui.click_title(app, "view")
            wait_until(lambda: s.setting("view_mode") == "quadrant", 8, what="view_mode=quadrant")
            time.sleep(0.8)
            app.x.move(main, 100, 46)
            q = app.shot("F00-main-quadrant", main)
            app.wait_text(main, "不紧急不重要", 8, region=ui.list_region(main))
            ui.click_title(app, "view")
            wait_until(lambda: s.setting("view_mode") == "list", 8, what="view_mode=list")
            return f"11 todos shown{compare_ref(p, 'main-list.png')}{compare_ref(q, 'main-quadrant.png')}"
        res.run("F00 startup with sample data: main list / quadrant render (screenshots)", f00, on_fail)
    finally:
        shutdown(app)


LOCAL_TITLES = {
    1: "Alpha report", 2: "Bravo meeting", 3: "Charlie errand", 4: "Delta reading", 5: "Golf custom",
    6: "Echo done",
}


LOCAL_CASES = ["F01", "F02", "F02b", "F03", "F03b", "F04", "F05", "F06", "F06b", "F07", "F08", "F09a", "F09b",
               "F10", "F11", "F12", "F13", "F14", "F15", "F16"]


def suite_local() -> None:
    if not any(res.wanted(c) for c in LOCAL_CASES):
        return
    s = Session("local")
    ts = fmt(now_local())
    yesterday_8 = (now_local() - dt.timedelta(days=1)).replace(hour=8, minute=0, second=0, microsecond=0)
    tomorrow = (now_local() + dt.timedelta(days=1)).strftime("%Y-%m-%d")
    seed(SEED_BIN, s, {
        "version": "4.0", "exportedAt": ts,
        "todos": [
            todo_doc(1, LOCAL_TITLES[1], ts, color="#EF4444", quadrant=1, sortOrder=0),
            todo_doc(2, LOCAL_TITLES[2], ts, color="#F59E0B", quadrant=2, sortOrder=1),
            todo_doc(3, LOCAL_TITLES[3], ts, color="#3B82F6", quadrant=3, sortOrder=2,
                     endTime=f"{tomorrow} 18:00:00"),
            todo_doc(4, LOCAL_TITLES[4], ts, color="#10B981", quadrant=4, sortOrder=3,
                     subtasks=[{"id": 401, "parentId": 4, "title": "delta child", "content": None,
                                "completed": False, "sortOrder": 0, "createdAt": ts, "updatedAt": ts}]),
            todo_doc(5, LOCAL_TITLES[5], ts, color="#6366F1", quadrant=2, sortOrder=4),
            todo_doc(6, LOCAL_TITLES[6], ts, completed=True, sortOrder=5),
            # 启动补发：昨天 08:00 的每日重复提醒
            todo_doc(7, "Juliet missed", ts, sortOrder=6, notifyAt=fmt(yesterday_8), repeatEnabled=True,
                     repeatType="daily", repeatInterval=1),
        ],
        "settings": mock_settings(notificationType="app"),
    })
    export_zip = s.root / "export" / "mini-todo-backup.zip"
    export_zip.parent.mkdir(parents=True, exist_ok=True)
    app = launch(s, extra_env={"MINI_TODO_DEBUG_DIALOG_PATH": str(export_zip)})
    try:
        _local_cases(app, s, export_zip, yesterday_8)
    finally:
        shutdown(app)


def _local_cases(app: App, s: Session, export_zip: Path, yesterday_8: dt.datetime) -> None:
    main = app.main_win()

    def f01():
        app.shot("F01-before", main)
        for t in ("Alpha report", "Bravo meeting", "Golf custom"):
            ui.wait_main_shows(app, t, 8)
        app.x.click(main, *ui.fab(main))
        ed = app.win(ui.EDITOR_NEW, 15)
        time.sleep(0.8)
        app.shot("F01-editor-new", ed)
        app.type(ed, "Foxtrot created")
        # 连按三次 Ctrl+Enter：保存有防重入，只能建一条
        app.x.focus(ed)
        for _ in range(3):
            app.x.xdo("key", "--clearmodifiers", "ctrl+Return")
        app.x.wait_window_gone(ui.EDITOR_NEW, 10)
        rows = s.query("SELECT * FROM todos WHERE title='Foxtrot created'")
        check(len(rows) == 1, f"expected exactly 1 created row, got {len(rows)}")
        r = rows[0]
        check(is_canon(r["created_at"]) and is_canon(r["updated_at"]), f"timestamps {r}")
        check(r["color"] == "#10B981" and r["quadrant"] == 4, f"defaults color/quadrant: {r['color']} {r['quadrant']}")
        ui.wait_main_shows(app, "Foxtrot created", 10)
        app.shot("F01-after", main)
        return f"id={r['id']} created_at={r['created_at']}"
    res.run("F01 FAB -> new editor -> type title -> Ctrl+Enter (x3): one row in db, shown in list", f01, on_fail)

    def f02():
        before = s.one("SELECT * FROM todos WHERE id=1")
        ed = ui.open_detail(app, "Alpha report")
        app.wait_text(ed, "Alpha report", 8, region=(0, 56, 700, 120))
        app.shot("F02-detail-view", ed)
        ui.editor_click_edit(app, ed)
        app.shot("F02-detail-edit", ed)
        ui.editor_set_title(app, ed, "Alpha report v2")
        ui.editor_save(app, ed)
        after = s.one("SELECT * FROM todos WHERE id=1")
        check(after["title"] == "Alpha report v2", f"title in db: {after['title']!r}")
        check(is_canon(after["updated_at"]) and after["updated_at"] > before["updated_at"],
              f"updated_at {before['updated_at']} -> {after['updated_at']}")
        ui.wait_main_shows(app, "Alpha report v2", 10)
        return f"updated_at {before['updated_at']} -> {after['updated_at']}"
    res.run("F02 click todo -> read-only detail -> 编辑 -> change title -> save: db + list updated", f02, on_fail)

    def f02b():
        before = s.one("SELECT title, updated_at FROM todos WHERE id=2")
        ed = ui.open_detail(app, before["title"])
        ui.editor_click_edit(app, ed)
        ui.editor_set_title(app, ed, "Bravo unsaved")
        app.key(ed, "Escape")
        app.wait_text(ed, "放弃修改", 6)
        app.shot("F02b-discard-confirm", ed)
        app.key(ed, "Escape")  # 确认框 Esc = 取消 → 留在编辑窗口
        time.sleep(0.6)
        check(app.x.find(ui.EDITOR), "editor closed although the discard dialog was cancelled")
        app.key(ed, "Escape")
        app.wait_text(ed, "放弃修改", 6)
        ui.confirm_dialog(app, ed, "放弃修改", "放弃")
        app.x.wait_window_gone(ui.EDITOR, 8)
        after = s.one("SELECT title, updated_at FROM todos WHERE id=2")
        check(after == before, f"discarded edit was written: {before} -> {after}")
        return "Esc on a dirty form asks 放弃修改; cancel keeps the window, 放弃 closes without writing"
    res.run("F02b Esc with unsaved edits: 放弃修改 confirm (cancel keeps editing, confirm discards)", f02b, on_fail)

    def f03():
        ed = ui.open_detail(app, "Bravo meeting")
        ui.editor_click_edit(app, ed)
        ui.subtask_panel_add(app, ed, "sub alpha", "sub beta", existing=0)
        subs = s.query("SELECT * FROM subtasks WHERE parent_id=2 ORDER BY sort_order")
        check([x["title"] for x in subs] == ["sub alpha", "sub beta"], f"subtasks {subs}")
        check(all(is_canon(x["created_at"]) for x in subs), f"subtask timestamps {subs}")
        ui.subtask_toggle(app, ed, "sub alpha")
        wait_until(lambda: s.one("SELECT completed FROM subtasks WHERE title='sub alpha'")["completed"] == 1, 8,
                   what="sub alpha completed")
        app.wait_text(ed, "1/2", 6, region=(700, 56, 380, 100), fuzzy=0.9)
        app.shot("F03-subtasks", ed)
        app.key(ed, "Escape")  # 表单没改 → 直接关闭
        app.x.wait_window_gone(ui.EDITOR, 8)
        x, y, w, h = ui.find_row(app, "Bravo meeting")
        main_ = app.main_win()
        app.wait_text(main_, "1/2", 8, region=(0, int(y + h), main_.w, 30), fuzzy=0.9)
        app.shot("F03-main-counter", main_)
        return "2 subtasks, 1 done, list shows 1/2"
    res.run("F03 subtasks: add two in editor, tick one -> db + 1/2 counter in main list", f03, on_fail)

    def f03b():
        # 「展开全部子任务」在左区最右（有已完成待办时紧挨「已完成」按钮左边 28px）
        check(s.one("SELECT COUNT(*) c FROM todos WHERE completed=1")["c"] > 0, "need a completed todo for the layout")
        if not s.query("SELECT id FROM subtasks WHERE parent_id=2"):
            n = fmt(now_local())
            s.execute("INSERT INTO subtasks (parent_id, title, sort_order, created_at, updated_at) "
                      "VALUES (2, 'sub alpha', 0, ?, ?)", (n, n))
        main_ = app.main_win()
        cx_, cy_ = ui.title_button(main_, "completed")
        state_file = s.data_dir / "ui-state.json"
        ui.wait_no_toast(app, main_)
        app.x.click(main_, cx_ - 28, cy_)
        sub = s.one("SELECT title FROM subtasks WHERE parent_id=2 AND completed=0")
        sub_title = sub["title"] if sub else "sub alpha"  # 已完成的子任务带删除线，OCR 认不准
        app.wait_text(main_, sub_title, 8, region=ui.list_region(main_))
        app.x.move(main_, 100, 46)
        app.shot("F03b-expanded", main_)
        saved = wait_until(lambda: state_file.exists() and json.loads(state_file.read_text()), 5, what="ui-state.json")
        check(any(v is True for v in saved.values()), f"ui-state.json {saved}")
        app.x.click(main_, cx_ - 28, cy_)
        app.wait_text(main_, sub_title, 8, region=ui.list_region(main_), present=False)
        return f"expand all shows subtasks in the list; ui-state.json {saved}"
    res.run("F03b 展开全部子任务 shows subtask titles in the list, persisted to ui-state.json", f03b, on_fail)

    def f04():
        seq0 = s.one("SELECT value FROM sync_meta WHERE key='local_seq'")["value"]
        ui.row_action(app, "Charlie errand", "complete")
        wait_until(lambda: s.one("SELECT completed FROM todos WHERE id=3")["completed"] == 1, 8,
                   what="Charlie completed in db")
        ui.wait_main_shows(app, "Charlie errand", 8, present=False)
        ui.click_title(app, "completed")
        cw = app.win(ui.COMPLETED, 10)
        time.sleep(0.8)
        # 已完成的标题带删除线，OCR 会错几个字母：放宽相似度
        app.wait_text(cw, "Charlie errand", 8, fuzzy=0.65)
        app.wait_text(cw, "Echo done", 6, fuzzy=0.65)
        app.shot("F04-completed-window", cw)
        seq1 = s.one("SELECT value FROM sync_meta WHERE key='local_seq'")["value"]
        check(seq1 > seq0, f"local_seq {seq0} -> {seq1}")
        # 悬停「Echo done」→「恢复为未完成」（行右侧两个 32px 操作：恢复 w-84、删除 w-48）
        b = app.wait_text(cw, "Echo done", 6, fuzzy=0.65)
        _, cy = box_center(b)  # type: ignore[arg-type]
        app.x.move(cw, 120, cy)
        time.sleep(0.5)
        app.x.click(cw, cw.w - 84, cy)
        wait_until(lambda: s.one("SELECT completed FROM todos WHERE id=6")["completed"] == 0, 8,
                   what="Echo restored")
        ui.close_completed(app, cw)
        ui.wait_main_shows(app, "Echo done", 10)
        return f"completed; local_seq {seq0}->{seq1}; Echo restored from 已完成"
    res.run("F04 complete via row action -> listed in 已完成 window; 恢复为未完成 brings one back", f04, on_fail)

    def f05():
        ui.row_action(app, "Delta reading", "delete")
        main_ = app.main_win()
        time.sleep(0.5)
        app.shot("F05-delete-confirm", main_)
        ui.confirm_dialog(app, main_, "删除确认", "删除")
        wait_until(lambda: not s.query("SELECT id FROM todos WHERE id=4"), 8, what="todo 4 deleted")
        tomb = s.query("SELECT * FROM tombstones")
        keys = {(t["entity_type"], t["entity_id"]) for t in tomb}
        check(("todo", 4) in keys and ("subtask", 401) in keys, f"tombstones {tomb}")
        check(all(is_canon(t["deleted_at"]) for t in tomb), f"deleted_at format {tomb}")
        check(not s.query("SELECT id FROM subtasks WHERE parent_id=4"), "subtask of deleted todo survived")
        ui.wait_main_shows(app, "Delta reading", 8, present=False)
        return f"tombstones {sorted(keys)}"
    res.run("F05 delete via row action + 删除确认 -> row, subtasks gone; tombstones written", f05, on_fail)

    def f06():
        ui.click_title(app, "view")
        wait_until(lambda: s.setting("view_mode") == "quadrant", 8, what="view_mode=quadrant")
        time.sleep(1)
        main_ = app.main_win()
        app.x.move(main_, 100, 46)
        app.shot("F06-quadrant-before", main_)
        try:
            # 默认色（#10B981，第 4 象限）拖到第 1 象限：颜色跟随新象限
            app.x.drag(main_, ui.row_handle(app, "Foxtrot created"), ui.quadrant_drop_point(app, 1))
            wait_until(lambda: s.one("SELECT quadrant FROM todos WHERE title='Foxtrot created'")["quadrant"] == 1,
                       8, what="Foxtrot moved to quadrant 1")
            r1 = s.one("SELECT quadrant, color FROM todos WHERE title='Foxtrot created'")
            check(r1["color"] == "#EF4444", f"color did not follow quadrant: {r1}")
            time.sleep(0.8)
            # 自定义色（#6366F1，第 2 象限）拖到第 3 象限：颜色保留
            app.x.drag(main_, ui.row_handle(app, "Golf custom"), ui.quadrant_drop_point(app, 3))
            wait_until(lambda: s.one("SELECT quadrant FROM todos WHERE title='Golf custom'")["quadrant"] == 3, 8,
                       what="Golf moved to quadrant 3")
            r2 = s.one("SELECT quadrant, color FROM todos WHERE title='Golf custom'")
            check(r2["color"] == "#6366F1", f"custom color changed: {r2}")
            app.x.move(main_, 100, 46)
            time.sleep(0.5)
            app.shot("F06-quadrant-after", main_)
            app.wait_text(main_, "Foxtrot created", 8, region=(0, 52, main_.w // 2, 230))
        finally:
            app.x.move(main_, 100, 46)
            ui.click_title(app, "view")
            wait_until(lambda: s.setting("view_mode") == "list", 8, what="view_mode=list")
        return f"Foxtrot -> q1 {r1['color']}; Golf -> q3 keeps {r2['color']}"
    res.run("F06 quadrant view drag & drop changes quadrant (+ default color follows)", f06, on_fail)

    def pending_order() -> list[str]:
        return [r["title"] for r in s.query("SELECT title FROM todos WHERE completed=0 ORDER BY sort_order, id")]

    def f06b():
        main_ = app.main_win()
        before = pending_order()
        check(len(before) >= 3, f"need 3 pending todos, got {before}")
        first, mover = before[0], before[2]
        fx, fy, fw, fh = ui.find_row(app, first)
        app.x.drag(main_, ui.row_handle(app, mover), (fx + 40, fy - 6))
        wait_until(lambda: pending_order()[0] == mover, 8, what=f"{mover!r} dragged to the top")
        after_drag = pending_order()
        check(after_drag[1:3] == [first, before[1]], f"order after drag {after_drag}")
        app.x.move(main_, 100, 46)
        time.sleep(0.6)
        app.shot("F06b-list-reordered", main_)
        app.wait_text(main_, mover, 8, region=(0, 52, main_.w, 60))
        # 悬停操作条「置顶」
        target = after_drag[-1]
        ui.row_action(app, target, "top")
        wait_until(lambda: pending_order()[0] == target, 8, what=f"{target!r} moved to top")
        return f"drag: {before[:3]} -> {after_drag[:3]}; 置顶 {target!r}"
    res.run("F06b list view: drag handle reorders, 置顶 action moves a todo to the top (sort_order)", f06b, on_fail)

    def f07():
        main_ = app.main_win()
        app.x.click(main_, *ui.fab(main_))
        ed = app.win(ui.EDITOR_NEW, 15)
        time.sleep(0.8)
        app.type(ed, "Hotel timed")
        app.x.scroll(ed, 350, 300, 10)
        app.shot("F07-editor-times", ed)
        fill_date_time(app, ed, "开始日期", "2026-11-28", None)
        fill_date_time(app, ed, "截止日期", "2026-12-01", "18:30")
        fill_date_time(app, ed, "提醒", "2026-11-30", "09:15", label=True)
        app.shot("F07-editor-times-filled", ed)
        ui.editor_save(app, ed, ui.EDITOR_NEW)
        r = s.one("SELECT * FROM todos WHERE title='Hotel timed'")
        check(r is not None, "Hotel timed not created")
        check(r["start_time"] == "2026-11-28 00:00:00", f"start_time {r['start_time']!r}")
        check(r["end_time"] == "2026-12-01 18:30:00", f"end_time {r['end_time']!r}")
        check(r["notify_at"] == "2026-11-30 09:15:00", f"notify_at {r['notify_at']!r}")
        check(is_canon(r["created_at"]) and is_canon(r["updated_at"]), f"timestamps {r}")
        # 编辑：改提醒时间 → 仍是规范格式
        ed = ui.open_detail(app, "Hotel timed")
        app.wait_text(ed, "2026-11-30 09:15", 6, fuzzy=0.85)
        ui.editor_click_edit(app, ed)
        app.x.scroll(ed, 350, 300, 10)
        fill_date_time(app, ed, "提醒", None, "21:05", label=True)
        ui.editor_save(app, ed)
        r2 = s.one("SELECT * FROM todos WHERE title='Hotel timed'")
        check(r2["notify_at"] == "2026-11-30 21:05:00", f"edited notify_at {r2['notify_at']!r}")
        check(r2["updated_at"] > r["updated_at"] and is_canon(r2["updated_at"]), f"updated_at {r2['updated_at']}")
        return f"start={r['start_time']} end={r['end_time']} notify={r['notify_at']} -> {r2['notify_at']}"
    res.run("F07 date/time pickers (typed) store canonical YYYY-MM-DD HH:MM:SS on create and edit", f07, on_fail)

    def f08():
        ts = fmt(now_local())
        main_ = app.main_win()
        new_id = s.execute("INSERT INTO todos (title, color, quadrant, sort_order, created_at, updated_at) "
                           "VALUES ('India external', '#10B981', 4, 50, ?, ?)", (ts, ts))
        t0 = time.time()
        ui.wait_main_shows(app, "India external", 12)
        took = time.time() - t0
        s.execute("UPDATE todos SET title='India renamed' WHERE id=?", (new_id,))
        ui.wait_main_shows(app, "India renamed", 12)
        app.shot("F08-polled", main_)
        return f"external insert visible after {took:.1f}s (poll 5s)"
    res.run("F08 external db writes show up via 5s change-seq polling", f08, on_fail)

    def f09a():
        r = s.one("SELECT notify_at, notified FROM todos WHERE id=7")
        nxt = dt.datetime.strptime(r["notify_at"], "%Y-%m-%d %H:%M:%S")
        check(nxt > now_local(), f"missed repeat not advanced to the future: {r}")
        check(nxt.time() == yesterday_8.time(), f"time of day changed: {r}")
        check(r["notified"] == 0, f"repeat should stay armed: {r}")
        return f"{fmt(yesterday_8)} -> {r['notify_at']}"
    res.run("F09a missed daily repeat reminder is caught up at startup and advanced", f09a, on_fail)

    def f09b():
        # 下一个整分钟到期（不足 25s 就再推一分钟）；调度器对齐整分钟
        n = now_local()
        due = (n + dt.timedelta(minutes=1)).replace(second=0, microsecond=0)
        if (due - n).total_seconds() < 25:
            due += dt.timedelta(minutes=1)
        ts = fmt(n)
        tid = s.execute(
            "INSERT INTO todos (title, description, color, quadrant, notify_at, notify_before, notified, sort_order, "
            "created_at, updated_at, repeat_enabled, repeat_type, repeat_interval) "
            "VALUES ('Kilo repeat', 'daily standup', '#10B981', 4, ?, 0, 0, 60, ?, ?, 1, 'daily', 1)",
            (fmt(due), ts, ts))
        before = {w.wid for w in app.x.find(ui.NOTIFICATION)}
        wait_s = (due - now_local()).total_seconds() + 70
        win = wait_until(lambda: [w for w in app.x.find(ui.NOTIFICATION) if w.wid not in before], wait_s, 1,
                         "notification window")[0]
        time.sleep(1)
        app.shot("F09b-notification", win)
        app.shot("F09b-screen", None)
        app.wait_text(win, "Kilo repeat", 6, fuzzy=0.75)
        check(0 <= win.x and win.x + win.w <= 1600 and 0 <= win.y and win.y + win.h <= 1000,
              f"notification off-screen: {win}")
        r = wait_until(lambda: (lambda x: x if x["notify_at"] != fmt(due) else None)(
            s.one("SELECT notify_at, notified FROM todos WHERE id=?", (tid,))), 15, what="notify_at advanced")
        check(r["notify_at"] == fmt(due + dt.timedelta(days=1)), f"advanced to {r['notify_at']}, want +1 day")
        check(r["notified"] == 0, f"repeat should stay armed: {r}")
        wins = app.x.find(ui.NOTIFICATION)
        spots = {(w.x, w.y) for w in wins}
        check(len(spots) == len(wins), f"notification windows overlap: {wins}")
        for w in wins:  # 关掉通知（右上角 ×）
            app.x.click(w, w.w - 20, 20)
        return f"window {win.w}x{win.h}+{win.x}+{win.y}; {fmt(due)} -> {r['notify_at']}; {len(wins)} notification(s)"
    res.run("F09b daily repeat reminder fires an in-app notification window and advances +1 day", f09b, on_fail)

    def f10():
        raise NotApplicable("单实例只在 Windows 实现（services/single_instance.rs：非 Windows 不做单实例；"
                            "debug 隔离 MINI_TODO_DATA_DIR 时也关闭），需在 Windows 上验收")
    res.run("F10 single instance: second launch exits and wakes the first", f10, on_fail)

    def f11():
        lp = s.log_path
        check(lp.exists(), f"no log file at {lp}")
        text = s.log_text()
        check("启动" in text, "startup line missing")
        check("[dev] 开发专用数据隔离已启用" in text, "isolation notice missing")
        check("panicked" not in text, "panic in log")
        return f"{lp.relative_to(s.root)} ({lp.stat().st_size} bytes)"
    res.run("F11 log written to <data>/logs/mini-todo.log (startup, isolation notice, no panic)", f11, on_fail)

    def f12():
        st = ui.open_settings(app, "数据与同步")
        app.shot("F12-settings-data", st)
        ui.settings_content_top(app, st)
        app.click_text(st, "导出数据", 6, region=(160, 56, 520, 200))
        ui.wait_toast(app, st, "导出成功", 10)
        check(export_zip.exists() and zipfile.is_zipfile(export_zip), "export zip missing")
        with zipfile.ZipFile(export_zip) as z:
            data = json.loads(z.read("data.json"))
        check(data.get("version") == "4.0", f"export version {data.get('version')}")
        exported_ids = {t["id"] for t in data["todos"]}
        ui.close_settings(app, st)
        ids_before = {r["id"] for r in s.query("SELECT id FROM todos")}
        check(ids_before == exported_ids, f"export ids {sorted(exported_ids)} vs db {sorted(ids_before)}")
        backups0 = len(list((s.data_dir / "backups").glob("*.db"))) if (s.data_dir / "backups").exists() else 0
        # 导出后新建一条（UI），导入后它应消失并留下墓碑
        main_ = app.main_win()
        app.x.click(main_, *ui.fab(main_))
        ed = app.win(ui.EDITOR_NEW, 15)
        time.sleep(0.8)
        app.type(ed, "Lima after export")
        ui.editor_save(app, ed, ui.EDITOR_NEW)
        extra = s.one("SELECT id FROM todos WHERE title='Lima after export'")["id"]
        st = ui.open_settings(app, "数据与同步")
        ui.settings_content_top(app, st)
        app.click_text(st, "导入数据", 6, region=(160, 56, 520, 200))
        app.shot("F12-import-confirm", st)
        ui.confirm_dialog(app, st, "导入确认", "确定导入")
        app.x.wait_window_gone(ui.SETTINGS, 15)
        ui.wait_toast(app, app.main_win(), "数据导入成功", 8)
        ids_after = {r["id"] for r in s.query("SELECT id FROM todos")}
        check(ids_after == ids_before, f"ids not preserved: {sorted(ids_before)} vs {sorted(ids_after)}")
        check(("todo", extra) in s.tombstones(), "no tombstone for the record removed by import")
        backups1 = len(list((s.data_dir / "backups").glob("*.db")))
        check(backups1 > backups0, f"no pre-import backup ({backups0} -> {backups1})")
        ui.wait_main_shows(app, "Lima after export", 10, present=False)
        return f"zip {export_zip.stat().st_size}B, {len(ids_after)} ids preserved, tombstone #{extra}, backups {backups1}"
    res.run("F12 settings: 导出数据 (zip) -> 导入数据 + 确认: ids kept, tombstone, auto backup", f12, on_fail)

    def f13():
        main_ = app.main_win()
        light_px = app.x.pixel(main_, 200, 300)
        ui.click_title(app, "theme")
        wait_until(lambda: s.setting("text_theme") == "light", 8, what="text_theme=light (dark theme)")
        time.sleep(0.8)
        app.x.move(main_, 100, 46)
        app.shot("F13-dark", main_)
        dark_px = app.x.pixel(main_, 200, 300)
        check(sum(dark_px) < sum(light_px) - 150, f"background did not darken: {light_px} -> {dark_px}")
        ui.click_title(app, "theme")
        wait_until(lambda: s.setting("text_theme") == "dark", 8, what="text_theme=dark (light theme)")
        # 日历：设置 → 常规 → 展示日历
        st = ui.open_settings(app, "常规")
        x, y = ui.settings_find(app, st, "展示日历", 0)
        app.x.click(st, 636, y + 8)
        wait_until(lambda: s.setting("show_calendar") == "true", 8, what="show_calendar=true")
        ui.close_settings(app, st)
        time.sleep(1.5)
        main_ = app.main_win()
        check(main_.w > 600, f"window did not widen for calendar: {main_.w}")
        app.x.move(main_, 100, 46)
        app.shot("F13-calendar", main_)
        cal = (int(main_.w * 0.4), 0, int(main_.w * 0.6), main_.h)
        app.wait_text(main_, "今天", 8, region=cal)
        # 待办在日历上是彩色横条（白字；没有开始时间的按创建日），点它打开只读详情。
        # 横条上的白字很小、还常被截断，OCR 不稳：按颜色找「Golf custom」那条（全表唯一的靛蓝 #6366F1）
        bar = wait_until(lambda: app.x.find_color(main_, (0x63, 0x66, 0xF1), (cal[0], 90, cal[2], main_.h - 100)),
                         8, what="indigo calendar bar")
        app.x.click(main_, bar[0] + min(bar[2] // 2, 30), bar[1] + bar[3] // 2)
        ed = app.win(ui.EDITOR, 10)
        time.sleep(0.8)
        app.wait_text(ed, "Golf custom", 6, region=(0, 56, 700, 120))
        app.key(ed, "Escape")
        app.x.wait_window_gone(ui.EDITOR, 8)
        st = ui.open_settings(app, "常规")
        x, y = ui.settings_find(app, st, "展示日历", 0)
        app.x.click(st, 636, y + 8)
        wait_until(lambda: s.setting("show_calendar") == "false", 8, what="show_calendar=false")
        ui.close_settings(app, st)
        time.sleep(1)
        w2 = app.main_win().w
        return f"bg {light_px}->{dark_px}; calendar width {main_.w} -> {w2}"
    res.run("F13 theme toggle; 展示日历 widens window, calendar bar opens the detail", f13, on_fail)

    def f16():
        mark = LogMark(s)
        ui.click_title(app, "sync")
        ui.wait_toast(app, app.main_win(), "请先在设置中配置 WebDAV 服务器", 6)
        time.sleep(0.5)
        check("[sync] 同步" not in mark.new_text(), "a sync ran without WebDAV configuration")
        return "warning toast, no sync attempted"
    res.run("F16 title-bar sync without WebDAV config warns 请先在设置中配置 WebDAV 服务器", f16, on_fail)

    def f14():
        # 旧版（Milkdown）点描述框任意位置都进入编辑；空描述时这里点框的中部
        ed = ui.open_detail(app, "Golf custom")
        ui.editor_click_edit(app, ed)
        app.x.click(ed, *ui.ED_DESC_MIDDLE)
        app.type(ed, "typed after clicking the middle")
        time.sleep(0.4)
        p = app.shot("F14-desc-middle-click", ed)
        app.key(ed, "ctrl+Return")
        app.x.wait_window_gone(ui.EDITOR, 10)
        desc = s.one("SELECT description FROM todos WHERE id=5")["description"]
        check(desc == "typed after clicking the middle",
              f"clicking the empty description box (y=260 of 183~307) did not start editing: description={desc!r}; "
              f"see {p.name}")
        return "description editable from anywhere in the box"
    res.run("F14 empty description box: clicking its middle starts Markdown editing", f14, on_fail)

    def f15():
        if not s.query("SELECT id FROM subtasks WHERE parent_id=2"):
            n = fmt(now_local())
            s.execute("INSERT INTO subtasks (parent_id, title, sort_order, created_at, updated_at) "
                      "VALUES (2, 'sub beta', 1, ?, ?)", (n, n))
        sub = s.one("SELECT id, title FROM subtasks WHERE parent_id=2 ORDER BY sort_order DESC")
        ed = ui.open_detail(app, "Bravo meeting")
        ui.editor_click_edit(app, ed)
        b = app.wait_text(ed, sub["title"], 8, region=(700, 56, 380, 544))
        _, cy = box_center(b)  # type: ignore[arg-type]
        app.x.move(ed, 880, cy)
        time.sleep(0.5)
        app.x.click(ed, 1011, cy)  # 悬停操作：查看 985 / 编辑 1011 / 删除 1037
        sw = app.win(ui.SUBTASK_EDIT, 10)
        time.sleep(0.8)
        app.x.click(sw, sw.w // 2, sw.h // 2)
        app.type(sw, "## Notes")
        app.key(sw, "Return")
        app.type(sw, "- item one")
        app.shot("F15-subtask-edit", sw)
        app.key(sw, "ctrl+Return")
        app.x.wait_window_gone(ui.SUBTASK_EDIT, 10)
        content = s.one("SELECT content FROM subtasks WHERE id=?", (sub["id"],))["content"] or ""
        check("## Notes" in content and "- item one" in content, f"subtask content {content!r}")
        app.x.move(ed, 880, cy)
        time.sleep(0.5)
        app.x.click(ed, 985, cy)
        sv = app.win(ui.SUBTASK_VIEW, 10)
        time.sleep(1)
        app.wait_text(sv, "item one", 8)
        app.wait_text(sv, "Notes", 4)
        app.shot("F15-subtask-view", sv)
        app.key(sv, "Escape")
        app.x.wait_window_gone(ui.SUBTASK_VIEW, 8)
        app.key(ed, "Escape")
        app.x.wait_window_gone(ui.EDITOR, 8)
        return f"subtask #{sub['id']} content saved and rendered"
    res.run("F15 subtask window: edit Markdown content (Ctrl+Enter) -> view renders it", f15, on_fail)


def fill_date_time(app: App, ed, placeholder: str | None, date: str | None, time_: str | None,
                   label: bool = False) -> None:
    """编辑表单里的日期 / 时间选择器：点输入框 → 输入 → 回车。label=True 时 placeholder 是表单项标题，
    选择器在它下方 32px。"""
    region = (0, 56, 700, 490)
    b = app.wait_text(ed, placeholder, 6, region=region)
    x, y, w, h = b  # type: ignore[misc]
    if label:
        date_xy = (126, y + h / 2 + 34)
    else:
        date_xy = (126, y + h / 2)
    time_xy = (date_xy[0] + 228, date_xy[1])
    if date:
        app.x.click(ed, *date_xy)
        app.key(ed, "ctrl+a")
        app.type(ed, date)
        app.key(ed, "Return")
        time.sleep(0.4)
    if time_:
        app.x.click(ed, *time_xy)
        app.key(ed, "ctrl+a")
        app.type(ed, time_)
        app.key(ed, "Return")
        time.sleep(0.4)


def ui_create(app: App, title: str) -> int:
    """FAB → 新建窗口 → 输入标题 → Ctrl+Enter；返回新 id"""
    main_ = app.main_win()
    app.x.click(main_, *ui.fab(main_))
    ed = app.win(ui.EDITOR_NEW, 15)
    time.sleep(0.8)
    app.type(ed, title)
    ui.editor_save(app, ed, ui.EDITOR_NEW)
    r = wait_until(lambda: app.session.one("SELECT id FROM todos WHERE title=?", (title,)), 8,
                   what=f"{title!r} in db")
    return r["id"]


def ui_rename(app: App, old: str, new: str) -> None:
    ed = ui.open_detail(app, old)
    ui.editor_click_edit(app, ed)
    ui.editor_set_title(app, ed, new)
    ui.editor_save(app, ed)
    wait_until(lambda: app.session.one("SELECT id FROM todos WHERE title=?", (new,)), 8, what=f"{new!r} in db")


def ui_delete(app: App, title: str) -> None:
    ui.row_action(app, title, "delete")
    ui.confirm_dialog(app, app.main_win(), "删除确认", "删除")
    wait_until(lambda: not app.session.query("SELECT id FROM todos WHERE title=?", (title,)), 8,
               what=f"{title!r} deleted")


# ============================================================================ S：同步


def suite_sync(kind: str) -> None:
    tag = kind[:1].upper()
    ids = [f"S{n:02d}[{kind}]" for n in range(1, 13)]
    if not any(res.wanted(i) for i in ids):
        return
    dav_reset(kind)
    ts = fmt(now_local())
    a = Session(f"sync-{kind}-a")
    seed(SEED_BIN, a, {"version": "4.0", "exportedAt": ts, "settings": mock_settings(),
                       "todos": [todo_doc(1, f"{tag} Alpha one", ts, color="#F59E0B", quadrant=2, sortOrder=0),
                                 todo_doc(2, f"{tag} Bravo two", ts, sortOrder=1)]})
    app = launch(a)
    cloud = Cloud(procs, CLOUD_BIN, kind, name=f"cloud-{kind}", port=8787 if kind == "apache" else 8797)
    ctx: dict = {"b": None}
    try:
        _sync_cases(kind, tag, a, app, cloud, ctx)
    finally:
        if ctx["b"] is not None:
            shutdown(ctx["b"])
        shutdown(app)
        cloud.stop()


def ensure_configured(s: Session, kind: str) -> None:
    """S01（界面配置）失败时后面的用例仍要能独立运行：直接写库配置 WebDAV（Linux 上密码明文保存）"""
    if s.setting("webdav_url") == DAV[kind]["url"]:
        return
    log(f"[{kind}] WebDAV not configured by the UI case; writing settings directly")
    for k, v in (("webdav_url", DAV[kind]["url"]), ("webdav_username", "e2e"), ("webdav_password", "e2e-pass")):
        s.execute("INSERT INTO settings (key, value, updated_at) VALUES (?, ?, datetime('now','localtime')) "
                  "ON CONFLICT(key) DO UPDATE SET value=excluded.value", (k, v))


def _sync_cases(kind: str, tag: str, a: Session, app: App, cloud: Cloud, ctx: dict) -> None:
    def S(n: int, text: str, fn) -> None:
        def wrapped():
            if n > 1:  # 让每个用例都能单独运行：WebDAV 已配置、远端已有文档
                ensure_configured(a, kind)
                if dav_get_sync(kind) is None:
                    ui.title_sync(app)
            return fn()
        res.run(f"S{n:02d}[{kind}] {text}", wrapped, on_fail)

    def s01():
        st = ui.open_settings(app, "数据与同步")
        ui.fill_webdav(app, st, kind, password="wrong-pass")
        ui.test_connection(app, st, expect="连接失败")
        check(not a.setting("webdav_url"), "test connection must not save settings")
        ui.fill_webdav(app, st, kind)
        ui.test_connection(app, st)
        app.click_text(st, "保存配置", 6, region=(160, 56, 520, 504))
        ui.wait_toast(app, st, "同步设置已保存", 10)
        check(a.setting("webdav_url") == DAV[kind]["url"], f"webdav_url {a.setting('webdav_url')!r}")
        check(a.setting("webdav_username") == "e2e", "username not saved")
        check(a.setting("webdav_password") == "e2e-pass", "password not stored (Linux: plaintext by design)")
        mark = LogMark(a)
        line = ui.settings_sync_now(app, st, mark)
        check("同步完成" in line, f"first sync: {line}")
        ui.wait_toast(app, st, "已将本地更改推送到云端", 8)
        doc = dav_get_sync(kind)
        check(doc is not None, "nothing uploaded")
        titles = {t["title"] for t in doc["todos"]}
        check({f"{tag} Alpha one", f"{tag} Bravo two"} <= titles, f"remote titles {titles}")
        check("tombstones" in doc and doc.get("settingsUpdatedAt"), f"remote keys {sorted(doc)}")
        app.shot(f"S01-{kind}-settings-synced", st)
        mark = LogMark(a)
        line2 = ui.settings_sync_now(app, st, mark)
        check("已是最新" in line2, f"second sync should be a no-op: {line2}")
        ui.close_settings(app, st)
        check(a.setting("webdav_last_sync_at"), "last sync time not recorded")
        return f"{line.split('）: ', 1)[-1]} / then {line2.split('）: ', 1)[-1]}"
    S(1, "settings UI: 测试连接 (wrong / right password) + 保存配置 + 立即同步 uploads; then no-op", s01)

    def s02():
        tid = ui_create(app, f"{tag} Charlie ui")
        line = ui.title_sync(app)
        check("推送" in line or "合并" in line, f"title-bar sync: {line}")
        if "推送" in line:
            ui.wait_toast(app, app.main_win(), "已将本地更改推送到云端", 6)
        doc = dav_get_sync(kind)
        check(any(t["id"] == tid for t in doc["todos"]), "new todo not uploaded")
        line2 = ui.title_sync(app)
        check("已是最新" in line2, f"second title-bar sync: {line2}")
        ui.wait_toast(app, app.main_win(), "已是最新", 6)
        # Skill CLI 经 cloud 看到 PC 新建的待办
        cloud.call("/sync/pull")
        out = cloud.skill("list")
        check(out.returncode == 0 and f"{tag} Charlie ui" in out.stdout, f"skill list: {out.stdout[-400:]} {out.stderr}")
        return f"{line.split('）: ', 1)[-1]}; then no-op toast; skill list shows it"
    S(2, "title-bar sync button pushes a UI-created todo; repeat click shows 已是最新", s02)

    def s03():
        due = (now_local() + dt.timedelta(days=3)).strftime("%Y-%m-%d")
        cloud.call("/sync/pull")
        out = cloud.skill("add", f"{tag} Delta skill", "--priority", "high", "--due", due)
        check(out.returncode == 0, f"skill add failed: {out.stdout} {out.stderr}")
        r = cloud.api("POST", "/todos", json={"title": f"{tag} Echo rest", "quadrant": "urgent_important",
                                              "notifyAt": "2026-12-01T08:30"})
        check(r.ok, f"cloud POST {r.status_code} {r.text[:200]}")
        cloud.call("/sync")
        line = ui.title_sync(app)
        tt = a.todos_by_title()
        d = tt.get(f"{tag} Delta skill")
        e = tt.get(f"{tag} Echo rest")
        check(d and e, f"cloud todos not pulled: {sorted(tt)}; {line}")
        check(d["color"].upper() == "#EF4444" and d["end_time"] == f"{due} 23:59:00", f"skill aliases: {d}")
        check(e["quadrant"] == 1 and e["notify_at"] == "2026-12-01 08:30:00", f"REST fields: {e}")
        ui.wait_main_shows(app, f"{tag} Delta skill", 10)
        ui.wait_main_shows(app, f"{tag} Echo rest", 6)
        app.shot(f"S03-{kind}-main", app.main_win())
        doc = dav_get_sync(kind)
        check(all("priority" not in t and "dueDate" not in t for t in doc["todos"]), "alias fields leaked")
        return f"{line.split('）: ', 1)[-1]}"
    S(3, "cloud writes (Skill CLI + REST) appear on PC after title-bar sync (db + list)", s03)

    def s04():
        bravo = a.one("SELECT id FROM todos WHERE title=?", (f"{tag} Bravo two",))["id"]
        ui_delete(app, f"{tag} Bravo two")
        ui.title_sync(app)
        cloud.call("/sync/pull")
        check(cloud.api("GET", f"/todos/{bravo}").status_code == 404, "cloud still has the PC-deleted todo")
        cloud.call("/sync/push")
        check(not any(t["id"] == bravo for t in dav_get_sync(kind)["todos"]), "resurrected by cloud push")
        echo = a.one("SELECT id FROM todos WHERE title=?", (f"{tag} Echo rest",))["id"]
        r = cloud.api("DELETE", f"/todos/{echo}")
        check(r.status_code in (200, 204), f"cloud DELETE {r.status_code}")
        cloud.call("/sync/push")
        ui.title_sync(app)
        check(not a.query("SELECT id FROM todos WHERE id=?", (echo,)), "cloud deletion not applied on PC")
        ui.wait_main_shows(app, f"{tag} Echo rest", 10, present=False)
        for _ in range(2):
            ui.title_sync(app)
            cloud.call("/sync")
        tt = a.todos_by_title()
        check(f"{tag} Bravo two" not in tt and f"{tag} Echo rest" not in tt, f"resurrection: {sorted(tt)}")
        check(("todo", echo) in a.tombstones() and ("todo", bravo) in a.tombstones(), "tombstones missing on PC")
        return "PC→cloud and cloud→PC deletions, no resurrection after 2 extra rounds"
    S(4, "deletions propagate both ways via tombstones (PC UI delete / cloud DELETE)", s04)

    def s05():
        b = Session(f"sync-{kind}-b")
        seed(SEED_BIN, b, {"version": "4.0", "exportedAt": fmt(now_local()), "todos": [],
                           "settings": mock_settings()})
        ctx["b"] = app_b = launch(b, 1)
        st = ui.configure_webdav(app_b, kind, check_connection=False)
        line = ui.settings_sync_now(app_b, st, LogMark(b))
        check("同步完成" in line, f"device B first sync: {line}")
        ui.close_settings(app_b, st)
        tb = b.todos_by_title()
        check({f"{tag} Alpha one", f"{tag} Charlie ui", f"{tag} Delta skill"} <= set(tb), f"B got {sorted(tb)}")
        check(f"{tag} Bravo two" not in tb, "B resurrected a deleted todo")
        ui.wait_main_shows(app_b, f"{tag} Charlie ui", 10)
        app_b.shot(f"S05-{kind}-deviceB", app_b.main_win())
        # B 改标题 → A 拿到
        ui_rename(app_b, f"{tag} Alpha one", f"{tag} Alpha fromB")
        ui.title_sync(app_b)
        ui.title_sync(app)
        check(a.one("SELECT id FROM todos WHERE title=?", (f"{tag} Alpha fromB",)), "A did not get B's edit")
        ui.wait_main_shows(app, f"{tag} Alpha fromB", 10)
        # A 删除 → B 删除，不复活
        ui_delete(app, f"{tag} Charlie ui")
        ui.title_sync(app)
        ui.title_sync(app_b)
        check(not b.query("SELECT id FROM todos WHERE title=?", (f"{tag} Charlie ui",)), "B kept A's deleted todo")
        ui.wait_main_shows(app_b, f"{tag} Charlie ui", 10, present=False)
        # B 新建 → A
        ui_create(app_b, f"{tag} Foxtrot fromB")
        ui.title_sync(app_b)
        ui.title_sync(app)
        for _ in range(2):
            ui.title_sync(app_b)
            ui.title_sync(app)
        ta, tb = a.todos_by_title(), b.todos_by_title()
        check(f"{tag} Foxtrot fromB" in ta, f"A missing B's new todo: {sorted(ta)}")
        check(f"{tag} Charlie ui" not in ta and f"{tag} Charlie ui" not in tb, "deleted todo came back")
        check(set(ta) == set(tb), f"devices diverged: A-B={sorted(set(ta) - set(tb))} B-A={sorted(set(tb) - set(ta))}")
        shutdown(app_b)
        ctx["b"] = None
        return f"A and B converged on {len(ta)} todos"
    S(5, "two PCs: new / edit / delete propagate both ways, no resurrection, converge", s05)

    def s06():
        title0 = f"{tag} Alpha fromB"
        row = a.one("SELECT id, updated_at FROM todos WHERE title=?", (title0,))
        ahead = fmt(now_local() + dt.timedelta(seconds=90))
        a.execute("UPDATE todos SET updated_at=? WHERE id=?", (ahead, row["id"]))  # 上一版来自时钟快 90s 的设备
        ui_rename(app, title0, f"{tag} Alpha mono")
        r1 = a.one("SELECT updated_at FROM todos WHERE id=?", (row["id"],))["updated_at"]
        want_ts = fmt(dt.datetime.strptime(ahead, "%Y-%m-%d %H:%M:%S") + dt.timedelta(seconds=1))
        check(r1 == want_ts, f"edit after a clock-ahead version: updated_at {r1}, want {want_ts} (= previous + 1s)")
        ui.title_sync(app)
        cloud.call("/sync/pull")
        rc = cloud.api("GET", f"/todos/{row['id']}").json()
        check(rc["title"] == f"{tag} Alpha mono", f"cloud kept {rc['title']!r}")
        stamps = []
        for v in ("ai1", "ai2"):
            r = cloud.api("PATCH", f"/todos/{row['id']}", json={"title": f"{tag} Alpha {v}"})
            check(r.ok, f"PATCH {v}: {r.status_code} {r.text[:200]}")
            stamps.append(r.json()["updatedAt"])
        check(stamps[1] > stamps[0] > r1, f"cloud versions not strictly increasing: {r1} {stamps}")
        cloud.call("/sync/push")
        ui.title_sync(app)
        check(a.one("SELECT title FROM todos WHERE id=?", (row["id"],))["title"] == f"{tag} Alpha ai2", "PC lost ai2")
        ui.wait_main_shows(app, f"{tag} Alpha ai2", 10)
        return f"PC {ahead} -> {r1}; cloud {stamps[0]} -> {stamps[1]}"
    S(6, "monotonic updated_at: UI edit after a clock-ahead version, back-to-back cloud edits", s06)

    def s07():
        name = f"e2e{kind}img.png"
        png(a.images_dir / name, "#FF00FF")
        title = a.one("SELECT title FROM todos WHERE id=1")["title"]
        ed = ui.open_detail(app, title)
        ui.editor_click_edit(app, ed)
        app.x.click(ed, *ui.ED_DESC)
        # 图片独占一段（与文字同行的情况由 ui/markdown.rs 的单测覆盖）
        app.type(ed, "pic")
        app.key(ed, "Return", "Return")
        app.type(ed, f"![e2e](minitodo-image://{name})")
        time.sleep(0.5)
        app.shot(f"S07-{kind}-desc-edit", ed)
        ui.editor_save(app, ed)
        desc = a.one("SELECT description FROM todos WHERE title=?", (title,))["description"] or ""
        check(f"minitodo-image://{name}" in desc and "asset" not in desc, f"stored description {desc!r}")
        ui.title_sync(app)
        check(name in dav_list(kind, "mini-todo/images"), f"image not uploaded: {dav_list(kind, 'mini-todo/images')}")
        cloud.call("/sync/pull")
        code = wait_until(lambda: cloud.api("GET", f"/images/{name}").status_code == 200, 30, 1, "cloud image mirror")
        # 只读详情把 minitodo-image:// 渲染成本机图片
        ed = ui.open_detail(app, title)
        time.sleep(1.5)
        p = app.shot(f"S07-{kind}-detail-image", ed)
        magenta = count_color(p, (255, 0, 255))
        app.key(ed, "Escape")
        app.x.wait_window_gone(ui.EDITOR, 8)
        check(magenta > 10000, f"image not rendered at natural size in detail view (magenta px={magenta}, "
                               f"160x100 image = 16000)")
        # 远端清单里的恶意文件名一律忽略
        doc = dav_get_sync(kind)
        doc["images"] = list(doc.get("images", [])) + ["../../evil.png", "..\\..\\evil2.png", "/etc/evil3.png"]
        dav_put_sync(kind, doc)
        ui.title_sync(app)
        check(not list(a.root.rglob("evil*.png")), "malicious image name written")
        return f"uploaded, mirrored by cloud ({code}), rendered ({magenta} px), hostile names ignored"
    S(7, "images: minitodo-image:// typed in editor -> upload, cloud mirror, rendered in detail", s07)

    def s08():
        ui.click_title(app, "theme")
        wait_until(lambda: a.setting("text_theme") == "light", 8, what="dark theme")
        ui.title_sync(app)
        doc = dav_get_sync(kind)
        check(doc["settings"].get("textTheme") == "light", f"remote textTheme {doc['settings'].get('textTheme')}")
        doc["settings"]["textTheme"] = "dark"
        doc["settings"]["windowPosition"] = {"x": 4000, "y": 4000}
        doc["settingsUpdatedAt"] = fmt(now_local() + dt.timedelta(minutes=5))
        doc["e2eFutureKey"] = {"keep": True}
        dav_put_sync(kind, doc)
        line = ui.title_sync(app)
        check("已应用云端设置" in line, f"settings not applied: {line}")
        check(a.setting("text_theme") == "dark", f"text_theme {a.setting('text_theme')}")
        pos = a.setting("window_position") or ""
        check("4000" not in pos, f"device geometry applied from remote: {pos}")
        time.sleep(1)
        main_ = app.main_win()
        check(main_.x < 2000, f"window moved off-screen: {main_}")
        app.x.move(main_, 100, 46)
        p = app.x.pixel(main_, 190, main_.h - 120)
        check(sum(p) > 600, f"main window did not switch back to the light theme: pixel {p}")
        ui_create(app, f"{tag} Golf after settings")
        ui.title_sync(app)
        cloud.call("/sync")
        check(dav_get_sync(kind).get("e2eFutureKey") == {"keep": True}, "unknown top-level key dropped")
        return "newer remote settings applied (geometry excluded), unknown key kept by PC + cloud"
    S(8, "settings LWW from remote (window geometry excluded) + unknown top-level keys preserved", s08)

    def s09():
        base = {"description": None, "color": "#10B981", "quadrant": 4, "notifyAt": None, "notifyBefore": 0,
                "notified": False, "completed": False, "sortOrder": 0, "startTime": None, "endTime": None,
                "subtasks": []}
        doc = dav_get_sync(kind)
        stamp = fmt(now_local())
        # 两条"只在远端"的记录：第二条的 id 比第一条小，避免与本机 AUTOINCREMENT 的下一个 id（max+1）撞号
        doc["todos"].append({**base, "id": 7700009, "title": f"{tag} remote only", "createdAt": stamp,
                             "updatedAt": stamp})
        dav_put_sync(kind, doc)
        ui_create(app, f"{tag} local only")
        st = ui.open_settings(app, "数据与同步")
        ui.settings_content_top(app, st)
        ui.open_advanced(app, st)
        x, y = ui.settings_find(app, st, "用云端覆盖本地", 1)
        mark = LogMark(a)
        app.x.click(st, x, y)
        app.shot(f"S09-{kind}-force-pull-confirm", st)
        ui.confirm_dialog(app, st, "用云端覆盖本地", "覆盖本地")
        line = mark.wait(r"\[sync\] 同步(完成|失败)", 60)
        check("完成" in line, f"force pull: {line}")
        tt = a.todos_by_title()
        check(f"{tag} remote only" in tt and f"{tag} local only" not in tt, f"force pull result {sorted(tt)}")
        doc = dav_get_sync(kind)
        doc["todos"].append({**base, "id": 7700002, "title": f"{tag} remote only 2", "createdAt": fmt(now_local()),
                             "updatedAt": fmt(now_local())})
        dav_put_sync(kind, doc)
        ui.close_settings(app, st)
        ui_create(app, f"{tag} local only 2")
        st = ui.open_settings(app, "数据与同步")
        ui.settings_content_top(app, st)
        ui.settings_content_bottom(app, st)
        try:
            x, y = ui.settings_find(app, st, "用本地覆盖云端", 0)
        except AssertionError:  # 「高级」折叠区在设置窗口重开后可能是收起的
            ui.open_advanced(app, st)
            x, y = ui.settings_find(app, st, "用本地覆盖云端", 1)
        mark = LogMark(a)
        app.x.click(st, x, y)
        ui.confirm_dialog(app, st, "用本地覆盖云端", "覆盖云端")
        line2 = mark.wait(r"\[sync\] 同步(完成|失败)", 60)
        check("完成" in line2, f"force push: {line2}")
        ui.close_settings(app, st)
        doc = dav_get_sync(kind)
        titles = {t["title"] for t in doc["todos"]}
        check(f"{tag} remote only 2" not in titles and f"{tag} local only 2" in titles, f"force push: {titles}")
        check(any(int(t.get("entityId", -1)) == 7700002 for t in doc.get("tombstones", [])),
              "no tombstone for the remote-only record removed by force push")
        cloud.call("/sync/pull")
        check(cloud.api("GET", "/todos/7700002").status_code == 404, "cloud kept the force-pushed-away record")
        cloud.call("/sync/push")
        check(not any(t["id"] == 7700002 for t in dav_get_sync(kind)["todos"]), "cloud push resurrected it")
        ui.wait_main_shows(app, f"{tag} remote only", 10)
        return "force pull = remote state; force push tombstoned the remote-only record"
    S(9, "settings 高级: 用云端覆盖本地 / 用本地覆盖云端 (with confirm dialogs)", s09)

    def s10():
        errs: list[str] = []

        def pc_writer(i: int) -> None:
            try:
                n = fmt(now_local())
                a.execute("INSERT INTO todos (title, color, quadrant, sort_order, created_at, updated_at) "
                          "VALUES (?, '#10B981', 4, 90, ?, ?)", (f"{tag} race pc {i}", n, n))
                ui.title_sync(app, 90)
            except Exception as e:  # noqa: BLE001
                errs.append(f"pc{i}: {e}")

        def cloud_writer(i: int) -> None:
            try:
                r = cloud.api("POST", "/todos", json={"title": f"{tag} race ai {i}"})
                if not r.ok:
                    errs.append(f"ai{i}: {r.status_code}")
                cloud.call("/sync/push")
            except Exception as e:  # noqa: BLE001
                errs.append(f"ai{i}: {e}")

        for i in range(4):
            th = [threading.Thread(target=pc_writer, args=(i,)), threading.Thread(target=cloud_writer, args=(i,))]
            [t.start() for t in th]
            [t.join() for t in th]
        for _ in range(3):
            try:
                cloud.call("/sync")
            except AssertionError as e:
                errs.append(f"settle cloud: {e}")
            try:
                ui.title_sync(app, 90)
            except Exception as e:  # noqa: BLE001
                errs.append(f"settle pc: {e}")
            time.sleep(1.2)
        need = {f"{tag} race pc {i}" for i in range(4)} | {f"{tag} race ai {i}" for i in range(4)}
        remote = {t["title"] for t in dav_get_sync(kind)["todos"]}
        local = set(a.todos_by_title())
        check(need <= remote, f"lost in remote: {sorted(need - remote)}; errs={errs}")
        check(need <= local, f"lost locally: {sorted(need - local)}; errs={errs}")
        return f"8/8 concurrent creates survived; transient errors={len(errs)} {errs[:3]}"
    S(10, "concurrent PC (title-bar sync) + cloud writers lose nothing", s10)

    def s11():
        a.execute("UPDATE settings SET value='1' WHERE key='webdav_sync_interval'")
        st = ui.open_settings(app, "数据与同步")
        ui.settings_content_top(app, st)
        x, y = ui.settings_find(app, st, "自动同步", 3)
        app.x.click(st, 636, y + 8)
        ui.wait_toast(app, st, "同步设置已保存", 8)
        check(a.setting("webdav_auto_sync") == "true", f"auto_sync {a.setting('webdav_auto_sync')}")
        check(a.setting("webdav_sync_interval") == "1", f"interval {a.setting('webdav_sync_interval')}")
        mark = LogMark(a)
        ui.close_settings(app, st)
        mark.wait(r"自动同步每 1 分钟一次", 15)
        cloud.api("POST", "/todos", json={"title": f"{tag} Hotel auto"})
        cloud.call("/sync/push")
        line = mark.wait(r"\[sync\] 同步完成（Smart \{ full: false \}）", 100)
        wait_until(lambda: a.one("SELECT id FROM todos WHERE title=?", (f"{tag} Hotel auto",)), 10,
                   what="auto-synced todo in db")
        ui.wait_main_shows(app, f"{tag} Hotel auto", 10)
        # 关掉自动同步，避免影响后面的用例
        st = ui.open_settings(app, "数据与同步")
        ui.settings_content_top(app, st)
        x, y = ui.settings_find(app, st, "自动同步", 3)
        app.x.click(st, 636, y + 8)
        ui.wait_toast(app, st, "同步设置已保存", 8)
        ui.close_settings(app, st)
        return line.split("[sync] ", 1)[-1]
    S(11, "auto sync (interval 1 min, enabled in settings) pulls a cloud write without clicks", s11)


# ============================================================================ L：生命周期

V28_TRIGGERS = [f"trg_{t}_{e}_local_seq" for t in ("todos", "subtasks") for e in ("insert", "update", "delete")]


def make_legacy_db(s: Session) -> None:
    """造一个 v27 结构的旧库：先让 core 建出最新库（v30），再按 v28 / v29 / v30 迁移的内容逆向剥掉
    （触发器、sync_meta、tombstones、三张收纳箱表（含 v30 的两列）、两次迁移新增的 settings 键与迁移记录），
    最后按旧版写法插入 `T` 分隔 / 无秒 / 小数秒 / 仅日期 的存量时间。
    v1~v27 的表结构就是 core 迁移表里的同一份 SQL，与旧版 Tauri 应用建出来的库一致。"""
    seed(SEED_BIN, s, {"version": "4.0", "exportedAt": fmt(now_local()), "todos": [],
                       "settings": mock_settings(notificationType="system")})
    for f in (s.data_dir / "backups").glob("*.db") if (s.data_dir / "backups").exists() else []:
        f.unlink()
    stmts = [f"DROP TRIGGER IF EXISTS {t}" for t in V28_TRIGGERS] + [
        "DROP TABLE IF EXISTS sync_meta", "DROP TABLE IF EXISTS tombstones",
        "DROP TABLE IF EXISTS fence_layouts", "DROP TABLE IF EXISTS fence_items", "DROP TABLE IF EXISTS fences",
        "DELETE FROM migrations WHERE version >= 28",
        "DELETE FROM settings WHERE key IN ('webdav_remote_etag', 'webdav_synced_settings_at', "
        "'fence_onedrive_notice_shown')",
        "DELETE FROM todos", "DELETE FROM subtasks",
        "INSERT INTO todos (id, title, color, quadrant, notify_at, notify_before, start_time, end_time, sort_order, "
        "created_at, updated_at) VALUES (1, 'Legacy one', '#EF4444', 1, '2030-01-02T03:04', 0, "
        "'2030-01-01T00:00:00', '2030-01-05', 0, '2026-09-01T10:00:00.123', '2026-09-02T11:12')",
        "INSERT INTO todos (id, title, color, quadrant, sort_order, created_at, updated_at) VALUES "
        "(2, 'Legacy two', '#10B981', 4, 1, '2026-09-03 08:00:00', '2026-09-03 08:00:00')",
        "INSERT INTO subtasks (id, parent_id, title, completed, sort_order, created_at, updated_at) VALUES "
        "(11, 1, 'legacy sub', 1, 0, '2026-09-01T10:05:00', '2026-09-01T10:05:00')",
    ]
    c = __import__("sqlite3").connect(str(s.db_path))
    try:
        for q in stmts:
            c.execute(q)
        c.commit()
        ver = c.execute("SELECT MAX(version) FROM migrations").fetchone()[0]
        check(ver == 27, f"legacy db version {ver}")
    finally:
        c.close()


def suite_lifecycle() -> None:
    if not any(res.wanted(p) for p in ("L01", "L02", "L03", "L04")):
        return
    s = Session("legacy")
    make_legacy_db(s)
    app = launch(s)
    try:
        def l01():
            backups = sorted((s.data_dir / "backups").glob("*.db"))
            check(backups, f"no pre-migration backup in {s.data_dir / 'backups'}")
            check(any("v27" in b.name for b in backups), f"backup name lacks the old version: {backups}")
            vers = {r["version"] for r in s.query("SELECT version FROM migrations")}
            check({28, 29, 30} <= vers, f"migrations {sorted(vers)}")
            r = s.one("SELECT * FROM todos WHERE id=1")
            check((r["notify_at"], r["start_time"], r["end_time"]) ==
                  ("2030-01-02 03:04:00", "2030-01-01 00:00:00", "2030-01-05 23:59:00"), f"times {r}")
            check(r["created_at"] == "2026-09-01 10:00:00" and r["updated_at"] == "2026-09-02 11:12:00",
                  f"created/updated {r['created_at']} {r['updated_at']}")
            sub = s.one("SELECT * FROM subtasks WHERE id=11")
            check(sub and sub["completed"] == 1 and sub["created_at"] == "2026-09-01 10:05:00", f"subtask {sub}")
            check(s.one("SELECT value FROM sync_meta WHERE key='local_seq'"), "sync_meta missing")
            check(s.query("SELECT name FROM sqlite_master WHERE name='fences'"), "v29 tables missing")
            check(s.setting("fence_onedrive_notice_shown") == "false", "v29 settings key missing")
            text = s.log_text()
            check("数据库将从 v27 升级到 v30" in text, f"migration backup not logged: {text[-400:]!r}")
            ui.wait_main_shows(app, "Legacy one", 10)
            ui.wait_main_shows(app, "Legacy two", 4)
            app.wait_text(app.main_win(), "1/1", 8, region=ui.list_region(app.main_win()), fuzzy=0.9)
            app.shot("L01-upgraded-main", app.main_win())
            return f"backup {backups[0].name}; v27 -> v30; times normalized; data intact"
        res.run("L01 legacy v27 database: auto backup + v28~v30 migrations + data intact + shown", l01, on_fail)

        def l02():
            nonlocal app
            tid = ui_create(app, "Lifecycle persisted")
            ui.click_title(app, "view")
            wait_until(lambda: s.setting("view_mode") == "quadrant", 8, what="quadrant")
            main_ = app.main_win()
            app.x.wm_close(main_)  # 关闭主窗口 → Linux 上最后一个窗口关闭即退出
            rc = wait_until(lambda: app.proc.poll() is not None and (app.proc.returncode,), 15, what="process exit")
            check(rc[0] == 0, f"exit code {rc[0]}")
            shutdown(app)
            check(s.one("PRAGMA integrity_check")["integrity_check"] == "ok", "integrity_check failed")
            app = launch(s)
            ui.wait_main_shows(app, "Lifecycle persisted", 10)
            app.wait_text(app.main_win(), "不紧急不重要", 8, region=ui.list_region(app.main_win()))  # 四象限视图
            app.shot("L02-restarted", app.main_win())
            ui.click_title(app, "view")
            wait_until(lambda: s.setting("view_mode") == "list", 8, what="list")
            # 异常退出（SIGKILL）后再启动：库完好、数据都在
            ui_create(app, "Lifecycle killed")
            shutdown(app, signal.SIGKILL)
            check(s.one("PRAGMA integrity_check")["integrity_check"] == "ok", "integrity_check after SIGKILL")
            app = launch(s)
            ui.wait_main_shows(app, "Lifecycle killed", 10)
            return f"graceful close rc=0 (todo #{tid}, quadrant view restored); SIGKILL -> restart ok"
        res.run("L02 close main window (WM_DELETE_WINDOW) exits; data + view persist; SIGKILL recovery", l02, on_fail)

        def l03():
            nonlocal app
            shutdown(app)
            s.execute("INSERT INTO migrations (version) VALUES (31)")
            mark = LogMark(s)
            app = launch(s)
            mark.wait(r"比本程序支持的 v30 新", 10)
            ui.wait_main_shows(app, "Legacy one", 10)
            return "newer schema version: warning logged, app runs"
        res.run("L03 database from a newer version (v31): warns and keeps working", l03, on_fail)
    finally:
        shutdown(app)


# ============================================================================ main


def main() -> int:
    t0 = time.time()
    try:
        if want("F"):
            suite_sample()
            suite_local()
        if want("S"):
            suite_sync("apache")
            suite_sync("nginx")
        if want("L"):
            suite_lifecycle()
    finally:
        procs.stop_all()
        summary = res.summary(time.time() - t0)
        print(summary)
        RUN.mkdir(parents=True, exist_ok=True)
        (RUN / "results.txt").write_text(summary + "\n", encoding="utf-8")
    return 0 if all(s != "FAIL" for _, s, _ in res.rows) else 1


if __name__ == "__main__":
    sys.exit(main())
