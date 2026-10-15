"""本地 WebDAV 服务器（e2e 用）：wsgidav + cheroot，Basic 认证（单用户），把每个请求记到 JSONL 访问日志。

    python dav_server.py --root <目录> --port 8765 --user e2e --password <密码> --access-log <jsonl>

依赖：pip install wsgidav cheroot
"""
from __future__ import annotations

import argparse
import base64
import json
import sys
import time


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", required=True)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8765)
    ap.add_argument("--user", required=True)
    ap.add_argument("--password", required=True)
    ap.add_argument("--access-log", required=True)
    args = ap.parse_args()

    from cheroot import wsgi
    from wsgidav.wsgidav_app import WsgiDAVApp

    config = {
        "host": args.host,
        "port": args.port,
        "provider_mapping": {"/": args.root},
        "http_authenticator": {
            "domain_controller": None,  # SimpleDomainController
            "accept_basic": True,
            "accept_digest": False,
            "default_to_digest": False,
        },
        "simple_dc": {"user_mapping": {"*": {args.user: {"password": args.password}}}},
        "verbose": 1,
        "logging": {"enable_loggers": []},
    }
    dav = WsgiDAVApp(config)

    def app(environ, start_response):
        record = {"t": time.time(), "method": environ.get("REQUEST_METHOD"), "path": environ.get("PATH_INFO"),
                  "user": None}
        auth = environ.get("HTTP_AUTHORIZATION", "")
        if auth.lower().startswith("basic "):
            try:
                record["user"] = base64.b64decode(auth[6:]).decode("utf-8", "replace").split(":", 1)[0]
            except Exception:  # noqa: BLE001
                record["user"] = "?"

        def sr(status, headers, exc_info=None):
            record["status"] = int(status.split()[0])
            with open(args.access_log, "a", encoding="utf-8") as f:
                f.write(json.dumps(record, ensure_ascii=False) + "\n")
            return start_response(status, headers, exc_info)

        return dav(environ, sr)

    server = wsgi.Server((args.host, args.port), app)
    print(f"WebDAV serving {args.root} on http://{args.host}:{args.port}/", flush=True)
    try:
        server.start()
    except KeyboardInterrupt:
        server.stop()
    return 0


if __name__ == "__main__":
    sys.exit(main())
