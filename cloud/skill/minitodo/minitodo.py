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
  python minitodo.py done 1234567890
  python minitodo.py search "milk"
  python minitodo.py show 1234567890 --with-subtasks
  python minitodo.py update 1234567890 --title="Renamed"
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import sys
from pathlib import Path
from typing import Any

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


# =============================================================================
# HTTP client
# =============================================================================


class Client:
    def __init__(self, endpoint: str, api_key: str, timeout: float = 10.0):
        self.endpoint = endpoint.rstrip("/")
        self.session = requests.Session()
        self.session.headers["Authorization"] = f"Bearer {api_key}"
        self.session.headers["Accept"] = "application/json"
        self.timeout = timeout

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
    ) -> Any:
        try:
            resp = self.session.request(
                method,
                self._url(path),
                params=params,
                json=json_body,
                timeout=self.timeout,
            )
        except requests.RequestException as e:
            die(f"HTTP error: {e}", 3)

        if resp.status_code == 204:
            return None
        if 200 <= resp.status_code < 300:
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
        die(f"HTTP {resp.status_code}: {detail}", 4)
        return None  # unreachable

    def get(self, path: str, params: dict[str, Any] | None = None) -> Any:
        return self.request("GET", path, params=params)

    def post(self, path: str, json_body: Any = None) -> Any:
        return self.request("POST", path, json_body=json_body)

    def patch(self, path: str, json_body: Any = None) -> Any:
        return self.request("PATCH", path, json_body=json_body)

    def delete(self, path: str) -> Any:
        return self.request("DELETE", path)


# =============================================================================
# 子命令
# =============================================================================


def cmd_today(client: Client, args: argparse.Namespace) -> Any:
    """今日相关：dueDate=today + startDate=today + 已 overdue 未完成。"""
    today_str = dt.date.today().isoformat()

    # 今天到期：[今天 00:00, 今天 23:59:59]
    due_today = client.get(
        "/todos",
        params={
            "dueDateAfter": today_str + "T00:00:00",
            "dueDateBefore": today_str + "T23:59:59",
        },
    )
    # 今天开始
    start_today = client.get("/todos", params={"startDate": today_str})
    # 已 overdue 未完成（截止日期 < 今天 00:00 且未完成）
    overdue = client.get(
        "/todos",
        params={
            "completed": "false",
            "dueDateBefore": today_str + "T00:00:00",
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
            t.get("dueDate") or t.get("endTime") or "",
        ),
        reverse=False,
    )
    return items


def cmd_due_soon(client: Client, args: argparse.Namespace) -> Any:
    """临期：未来 N 小时内到期未完成 ∪ 已逾期未完成。

    给 openclaw / Claude Code cron 类调度场景用。
    输出顺序：先 overdue（旧的在前），再 upcoming（近的在前）。
    过滤：同时缺 notifyAt 与 dueDate/endTime 的 todo 不展示（避免噪音）。
    """
    hours = max(1, int(args.hours or 24))
    now = dt.datetime.now()
    future = now + dt.timedelta(hours=hours)
    now_str = now.strftime("%Y-%m-%dT%H:%M:%S")
    future_str = future.strftime("%Y-%m-%dT%H:%M:%S")

    upcoming = client.get(
        "/todos",
        params={
            "completed": "false",
            "dueDateAfter": now_str,
            "dueDateBefore": future_str,
            "sort": "+dueDate",
        },
    ) or []
    overdue = client.get(
        "/todos",
        params={
            "completed": "false",
            "dueDateBefore": now_str,
            "sort": "+dueDate",
        },
    ) or []

    seen: set[str] = set()
    items: list[dict[str, Any]] = []
    for batch in (overdue, upcoming):
        for t in batch:
            tid = str(t.get("id"))
            if tid in seen:
                continue
            if not _has_reminder_window(t):
                continue
            seen.add(tid)
            items.append(t)
    return items


def _has_reminder_window(t: dict[str, Any]) -> bool:
    """要求至少有一个时间字段：notifyAt 或 dueDate/endTime。

    没有任何时间锚点的 todo 不应出现在临期提醒里——既无法判断"几小时后"，
    也容易让 channel 推送出现一堆"无时间"条目造成噪音。
    """
    return bool(
        (t.get("notifyAt") or "").strip()
        or (t.get("dueDate") or "").strip()
        or (t.get("endTime") or "").strip()
    )


def cmd_add(client: Client, args: argparse.Namespace) -> Any:
    body: dict[str, Any] = {"title": args.title}
    if args.priority:
        body["priority"] = args.priority
    if args.due:
        body["dueDate"] = args.due
    if args.quadrant:
        body["quadrant"] = _quadrant_to_int(args.quadrant)
    if args.color:
        body["color"] = args.color
    return client.post("/todos", json_body=body)


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


def cmd_update(client: Client, args: argparse.Namespace) -> Any:
    body: dict[str, Any] = {}
    for assignment in args.fields:
        if "=" not in assignment:
            die(f"无效字段赋值: {assignment}（应形如 key=value）")
        key, value = assignment.split("=", 1)
        body[key] = _coerce_value(value)
    if not body:
        die("update 至少需要一个 --field=value")
    return client.patch(f"/todos/{args.id}", json_body=body)


