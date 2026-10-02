"""Mini-Todo Linux e2e harness.

Drives the real Tauri app under Xvfb through tauri-driver (WebKitWebDriver), against two
local WebDAV servers (Apache mod_dav honours conditional requests, nginx dav ignores them),
the cloud binary and the skill CLI. See README.md in this directory for the setup.

Paths can be overridden with environment variables:
  MT_E2E_WORK          scratch dir for logs / isolated homes   (default /tmp/mini-todo-e2e)
  MT_E2E_TAURI_DRIVER  tauri-driver binary                      (default: PATH or ~/.cargo/bin)
  MT_E2E_DAV_ROOT      root created by setup_servers.sh         (default /srv/mtodo-e2e)
"""
from __future__ import annotations

import base64
import gzip
import json
import os
import shutil
import signal
import socket
import sqlite3
import subprocess
import time
import traceback
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

import requests
from selenium import webdriver
from selenium.webdriver.common.by import By  # noqa: F401  (re-exported for test modules)
from selenium.webdriver.common.options import ArgOptions

REPO = Path(__file__).resolve().parents[3]
WORK = Path(os.environ.get("MT_E2E_WORK", "/tmp/mini-todo-e2e"))
RUN = WORK / "run"
TAURI_DRIVER = os.environ.get("MT_E2E_TAURI_DRIVER") or shutil.which("tauri-driver") \
    or str(Path.home() / ".cargo" / "bin" / "tauri-driver")
DAV_ROOT = Path(os.environ.get("MT_E2E_DAV_ROOT", "/srv/mtodo-e2e"))
DAV_USER, DAV_PASS = "e2e", "e2e-pass"
DAV = {
    "apache": {"url": "http://127.0.0.1:8081/", "root": DAV_ROOT / "apache" / "root"},
    "nginx": {"url": "http://127.0.0.1:8082/", "root": DAV_ROOT / "nginx" / "root"},
}
TZ = "Asia/Shanghai"
DISPLAY = ":99"


def log(*a: Any) -> None:
    print(time.strftime("%H:%M:%S"), *a, flush=True)


