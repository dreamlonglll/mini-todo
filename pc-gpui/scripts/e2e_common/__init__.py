"""跨平台 E2E 测试共享模块（e2e-windows / e2e-macos / e2e-linux 共用）。"""

from .reporting import (
    CURRENT_REPORT,
    Checks,
    Skip,
    Tee,
    db_query,
    db_query_one,
    seed_database,
    write_report_summary,
)

__all__ = [
    "CURRENT_REPORT",
    "Checks",
    "Skip",
    "Tee",
    "db_query",
    "db_query_one",
    "seed_database",
    "write_report_summary",
]
