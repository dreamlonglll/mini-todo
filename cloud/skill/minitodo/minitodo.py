#!/usr/bin/env python3
"""minitodo CLI wrapper for Claude Code Skill.

Reads `~/.claude/skills/minitodo/config.toml` for endpoint + api_key, then talks
to the cloud HTTP API. All write paths return the updated record so AI can
verify.

Usage examples:
  python minitodo.py today
  python minitodo.py today --json
  python minitodo.py add "Buy groceries" --priority high --due 2026-05-20
  python minitodo.py list --pending
  python minitodo.py done C3
  python minitodo.py search "milk"
  python minitodo.py show C3
  python minitodo.py update C3 title="Renamed" endTime="2026-05-20 18:00"

Exit codes: 0 ok (including a degraded /health) · 2 config / usage error ·
3 network error · 4 HTTP error from the server (message on stderr).
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import sys
import time
from pathlib import Path
from typing import Any, Callable

try:
    import requests
except ImportError:
    sys.stderr.write(
        "ERROR: 缺少 requests 库。请运行: pip install requests\n"
    )
    sys.exit(2)

# Python 3.11+ has tomllib; 3.10 falls back to tomli
try:
    import tomllib  # type: ignore[import-not-found]
except ImportError:  # pragma: no cover - py<3.11
    try:
        import tomli as tomllib  # type: ignore[no-redef]
    except ImportError:
        sys.stderr.write(
            "ERROR: 需要 Python 3.11+ 或安装 tomli (pip install tomli)。\n"
        )
        sys.exit(2)


DEFAULT_CONFIG = Path.home() / ".claude" / "skills" / "minitodo" / "config.toml"

# 普通请求的默认超时（秒，可被 config.toml 的 timeout 覆盖）
DEFAULT_TIMEOUT = 10.0
# /sync 系列会在服务端内部做 WebDAV 拉取 + 推送（含 412 重试与等待），给足时间
SYNC_TIMEOUT = 90.0
# GET 遇到连接错误 / 超时 / 5xx 时的重试等待（秒）：共重试两次
GET_RETRY_DELAYS = (1.0, 2.0)

# 规范时间格式（与 PC SQLite / 云端一致）
CANONICAL_FORMAT = "%Y-%m-%d %H:%M:%S"


# =============================================================================
# 输出编码
# =============================================================================


def force_utf8_stdio() -> None:
    """stdout / stderr 统一 UTF-8：Windows 中文控制台（cp936）默认编码写不出 ✓ 等字符，
    会直接抛 UnicodeEncodeError；AI 通过管道读取输出时也期望 UTF-8。"""
    for stream in (sys.stdout, sys.stderr):
        reconfigure = getattr(stream, "reconfigure", None)
        if reconfigure is None:
            continue
        try:
            reconfigure(encoding="utf-8", errors="replace")
        except (ValueError, OSError):  # pragma: no cover - 已被替换成非文本流
            pass


# =============================================================================
# 配置加载
# =============================================================================


def load_config(path: Path | None) -> dict[str, Any]:
    cfg_path = path or DEFAULT_CONFIG
    if not cfg_path.exists():
        die(
            f"找不到配置 {cfg_path}\n"
            "请先复制 config.example.toml 到该路径并填入 endpoint / api_key。"
        )
    with cfg_path.open("rb") as f:
        cfg = tomllib.load(f)
    if not cfg.get("endpoint"):
        die(f"{cfg_path} 缺少 endpoint")
    if not cfg.get("api_key"):
        die(f"{cfg_path} 缺少 api_key")
    return cfg


def die(msg: str, code: int = 2) -> None:
    sys.stderr.write(msg + "\n")
    sys.exit(code)


def resolve_timezone(name: str | None) -> dt.tzinfo | None:
    """config.toml 的 `timezone`（IANA 名，例如 Asia/Shanghai）；未配置返回 None（本机时区）。"""
    if not name:
        return None
    try:
        from zoneinfo import ZoneInfo, ZoneInfoNotFoundError
    except ImportError:  # pragma: no cover - py<3.9
        die("配置了 timezone，但当前 Python 没有 zoneinfo（需要 3.9+）")
        return None
    try:
        return ZoneInfo(name)
    except (ZoneInfoNotFoundError, ValueError):
        die(
            f"config.toml 的 timezone '{name}' 无效。"
            "请使用 IANA 时区名（例如 Asia/Shanghai）；Windows 需要先 pip install tzdata"
        )
        return None


def today_in(tz: dt.tzinfo | None) -> dt.date:
    """按配置时区（缺省本机时区）计算"今天"。"""
    if tz is None:
        return dt.date.today()
    return dt.datetime.now(tz).date()


# =============================================================================
# 时间格式（跨端契约 K1）
# =============================================================================


def normalize_datetime(raw: str, default_time: str) -> str:
    """把用户输入规范成 `YYYY-MM-DD HH:MM:SS`（本地墙钟，无时区）。

    - 仅日期 `YYYY-MM-DD` → 补 `default_time`（截止时间 23:59:00）
    - `YYYY-MM-DD HH:MM[:SS]` / `YYYY-MM-DDTHH:MM[:SS]`、可选小数秒 → 规范格式
    - 带 `Z` / `±HH:MM` 时区后缀的值原样发给服务端，由服务端换算成它配置的时区

    无法识别时抛 ValueError。
    """
    s = raw.strip()
    if ":" not in s:
        # 仅日期（没有时刻）；不是合法日期时 fromisoformat 抛 ValueError
        return f"{dt.date.fromisoformat(s).isoformat()} {default_time}"
    value = dt.datetime.fromisoformat(re.sub(r"[zZ]$", "+00:00", s))
    if value.tzinfo is not None:
        return value.isoformat()
    return value.strftime(CANONICAL_FORMAT)


# =============================================================================
# HTTP client
# =============================================================================


class Client:
    def __init__(
        self,
        endpoint: str,
        api_key: str,
        timeout: float = DEFAULT_TIMEOUT,
        sleep: Callable[[float], None] = time.sleep,
    ):
        self.endpoint = endpoint.rstrip("/")
        self.session = requests.Session()
        self.session.headers["Authorization"] = f"Bearer {api_key}"
        self.session.headers["Accept"] = "application/json"
        self.timeout = timeout
        self._sleep = sleep

    def _url(self, path: str) -> str:
        if not path.startswith("/"):
            path = "/" + path
        return f"{self.endpoint}{path}"

    def request(
        self,
        method: str,
        path: str,
        params: dict[str, Any] | None = None,
        json_body: Any = None,
        timeout: float | None = None,
        accept_status: tuple[int, ...] = (),
    ) -> Any:
        """发请求并返回解析后的 JSON。

        - GET 是幂等的：连接错误 / 超时 / 5xx 时按 `GET_RETRY_DELAYS` 退避重试两次；
          写请求（POST / PATCH / DELETE）不自动重试，避免重复创建
        - `accept_status` 里的非 2xx 状态码（例如 /health 的 503）照常返回响应体
        - 其它错误：网络错误退出码 3，HTTP 错误退出码 4（stderr 带服务端 detail）
        """
        delays = GET_RETRY_DELAYS if method.upper() == "GET" else ()
        attempt = 0
        while True:
            try:
                resp = self.session.request(
                    method,
                    self._url(path),
                    params=params,
                    json=json_body,
                    timeout=timeout or self.timeout,
                )
            except (requests.ConnectionError, requests.Timeout) as e:
                if attempt < len(delays):
                    self._sleep(delays[attempt])
                    attempt += 1
                    continue
                die(f"HTTP error: {e}", 3)
                return None  # unreachable
            except requests.RequestException as e:
                die(f"HTTP error: {e}", 3)
                return None  # unreachable

            status = resp.status_code
            if status >= 500 and status not in accept_status and attempt < len(delays):
                self._sleep(delays[attempt])
                attempt += 1
                continue
            break

        if status == 204:
            return None
        if 200 <= status < 300 or status in accept_status:
            if not resp.content:
                return None
            try:
                return resp.json()
            except ValueError:
                return resp.text

        # error: try to surface server detail
        try:
            err = resp.json()
            detail = err.get("detail") or err.get("error") or err
        except ValueError:
            detail = resp.text
        die(f"HTTP {status}: {detail}", 4)
        return None  # unreachable

    def get(self, path: str, params: dict[str, Any] | None = None, **kw: Any) -> Any:
        return self.request("GET", path, params=params, **kw)

    def post(self, path: str, json_body: Any = None, **kw: Any) -> Any:
        return self.request("POST", path, json_body=json_body, **kw)

    def patch(self, path: str, json_body: Any = None, **kw: Any) -> Any:
        return self.request("PATCH", path, json_body=json_body, **kw)

    def delete(self, path: str, **kw: Any) -> Any:
        return self.request("DELETE", path, **kw)


# =============================================================================
# 子命令
# =============================================================================


def cmd_today(client: Client, args: argparse.Namespace) -> Any:
    """今日相关：今天到期 ∪ 今天开始 ∪ 已逾期未完成。"今天"按 config 的 timezone 计算。"""
    today = today_in(getattr(args, "tz", None))
    today_str = today.isoformat()
    yesterday_str = (today - dt.timedelta(days=1)).isoformat()

    # 服务端把时间参数规范化后按 `YYYY-MM-DD HH:MM:SS` 字符串比较；仅日期的
    # dueDateAfter 视为当天 00:00:00，dueDateBefore 视为当天 23:59:59（含当天）。
    due_today = client.get(
        "/todos",
        params={
            "dueDateAfter": today_str,
            "dueDateBefore": today_str + " 23:59:59",
        },
    )
    # 今天开始
    start_today = client.get("/todos", params={"startDate": today_str})
    # 已逾期未完成（截止时间在昨天 23:59:59 及以前）
    overdue = client.get(
        "/todos",
        params={
            "completed": "false",
            "dueDateBefore": yesterday_str + " 23:59:59",
        },
    )

    merged: dict[str, Any] = {}
    for batch in (due_today or [], start_today or [], overdue or []):
        for t in batch:
            merged[str(t.get("id"))] = t

    items = list(merged.values())
    items.sort(
        key=lambda t: (
            _priority_rank(t.get("priority")),
            t.get("endTime") or t.get("dueDate") or "",
        ),
    )
    return items


def build_add_body(args: argparse.Namespace) -> dict[str, Any]:
    body: dict[str, Any] = {"title": args.title}
    if args.priority:
        # 服务端把 priority 映射成 PC 的颜色（high/medium/low → #EF4444/#F59E0B/#10B981）
        body["priority"] = args.priority
    if args.due:
        try:
            body["endTime"] = normalize_datetime(args.due, "23:59:00")
        except ValueError:
            die(
                f"无效 --due: {args.due}（应为 YYYY-MM-DD 或 'YYYY-MM-DD HH:MM[:SS]'）"
            )
    if args.quadrant:
        body["quadrant"] = _quadrant_to_int(args.quadrant)
    if args.color:
        body["color"] = args.color
    return body


def cmd_add(client: Client, args: argparse.Namespace) -> Any:
    return client.post("/todos", json_body=build_add_body(args))


def cmd_done(client: Client, args: argparse.Namespace) -> Any:
    return client.patch(f"/todos/{args.id}", json_body={"completed": True})


def cmd_list(client: Client, args: argparse.Namespace) -> Any:
    params: dict[str, Any] = {}
    if args.completed:
        params["completed"] = "true"
    if args.pending:
        params["completed"] = "false"
    if args.priority:
        params["priority"] = args.priority
    if args.quadrant:
        params["quadrant"] = _quadrant_to_int(args.quadrant)
    if args.limit:
        params["limit"] = args.limit
    if args.sort:
        params["sort"] = args.sort
    return client.get("/todos", params=params)


def cmd_search(client: Client, args: argparse.Namespace) -> Any:
    return client.get("/todos", params={"q": args.keyword})


def cmd_show(client: Client, args: argparse.Namespace) -> Any:
    params: dict[str, Any] = {}
    if args.with_subtasks:
        params["withSubtasks"] = "true"
    return client.get(f"/todos/{args.id}", params=params)


def build_update_body(assignments: list[str]) -> dict[str, Any]:
    body: dict[str, Any] = {}
    for assignment in assignments:
        if "=" not in assignment:
            die(f"无效字段赋值: {assignment}（应形如 key=value）")
        key, value = assignment.split("=", 1)
        key = key.strip()
        if key == "quadrant":
            body[key] = _quadrant_to_int(value.strip())
        else:
            body[key] = _coerce_value(value)
    if not body:
        die("update 至少需要一个 key=value")
    return body


def cmd_update(client: Client, args: argparse.Namespace) -> Any:
    return client.patch(f"/todos/{args.id}", json_body=build_update_body(args.fields))


def cmd_delete(client: Client, args: argparse.Namespace) -> Any:
    client.delete(f"/todos/{args.id}")
    return {"deleted": args.id}


def cmd_health(client: Client, _args: argparse.Namespace) -> Any:
    """/health 在同步降级时返回 503 + 详情：这是状态信息而不是错误，照常输出、退出码 0。"""
    result = client.get("/health", accept_status=(503,))
    if isinstance(result, dict) and result.get("status") not in (None, "healthy"):
        sys.stderr.write(
            f"注意：同步状态 {result.get('sync')}（pull={result.get('pull')}, "
            f"push={result.get('push')}），数据可能不是最新；API 读写仍可用。\n"
        )
    return result


def cmd_sync(client: Client, args: argparse.Namespace) -> Any:
    timeout = max(SYNC_TIMEOUT, client.timeout)
    mode = getattr(args, "mode", None)
    if mode == "pull":
        return client.post("/sync/pull", timeout=timeout)
    if mode == "push":
        return client.post("/sync/push", timeout=timeout)
    # 部分失败时服务端回 207 + 详情
    return client.post("/sync", timeout=timeout)


# =============================================================================
# 输出
# =============================================================================


def print_result(result: Any, as_json: bool) -> None:
    if as_json:
        json.dump(result, sys.stdout, ensure_ascii=False, indent=2, default=str)
        sys.stdout.write("\n")
        return
    if result is None:
        sys.stdout.write("(no content)\n")
        return
    if isinstance(result, list):
        print_todo_table(result)
        return
    if isinstance(result, dict) and "title" in result:
        print_todo_detail(result)
        return
    json.dump(result, sys.stdout, ensure_ascii=False, indent=2, default=str)
    sys.stdout.write("\n")


def print_todo_table(items: list[dict[str, Any]]) -> None:
    if not items:
        sys.stdout.write("(no todos)\n")
        return
    rows = [
        (
            _ref_short(t),
            "[x]" if t.get("completed") else "[ ]",
            (t.get("priority") or "")[:6],
            (t.get("endTime") or t.get("dueDate") or "")[:16],
            (t.get("title") or "")[:60],
        )
        for t in items
    ]
    headers = ["REF", "DONE", "PRI", "DUE", "TITLE"]
    widths = [max(len(r[i]) for r in rows + [tuple(headers)])
              for i in range(5)]
    sys.stdout.write(_format_row(headers, widths) + "\n")
    sys.stdout.write("-" * (sum(widths) + len(widths) * 2) + "\n")
    for r in rows:
        sys.stdout.write(_format_row(list(r), widths) + "\n")


def _ref_short(t: dict[str, Any]) -> str:
    """优先展示 cloud 短码 `C{seq}`；缺 seq 时降级为完整 i64 id（截断）。"""
    seq = t.get("seq")
    if isinstance(seq, int) and seq >= 1:
        return f"C{seq}"
    return str(t.get("id", "?"))[:18]


def print_todo_detail(t: dict[str, Any]) -> None:
    seq = t.get("seq")
    seq_display = f"C{seq}" if isinstance(seq, int) and seq >= 1 else None
    fields = [
        ("Ref", seq_display),
        ("ID", t.get("id")),
        ("Title", t.get("title")),
        ("Completed", t.get("completed")),
        ("Priority", t.get("priority")),
        ("Quadrant", t.get("quadrant")),
        ("Color", t.get("color")),
        ("Start", t.get("startTime") or t.get("startDate")),
        ("Due", t.get("endTime") or t.get("dueDate")),
        ("Notify", t.get("notifyAt")),
        ("Repeat", _describe_repeat(t)),
        ("Created", t.get("createdAt")),
        ("Updated", t.get("updatedAt")),
    ]
    for k, v in fields:
        if v is None or v == "":
            continue
        sys.stdout.write(f"{k:10} {v}\n")
    if t.get("description"):
        sys.stdout.write("\nDescription:\n")
        sys.stdout.write(str(t["description"]) + "\n")
    subs = t.get("subtasks") or []
    if subs:
        sys.stdout.write(f"\nSubtasks ({len(subs)}):\n")
        for s in subs:
            mark = "[x]" if s.get("completed") else "[ ]"
            sys.stdout.write(f"  {mark} {s.get('title')}  ({s.get('id')})\n")


def _format_row(cols: list[str], widths: list[int]) -> str:
    return "  ".join(c.ljust(w) for c, w in zip(cols, widths))


# =============================================================================
# 辅助
# =============================================================================


_WEEKDAY_CN = {1: "周一", 2: "周二", 3: "周三", 4: "周四", 5: "周五", 6: "周六", 7: "周日"}


def _describe_repeat(t: dict[str, Any]) -> str | None:
    """把 repeat_* 字段拍成中文描述，便于人和 AI 直接读懂。

    None 表示该 todo 不是重复任务（或字段缺失）。规则与 PC 端
    notification.rs 的重复提醒语义对齐：
    - daily：间隔 N 天
    - weekly：间隔 N 周 + repeat_weekdays（"1,3,5" → 周一/三/五）
    - monthly：间隔 N 月 + repeat_month_day（每月第几天）
    """
    if not t.get("repeatEnabled"):
        return None
    rtype = (t.get("repeatType") or "").lower()
    interval = int(t.get("repeatInterval") or 1)
    if rtype == "daily":
        return "每天" if interval == 1 else f"每 {interval} 天"
    if rtype == "weekly":
        base = "每周" if interval == 1 else f"每 {interval} 周"
        raw = (t.get("repeatWeekdays") or "").strip()
        if raw:
            days: list[str] = []
            for piece in raw.split(","):
                piece = piece.strip()
                if piece.isdigit():
                    days.append(_WEEKDAY_CN.get(int(piece), piece))
            if days:
                return f"{base}的{'、'.join(days)}"
        return base
    if rtype == "monthly":
        day = t.get("repeatMonthDay")
        base = "每月" if interval == 1 else f"每 {interval} 月"
        if day is not None:
            return f"{base} {day} 号"
        return base
    if rtype:
        return f"重复（{rtype}，间隔 {interval}）"
    return "重复（未指定类型）"


def _priority_rank(p: str | None) -> int:
    # 升序排序用：high 最先，自定义颜色（priority 为 null）最后
    return {"high": 0, "medium": 1, "low": 2}.get(p or "", 3)


_QUADRANT_ALIASES = {
    "urgent_important": 1,
    "important_urgent": 1,
    "important_not_urgent": 2,
    "urgent_not_important": 3,
    "not_urgent_not_important": 4,
}


def _quadrant_to_int(s: str) -> int:
    key = s.strip().lower()
    if key in ("1", "2", "3", "4"):
        return int(key)
    if key not in _QUADRANT_ALIASES:
        die(
            f"无效 quadrant: {s}（1-4 或 {', '.join(sorted(set(_QUADRANT_ALIASES) - {'important_urgent'}))}）"
        )
    return _QUADRANT_ALIASES[key]


def _coerce_value(raw: str) -> Any:
    """把字符串 value 尽量解析为 bool / int / null / json。"""
    low = raw.lower()
    if low == "true":
        return True
    if low == "false":
        return False
    if low in ("null", "none"):
        return None
    if raw.isdigit() or (raw.startswith("-") and raw[1:].isdigit()):
        return int(raw)
    if raw.startswith("{") or raw.startswith("["):
        try:
            return json.loads(raw)
        except json.JSONDecodeError:
            pass
    return raw


# =============================================================================
# argparse
# =============================================================================


def build_parser() -> argparse.ArgumentParser:
    # --json / --config 同时挂在顶层和每个子命令上：argparse 要求选项紧跟其所属
    # parser，只挂顶层时 `minitodo.py today --json` 会直接报 unrecognized arguments。
    # 子命令侧默认值必须用 SUPPRESS：子命令解析走独立 namespace 再整体拷回，普通
    # 默认值（False/None）会把前置写法 `--json today` 已解析出的 True 覆盖掉；
    # SUPPRESS 让子 parser 只在 flag 真出现时才写值。顶层则用普通默认值兜底。
    # 注意顶层不能与子命令共享 action 对象（parents= 是引用共享），否则改一处
    # default 两处都变。
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument(
        "--config",
        default=argparse.SUPPRESS,
        help="config.toml 路径（默认 ~/.claude/skills/minitodo/config.toml）",
    )
    common.add_argument(
        "--json",
        action="store_true",
        default=argparse.SUPPRESS,
        help="输出原始 JSON 而非表格",
    )

    p = argparse.ArgumentParser(prog="minitodo", description="mini-todo CLI")
    p.add_argument("--config", help="config.toml 路径（默认 ~/.claude/skills/minitodo/config.toml）")
    p.add_argument("--json", action="store_true", help="输出原始 JSON 而非表格")
    sub = p.add_subparsers(dest="command", required=True)

    sub.add_parser(
        "today",
        help="今日相关待办（今天按 config 的 timezone 计算，缺省本机时区）",
        parents=[common],
    )

    sp = sub.add_parser("add", help="新增待办", parents=[common])
    sp.add_argument("title")
    sp.add_argument(
        "--priority",
        choices=["high", "medium", "low"],
        help="优先级；服务端映射为 PC 颜色 high=#EF4444 medium=#F59E0B low=#10B981",
    )
    sp.add_argument(
        "--due",
        help="截止时间，写入 endTime，规范格式 'YYYY-MM-DD HH:MM:SS'；"
        "也可写 YYYY-MM-DD（当天 23:59:00）或 'YYYY-MM-DD HH:MM'",
    )
    sp.add_argument("--quadrant", help="1-4 或 urgent_important 等别名")
    sp.add_argument("--color", help="HEX 颜色 (e.g. #EF4444)，与 --priority 同时给时以颜色为准")

    sp = sub.add_parser("done", help="标记完成", parents=[common])
    sp.add_argument("id", help="完整 i64 id 或 C{seq} 短码（cloud 端反查），大小写不敏感")

    sp = sub.add_parser("list", help="列表", parents=[common])
    sp.add_argument("--completed", action="store_true", help="仅显示已完成")
    sp.add_argument("--pending", action="store_true", help="仅显示未完成")
    sp.add_argument("--priority", choices=["high", "medium", "low"], help="按颜色派生的优先级过滤")
    sp.add_argument("--quadrant")
    sp.add_argument("--limit", type=int)
    sp.add_argument("--sort", help="例如 -dueDate / +priority")

    sp = sub.add_parser("search", help="关键词搜索（标题 + 描述）", parents=[common])
    sp.add_argument("keyword")

    sp = sub.add_parser("show", help="查看详情", parents=[common])
    sp.add_argument("id", help="完整 i64 id 或 C{seq} 短码")
    sp.add_argument("--with-subtasks", action="store_true", default=True)

    sp = sub.add_parser("update", help="更新字段", parents=[common])
    sp.add_argument("id", help="完整 i64 id 或 C{seq} 短码")
    sp.add_argument(
        "fields",
        nargs="+",
        help="key=value，可多个；quadrant 接受别名；时间用 'YYYY-MM-DD HH:MM:SS'；未知字段会被服务端拒绝",
    )

    sp = sub.add_parser("delete", help="删除", parents=[common])
    sp.add_argument("id", help="完整 i64 id 或 C{seq} 短码")

    sub.add_parser("health", help="health check（同步降级时也照常输出，退出码 0）", parents=[common])

    sp = sub.add_parser("sync", help="手动触发 WebDAV 同步", parents=[common])
    sp.add_argument("mode", nargs="?", choices=["pull", "push"], help="仅 pull 或仅 push；省略则两者都做")

    return p


def main(argv: list[str] | None = None) -> int:
    force_utf8_stdio()
    parser = build_parser()
    args = parser.parse_args(argv)

    cfg_path = Path(args.config) if args.config else None
    cfg = load_config(cfg_path)
    endpoint = cfg["endpoint"]
    api_key = cfg["api_key"]
    timeout = float(cfg.get("timeout", DEFAULT_TIMEOUT))
    args.tz = resolve_timezone(cfg.get("timezone"))
    client = Client(endpoint, api_key, timeout=timeout)

    handlers = {
        "today": cmd_today,
        "add": cmd_add,
        "done": cmd_done,
        "list": cmd_list,
        "search": cmd_search,
        "show": cmd_show,
        "update": cmd_update,
        "delete": cmd_delete,
        "health": cmd_health,
        "sync": cmd_sync,
    }
    handler = handlers.get(args.command)
    if not handler:
        die(f"未知子命令: {args.command}")
    assert handler is not None
    result = handler(client, args)
    print_result(result, args.json)
    return 0


if __name__ == "__main__":
    sys.exit(main())
