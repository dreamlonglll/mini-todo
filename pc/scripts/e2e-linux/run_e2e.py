"""Full Linux e2e suite: real app + Apache/nginx WebDAV + cloud binary + skill CLI.

usage: python3 run_e2e.py <app-binary> <cloud-binary> <old-app-binary> [prefix ...]
  prefixes filter tests, e.g. `F` (local suite), `S` (sync suites), `L` (lifecycle),
  or individual ids such as `F03 S02 L01`.
<old-app-binary> is a build from before the v28 migration (used by L01 to create a legacy DB).
"""
import base64
import datetime as dt
import os
import re
import subprocess
import sys
import threading
import time
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from harness import (DAV, DAV_PASS, DAV_USER, DISPLAY, RUN, App, By, Cloud, Display,  # noqa: E402
                     InvokeError, Procs, Results, Session, check, dav_get_sync, dav_list,
                     dav_put_sync, dav_reset)

APP, CLOUD_BIN, OLD_APP = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
ONLY = sys.argv[4:]
procs = Procs()
disp = Display(procs)
ENV = disp.start()
res = Results(ONLY)
PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==")
CANON = re.compile(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$")


def now_local() -> dt.datetime:
    return dt.datetime.now(dt.timezone(dt.timedelta(hours=8))).replace(tzinfo=None)


def fmt(d: dt.datetime) -> str:
    return d.strftime("%Y-%m-%d %H:%M:%S")


def is_canon(s: str | None) -> bool:
    return s is None or bool(CANON.match(s))


def want(suite: str) -> bool:
    return not ONLY or any(p.startswith(suite) or suite.startswith(p) for p in ONLY)


def cloud_call(cloud: Cloud, path: str) -> dict:
    """POST /sync, /sync/pull or /sync/push and fail loudly if the cloud reports an error."""
    r = cloud.api("POST", path)
    check(r.ok, f"cloud {path} -> {r.status_code} {r.text[:300]}")
    try:
        j = r.json()
    except Exception:
        j = {}
    bad = {k: v for k, v in j.items()
           if (k.endswith("Error") and v) or (isinstance(v, str) and v.lower().startswith("err"))}
    check(not bad, f"cloud {path} reported errors: {bad}")
    return j


def configure_sync(app: App, kind: str, auto: bool = False) -> None:
    app.invoke("save_sync_settings", {"settings": {
        "webdavUrl": DAV[kind]["url"], "webdavUsername": DAV_USER, "webdavPassword": DAV_PASS,
        "autoSync": auto, "syncInterval": 15}})


def todos_by_title(s: Session) -> dict[str, dict]:
    return {r["title"]: r for r in s.query("SELECT * FROM todos")}


def items_of(payload) -> list[dict]:
    return payload if isinstance(payload, list) else payload.get("items", payload.get("todos", []))


def open_todo_window(app: App, title: str) -> str:
    before = set(app.handles())
    for el in app.d.find_elements(By.CSS_SELECTOR, ".todo-item"):
        if title in el.text:
            el.find_element(By.CSS_SELECTOR, ".todo-content").click()
            break
    else:
        raise AssertionError(f"todo {title!r} not in list {app.texts('.todo-title')}")
    deadline = time.time() + 15
    while time.time() < deadline:
        new = set(app.handles()) - before
        if new:
            h = new.pop()
            app.d.switch_to.window(h)
            return h
        time.sleep(0.3)
    raise AssertionError("editor window did not open")


def close_current_window(app: App) -> None:
    try:
        app.d.close()
    except Exception:
        pass
    app.d.switch_to.window(app.main_handle)


def reload_main(app: App) -> None:
    app.d.switch_to.window(app.main_handle)
    app.js("location.reload()")
    time.sleep(3)
    app.wait_js("return document.querySelector('.title-bar') !== null", 20)


# =============================================================================== local (F)

def suite_local() -> None:
    s = Session("local")
    app = App(procs, APP, s, ENV)
    try:
        def f00():
            check(app.js("return document.querySelector('.title-bar') !== null"), "no title bar")
            check(app.invoke("get_todos") == [], "fresh db should be empty")
            return "ok"
        res.run("F00 smoke: app boots under Xvfb, IPC works", f00)

        def f01():
            t = app.invoke("create_todo", {"data": {"title": "time-norm", "color": "#10B981", "quadrant": 4,
                                                     "notifyAt": "2026-10-05T09:30", "startTime": "2026-10-04T08:00:00",
                                                     "endTime": "2026-10-06"}})
            r = s.query("SELECT notify_at, start_time, end_time FROM todos WHERE id=?", (t["id"],))[0]
            check(r["notify_at"] == "2026-10-05 09:30:00", f"notify_at {r}")
            check(r["start_time"] == "2026-10-04 08:00:00", f"start_time {r}")
            check(r["end_time"] == "2026-10-06 23:59:00", f"end_time {r}")
            app.invoke("update_todo", {"id": t["id"], "data": {"notifyAt": "2026-10-07 10:00"}})
            r = s.query("SELECT notify_at FROM todos WHERE id=?", (t["id"],))[0]
            check(r["notify_at"] == "2026-10-07 10:00:00", f"after update {r}")
            try:
                app.invoke("update_todo", {"id": t["id"], "data": {"notifyAt": "not a date"}})
                raise AssertionError("garbage time accepted")
            except InvokeError:
                pass
            one = app.invoke("get_todo", {"id": t["id"]})
            check(one["title"] == "time-norm", "get_todo")
            return f"stored {r['notify_at']}; garbage rejected"
        res.run("F01 time inputs (T / no-seconds / date-only) stored canonical", f01)

        def f02():
            seq0 = app.invoke("get_change_seq")
            t = app.invoke("create_todo", {"data": {"title": "to-delete", "color": "#10B981", "quadrant": 4}})
            st = app.invoke("create_subtask", {"data": {"parentId": t["id"], "title": "child"}})
            seq1 = app.invoke("get_change_seq")
            app.invoke("delete_todo", {"id": t["id"]})
            seq2 = app.invoke("get_change_seq")
            check(seq0 < seq1 < seq2, f"change seq {seq0},{seq1},{seq2}")
            tomb = s.query("SELECT entity_type, entity_id, deleted_at FROM tombstones")
            kinds = {(r["entity_type"], r["entity_id"]) for r in tomb}
            check(("todo", t["id"]) in kinds and ("subtask", st["id"]) in kinds, f"tombstones {tomb}")
            check(all(is_canon(r["deleted_at"]) for r in tomb), f"deleted_at format {tomb}")
            return f"seq {seq0}->{seq1}->{seq2}, tombstones={len(tomb)}"
        res.run("F02 change counter + tombstones on delete (todo & subtasks)", f02)

        def f03():
            app.invoke("create_todo", {"data": {
                "title": "xss-probe", "color": "#10B981", "quadrant": 4,
                "description": "[js](javascript:document.title='PWNED') "
                               "[data](data:text/html,<script>document.title='PWNED2'</script>) [vb](vbscript:msgbox)"}})
            reload_main(app)
            open_todo_window(app, "xss-probe")
            app.wait_js("return document.querySelectorAll('.ProseMirror a').length >= 1", 20)
            url0 = app.d.current_url
            links = app.d.find_elements(By.CSS_SELECTOR, ".ProseMirror a")
            for a in links:
                try:
                    a.click()
                except Exception:
                    pass
                time.sleep(0.8)
            title, url1 = app.js("return document.title"), app.d.current_url
            close_current_window(app)
            check("PWNED" not in (title or ""), f"script executed: title={title}")
            check(url1 == url0, f"webview navigated: {url0} -> {url1}")
            return f"{len(links)} malicious links clicked, title={title!r}, no navigation"
        res.run("F03 javascript:/data:/vbscript: links are inert in read-only detail", f03)

        def f04():
            r = app.d.execute_async_script("""
              const done = arguments[0]; const out = {};
              try { out.eval = String(eval('1+1')); } catch (e) { out.eval = 'blocked'; }
              try { out.fn = String(new Function('return 2')()); } catch (e) { out.fn = 'blocked'; }
              const src = window.__TAURI_INTERNALS__.convertFileSrc('/etc/hostname');
              fetch(src).then(r => r.text().then(t => { out.etc = 'read:' + r.status + ':' + t.slice(0, 20); done(out); }))
                .catch(e => { out.etc = 'blocked:' + e; done(out); });
            """)
            check(r["eval"] == "blocked" and r["fn"] == "blocked", f"eval not blocked by CSP: {r}")
            check(not str(r["etc"]).startswith("read:200"), f"asset protocol served /etc/hostname: {r}")
            return str(r)
        res.run("F04 CSP blocks eval; asset protocol cannot read files outside images dir", f04)

        def f05():
            p = Path(app.invoke_raw_bytes("save_subtask_image", PNG, {"x-image-ext": "png"}))
            check(p.parent == s.images_dir and p.exists(), f"saved to {p}")
            bad = []
            for ext in ["../../evil", "png/../../x", "exe", "..", ""]:
                try:
                    q = app.invoke_raw_bytes("save_subtask_image", PNG, {"x-image-ext": ext})
                    bad.append((ext, q))
                except InvokeError:
                    pass
            check(not bad, f"accepted bad ext: {bad}")
            try:
                app.invoke("save_subtask_image", {"imageData": base64.b64encode(PNG).decode(),
                                                  "fileName": "../escaped.png"})
            except InvokeError:
                pass
            check(not list(s.data_dir.glob("evil*")) and not (s.data_dir / "escaped.png").exists(), "file escaped")
            return f"saved {p.name}; bad exts and legacy JSON form rejected"
        res.run("F05 raw image save: generated safe names, traversal rejected", f05)

        def f06():
            end = fmt(now_local() + dt.timedelta(days=1))
            app.invoke("create_todo", {"data": {"title": "cal-no-start", "color": "#10B981", "quadrant": 4, "endTime": end}})
            app.invoke("set_show_calendar", {"show": True})
            reload_main(app)
            app.wait_js("return document.querySelector('.calendar-view') !== null", 25)
            deadline = time.time() + 15
            bars: list[str] = []
            while time.time() < deadline:
                bars = app.js("return [...document.querySelectorAll('.calendar-view .todo-bar')]"
                              ".map(b => b.textContent.trim())")
                if any("cal-no-start" in b for b in bars):
                    break
                time.sleep(0.5)
            app.invoke("set_show_calendar", {"show": False})
            check(any("cal-no-start" in b for b in bars), f"calendar bars={bars}")
            return f"bars={bars}"
        res.run("F06 calendar shows todo with end date but no start date", f06)

        def f07():
            app.invoke("set_notification_type", {"notificationType": "app"})
            t = app.invoke("create_todo", {"data": {"title": "repeat-space", "color": "#10B981", "quadrant": 4}})
            past = (now_local() - dt.timedelta(minutes=2)).strftime("%Y-%m-%d %H:%M:00")
            app.invoke("update_todo", {"id": t["id"], "data": {"notifyAt": past, "notifyBefore": 0, "repeatEnabled": True,
                                                                "repeatType": "daily", "repeatInterval": 1}})
            deadline = time.time() + 80
            row = None
            while time.time() < deadline:
                row = s.query("SELECT notify_at, notified FROM todos WHERE id=?", (t["id"],))[0]
                if row["notify_at"] != past:
                    break
                time.sleep(3)
            check(row["notify_at"] != past, f"not advanced: {row}")
            nxt = dt.datetime.strptime(row["notify_at"], "%Y-%m-%d %H:%M:%S")
            check(nxt > now_local(), f"next occurrence not in future: {row}")
            check(row["notified"] == 0, f"repeat should stay armed: {row}")
            return f"{past} -> {row['notify_at']}"
        res.run("F07 repeat reminder with space-format time advances after firing", f07)

    finally:
        app.quit()


def suite_notify_scale() -> None:
    s = Session("notify-scale")
    app = App(procs, APP, s, ENV, extra_env={"GDK_SCALE": "2"})
    try:
        def f09():
            app.invoke("set_notification_type", {"notificationType": "app"})
            for i in range(3):
                t = app.invoke("create_todo", {"data": {"title": f"notify-{i}", "color": "#10B981", "quadrant": 4}})
                app.invoke("update_todo", {"id": t["id"], "data": {
                    "notifyAt": (now_local() - dt.timedelta(minutes=1)).strftime("%Y-%m-%d %H:%M:00"), "notifyBefore": 0}})
            deadline = time.time() + 80
            geos: list[str] = []
            while time.time() < deadline:
                out = subprocess.run(["xwininfo", "-root", "-tree", "-display", DISPLAY],
                                     capture_output=True, text=True).stdout
                geos = [ln.strip() for ln in out.splitlines() if "\\351\\200\\232\\347\\237\\245" in ln or "通知" in ln]
                if len(geos) >= 3:
                    break
                time.sleep(2)
            check(geos, "no notification windows")
            pos = []
            for g in geos:
                m = re.search(r"(\d+)x(\d+)\+(-?\d+)\+(-?\d+)", g)
                check(m, f"cannot parse geometry: {g}")
                w, h, x, y = (int(v) for v in m.groups())
                pos.append((x, y, w, h))
                check(0 <= x and x + w <= 1600 and 0 <= y and y + h <= 1000, f"window off-screen: {g}")
            check(len({(p[0], p[1]) for p in pos}) == len(pos), f"windows overlap: {pos}")
            return f"{len(pos)} windows: {pos}"
        res.run("F09 app notifications on-screen & stacked at scale factor 2", f09)
    finally:
        app.quit()


# =============================================================================== sync (S)

def suite_sync(kind: str) -> None:
    dav_reset(kind)
    s = Session(f"sync-{kind}")
    app = App(procs, APP, s, ENV)
    cloud = Cloud(procs, CLOUD_BIN, kind, name=f"cloud-{kind}", port=8787 if kind == "apache" else 8797,
                  pull_interval=3600)
    tag = kind[:1].upper()
    try:
        def s01():
            configure_sync(app, kind)
            ss = app.invoke("get_sync_settings")
            check(ss["webdavPassword"] == "" and ss["hasPassword"] is True, f"password leaked/missing: {ss}")
            check(app.invoke("webdav_test_connection", {"url": DAV[kind]["url"], "username": DAV_USER, "password": None}),
                  "test connection with stored password")
            app.invoke("create_todo", {"data": {"title": f"{tag}-A", "color": "#F59E0B", "quadrant": 2}})
            rep = app.invoke("webdav_sync")
            check(rep["status"] in ("pushed", "merged"), f"report {rep}")
            doc = dav_get_sync(kind)
            check(doc and any(t["title"] == f"{tag}-A" for t in doc["todos"]), "remote lacks A")
            check("tombstones" in doc and "settingsUpdatedAt" in doc, f"remote keys {list(doc)}")
            rep2 = app.invoke("webdav_sync")
            check(rep2["status"] == "no_changes", f"second sync should be no_changes: {rep2}")
            return f"{rep['status']} then {rep2['status']}"
        res.run(f"S01[{kind}] first sync uploads, second is no-op; password never returned", s01)

        def s02():
            cloud_call(cloud, "/sync/pull")
            items = items_of(cloud.api("GET", "/todos").json())
            a = [t for t in items if t["title"] == f"{tag}-A"]
            check(a, f"cloud view of A missing: {items}")
            check(a[0].get("priority") == "medium", f"derived priority: {a}")
            app.invoke("create_todo", {"data": {"title": f"{tag}-X-local", "color": "#10B981", "quadrant": 4}})
            due = (now_local() + dt.timedelta(days=3)).strftime("%Y-%m-%d")
            out = cloud.skill("add", f"{tag}-Y-ai", "--priority", "high", "--due", due)
            check(out.returncode == 0, f"skill add failed: {out.stdout} {out.stderr}")
            cloud_call(cloud, "/sync")
            rep = app.invoke("webdav_sync")
            tt = todos_by_title(s)
            check(f"{tag}-X-local" in tt, f"local same-day todo lost! report={rep} titles={list(tt)}")
            y = tt.get(f"{tag}-Y-ai")
            check(y is not None, f"AI todo not pulled: {list(tt)}")
            check(y["color"].upper() == "#EF4444", f"priority not mapped to color: {y['color']}")
            check(y["end_time"] == f"{due} 23:59:00", f"due not mapped to endTime: {y['end_time']}")
            doc = dav_get_sync(kind)
            check(any(t["title"] == f"{tag}-X-local" for t in doc["todos"]), "X not uploaded")
            check(all("priority" not in t and "dueDate" not in t for t in doc["todos"]), "alias fields leaked into sync-data")
            cloud_call(cloud, "/sync/pull")
            check(items_of(cloud.api("GET", "/todos?q=X-local").json()), "cloud did not get X after pull")
            return f"report={rep['status']}; Y color={y['color']} end={y['end_time']}"
        res.run(f"S02[{kind}] AI write + same-day local create merge without loss (A1)", s02)

        def s03():
            tt = todos_by_title(s)
            x_id = tt[f"{tag}-X-local"]["id"]
            app.invoke("delete_todo", {"id": x_id})
            app.invoke("webdav_sync")
            cloud_call(cloud, "/sync/pull")
            r = cloud.api("GET", f"/todos/{x_id}")
            check(r.status_code == 404, f"cloud still has deleted X: {r.status_code}")
            cloud_call(cloud, "/sync/push")
            doc = dav_get_sync(kind)
            check(not any(t["id"] == x_id for t in doc["todos"]), "X resurrected remotely after cloud push")
            y_id = tt[f"{tag}-Y-ai"]["id"]
            r = cloud.api("DELETE", f"/todos/{y_id}")
            check(r.status_code in (200, 204), f"cloud delete {r.status_code}")
            cloud_call(cloud, "/sync/push")
            app.invoke("webdav_sync")
            check(f"{tag}-Y-ai" not in todos_by_title(s), "cloud deletion not applied on PC")
            app.invoke("webdav_sync")
            cloud_call(cloud, "/sync")
            app.invoke("webdav_sync")
            tt2 = todos_by_title(s)
            check(f"{tag}-Y-ai" not in tt2 and f"{tag}-X-local" not in tt2, f"resurrection: {list(tt2)}")
            return "both directions propagate, no resurrection after extra rounds"
        res.run(f"S03[{kind}] deletions propagate both ways via tombstones", s03)

        def s04():
            a = todos_by_title(s)[f"{tag}-A"]
            b = app.invoke("create_todo", {"data": {"title": f"{tag}-B", "color": "#10B981", "quadrant": 4}})
            app.invoke("webdav_sync")
            cloud_call(cloud, "/sync/pull")
            time.sleep(1.1)
            app.invoke("update_todo", {"id": a["id"], "data": {"title": f"{tag}-A-edited-pc"}})
            r = cloud.api("PATCH", f"/todos/{b['id']}", json={"completed": True})
            check(r.ok, f"cloud patch {r.status_code} {r.text}")
            cloud_call(cloud, "/sync/push")
            app.invoke("webdav_sync")
            tt = todos_by_title(s)
            check(f"{tag}-A-edited-pc" in tt, "PC edit lost")
            check(tt[f"{tag}-B"]["completed"] == 1, "cloud edit lost on PC")
            cloud_call(cloud, "/sync/pull")
            ra = cloud.api("GET", f"/todos/{a['id']}").json()
            check(ra["title"] == f"{tag}-A-edited-pc", f"cloud did not see PC edit: {ra['title']}")
            return "edits on different records both survive"
        res.run(f"S04[{kind}] concurrent edits on different records merge", s04)

        def s05():
            b = todos_by_title(s)[f"{tag}-B"]
            r1 = cloud.api("PATCH", f"/todos/{b['id']}", json={"quadrant": "urgent_important"})
            check(r1.ok and r1.json()["quadrant"] == 1, f"alias quadrant: {r1.status_code} {r1.text}")
            r2 = cloud.api("PATCH", f"/todos/{b['id']}", json={"bogusField": 1})
            r3 = cloud.api("PATCH", f"/todos/{b['id']}", json={"color": None})
            r4 = cloud.api("PATCH", f"/todos/{b['id']}", json={"notifyAt": "not-a-date"})
            check(r2.status_code == 400 and r3.status_code == 400 and r4.status_code == 400,
                  f"validation: {r2.status_code} {r3.status_code} {r4.status_code}")
            r5 = cloud.api("POST", f"/todos/{b['id']}/subtasks", json={"title": "sub-from-ai"})
            check(r5.ok, f"create subtask {r5.status_code}")
            sid = r5.json()["id"]
            r6 = cloud.api("PATCH", f"/subtasks/{sid}", json={"parentId": 1})
            check(r6.status_code == 400, f"parentId change allowed: {r6.status_code}")
            r7 = cloud.api("PATCH", f"/todos/{b['id']}", json={"notifyAt": "2026-12-01T08:30"})
            check(r7.ok and r7.json()["notifyAt"] == "2026-12-01 08:30:00", f"time normalization: {r7.text}")
            cloud_call(cloud, "/sync/push")
            rep = app.invoke("webdav_sync")
            check(rep.get("recordsSkipped", 0) == 0, f"PC skipped records: {rep}")
            row = todos_by_title(s)[f"{tag}-B"]
            check(row["quadrant"] == 1 and row["notify_at"] == "2026-12-01 08:30:00", f"PC row {row}")
            subs = s.query("SELECT title FROM subtasks WHERE parent_id=?", (b["id"],))
            check(any(x["title"] == "sub-from-ai" for x in subs), "AI subtask missing on PC")
            return "aliases mapped, bad input rejected (400), PC applies all records"
        res.run(f"S05[{kind}] cloud write validation keeps PC model intact", s05)

        def s06():
            p = Path(app.invoke_raw_bytes("save_subtask_image", PNG, {"x-image-ext": "png"}))
            t = app.invoke("create_todo", {"data": {"title": f"{tag}-img", "color": "#10B981", "quadrant": 4,
                                                     "description": f"图\n\n![shot](minitodo-image://{p.name})"}})
            app.invoke("webdav_sync")
            check(p.name in dav_list(kind, "mini-todo/images"), "image not uploaded")
            cloud_call(cloud, "/sync/pull")
            deadline = time.time() + 30
            code = 0
            while time.time() < deadline:
                code = cloud.api("GET", f"/images/{p.name}").status_code
                if code == 200:
                    break
                time.sleep(1)
            check(code == 200, f"cloud GET /images -> {code}")
            doc = dav_get_sync(kind)
            doc["images"] = list(doc.get("images", [])) + ["../../evil.png", "..\\..\\evil2.png", "/etc/evil3.png"]
            dav_put_sync(kind, doc)
            app.invoke("webdav_sync")
            check(not list(s.home.rglob("evil*.png")), "malicious image name written")
            reload_main(app)
            open_todo_window(app, f"{tag}-img")
            ok = app.wait_js("const i=document.querySelector('.ProseMirror img:not(.ProseMirror-separator)'); "
                             "return i && i.complete && i.naturalWidth > 0 ? i.src : false", 20)
            close_current_window(app)
            stored = s.query("SELECT description FROM todos WHERE id=?", (t["id"],))[0]["description"]
            check("minitodo-image://" in stored and "asset" not in stored, f"stored description not canonical: {stored}")
            return f"uploaded+mirrored; rendered via {ok[:48]}…"
        res.run(f"S06[{kind}] images: upload, cloud mirror, render, hostile names ignored", s06)

        def s07():
            app.invoke("set_text_theme", {"theme": "dark"})
            app.invoke("webdav_sync")
            doc = dav_get_sync(kind)
            check(doc.get("settingsUpdatedAt"), "no settingsUpdatedAt")
            doc["settings"]["textTheme"] = "light"
            doc["settings"]["windowPosition"] = {"x": 4000, "y": 4000}
            doc["settingsUpdatedAt"] = fmt(now_local() + dt.timedelta(minutes=5))
            doc["e2eFutureKey"] = {"keep": True}
            dav_put_sync(kind, doc)
            rep = app.invoke("webdav_sync")
            check(rep["settingsApplied"] is True, f"settings not applied: {rep}")
            theme = s.query("SELECT value FROM settings WHERE key='text_theme'")[0]["value"]
            pos = s.query("SELECT value FROM settings WHERE key='window_position'")
            check(theme == "light", f"text_theme={theme}")
            check(not pos or "4000" not in pos[0]["value"], f"device geometry applied: {pos}")
            app.invoke("create_todo", {"data": {"title": f"{tag}-after-settings", "color": "#10B981", "quadrant": 4}})
            app.invoke("webdav_sync")
            cloud_call(cloud, "/sync")
            check(dav_get_sync(kind).get("e2eFutureKey") == {"keep": True}, "unknown top-level key dropped")
            return "newer remote settings applied (geometry excluded); unknown keys preserved by PC and cloud"
        res.run(f"S07[{kind}] settings LWW + unknown top-level keys preserved", s07)

        def s08():
            doc = dav_get_sync(kind)
            ts = fmt(now_local())
            base = {"description": None, "color": "#10B981", "quadrant": 4, "notifyAt": None, "notifyBefore": 0,
                    "notified": False, "completed": False, "sortOrder": 0, "startTime": None, "endTime": None,
                    "createdAt": ts, "updatedAt": ts, "subtasks": []}
            doc["todos"].append({**base, "id": 7700001, "title": f"{tag}-remote-only"})
            dav_put_sync(kind, doc)
            app.invoke("create_todo", {"data": {"title": f"{tag}-local-only", "color": "#10B981", "quadrant": 4}})
            rep = app.invoke("webdav_force_pull")
            tt = todos_by_title(s)
            check(f"{tag}-remote-only" in tt and f"{tag}-local-only" not in tt, f"force pull: {list(tt)}")
            doc = dav_get_sync(kind)
            doc["todos"].append({**base, "id": 7700002, "title": f"{tag}-remote-only-2", "updatedAt": fmt(now_local())})
            dav_put_sync(kind, doc)
            app.invoke("create_todo", {"data": {"title": f"{tag}-local-only-2", "color": "#10B981", "quadrant": 4}})
            app.invoke("webdav_force_push")
            doc = dav_get_sync(kind)
            titles = {t["title"] for t in doc["todos"]}
            check(f"{tag}-remote-only-2" not in titles and f"{tag}-local-only-2" in titles, f"force push: {titles}")
            check(any(int(tb.get("entityId", -1)) == 7700002 for tb in doc.get("tombstones", [])),
                  f"no tombstone for removed remote record: {doc.get('tombstones')}")
            cloud_call(cloud, "/sync/pull")
            check(cloud.api("GET", "/todos/7700002").status_code == 404, "cloud kept force-pushed-away record")
            cloud_call(cloud, "/sync/push")
            check(not any(t["id"] == 7700002 for t in dav_get_sync(kind)["todos"]), "cloud push resurrected it")
            return f"force_pull={rep['status']}; force_push tombstoned remote-only record"
        res.run(f"S08[{kind}] force pull / force push semantics", s08)

        def s09():
            errs: list[str] = []

            def pc_writer(i):
                try:
                    app.invoke("create_todo", {"data": {"title": f"{tag}-race-pc-{i}", "color": "#10B981", "quadrant": 4}})
                    app.invoke("webdav_sync")
                except Exception as e:
                    errs.append(f"pc{i}: {e}")

            def cloud_writer(i):
                try:
                    r = cloud.api("POST", "/todos", json={"title": f"{tag}-race-ai-{i}"})
                    if not r.ok:
                        errs.append(f"ai{i}: {r.status_code}")
                    cloud_call(cloud, "/sync/push")
                except Exception as e:
                    errs.append(f"ai{i}: {e}")

            for i in range(5):
                th = [threading.Thread(target=pc_writer, args=(i,)), threading.Thread(target=cloud_writer, args=(i,))]
                [x.start() for x in th]
                [x.join() for x in th]
            for _ in range(3):  # settle
                try:
                    cloud_call(cloud, "/sync")
                except AssertionError as e:
                    errs.append(f"settle cloud: {e}")
                try:
                    app.invoke("webdav_sync")
                except InvokeError as e:
                    errs.append(str(e))
                time.sleep(1.2)
            remote = {t["title"] for t in dav_get_sync(kind)["todos"]}
            local = set(todos_by_title(s))
            need = {f"{tag}-race-pc-{i}" for i in range(5)} | {f"{tag}-race-ai-{i}" for i in range(5)}
            check(need <= remote, f"lost in remote: {sorted(need - remote)}; errs={errs}")
            check(need <= local, f"lost locally: {sorted(need - local)}; errs={errs}")
            return f"10/10 concurrent creates survived; transient errors={len(errs)}"
        res.run(f"S09[{kind}] concurrent PC + cloud writers lose nothing", s09)

        def s10():
            reload_main(app)
            app.d.find_element(By.CSS_SELECTOR, "button.title-btn[title='同步数据']").click()
            msg = app.wait_js("const m=[...document.querySelectorAll('.el-message')].map(e=>e.innerText).join('|'); "
                              "return m || false", 20)
            check("失败" not in msg, f"sync button error: {msg}")
            return f"toast: {msg}"
        res.run(f"S10[{kind}] UI sync button shows a result toast", s10)
    finally:
        app.quit()
        cloud.stop()


# =============================================================================== lifecycle (L)

def suite_lifecycle() -> None:
    s = Session("migrate")
    old = App(procs, OLD_APP, s, ENV)
    try:
        t = old.invoke("create_todo", {"data": {"title": "legacy-1", "color": "#EF4444", "quadrant": 1}})
        old.invoke("update_todo", {"id": t["id"], "data": {"notifyAt": "2030-01-02T03:04:00",
                                                            "startTime": "2030-01-01T00:00:00"}})
        old.invoke("create_subtask", {"data": {"parentId": t["id"], "title": "legacy-sub"}})
    finally:
        old.quit()

    app = App(procs, APP, s, ENV)
    try:
        def l01():
            bdir = s.data_dir / "backups"
            backups = list(bdir.glob("*.db")) if bdir.exists() else []
            check(backups, f"no pre-migration backup in {s.data_dir}")
            r = s.query("SELECT notify_at, start_time FROM todos WHERE title='legacy-1'")[0]
            check(r["notify_at"] == "2030-01-02 03:04:00" and r["start_time"] == "2030-01-01 00:00:00", f"not normalized {r}")
            check(s.query("SELECT COUNT(*) c FROM sync_meta")[0]["c"] >= 2, "sync_meta missing")
            check(s.query("SELECT COUNT(*) c FROM subtasks WHERE title='legacy-sub'")[0]["c"] == 1, "subtask lost")
            app.wait_text(".todo-title", "legacy-1", 15)
            return f"backup {backups[0].name}; legacy data intact and normalized"
        res.run("L01 upgrade from an old DB: backup + v28 migration + data intact", l01)

        def l01b():
            logs = list(s.home.rglob("mini-todo*.log"))
            check(logs, f"no log file under {s.home}")
            text = logs[0].read_text(encoding="utf-8", errors="replace")
            check("v28" in text and "备份" in text, f"migration/backup not logged: {text[:300]}")
            return f"{logs[0].relative_to(s.home)}: {text.splitlines()[0][:90]}"
        res.run("L01b tauri-plugin-log records the migration backup in the app log dir", l01b)

        def l02():
            z = s.home / "export.zip"
            app.invoke("export_data_to_file", {"filePath": str(z)})
            check(z.exists() and zipfile.is_zipfile(z), "export zip missing")
            ids_before = {r["id"] for r in s.query("SELECT id FROM todos")}
            extra = app.invoke("create_todo", {"data": {"title": "after-export", "color": "#10B981", "quadrant": 4}})
            app.invoke("import_data_from_file", {"filePath": str(z)})
            ids_after = {r["id"] for r in s.query("SELECT id FROM todos")}
            check(ids_after == ids_before, f"ids not preserved: {ids_before} vs {ids_after}")
            tomb = {(r["entity_type"], r["entity_id"]) for r in s.query("SELECT * FROM tombstones")}
            check(("todo", extra["id"]) in tomb, "no tombstone for record removed by import")
            n_backups = len(list((s.data_dir / "backups").glob("*.db")))
            check(n_backups >= 2, f"no pre-import backup ({n_backups})")
            return f"ids preserved ({len(ids_after)}), tombstone for removed, backups={n_backups}"
        res.run("L02 export → import keeps ids, tombstones removed records, auto-backup", l02)

        def l03():
            p = subprocess.Popen([str(APP)], env=app.env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                rc = p.wait(timeout=20)
            except subprocess.TimeoutExpired:
                p.kill()
                raise AssertionError("second instance kept running (single-instance not enforced)")
            check(app.js("return 1") == 1, "first instance died")
            return f"second instance exited rc={rc}; first instance alive"
        res.run("L03 single instance: second launch exits, first keeps running", l03)

        def l04():
            n0 = len(app.texts(".todo-title"))
            app.invoke("create_todo", {"data": {"title": "poll-visible", "color": "#10B981", "quadrant": 4}})
            app.wait_text(".todo-title", "poll-visible", 12)
            return f"list {n0} -> {len(app.texts('.todo-title'))} without manual refresh"
        res.run("L04 main list picks up backend changes via change-seq polling", l04)

        def l05():
            before = set(app.handles())
            app.d.find_element(By.CSS_SELECTOR, ".fab-add").click()
            deadline = time.time() + 15
            while time.time() < deadline and not (set(app.handles()) - before):
                time.sleep(0.3)
            app.d.switch_to.window((set(app.handles()) - before).pop())
            inp = app.wait_js("return document.querySelector(\"input[placeholder='请输入待办标题']\")", 15)
            inp.send_keys("ui-created-once")
            btn = [b for b in app.d.find_elements(By.CSS_SELECTOR, "button.el-button--primary") if "创建" in b.text][0]
            app.js("arguments[0].click(); arguments[0].click(); arguments[0].click();", btn)
            time.sleep(3)
            app.d.switch_to.window(app.main_handle)
            n = s.query("SELECT COUNT(*) c FROM todos WHERE title='ui-created-once'")[0]["c"]
            check(n == 1, f"triple click created {n} todos")
            r = s.query("SELECT created_at, updated_at FROM todos WHERE title='ui-created-once'")[0]
            check(is_canon(r["created_at"]) and is_canon(r["updated_at"]), f"timestamps {r}")
            app.wait_text(".todo-title", "ui-created-once", 12)
            return "triple-click create -> exactly 1 todo, shown in main list"
        res.run("L05 editor create via UI is re-entrancy safe", l05)
    finally:
        app.quit()


try:
    if want("F"):
        suite_local()
        suite_notify_scale()
    if want("S"):
        suite_sync("apache")
        suite_sync("nginx")
    if want("L"):
        suite_lifecycle()
finally:
    procs.stop_all()
    disp.stop()
    summary = res.summary()
    print(summary)
    RUN.mkdir(parents=True, exist_ok=True)
    (RUN / "results.txt").write_text(summary)
