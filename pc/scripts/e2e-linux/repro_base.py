"""Reproduce key audit findings on an OLD build (expected: every bug reproduces = PASS).

usage: python3 repro_base.py <old-app-binary> [A1 B1 A6 E1 B2 C2 ...]

Uses the pre-2026-10 command signatures (webdav_upload_sync / webdav_auto_sync, JSON
save_subtask_image), so it only runs against builds from before the sync rewrite.
"""
import base64
import datetime as dt
import sys

sys.path.insert(0, str(__import__("pathlib").Path(__file__).parent))
from harness import (DAV, DAV_PASS, DAV_USER, DISPLAY, App, By, Display, Procs, Results,  # noqa: E402
                     Session, check, dav_get_sync, dav_put_sync, dav_reset)
import subprocess  # noqa: E402
import time  # noqa: E402
from pathlib import Path  # noqa: E402

app_path = Path(sys.argv[1])
only = set(sys.argv[2:])
procs = Procs()
disp = Display(procs)
env = disp.start()
res = Results()
PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==")


def now_local() -> dt.datetime:
    return dt.datetime.now(dt.timezone(dt.timedelta(hours=8))).replace(tzinfo=None)


def with_app(sess: str, fn, extra_env=None):
    s = Session(sess)
    app = App(procs, app_path, s, env, extra_env=extra_env)
    try:
        return fn(app, s)
    finally:
        app.quit()


def repro_a1():
    """same-day local creation deleted by auto sync when the remote is newer"""
    dav_reset("apache")

    def body(app, s):
        app.invoke("save_sync_settings", {"settings": {
            "webdavUrl": DAV["apache"]["url"], "webdavUsername": DAV_USER, "webdavPassword": DAV_PASS,
            "autoSync": True, "syncInterval": 15, "lastSyncAt": None, "deviceId": "dev_e2e"}})
        app.invoke("create_todo", {"data": {"title": "A-synced", "color": "#10B981", "quadrant": 4}})
        app.invoke("webdav_upload_sync")
        time.sleep(1.2)
        x = app.invoke("create_todo", {"data": {"title": "X-same-day-unsynced", "color": "#10B981", "quadrant": 4}})
        doc = dav_get_sync("apache")
        ts = now_local().strftime("%Y-%m-%d %H:%M:%S")
        doc["todos"].append({"id": 900001, "title": "Y-from-cloud", "description": None, "color": "#EF4444",
                             "quadrant": 1, "notifyAt": None, "notifyBefore": 0, "notified": False,
                             "completed": False, "sortOrder": 0, "startTime": None, "endTime": None,
                             "createdAt": ts, "updatedAt": ts, "subtasks": []})
        doc["updatedAt"] = (now_local() + dt.timedelta(minutes=1)).strftime("%Y-%m-%dT%H:%M:%S+08:00")
        check(dav_put_sync("apache", doc) in (200, 201, 204), "PUT failed")
        out = app.invoke("webdav_auto_sync")
        titles = [r["title"] for r in s.query("SELECT title FROM todos")]
        check("X-same-day-unsynced" not in titles,
              f"bug NOT reproduced: X still present (auto_sync={out}, titles={titles})")
        return f"auto_sync={out}; local titles after sync={titles} (X id {x['id']} deleted)"

    return with_app("repro-a1", body)


def repro_xss():
    """javascript: link in the read-only detail view executes"""
    def body(app, s):
        t = app.invoke("create_todo", {"data": {"title": "XSS probe", "color": "#10B981", "quadrant": 4,
                                                 "description": "[点我](javascript:document.title='PWNED')"}})
        app.js("location.reload()")
        time.sleep(3)
        before = set(app.handles())
        app.d.find_element(By.CSS_SELECTOR, ".todo-item .todo-content").click()
        deadline = time.time() + 15
        while time.time() < deadline and not (set(app.handles()) - before):
            time.sleep(0.3)
        h = (set(app.handles()) - before).pop()
        app.d.switch_to.window(h)
        app.wait_js("return document.querySelector('.ProseMirror a') !== null", 20)
        app.d.find_element(By.CSS_SELECTOR, ".ProseMirror a").click()
        time.sleep(1.5)
        title = app.js("return document.title")
        app.d.switch_to.window(app.main_handle)
        check(title == "PWNED", f"bug NOT reproduced: document.title={title!r}")
        return f"todo {t['id']}: clicking link executed script (document.title={title!r})"

    return with_app("repro-xss", body)