def wait_port(port: int, timeout: float = 30.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        with socket.socket() as s:
            s.settimeout(0.5)
            if s.connect_ex(("127.0.0.1", port)) == 0:
                return
        time.sleep(0.2)
    raise TimeoutError(f"port {port} not open after {timeout}s")


# --------------------------------------------------------------------------- processes


class Procs:
    def __init__(self) -> None:
        self.items: list[tuple[str, subprocess.Popen]] = []

    def start(self, name: str, cmd: list[str], env: dict[str, str] | None = None,
              cwd: str | None = None) -> subprocess.Popen:
        RUN.mkdir(parents=True, exist_ok=True)
        out = open(RUN / f"{name}.log", "ab")
        p = subprocess.Popen(cmd, env=env, cwd=cwd, stdout=out, stderr=subprocess.STDOUT,
                             start_new_session=True)
        self.items.append((name, p))
        return p

    def stop(self, name: str) -> None:
        for n, p in list(self.items):
            if n == name:
                _kill(p)
                self.items.remove((n, p))

    def stop_all(self) -> None:
        for _, p in reversed(self.items):
            _kill(p)
        self.items.clear()


def _kill(p: subprocess.Popen) -> None:
    if p.poll() is not None:
        return
    try:
        os.killpg(p.pid, signal.SIGTERM)
        p.wait(timeout=8)
    except Exception:
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except Exception:
            pass


@dataclass
class Session:
    """One isolated "machine": its own HOME / XDG dirs, so app data never collides."""

    name: str
    home: Path = field(init=False)

    def __post_init__(self) -> None:
        self.home = RUN / "homes" / self.name
        for sub in (".local/share", ".config", ".cache"):
            (self.home / sub).mkdir(parents=True, exist_ok=True)

    @property
    def data_dir(self) -> Path:
        return self.home / ".local" / "share" / "mini-todo"

    @property
    def db_path(self) -> Path:
        return self.data_dir / "data.db"

    @property
    def images_dir(self) -> Path:
        return self.data_dir / "images"

    def env(self, base: dict[str, str]) -> dict[str, str]:
        e = dict(base)
        e.update({
            "HOME": str(self.home),
            "XDG_DATA_HOME": str(self.home / ".local" / "share"),
            "XDG_CONFIG_HOME": str(self.home / ".config"),
            "XDG_CACHE_HOME": str(self.home / ".cache"),
        })
        return e

    def db(self) -> sqlite3.Connection:
        c = sqlite3.connect(f"file:{self.db_path}?mode=ro", uri=True, timeout=5)
        c.row_factory = sqlite3.Row
        return c

    def query(self, sql: str, args: tuple = ()) -> list[dict]:
        with self.db() as c:
            return [dict(r) for r in c.execute(sql, args).fetchall()]


class Display:
    """Xvfb + a session D-Bus shared by everything."""

    def __init__(self, procs: Procs) -> None:
        self.procs = procs
        self.env: dict[str, str] = {}
        self.dbus_pid = 0

    def start(self) -> dict[str, str]:
        subprocess.run(["pkill", "-f", f"Xvfb {DISPLAY}"], check=False)
        self.procs.start("xvfb", ["Xvfb", DISPLAY, "-screen", "0", "1600x1000x24", "-nolisten", "tcp"])
        time.sleep(1.0)
        out = subprocess.run(["dbus-daemon", "--session", "--fork", "--print-address=1",
                              "--print-pid=1"], capture_output=True, text=True, check=True).stdout.split()
        addr, pid = out[0], out[1]
        self.dbus_pid = int(pid)
        env = dict(os.environ)
        env.update({"DISPLAY": DISPLAY, "DBUS_SESSION_BUS_ADDRESS": addr, "TZ": TZ,
                    "NO_AT_BRIDGE": "1", "WEBKIT_DISABLE_COMPOSITING_MODE": "1",
                    "LIBGL_ALWAYS_SOFTWARE": "1", "RUST_BACKTRACE": "1"})
        env["PATH"] = f"{Path(TAURI_DRIVER).parent}:{env['PATH']}"
        self.env = env
        return env

    def stop(self) -> None:
        try:
            os.kill(self.dbus_pid, signal.SIGTERM)
        except Exception:
            pass


class InvokeError(Exception):
    def __init__(self, cmd: str, err: Any) -> None:
        super().__init__(f"invoke {cmd} failed: {err}")
        self.cmd, self.err = cmd, err


class App:
    """A running app instance (one tauri-driver + one WebDriver session)."""

    def __init__(self, procs: Procs, app_path: Path, session: Session, base_env: dict[str, str],
                 port: int = 4444, extra_env: dict[str, str] | None = None) -> None:
        self.procs, self.app_path, self.session, self.port = procs, app_path, session, port
        env = session.env(base_env)
        if extra_env:
            env.update(extra_env)
        self.env = env
        self.driver_name = f"tauri-driver-{session.name}-{port}"
        procs.start(self.driver_name, [TAURI_DRIVER, "--port", str(port), "--native-port", str(port + 1)], env=env)
        wait_port(port, 20)
        opts = ArgOptions()
        opts.set_capability("browserName", "wry")
        opts.set_capability("tauri:options", {"application": str(app_path)})
        last: Exception | None = None
        for _ in range(3):
            try:
                self.d = webdriver.Remote(command_executor=f"http://127.0.0.1:{port}", options=opts)
                break
            except Exception as e:  # webkit driver sometimes needs a second try
                last = e
                time.sleep(2)
        else:
            raise RuntimeError(f"could not start session: {last}")
        self.d.set_script_timeout(180)
        self.main_handle = self.d.current_window_handle
        self.wait_js("return !!(window.__TAURI_INTERNALS__ && document.querySelector('#app') "
                     "&& document.querySelector('#app').children.length)", 30)

    # -- js / ipc ---------------------------------------------------------------
    def js(self, script: str, *args: Any) -> Any:
        return self.d.execute_script(script, *args)

    def wait_js(self, script: str, timeout: float = 15, *args: Any) -> Any:
        deadline = time.time() + timeout
        last = None
        while time.time() < deadline:
            try:
                last = self.d.execute_script(script, *args)
                if last:
                    return last
            except Exception as e:  # page may be navigating
                last = e
            time.sleep(0.25)
        raise TimeoutError(f"wait_js timed out: {script[:120]} (last={last!r})")

    def invoke(self, cmd: str, args: dict | None = None) -> Any:
        res = self.d.execute_async_script(
            """
            const [cmd, args, done] = arguments;
            window.__TAURI_INTERNALS__.invoke(cmd, args || {})
              .then(v => done({ok: true, value: v === undefined ? null : v}))
              .catch(e => done({ok: false, error: (e && e.message) ? e.message : String(e)}));
            """, cmd, args or {})
        if not res.get("ok"):
            raise InvokeError(cmd, res.get("error"))
        return res.get("value")

    def invoke_raw_bytes(self, cmd: str, data: bytes, headers: dict[str, str]) -> Any:
        res = self.d.execute_async_script(
            """
            const [cmd, b64, headers, done] = arguments;
            const bin = atob(b64); const bytes = new Uint8Array(bin.length);
            for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
            window.__TAURI_INTERNALS__.invoke(cmd, bytes, { headers })
              .then(v => done({ok: true, value: v === undefined ? null : v}))
              .catch(e => done({ok: false, error: (e && e.message) ? e.message : String(e)}));
            """, cmd, base64.b64encode(data).decode(), headers)
        if not res.get("ok"):
            raise InvokeError(cmd, res.get("error"))
        return res.get("value")

    # -- dom ----------------------------------------------------------------------
    def texts(self, css: str) -> list[str]:
        return [e.text for e in self.d.find_elements(By.CSS_SELECTOR, css)]

    def wait_text(self, css: str, text: str, timeout: float = 15, present: bool = True) -> None:
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                found = any(text in t for t in self.texts(css))
            except Exception:
                found = False
            if found == present:
                return
            time.sleep(0.3)
        raise TimeoutError(f"text {text!r} {'not found' if present else 'still present'} in {css}; "
                           f"got {self.texts(css)}")

    def handles(self) -> list[str]:
        return list(self.d.window_handles)

    def screenshot(self, name: str) -> Path:
        p = RUN / "shots" / f"{name}.png"
        p.parent.mkdir(parents=True, exist_ok=True)
        try:
            self.d.save_screenshot(str(p))
        except Exception:
            subprocess.run(["import", "-display", DISPLAY, "-window", "root", str(p)], check=False)
        return p

    def quit(self) -> None:
        try:
            self.d.quit()
        except Exception:
            pass
        self.procs.stop(self.driver_name)
        time.sleep(1)


# --------------------------------------------------------------------------- WebDAV helpers


def dav_reset(kind: str) -> None:
    root = DAV[kind]["root"]
    for child in (root.iterdir() if root.exists() else []):
        if child.is_dir():
            shutil.rmtree(child)
        else:
            child.unlink()


def dav_get_sync(kind: str) -> dict | None:
    r = requests.get(DAV[kind]["url"] + "mini-todo/sync-data.json.gz", auth=(DAV_USER, DAV_PASS), timeout=15)
    if r.status_code == 404:
        return None
    r.raise_for_status()
    return json.loads(gzip.decompress(r.content).decode("utf-8"))


def dav_put_sync(kind: str, doc: dict) -> int:
    body = gzip.compress(json.dumps(doc, ensure_ascii=False).encode("utf-8"))
    r = requests.put(DAV[kind]["url"] + "mini-todo/sync-data.json.gz", data=body,
                     auth=(DAV_USER, DAV_PASS), timeout=15, headers={"Content-Type": "application/gzip"})
    return r.status_code


def dav_list(kind: str, sub: str) -> list[str]:
    root = DAV[kind]["root"] / sub
    return sorted(p.name for p in root.iterdir()) if root.exists() else []


# --------------------------------------------------------------------------- cloud helpers


class Cloud:
    def __init__(self, procs: Procs, binary: Path, dav_kind: str, name: str = "cloud",
                 port: int = 8787, pull_interval: int = 60) -> None:
        self.procs, self.name, self.port = procs, name, port
        self.dir = RUN / name
        if self.dir.exists():
            shutil.rmtree(self.dir)
        (self.dir / "data").mkdir(parents=True)
        self.api_key = "e2e-" + "k" * 40
        cfg = self.dir / "config.toml"
        cfg.write_text(
            f'webdav_url = "{DAV[dav_kind]["url"]}"\n'
            f'webdav_username = "{DAV_USER}"\nwebdav_password = "{DAV_PASS}"\n'
            f'api_key = "{self.api_key}"\nbind = "127.0.0.1:{port}"\n'
            f'timezone = "{TZ}"\npull_interval = {pull_interval}\n'
            f'data_dir = "{self.dir / "data"}"\nimages_dir = "{self.dir / "data" / "images"}"\n')
        self.skill_cfg = self.dir / "skill.toml"
        self.skill_cfg.write_text(f'endpoint = "http://127.0.0.1:{port}"\napi_key = "{self.api_key}"\n'
                                  f'timeout = 60\ntimezone = "{TZ}"\n')
        env = dict(os.environ)
        env.update({"TZ": TZ, "RUST_LOG": "info"})
        procs.start(name, [str(binary), "--config", str(cfg)], env=env)
        wait_port(port, 60)
        self.base = f"http://127.0.0.1:{port}"

    def api(self, method: str, path: str, **kw: Any) -> requests.Response:
        h = kw.pop("headers", {})
        h["Authorization"] = f"Bearer {self.api_key}"
        return requests.request(method, self.base + path, headers=h, timeout=180, **kw)

    def skill(self, *args: str) -> subprocess.CompletedProcess:
        env = dict(os.environ)
        env["TZ"] = TZ
        return subprocess.run(["python3", str(REPO / "cloud/skill/minitodo/minitodo.py"), "--config",
                               str(self.skill_cfg), *args], capture_output=True, text=True, env=env, timeout=240)

    def stop(self) -> None:
        self.procs.stop(self.name)


# --------------------------------------------------------------------------- tiny test runner


class Results:
    def __init__(self, only: list[str] | None = None) -> None:
        self.rows: list[tuple[str, str, str]] = []
        self.only = [p for p in (only or []) if len(p) > 1]

    def run(self, name: str, fn: Callable[[], Any]) -> bool:
        if self.only and not any(name.startswith(p) for p in self.only):
            self.rows.append((name, "SKIP", "filtered"))
            return True
        log(f"=== {name}")
        t0 = time.time()
        try:
            detail = fn()
            self.rows.append((name, "PASS", f"{time.time() - t0:.1f}s {detail or ''}".strip()))
            log(f"--- PASS {name}")
            return True
        except Exception as e:
            tb = traceback.format_exc(limit=6)
            self.rows.append((name, "FAIL", f"{e}"))
            log(f"--- FAIL {name}: {e}\n{tb}")
            return False

    def summary(self) -> str:
        lines = [f"{'RESULT':6}  {'TEST':66}  DETAIL"]
        for n, s, d in self.rows:
            lines.append(f"{s:6}  {n:66}  {d[:200]}")
        ran = [r for r in self.rows if r[1] != "SKIP"]
        passed = sum(1 for _, s, _ in ran if s == "PASS")
        lines.append(f"\n{passed}/{len(ran)} passed")
        return "\n".join(lines)


def check(cond: Any, msg: str) -> None:
    if not cond:
        raise AssertionError(msg)