def cmd_delete(client: Client, args: argparse.Namespace) -> Any:
    client.delete(f"/todos/{args.id}")
    return {"deleted": args.id}


def cmd_health(client: Client, _args: argparse.Namespace) -> Any:
    return client.get("/health")


# =============================================================================
# 输出
# =============================================================================


def print_result(
    result: Any,
    as_json: bool,
    notify_text: bool = False,
    notify_hours: int = 24,
    silent_if_empty: bool = False,
) -> None:
    if notify_text:
        text = format_notify_text(result, hours=notify_hours)
        if silent_if_empty and _notify_is_empty(result):
            return
        sys.stdout.write(text + "\n")
        return
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


def _notify_is_empty(result: Any) -> bool:
    return isinstance(result, list) and len(result) == 0


def format_notify_text(result: Any, hours: int = 24) -> str:
    """把 due-soon 结果拍成紧凑中文 channel 消息。"""
    if not isinstance(result, list):
        return str(result)
    if not result:
        return f"mini-todo｜未来 {hours}h 无临期事项。"

    now = dt.datetime.now()
    overdue: list[dict[str, Any]] = []
    upcoming: list[dict[str, Any]] = []
    for t in result:
        due_raw = t.get("dueDate") or t.get("endTime") or ""
        when = _parse_dt(due_raw)
        if when is not None and when < now:
            overdue.append(t)
        else:
            upcoming.append(t)

    lines: list[str] = []
    lines.append(f"mini-todo 临期提醒｜{now.strftime('%Y-%m-%d %H:%M')}")
    if overdue:
        lines.append(f"已逾期（{len(overdue)}）：")
        for t in overdue:
            lines.append("  - " + _notify_line(t, now))
    if upcoming:
        lines.append(f"未来 {hours}h 到期（{len(upcoming)}）：")
        for t in upcoming:
            lines.append("  - " + _notify_line(t, now))
    return "\n".join(lines)