def repro_repeat_space():
    """repeat reminder whose notifyAt uses the space format never advances"""
    def body(app, s):
        # in-app notifications: no dependency on a desktop notification daemon, so a failure
        # to advance can only come from parsing the space-format time
        app.invoke("set_notification_type", {"notificationType": "app"})
        t = app.invoke("create_todo", {"data": {"title": "repeat-space", "color": "#10B981", "quadrant": 4}})
        past = (now_local() - dt.timedelta(minutes=2)).strftime("%Y-%m-%d %H:%M:00")
        app.invoke("update_todo", {"id": t["id"], "data": {"notifyAt": past, "notifyBefore": 0,
                                                            "repeatEnabled": True, "repeatType": "daily",
                                                            "repeatInterval": 1}})
        time.sleep(70)  # the scheduler ticks once per minute
        row = s.query("SELECT notify_at, notified FROM todos WHERE id=?", (t["id"],))[0]
        check(row["notify_at"] == past and row["notified"] == 0, f"bug NOT reproduced: row={row}")
        return f"after a scheduler tick notify_at still {row['notify_at']} (not advanced) -> fires every minute"

    return with_app("repro-repeat", body)


def repro_calendar():
    """todo without a start time is missing from the calendar"""
    def body(app, s):
        app.invoke("create_todo", {"data": {"title": "cal-no-start", "color": "#10B981", "quadrant": 4,
                                             "endTime": (now_local() + dt.timedelta(days=1)).strftime("%Y-%m-%dT18:00:00")}})
        app.invoke("set_show_calendar", {"show": True})
        app.js("location.reload()")
        time.sleep(4)
        app.wait_js("return document.querySelector('.calendar-view') !== null", 20)
        bars = app.texts(".calendar-view .todo-bar")
        check(not any("cal-no-start" in b for b in bars), f"bug NOT reproduced: bars={bars}")
        return f"calendar bars={bars} (todo with end date but no start date not shown)"

    return with_app("repro-calendar", body)


def repro_traversal():
    """path traversal in save_subtask_image"""
    def body(app, s):
        p = app.invoke("save_subtask_image", {"imageData": base64.b64encode(PNG).decode(),
                                              "fileName": "../escaped.png"})
        escaped = s.data_dir / "escaped.png"
        check(escaped.exists(), f"bug NOT reproduced: {p}")
        return f"file written outside images dir: {escaped}"

    return with_app("repro-traversal", body)


def repro_notify_scale():
    """in-app notification window off-screen at scale factor 2"""
    def body(app, s):
        app.invoke("set_notification_type", {"notificationType": "app"})
        t = app.invoke("create_todo", {"data": {"title": "notify-scale", "color": "#10B981", "quadrant": 4}})
        past = (now_local() - dt.timedelta(minutes=1)).strftime("%Y-%m-%dT%H:%M:00")
        app.invoke("update_todo", {"id": t["id"], "data": {"notifyAt": past, "notifyBefore": 0}})
        deadline = time.time() + 75
        geo = None
        while time.time() < deadline:
            out = subprocess.run(["xwininfo", "-root", "-tree", "-display", DISPLAY],
                                 capture_output=True, text=True).stdout
            lines = [ln for ln in out.splitlines() if "通知" in ln or "\\351\\200\\232\\347\\237\\245" in ln]
            if lines:
                geo = lines[0].strip()
                break
            time.sleep(2)
        check(geo, "no notification window appeared")
        x, y = [int(v) for v in geo.split()[-1].strip("+").split("+")]
        check(x >= 1600 or y >= 1000, f"bug NOT reproduced: window at {x},{y} inside 1600x1000 screen")
        return f"notification window at +{x}+{y} on a 1600x1000 screen (GDK_SCALE=2): off-screen"

    return with_app("repro-notify", body, extra_env={"GDK_SCALE": "2"})


try:
    for key, name, fn in [("A1", "A1 same-day local todo deleted by auto sync", repro_a1),
                          ("B1", "B1 javascript: link executes in read-only detail", repro_xss),
                          ("A6", "A6 repeat reminder with space-format time never advances", repro_repeat_space),
                          ("E1", "E1 calendar misses todo without start time", repro_calendar),
                          ("B2", "B2 save_subtask_image path traversal", repro_traversal),
                          ("C2", "C2 app notification off-screen at scale 2", repro_notify_scale)]:
        if not only or key in only:
            res.run(name, fn)
finally:
    procs.stop_all()
    disp.stop()
    print(res.summary())
