#!/usr/bin/env python3
"""e2e 脚本里不碰窗口的纯逻辑自测（不启动应用、不产生任何输入）：

    python selftest.py

- winlib.parse_tray_menu_ids：按 tray.rs 的创建顺序算出的 muda 命令 id 与已验证的表一致；顺序变了 id 跟着变
- run_e2e.pending_ids：默认清单兜住 list_id 指向不存在的清单的待办，其它清单只看自己的
"""
from __future__ import annotations

import sqlite3
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import winlib as W  # noqa: E402
import run_e2e as R  # noqa: E402


class TrayMenuIds(unittest.TestCase):
    def test_real_tray_rs_matches_the_verified_table(self):
        ids = W.parse_tray_menu_ids(R.TRAY_RS.read_text(encoding="utf-8"))
        self.assertEqual(ids, W.TRAY_MENU_IDS_FALLBACK)
        self.assertEqual(W.tray_menu_ids(R.TRAY_RS), W.TRAY_MENU_IDS_FALLBACK)

    def test_ids_follow_creation_order_including_menu_and_separators(self):
        src = (
            'pub mod menu_id {\r\n    pub const QUIT: &str = "quit";\r\n    pub const RESET: &str = "reset";\r\n}\r\n'
            "fn start(cx: &mut App) {\r\n    if cx.has_global::<Tray>() { return; }\r\n"
            "    let a = CheckMenuItem::with_id(\r\n        menu_id::RESET, \"重置\", true, false, None);\r\n"
            "    let menu = Menu::new();\r\n"
            "    Box::new(PredefinedMenuItem::separator()),\r\n"
            "    Box::new(MenuItem::with_id(menu_id::QUIT, \"退出\", true, None)),\r\n"
            "    let builder = TrayIconBuilder::new();\r\n"
            "    Box::new(MenuItem::with_id(menu_id::RESET, \"后面的不算\", true, None)),\r\n}\r\n"
        )
        self.assertEqual(W.parse_tray_menu_ids(src), {"reset": 1000, "quit": 1003})

    def test_unreadable_source_falls_back_to_the_table(self):
        self.assertEqual(W.tray_menu_ids(HERE / "no-such-tray.rs"), W.TRAY_MENU_IDS_FALLBACK)
        self.assertEqual(W.parse_tray_menu_ids("fn nothing() {}"), {})


class PendingIds(unittest.TestCase):
    def test_default_list_also_shows_todos_of_missing_lists(self):
        with tempfile.TemporaryDirectory() as d:
            con = sqlite3.connect(Path(d) / "data.db")
            con.executescript(
                "CREATE TABLE todo_lists (id INTEGER PRIMARY KEY, name TEXT);"
                "CREATE TABLE todos (id INTEGER PRIMARY KEY, completed INTEGER, list_id INTEGER, sort_order INTEGER,"
                " created_at TEXT);"
                "INSERT INTO todo_lists VALUES (1, '待办清单'), (7, 'E2E');"
                "INSERT INTO todos VALUES (1, 0, 1, 2, 'a'), (2, 0, 7, 1, 'b'), (3, 0, 99, 0, 'c'), (4, 1, 1, 0, 'd');"
            )
            con.commit()
            con.close()
            app = W.App(Path(sys.executable), data_dir=Path(d))
            self.assertEqual(R.pending_ids(app), [3, 1])
            self.assertEqual(R.pending_ids(app, 7), [2])


if __name__ == "__main__":
    unittest.main(verbosity=2)