def _notify_line(t: dict[str, Any], now: dt.datetime) -> str:
    pri_map = {"high": "高", "medium": "中", "low": "低"}
    pri = pri_map.get((t.get("priority") or "").lower(), "")
    title = (t.get("title") or "(无标题)").strip()
    tid = str(t.get("id") or "").strip()
    # 排序时优先用 dueDate/endTime（这是 cmd_due_soon 查询所基于的字段），
    # 缺这俩才退到 notifyAt（仅提醒时间、无截止时间的场景）
    due_raw = (
        t.get("dueDate") or t.get("endTime") or t.get("notifyAt") or ""
    )
    when = _parse_dt(due_raw)
    if when is None:
        delta_str = ""
    else:
        delta = when - now
        total_min = int(delta.total_seconds() // 60)
        if total_min >= 0:
            if total_min < 60:
                delta_str = f"，{total_min} 分钟后"
            elif total_min < 60 * 24:
                delta_str = f"，{total_min // 60} 小时后"
            else:
                delta_str = f"，{total_min // (60 * 24)} 天后"
        else:
            ago_min = -total_min
            if ago_min < 60:
                delta_str = f"，已逾期 {ago_min} 分钟"
            elif ago_min < 60 * 24:
                delta_str = f"，已逾期 {ago_min // 60} 小时"
            else:
                delta_str = f"，已逾期 {ago_min // (60 * 24)} 天"
    id_str = f"#{tid} " if tid else ""
    pri_str = f"[{pri}] " if pri else ""
    when_str = when.strftime("%m-%d %H:%M") if when else (due_raw[:16] if due_raw else "")
    when_part = f" ({when_str}{delta_str})" if when_str else ""
    return f"{id_str}{pri_str}{title}{when_part}"


def _parse_dt(s: str) -> dt.datetime | None:
    """容错解析 mini-todo 时间字符串。

    云端写出的是 'YYYY-MM-DD HH:MM:SS'（无时区），前端有时回传带 T 或带 Z。
    这里都按本地墙钟解析，与 cloud/src/time.rs 的语义保持一致。
    """
    if not s:
        return None
    s = s.strip()
    if s.endswith("Z"):
        s = s[:-1]
    # 截掉 +08:00 / -05:00 这种 offset 后缀（mini-todo 内部不带，外部偶发）
    if len(s) >= 6 and s[-6] in "+-" and s[-3] == ":":
        s = s[:-6]
    for fmt in (
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%d",
    ):
        try:
            return dt.datetime.strptime(s, fmt)
        except ValueError:
            continue
    return None


def print_todo_table(items: list[dict[str, Any]]) -> None:
    if not items:
        sys.stdout.write("(no todos)\n")
        return
    rows = [
        (
            str(t.get("id", "?"))[:18],
            "[x]" if t.get("completed") else "[ ]",
            (t.get("priority") or "")[:6],
            (t.get("dueDate") or t.get("endTime") or "")[:16],
            (t.get("title") or "")[:60],
        )
        for t in items
    ]
    widths = [max(len(r[i]) for r in rows + [("ID", "✓", "PRI", "DUE", "TITLE")])
              for i in range(5)]
    headers = ["ID", "✓", "PRI", "DUE", "TITLE"]
    sys.stdout.write(_format_row(headers, widths) + "\n")
    sys.stdout.write("-" * (sum(widths) + len(widths) * 2) + "\n")
    for r in rows:
        sys.stdout.write(_format_row(list(r), widths) + "\n")


def print_todo_detail(t: dict[str, Any]) -> None:
    fields = [
        ("ID", t.get("id")),
        ("Title", t.get("title")),
        ("Completed", t.get("completed")),
        ("Priority", t.get("priority")),
        ("Quadrant", t.get("quadrant")),
        ("Color", t.get("color")),
        ("Start", t.get("startTime") or t.get("startDate")),
        ("Due", t.get("endTime") or t.get("dueDate")),
        ("Notify", t.get("notifyAt")),
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


def _priority_rank(p: str | None) -> int:
    # 排序时用 negative ranks 让 high 先出来；这里返回 ascending 用的小值=优先
    return {"high": 0, "medium": 1, "low": 2}.get(p or "", 3)


def _quadrant_to_int(s: str) -> int:
    mapping = {
        "1": 1,
        "2": 2,
        "3": 3,
        "4": 4,
        "urgent_important": 1,
        "important_urgent": 1,
        "important_not_urgent": 2,
        "urgent_not_important": 3,
        "not_urgent_not_important": 4,
    }
    if s not in mapping:
        die(f"无效 quadrant: {s}")
    return mapping[s]


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
    p = argparse.ArgumentParser(prog="minitodo", description="mini-todo CLI")
    p.add_argument("--config", help="config.toml 路径（默认 ~/.claude/skills/minitodo/config.toml）")
    p.add_argument("--json", action="store_true", help="输出原始 JSON 而非表格")
    sub = p.add_subparsers(dest="command", required=True)

    sp = sub.add_parser("today", help="今日相关待办")

    sp = sub.add_parser(
        "due-soon",
        help="临期：未来 N 小时内到期 ∪ 已逾期未完成（适合 cron 推送）",
    )
    sp.add_argument(
        "--hours",
        type=int,
        default=24,
        help="未来多少小时内算临期，默认 24",
    )
    sp.add_argument(
        "--notify-text",
        action="store_true",
        help="输出适合 channel 推送的中文紧凑文本（与 --json 互斥）",
    )
    sp.add_argument(
        "--silent-if-empty",
        action="store_true",
        help="无任何条目时不输出（配合 --notify-text 使用，避免 cron 空推）",
    )

    sp = sub.add_parser("add", help="新增待办")
    sp.add_argument("title")
    sp.add_argument("--priority", choices=["high", "medium", "low"])
    sp.add_argument("--due", help="dueDate (ISO 8601)")
    sp.add_argument("--quadrant", help="1-4 或 urgent_important 等别名")
    sp.add_argument("--color", help="HEX 颜色 (e.g. #EF4444)")

    sp = sub.add_parser("done", help="标记完成")
    sp.add_argument("id")

    sp = sub.add_parser("list", help="列表")
    sp.add_argument("--completed", action="store_true", help="仅显示已完成")
    sp.add_argument("--pending", action="store_true", help="仅显示未完成")
    sp.add_argument("--priority", choices=["high", "medium", "low"])
    sp.add_argument("--quadrant")
    sp.add_argument("--limit", type=int)
    sp.add_argument("--sort", help="例如 -dueDate / +priority")

    sp = sub.add_parser("search", help="关键词搜索")
    sp.add_argument("keyword")

    sp = sub.add_parser("show", help="查看详情")
    sp.add_argument("id")
    sp.add_argument("--with-subtasks", action="store_true", default=True)

    sp = sub.add_parser("update", help="更新字段")
    sp.add_argument("id")
    sp.add_argument("fields", nargs="+", help="key=value，可多个")

    sp = sub.add_parser("delete", help="删除")
    sp.add_argument("id")

    sp = sub.add_parser("health", help="health check")

    return p


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    cfg_path = Path(args.config) if args.config else None
    cfg = load_config(cfg_path)
    endpoint = cfg["endpoint"]
    api_key = cfg["api_key"]
    timeout = float(cfg.get("timeout", 10.0))
    client = Client(endpoint, api_key, timeout=timeout)

    handlers = {
        "today": cmd_today,
        "due-soon": cmd_due_soon,
        "add": cmd_add,
        "done": cmd_done,
        "list": cmd_list,
        "search": cmd_search,
        "show": cmd_show,
        "update": cmd_update,
        "delete": cmd_delete,
        "health": cmd_health,
    }
    handler = handlers.get(args.command)
    if not handler:
        die(f"未知子命令: {args.command}")
    assert handler is not None
    result = handler(client, args)
    notify_text = bool(getattr(args, "notify_text", False))
    silent_if_empty = bool(getattr(args, "silent_if_empty", False))
    notify_hours = int(getattr(args, "hours", 24) or 24)
    print_result(
        result,
        args.json,
        notify_text=notify_text,
        notify_hours=notify_hours,
        silent_if_empty=silent_if_empty,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
