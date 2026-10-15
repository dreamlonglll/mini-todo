//! 把导出格式 4.0 的 JSON 导入到一个**隔离的**数据目录（截图 / 冒烟测试用）：
//!
//! ```bash
//! cargo run -p mini-todo-core --example seed -- <data-dir> <json>
//! ```
//!
//! - 以 debug 构建运行：通过 `MINI_TODO_DATA_DIR=<data-dir>` 让 core 的全部路径（库、图片、备份、节假日缓存）
//!   都落在 `<data-dir>`；release 构建会忽略该变量，因此直接拒绝运行，避免写到真实数据目录。
//! - `settings` 既可以是 `AppSettings` 形状（真实导出文件），也可以是参考截图 harness（`fixtures.mjs`）的 mock 形状：
//!   `{ autoHideEnabled, topOnWake, fixedEmbedDesktop, windowBackground: { color, alpha }, todoFontFamily,
//!   todoFontSize, notificationType, darkTheme?, showCalendar?, viewMode? … }`，后者转换为 `AppSettings` 后导入，
//!   字体与字号另外写入。
//! - 导入走 `data::import_json`（恢复备份语义：保留原 id）。

use std::path::PathBuf;

use mini_todo_core::db::paths;
use mini_todo_core::{data, settings, Database};
use serde_json::{json, Value};

fn main() {
    if let Err(e) = run() {
        // 命令行工具：错误直接给到终端
        eprintln!("seed 失败: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [data_dir, json_path] = args.as_slice() else {
        return Err("用法: seed <data-dir> <json>".into());
    };
    let data_dir = std::path::absolute(PathBuf::from(data_dir)).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    // core 的路径（图片 / 节假日缓存）都经 app_data_dir()；只有 debug 构建才读这个变量
    std::env::set_var(paths::DEV_DATA_DIR_ENV, &data_dir);
    if paths::dev_data_dir_override().as_deref() != Some(data_dir.as_path()) {
        return Err(
            "请用 debug 构建运行（release 会忽略 MINI_TODO_DATA_DIR，可能写到真实数据目录）".into(),
        );
    }

    let text = std::fs::read_to_string(json_path).map_err(|e| format!("读取 {json_path}: {e}"))?;
    let mut doc: Value = serde_json::from_str(&text).map_err(|e| format!("解析 JSON: {e}"))?;
    let raw_settings = doc.get("settings").cloned().unwrap_or(Value::Null);
    let font = if raw_settings.get("isFixed").is_none() {
        let (converted, font) = convert_mock_settings(&raw_settings);
        doc["settings"] = converted;
        font
    } else {
        None
    };

    let db = Database::open(&paths::db_path())?;
    let todos = doc["todos"].as_array().map_or(0, Vec::len);
    data::import_json(&db, &doc.to_string())?;
    if let Some((family, size)) = font {
        settings::set_font_family(&db, &family)?;
        settings::set_font_size(&db, size)?;
    }
    println!(
        "已导入 {todos} 条待办到 {}（数据目录 {}）",
        paths::db_path().display(),
        data_dir.display()
    );
    Ok(())
}

/// 截图 harness 的 mock 设置 → `AppSettings`（返回值第二项是待办字体与字号）
fn convert_mock_settings(mock: &Value) -> (Value, Option<(String, i32)>) {
    let bool_of =
        |key: &str, default: bool| mock.get(key).and_then(Value::as_bool).unwrap_or(default);
    let str_of = |key: &str, default: &str| {
        mock.get(key)
            .and_then(Value::as_str)
            .unwrap_or(default)
            .to_string()
    };
    let bg = mock.get("windowBackground");
    let settings = json!({
        "isFixed": bool_of("isFixed", false),
        "fixedEmbedDesktop": bool_of("fixedEmbedDesktop", false),
        "windowPosition": null,
        "windowSize": null,
        "autoHideEnabled": bool_of("autoHideEnabled", true),
        "topOnWake": bool_of("topOnWake", true),
        "windowBgColor": bg.and_then(|b| b.get("color")).and_then(Value::as_str).unwrap_or("#000000"),
        "windowBgAlpha": bg.and_then(|b| b.get("alpha")).and_then(Value::as_f64).unwrap_or(0.45),
        // text_theme 语义反转："light" = 深色主题
        "textTheme": if bool_of("darkTheme", false) { "light" } else { "dark" },
        "showCalendar": bool_of("showCalendar", false),
        "viewMode": str_of("viewMode", "list"),
        "notificationType": str_of("notificationType", "system"),
    });
    let font = mock
        .get("todoFontSize")
        .and_then(Value::as_i64)
        .map(|size| {
            (
                str_of("todoFontFamily", ""),
                i32::try_from(size).unwrap_or(14),
            )
        });
    (settings, font)
}
