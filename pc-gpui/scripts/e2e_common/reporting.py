"""跨平台 E2E 用例框架、结果汇总、只读 SQLite 查询与 seed 辅助函数。"""
from __future__ import annotations

import json
import os
import shutil
import sqlite3
import subprocess
from pathlib import Path
from typing import Any


class Skip(Exception):
    """在用例内部主动标记当前用例跳过。"""


CURRENT_REPORT: list[Any] = [None]


class Checks:
    """一个用例里的多项软断言：全部检查完再统一判定。"""

    def __init__(self) -> None:
        self.items: list[tuple[bool, str]] = []

    def ok(self, cond: Any, what: str) -> bool:
        self.items.append((bool(cond), what))
        print(f"    {'ok ' if cond else 'BAD'} {what}", flush=True)
        return bool(cond)

    def near(self, actual: float, expected: float, tol: float, what: str) -> bool:
        return self.ok(
            abs(actual - expected) <= tol,
            f"{what}: {actual:g} ≈ {expected:g} (±{tol:g})",
        )

    def eq(self, actual: Any, expected: Any, what: str) -> bool:
        return self.ok(actual == expected, f"{what}: {actual!r} == {expected!r}")

    def done(self, summary: str = "") -> str:
        bad = [w for c, w in self.items if not c]
        if bad:
            if CURRENT_REPORT[0]:
                CURRENT_REPORT[0].snapshot()
            raise AssertionError("; ".join(bad))
        return summary or f"{len(self.items)} 项检查通过"


class Tee:
    """把 stdout 同时写回原流并缓冲，失败时落盘 `fail-<id>.txt`。"""

    def __init__(self, stream: Any) -> None:
        self.stream = stream
        self.buf: list[str] = []

    def write(self, data: str) -> int:
        self.buf.append(data)
        return self.stream.write(data)

    def flush(self) -> None:
        self.stream.flush()

    def __getattr__(self, name: str) -> Any:
        return getattr(self.stream, name)


def write_report_summary(out_dir: Path, title: str, results: list[dict[str, Any]]) -> int:
    """写出 results.json / summary.md / GITHUB_STEP_SUMMARY 并打印终端汇总；有失败返回 1。"""
    (out_dir / "results.json").write_text(
        json.dumps(results, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )
    counts = {
        s: sum(1 for r in results if r["status"] == s)
        for s in ("PASS", "FAIL", "SKIP")
    }
    lines = [
        f"## {title}",
        "",
        f"PASS {counts['PASS']} / FAIL {counts['FAIL']} / SKIP {counts['SKIP']}",
        "",
        "| 用例 | 结果 | 说明 |",
        "|---|---|---|",
    ]
    for r in results:
        detail = r["detail"].replace("|", "\\|").replace("\n", " ")
        if len(detail) > 300:
            detail = detail[:300] + "…"
        lines.append(f"| {r['id']} {r['title']} | {r['status']} | {detail} |")
    md = "\n".join(lines) + "\n"
    (out_dir / "summary.md").write_text(md, encoding="utf-8")
    step_summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if step_summary:
        with open(step_summary, "a", encoding="utf-8") as f:
            f.write(md + "\n")
    print("\n========== E2E SUMMARY ==========")
    w = max((len(r["id"]) for r in results), default=4)
    for r in results:
        detail = r["detail"].replace("\n", " ")
        if len(detail) > 220:
            detail = detail[:220] + "…"
        print(f"{r['status']:<4} {r['id']:<{w}} {r['title']} — {detail}")
    print(f"TOTAL: PASS {counts['PASS']}  FAIL {counts['FAIL']}  SKIP {counts['SKIP']}")
    print("=================================", flush=True)
    return 1 if counts["FAIL"] else 0


def db_query(db_path: Path, sql: str, params: tuple[Any, ...] = ()) -> list[tuple[Any, ...]]:
    """只读打开 SQLite 执行查询，不影响正在运行的应用进程。"""
    con = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True, timeout=10)
    try:
        return con.execute(sql, params).fetchall()
    finally:
        con.close()


def db_query_one(db_path: Path, sql: str, params: tuple[Any, ...] = ()) -> Any:
    rows = db_query(db_path, sql, params)
    return rows[0][0] if rows and rows[0] else None


def seed_database(seed_exe: Path, data_dir: Path, sample: Path) -> subprocess.CompletedProcess[str]:
    """清空隔离数据目录并运行 core 的 seed 示例导入样例数据。"""
    if data_dir.exists():
        shutil.rmtree(data_dir, ignore_errors=True)
    return subprocess.run(
        [str(seed_exe), str(data_dir), str(sample)],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=120,
    )
