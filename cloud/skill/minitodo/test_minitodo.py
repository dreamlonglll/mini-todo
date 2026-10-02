#!/usr/bin/env python3
"""minitodo CLI 自检（只用标准库 unittest，不引入测试框架依赖）。

运行：
    python3 -m unittest discover -s cloud/skill/minitodo -p 'test_*.py'
或  python3 cloud/skill/minitodo/test_minitodo.py

本文件不会被 install.sh / install.ps1 安装到 skill 目录。
"""

from __future__ import annotations

import argparse
import contextlib
import datetime as dt
import io
import json
import os
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import minitodo  # noqa: E402


class FakeServer:
    """本地 HTTP 服务：按顺序返回预设响应，并记录收到的请求。"""

    def __init__(self, responses: list[tuple[int, object]]):
        self.responses = list(responses)
        self.requests: list[dict] = []
        outer = self

        class Handler(BaseHTTPRequestHandler):
            def _handle(self) -> None:
                length = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(length) if length else b""
                outer.requests.append(
                    {
                        "method": self.command,
                        "path": self.path,
                        "auth": self.headers.get("Authorization"),
                        "body": json.loads(body) if body else None,
                    }
                )
                status, payload = outer.responses.pop(0) if outer.responses else (200, {})
                data = json.dumps(payload).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            do_GET = do_POST = do_PATCH = do_DELETE = _handle

            def log_message(self, *args: object) -> None:  # 静音
                pass

        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)
        self.thread.start()
        self.url = f"http://127.0.0.1:{self.httpd.server_address[1]}"

    def close(self) -> None:
        self.httpd.shutdown()
        self.httpd.server_close()


def client_for(server: FakeServer, sleeps: list[float] | None = None) -> minitodo.Client:
    record = sleeps if sleeps is not None else []
    return minitodo.Client(server.url, "k" * 32, timeout=5, sleep=record.append)


class NormalizeTests(unittest.TestCase):
    def test_canonical_forms(self) -> None:
        n = minitodo.normalize_datetime
        self.assertEqual(n("2026-05-20", "23:59:00"), "2026-05-20 23:59:00")
        self.assertEqual(n(" 2026-05-20 ", "23:59:00"), "2026-05-20 23:59:00")
        self.assertEqual(n("2026-05-20 18:30", "23:59:00"), "2026-05-20 18:30:00")
        self.assertEqual(n("2026-05-20T18:30:15", "23:59:00"), "2026-05-20 18:30:15")
        self.assertEqual(n("2026-05-20 18:30:15.250", "23:59:00"), "2026-05-20 18:30:15")

    def test_offsets_are_left_to_the_server(self) -> None:
        self.assertEqual(
            minitodo.normalize_datetime("2026-05-20T10:00:00Z", "23:59:00"),
            "2026-05-20T10:00:00+00:00",
        )
        self.assertEqual(
            minitodo.normalize_datetime("2026-05-20 10:00+08:00", "23:59:00"),
            "2026-05-20T10:00:00+08:00",
        )

    def test_rejects_garbage(self) -> None:
        for bad in ["", "tomorrow", "2026-13-01", "2026/05/20", "2026-05-20 25:00"]:
            with self.assertRaises(ValueError, msg=bad):
                minitodo.normalize_datetime(bad, "23:59:00")


class BodyBuilderTests(unittest.TestCase):
    def test_add_writes_canonical_end_time_and_keeps_priority(self) -> None:
        args = argparse.Namespace(
            title="买菜", priority="high", due="2026-05-20", quadrant="urgent_important", color=None
        )
        self.assertEqual(
            minitodo.build_add_body(args),
            {"title": "买菜", "priority": "high", "endTime": "2026-05-20 23:59:00", "quadrant": 1},
        )

    def test_add_rejects_bad_due(self) -> None:
        args = argparse.Namespace(title="x", priority=None, due="next friday", quadrant=None, color=None)
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as cm:
            minitodo.build_add_body(args)
        self.assertEqual(cm.exception.code, 2)

    def test_update_maps_quadrant_aliases_and_coerces_values(self) -> None:
        body = minitodo.build_update_body(
            ["quadrant=Important_Not_Urgent", "completed=true", "sortOrder=3", "notifyAt=null",
             "endTime=2026-05-20 18:00"]
        )
        self.assertEqual(
            body,
            {"quadrant": 2, "completed": True, "sortOrder": 3, "notifyAt": None,
             "endTime": "2026-05-20 18:00"},
        )
        self.assertEqual(minitodo.build_update_body(["quadrant=4"]), {"quadrant": 4})
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            minitodo.build_update_body(["quadrant=soon"])


class TimezoneTests(unittest.TestCase):
    def test_today_uses_configured_zone(self) -> None:
        from zoneinfo import ZoneInfo

        for name in ["Pacific/Kiritimati", "Pacific/Pago_Pago"]:  # UTC+14 / UTC-11
            tz = minitodo.resolve_timezone(name)
            self.assertEqual(minitodo.today_in(tz), dt.datetime.now(ZoneInfo(name)).date())
        self.assertIsNone(minitodo.resolve_timezone(None))
        self.assertEqual(minitodo.today_in(None), dt.date.today())

    def test_invalid_zone_is_a_config_error(self) -> None:
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as cm:
            minitodo.resolve_timezone("Mars/Olympus")
        self.assertEqual(cm.exception.code, 2)


class ClientTests(unittest.TestCase):
    def test_get_retries_5xx_twice_with_backoff(self) -> None:
        srv = FakeServer([(502, {"error": "x"}), (503, {"error": "y"}), (200, [{"id": 1}])])
        try:
            sleeps: list[float] = []
            self.assertEqual(client_for(srv, sleeps).get("/todos"), [{"id": 1}])
            self.assertEqual(sleeps, list(minitodo.GET_RETRY_DELAYS))
            self.assertEqual(len(srv.requests), 3)
            self.assertEqual(srv.requests[0]["auth"], "Bearer " + "k" * 32)
        finally:
            srv.close()

    def test_get_gives_up_after_two_retries(self) -> None:
        srv = FakeServer([(500, {"detail": "boom"})] * 3)
        try:
            with contextlib.redirect_stderr(io.StringIO()) as err, self.assertRaises(SystemExit) as cm:
                client_for(srv).get("/todos")
            self.assertEqual(cm.exception.code, 4)
            self.assertIn("boom", err.getvalue())
            self.assertEqual(len(srv.requests), 3)
        finally:
            srv.close()

    def test_client_errors_and_writes_are_not_retried(self) -> None:
        srv = FakeServer([(400, {"detail": "unknown field(s): foo"}), (503, {"detail": "down"})])
        try:
            c = client_for(srv)
            with contextlib.redirect_stderr(io.StringIO()) as err, self.assertRaises(SystemExit) as cm:
                c.get("/todos")
            self.assertEqual(cm.exception.code, 4)
            self.assertIn("unknown field(s): foo", err.getvalue())
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                c.post("/todos", json_body={"title": "x"})
            self.assertEqual(len(srv.requests), 2, "POST 不能自动重试（会重复创建）")
        finally:
            srv.close()

    def test_connection_errors_are_retried_then_exit_3(self) -> None:
        srv = FakeServer([])
        url = srv.url
        srv.close()  # 端口已关闭：连接被拒绝
        sleeps: list[float] = []
        c = minitodo.Client(url, "k", timeout=1, sleep=sleeps.append)
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as cm:
            c.get("/health")
        self.assertEqual(cm.exception.code, 3)
        self.assertEqual(sleeps, list(minitodo.GET_RETRY_DELAYS))

    def test_degraded_health_is_information_not_an_error(self) -> None:
        payload = {"status": "degraded", "sync": "offline", "pull": "offline", "push": "healthy"}
        srv = FakeServer([(503, payload)])
        try:
            sleeps: list[float] = []
            with contextlib.redirect_stderr(io.StringIO()) as err:
                result = minitodo.cmd_health(client_for(srv, sleeps), argparse.Namespace())
            self.assertEqual(result, payload)
            self.assertEqual(sleeps, [], "503 的 /health 不重试")
            self.assertIn("offline", err.getvalue())
        finally:
            srv.close()

    def test_sync_uses_long_timeout(self) -> None:
        seen: list[tuple[str, float]] = []

        class Recorder(minitodo.Client):
            def request(self, method, path, params=None, json_body=None, timeout=None, accept_status=()):
                seen.append((path, timeout))
                return {"pull": "ok", "push": "ok"}

        c = Recorder("http://127.0.0.1:9", "k", timeout=10)
        for mode in (None, "pull", "push"):
            minitodo.cmd_sync(c, argparse.Namespace(mode=mode))
        self.assertEqual([p for p, _ in seen], ["/sync", "/sync/pull", "/sync/push"])
        self.assertTrue(all(t is not None and t >= 90 for _, t in seen), seen)


class CliTests(unittest.TestCase):
    def test_help_and_end_to_end_today(self) -> None:
        with contextlib.redirect_stdout(io.StringIO()) as out, self.assertRaises(SystemExit) as cm:
            minitodo.main(["--help"])
        self.assertEqual(cm.exception.code, 0)
        self.assertIn("today", out.getvalue())
        with contextlib.redirect_stdout(io.StringIO()) as out, self.assertRaises(SystemExit):
            minitodo.main(["add", "--help"])
        self.assertIn("YYYY-MM-DD HH:MM:SS", out.getvalue())

        todo = {"id": 1, "seq": 3, "title": "写报告", "priority": "high", "completed": False,
                "endTime": "2026-05-20 23:59:00"}
        srv = FakeServer([(200, [todo]), (200, []), (200, [])])
        try:
            with tempfile.TemporaryDirectory() as d:
                cfg = Path(d) / "config.toml"
                cfg.write_text(
                    f'endpoint = "{srv.url}"\napi_key = "{"k" * 32}"\ntimezone = "Asia/Shanghai"\n',
                    encoding="utf-8",
                )
                with contextlib.redirect_stdout(io.StringIO()) as out:
                    code = minitodo.main(["--config", str(cfg), "today"])
            self.assertEqual(code, 0)
            table = out.getvalue()
            self.assertIn("C3", table)
            self.assertIn("DONE", table)
            today = dt.datetime.now(minitodo.resolve_timezone("Asia/Shanghai")).date().isoformat()
            self.assertIn(f"dueDateAfter={today}", srv.requests[0]["path"])
        finally:
            srv.close()


if __name__ == "__main__":
    unittest.main()
