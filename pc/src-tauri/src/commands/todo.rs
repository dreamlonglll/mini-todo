use crate::db::paths::{self, is_safe_image_name};
use crate::db::sync_store;
use crate::db::time::{normalize_datetime, now_local, DefaultTime, SQL_SET_UPDATED_AT};
use crate::db::{
    subtask_from_row, todo_from_row, CreateSubTaskRequest, CreateTodoRequest, Database, SubTask,
    Todo, UpdateSubTaskRequest, UpdateTodoRequest, SUBTASK_COLUMNS, TODO_COLUMNS,
};
use rusqlite::{Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Manager, State};

/// 待办列表顺序：未完成在前，其次手动排序，最后新建的在前
const TODO_LIST_ORDER: &str = "completed ASC, sort_order ASC, created_at DESC";

#[tauri::command]
pub fn get_todos(db: State<Database>) -> Result<Vec<Todo>, String> {
    // 一次查待办、一次查子任务按 parent 分组（以前每条待办单独查一次子任务，N+1）
    db.with_connection(|conn| sync_store::load_todos_with_subtasks(conn, TODO_LIST_ORDER))
        .map_err(|e| e.to_string())
}

/// 读取单条待办（含子任务）。编辑窗口只需要这一条，不必每次子任务操作后拉全库。
#[tauri::command]
pub fn get_todo(db: State<Database>, id: i64) -> Result<Todo, String> {
    db.with_connection(|conn| load_todo(conn, id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "待办不存在".to_string())
}

/// 本地数据变更计数：todos / subtasks 任意增删改都会增大（v28 触发器维护）。
/// 主窗口轮询它，数值变了才全量刷新列表。
#[tauri::command]
pub fn get_change_seq(db: State<Database>) -> Result<i64, String> {
    db.with_connection(sync_store::local_seq)
        .map_err(|e| e.to_string())
}

fn load_todo(conn: &Connection, id: i64) -> rusqlite::Result<Option<Todo>> {
    let todo_sql = format!("SELECT {} FROM todos WHERE id = ?1", TODO_COLUMNS);
    let Some(mut todo) = conn
        .prepare_cached(&todo_sql)?
        .query_row([id], todo_from_row)
        .optional()?
    else {
        return Ok(None);
    };
    let subtask_sql = format!(
        "SELECT {} FROM subtasks WHERE parent_id = ?1 ORDER BY sort_order ASC, id ASC",
        SUBTASK_COLUMNS
    );
    let mut stmt = conn.prepare_cached(&subtask_sql)?;
    let rows = stmt.query_map([id], subtask_from_row)?;
    todo.subtasks = rows.collect::<rusqlite::Result<_>>()?;
    Ok(Some(todo))
}

fn load_subtask(conn: &Connection, id: i64) -> rusqlite::Result<SubTask> {
    let sql = format!("SELECT {} FROM subtasks WHERE id = ?1", SUBTASK_COLUMNS);
    conn.prepare_cached(&sql)?.query_row([id], subtask_from_row)
}

/// K1：把前端 / 外部传来的时间统一成 `YYYY-MM-DD HH:MM:SS`。
/// 空串视为未填写；仅日期按字段补默认时刻；无法识别时报错而不是原样落库。
fn normalize_time_field(
    value: Option<&str>,
    default: DefaultTime,
    label: &str,
) -> Result<Option<String>, String> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(v) => normalize_datetime(v, default)
            .map(Some)
            .ok_or_else(|| format!("{}格式无法识别：{}", label, v)),
    }
}

/// 待办的三个时间字段（提醒 09:00、开始 00:00、截止 23:59 为仅日期时的默认时刻）
struct TodoTimes {
    notify_at: Option<String>,
    start_time: Option<String>,
    end_time: Option<String>,
}

impl TodoTimes {
    fn normalize(
        notify_at: Option<&str>,
        start_time: Option<&str>,
        end_time: Option<&str>,
    ) -> Result<Self, String> {
        Ok(Self {
            notify_at: normalize_time_field(notify_at, DefaultTime::Notify, "提醒时间")?,
            start_time: normalize_time_field(start_time, DefaultTime::StartOfDay, "开始时间")?,
            end_time: normalize_time_field(end_time, DefaultTime::EndOfDay, "截止时间")?,
        })
    }
}

#[tauri::command]
pub fn create_todo(db: State<Database>, data: CreateTodoRequest) -> Result<Todo, String> {
    create_todo_inner(&db, &data)
}

fn create_todo_inner(db: &Database, data: &CreateTodoRequest) -> Result<Todo, String> {
    let times = TodoTimes::normalize(
        data.notify_at.as_deref(),
        data.start_time.as_deref(),
        data.end_time.as_deref(),
    )?;
    db.with_transaction(|tx| -> rusqlite::Result<Option<Todo>> {
        let max_order: i32 = tx.query_row(
            "SELECT COALESCE(MAX(sort_order), -1) FROM todos WHERE completed = 0",
            [],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO todos (title, description, color, quadrant, notify_at, notify_before, start_time, end_time, sort_order)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                data.title,
                data.description,
                data.color,
                data.quadrant,
                times.notify_at,
                data.notify_before.unwrap_or(0),
                times.start_time,
                times.end_time,
                max_order + 1,
            ],
        )?;
        load_todo(tx, tx.last_insert_rowid())
    })
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "新建待办后读取失败".to_string())
}

#[tauri::command]
pub fn update_todo(db: State<Database>, id: i64, data: UpdateTodoRequest) -> Result<Todo, String> {
    update_todo_inner(&db, id, &data)
}

fn update_todo_inner(db: &Database, id: i64, data: &UpdateTodoRequest) -> Result<Todo, String> {
    let times = TodoTimes::normalize(
        data.notify_at.as_deref(),
        data.start_time.as_deref(),
        data.end_time.as_deref(),
    )?;

    let mut updates = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(ref title) = data.title {
        updates.push("title = ?");
        params.push(Box::new(title.clone()));
    }
    if let Some(ref desc) = data.description {
        updates.push("description = ?");
        params.push(Box::new(desc.clone()));
    }
    if let Some(ref color) = data.color {
        updates.push("color = ?");
        params.push(Box::new(color.clone()));
    }
    if let Some(quadrant) = data.quadrant {
        updates.push("quadrant = ?");
        params.push(Box::new(quadrant));
    }
    // 明确清除通知时间
    if data.clear_notify_at {
        updates.push("notify_at = NULL");
        updates.push("notified = 0");
    } else if let Some(notify_at) = times.notify_at {
        updates.push("notify_at = ?");
        params.push(Box::new(notify_at));
        // 设置新通知时间时，重置已通知状态
        updates.push("notified = 0");
    }
    if let Some(notify_before) = data.notify_before {
        updates.push("notify_before = ?");
        params.push(Box::new(notify_before));
    }
    if let Some(completed) = data.completed {
        updates.push("completed = ?");
        params.push(Box::new(if completed { 1 } else { 0 }));
    }
    if let Some(sort_order) = data.sort_order {
        updates.push("sort_order = ?");
        params.push(Box::new(sort_order));
    }
    // 开始时间
    if data.clear_start_time {
        updates.push("start_time = NULL");
    } else if let Some(start_time) = times.start_time {
        updates.push("start_time = ?");
        params.push(Box::new(start_time));
    }
    // 截止时间
    if data.clear_end_time {
        updates.push("end_time = NULL");
    } else if let Some(end_time) = times.end_time {
        updates.push("end_time = ?");
        params.push(Box::new(end_time));
    }
    // 重复提醒
    if data.clear_repeat {
        updates.push("repeat_enabled = 0");
        updates.push("repeat_type = NULL");
        updates.push("repeat_interval = 1");
        updates.push("repeat_weekdays = NULL");
        updates.push("repeat_month_day = NULL");
        updates.push("notify_before = 0");
    } else {
        if let Some(enabled) = data.repeat_enabled {
            updates.push("repeat_enabled = ?");
            params.push(Box::new(if enabled { 1i32 } else { 0 }));
            if enabled {
                updates.push("notify_before = 0");
            }
        }
        if let Some(ref repeat_type) = data.repeat_type {
            updates.push("repeat_type = ?");
            params.push(Box::new(repeat_type.clone()));
        }
        if let Some(interval) = data.repeat_interval {
            updates.push("repeat_interval = ?");
            params.push(Box::new(interval));
        }
        if let Some(ref weekdays) = data.repeat_weekdays {
            updates.push("repeat_weekdays = ?");
            params.push(Box::new(weekdays.clone()));
        }
        if let Some(month_day) = data.repeat_month_day {
            updates.push("repeat_month_day = ?");
            params.push(Box::new(month_day));
        }
    }

    if updates.is_empty() {
        return Err("No fields to update".to_string());
    }

    // 严格晚于原值：同一秒内的两次修改、或原值来自时钟偏快的设备时，新版本仍然胜出
    updates.push(SQL_SET_UPDATED_AT);
    let sql = format!("UPDATE todos SET {} WHERE id = ?", updates.join(", "));
    params.push(Box::new(id));

    db.with_connection(|conn| {
        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        conn.execute(&sql, params_refs.as_slice())?;
        load_todo(conn, id)
    })
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "待办不存在".to_string())
}

/// 删除待办及其全部子任务，并为它们写墓碑（同步时删除才能传播，不会被另一端复活）
#[tauri::command]
pub fn delete_todo(db: State<Database>, id: i64) -> Result<(), String> {
    let now = now_local();
    db.with_transaction(|tx| sync_store::delete_todo_with_tombstones(tx, id, &now).map(|_| ()))
        .map_err(|e: rusqlite::Error| e.to_string())
}

/// 按传入顺序重排待办：数组下标即 sort_order
#[tauri::command]
pub fn reorder_todos(db: State<Database>, ids: Vec<i64>) -> Result<(), String> {
    db.with_transaction(|tx| apply_order(tx, OrderTable::Todos, &ids).map(|_| ()))
        .map_err(|e: rusqlite::Error| e.to_string())
}

/// 按传入顺序重排子任务
///
/// 与 `reorder_todos` 同构：数组下标即 sort_order，查询侧统一 `ORDER BY sort_order ASC`。
#[tauri::command]
pub fn reorder_subtasks(db: State<Database>, ids: Vec<i64>) -> Result<(), String> {
    db.with_transaction(|tx| apply_order(tx, OrderTable::Subtasks, &ids).map(|_| ()))
        .map_err(|e: rusqlite::Error| e.to_string())
}

#[derive(Clone, Copy)]
enum OrderTable {
    Todos,
    Subtasks,
}

/// 把 `ids` 的下标写成 sort_order，只更新 sort_order 真正变化的行，返回更新的行数。
///
/// 只有这些行刷新 updated_at（排序也要随同步传播，新时间严格晚于原值）。以前每次拖拽把传入的
/// 所有行都刷成"现在"，在记录级 LWW 下会压掉其它设备 / AI 刚对这些待办做的修改（A7）。
/// 调用方提供事务。
fn apply_order(conn: &Connection, table: OrderTable, ids: &[i64]) -> rusqlite::Result<usize> {
    let table_name = match table {
        OrderTable::Todos => "todos",
        OrderTable::Subtasks => "subtasks",
    };
    let sql = format!(
        "UPDATE {table_name} SET sort_order = ?1, {SQL_SET_UPDATED_AT}
         WHERE id = ?2 AND sort_order IS NOT ?1"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut changed = 0;
    for (index, id) in ids.iter().enumerate() {
        changed += stmt.execute(rusqlite::params![index as i64, id])?;
    }
    Ok(changed)
}

// 子任务操作
#[tauri::command]
pub fn create_subtask(db: State<Database>, data: CreateSubTaskRequest) -> Result<SubTask, String> {
    db.with_transaction(|tx| -> rusqlite::Result<SubTask> {
        let max_order: i32 = tx.query_row(
            "SELECT COALESCE(MAX(sort_order), -1) FROM subtasks WHERE parent_id = ?1",
            [data.parent_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO subtasks (parent_id, title, content, sort_order) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![data.parent_id, data.title, data.content, max_order + 1],
        )?;
        load_subtask(tx, tx.last_insert_rowid())
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_subtask(
    db: State<Database>,
    id: i64,
    data: UpdateSubTaskRequest,
) -> Result<SubTask, String> {
    update_subtask_inner(&db, id, &data)
}

fn update_subtask_inner(
    db: &Database,
    id: i64,
    data: &UpdateSubTaskRequest,
) -> Result<SubTask, String> {
    db.with_connection(|conn| {
        let mut updates = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref title) = data.title {
            updates.push("title = ?");
            params.push(Box::new(title.clone()));
        }
        if let Some(ref content) = data.content {
            updates.push("content = ?");
            params.push(Box::new(content.clone()));
        }
        if let Some(completed) = data.completed {
            updates.push("completed = ?");
            params.push(Box::new(if completed { 1 } else { 0 }));
        }
        if let Some(sort_order) = data.sort_order {
            updates.push("sort_order = ?");
            params.push(Box::new(sort_order));
        }

        if updates.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "No fields to update".to_string(),
            ));
        }

        updates.push(SQL_SET_UPDATED_AT);

        let sql = format!("UPDATE subtasks SET {} WHERE id = ?", updates.join(", "));
        params.push(Box::new(id));

        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        conn.execute(&sql, params_refs.as_slice())?;

        load_subtask(conn, id)
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_subtask(db: State<Database>, id: i64) -> Result<SubTask, String> {
    db.with_connection(|conn| load_subtask(conn, id))
        .map_err(|e| e.to_string())
}

/// 删除子任务并写墓碑
#[tauri::command]
pub fn delete_subtask(db: State<Database>, id: i64) -> Result<(), String> {
    let now = now_local();
    db.with_transaction(|tx| sync_store::delete_subtask_with_tombstone(tx, id, &now).map(|_| ()))
        .map_err(|e: rusqlite::Error| e.to_string())
}

// ============================================================================
// 图片
// ============================================================================

/// 单张图片上限（K5）
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// 编辑器可上传的图片扩展名（K5，存小写）
const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "gif", "bmp"];

#[tauri::command]
pub fn get_images_dir() -> Result<String, String> {
    let dir = paths::images_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    dir.to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "Invalid path".to_string())
}

/// 保存编辑器里粘贴 / 拖入的图片（K7）。
///
/// 请求体是图片原始字节（前端 `invoke('save_subtask_image', bytes, { headers: { 'x-image-ext': 'png' } })`，
/// 不再走 base64 + JSON；IPC 退回 postMessage 时的数字数组同样接受，见 [`image_bytes`]）；
/// 扩展名必须在白名单内，单张不超过 20MB。文件名由后端生成
/// `<毫秒>_<6 位随机>.<ext>` 并经 K5 安全文件名校验——以前文件名由前端给、直接 `join`，
/// 可以写到图片目录之外（B2）。返回图片的绝对路径。
#[tauri::command]
pub async fn save_subtask_image(request: tauri::ipc::Request<'_>) -> Result<String, String> {
    let ext = image_extension(
        request
            .headers()
            .get("x-image-ext")
            .and_then(|v| v.to_str().ok()),
    )?;
    let bytes = image_bytes(request.body(), MAX_IMAGE_BYTES)?;

    let path = tauri::async_runtime::spawn_blocking(move || {
        store_image(&paths::images_dir(), ext, &bytes)
    })
    .await
    .map_err(|e| format!("保存图片异常: {}", e))??;
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| "图片路径包含非法字符".to_string())
}

/// 校验并规范化 `x-image-ext`：大小写不敏感、允许带前导点，返回小写扩展名
fn image_extension(header: Option<&str>) -> Result<&'static str, String> {
    let raw = header.map(str::trim).unwrap_or("");
    let lower = raw.trim_start_matches('.').to_ascii_lowercase();
    IMAGE_EXTENSIONS
        .iter()
        .find(|ext| **ext == lower)
        .copied()
        .ok_or_else(|| {
            format!(
                "不支持的图片格式：{:?}（支持 png / jpg / jpeg / webp / gif / bmp）",
                raw
            )
        })
}

fn check_image_size(len: usize, max_bytes: usize) -> Result<(), String> {
    if len == 0 {
        Err("图片内容为空".to_string())
    } else if len > max_bytes {
        Err(format!("图片超过 {}MB，未保存", max_bytes / 1024 / 1024))
    } else {
        Ok(())
    }
}

/// 取出请求体里的图片字节（先查长度上限，再拷贝 / 转换）。
///
/// 前端正常走自定义协议 IPC，请求体是原始字节（`InvokeBody::Raw`）。自定义协议被拦截（CSP、
/// WebView 限制）时 Tauri 退回 postMessage，`Uint8Array` 会被序列化成数字数组，到这里是
/// `InvokeBody::Json(Array)`：同样接受，但每一项都必须是 0–255 的整数，否则整体报错（不截断、
/// 不猜测）。请求头（`x-image-ext`）两条通路都会带上，扩展名规则不变。
fn image_bytes(body: &tauri::ipc::InvokeBody, max_bytes: usize) -> Result<Vec<u8>, String> {
    match body {
        tauri::ipc::InvokeBody::Raw(bytes) => {
            check_image_size(bytes.len(), max_bytes)?;
            Ok(bytes.clone())
        }
        tauri::ipc::InvokeBody::Json(serde_json::Value::Array(items)) => {
            check_image_size(items.len(), max_bytes)?;
            items
                .iter()
                .map(|v| v.as_u64().and_then(|n| u8::try_from(n).ok()))
                .collect::<Option<Vec<u8>>>()
                .ok_or_else(|| "图片数据格式错误：字节数组只能包含 0–255 的整数".to_string())
        }
        _ => Err("图片数据必须是原始字节或字节数组".to_string()),
    }
}

/// 6 位小写字母 / 数字的随机串（只求不撞名，不用于安全场景）
fn random_suffix() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

    // RandomState 每个实例的种子都不同（进程级随机 + 递增），再混入计数与纳秒
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default(),
    );
    let mut n = hasher.finish();
    (0..6)
        .map(|_| {
            let c = ALPHABET[(n % ALPHABET.len() as u64) as usize] as char;
            n /= ALPHABET.len() as u64;
            c
        })
        .collect()
}

fn generate_image_name(ext: &str) -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    format!("{}_{}.{}", millis, random_suffix(), ext)
}

/// 把图片写进 `dir`：先写同目录下以 `.` 开头的临时文件（不满足安全图片名，同步不会把半截文件
/// 当成图片上传），再改名为生成的文件名；撞名时换一个随机名重试。
fn store_image(dir: &Path, ext: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建图片目录失败: {}", e))?;
    for _ in 0..8 {
        let name = generate_image_name(ext);
        if !is_safe_image_name(&name) {
            return Err(format!("生成的图片文件名不合法: {}", name));
        }
        let target = dir.join(&name);
        if target.exists() {
            continue;
        }
        let temp = dir.join(format!(".{}.part", name));
        std::fs::write(&temp, bytes).map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            format!("写入图片失败: {}", e)
        })?;
        std::fs::rename(&temp, &target).map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            format!("写入图片失败: {}", e)
        })?;
        return Ok(target);
    }
    Err("生成图片文件名失败，请重试".to_string())
}

// ============================================================================
// 从文件导入子任务
// ============================================================================

/// 可导入的文本扩展名
const IMPORT_EXTENSIONS: [&str; 2] = ["md", "txt"];
/// 文件夹递归深度上限（所选文件夹本身为第 0 层）
const MAX_IMPORT_DEPTH: usize = 5;
/// 一次最多导入的文件数：误选了用户主目录这类大目录时及早报错，而不是建出成千上万条子任务
const MAX_IMPORT_FILES: usize = 1000;
/// 单个文件上限：子任务内容要进编辑器渲染，超大文本会卡死界面
const MAX_IMPORT_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// 从 .md / .txt 文件（或文件夹，递归）导入子任务。
///
/// 读文件在数据库锁之外进行；任一文件读不出来（权限、编码不是 UTF-8 / 带 BOM 的 UTF-16 / GBK、
/// 过大）就整体报错并列出文件名，一条都不导入——以前会静默建出内容为空的子任务。
/// 全部读取成功后在一个事务里插入。
#[tauri::command]
pub async fn import_subtasks_from_paths(
    app: AppHandle,
    parent_id: i64,
    paths: Vec<String>,
) -> Result<Vec<SubTask>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        import_subtasks_inner(&app.state::<Database>(), parent_id, &paths)
    })
    .await
    .map_err(|e| format!("导入任务异常: {}", e))?
}

fn import_subtasks_inner(
    db: &Database,
    parent_id: i64,
    paths: &[String],
) -> Result<Vec<SubTask>, String> {
    let files = collect_import_files(paths)?;
    let items = read_import_files(&files)?;
    db.with_transaction(|tx| insert_imported_subtasks(tx, parent_id, &items))
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => "待办不存在".to_string(),
            e => e.to_string(),
        })
}

fn has_import_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| IMPORT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// 展开用户选择的路径：文件直接收（符号链接跟随——是用户亲手选的），
/// 文件夹递归收集，递归时跳过符号链接（防环）且不超过 [`MAX_IMPORT_DEPTH`] 层。
fn collect_import_files(paths: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for p in paths {
        let path = Path::new(p);
        if path.is_dir() {
            collect_dir(path, 0, &mut files)?;
        } else if path.is_file() && has_import_extension(path) {
            files.push(path.to_path_buf());
        }
        if files.len() > MAX_IMPORT_FILES {
            return Err(too_many_files());
        }
    }
    if files.is_empty() {
        return Err("未找到 .md 或 .txt 文件".to_string());
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn too_many_files() -> String {
    format!(
        "文件过多（超过 {} 个），请选择更小的文件夹",
        MAX_IMPORT_FILES
    )
}

fn collect_dir(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("[import] 无法读取文件夹 {}: {}", dir.display(), e);
            return Ok(());
        }
    };
    for entry in entries.flatten() {
        // DirEntry::file_type 不跟随符号链接
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if depth < MAX_IMPORT_DEPTH {
                collect_dir(&path, depth + 1, out)?;
            }
        } else if file_type.is_file() && has_import_extension(&path) {
            out.push(path);
            if out.len() > MAX_IMPORT_FILES {
                return Err(too_many_files());
            }
        }
    }
    Ok(())
}

/// 按 UTF-8 → 带 BOM 的 UTF-8 / UTF-16LE / UTF-16BE → GBK 的顺序解码，都不合法返回 `None`
fn decode_text(bytes: &[u8]) -> Option<String> {
    if let Some((encoding, bom_len)) = encoding_rs::Encoding::for_bom(bytes) {
        return encoding
            .decode_without_bom_handling_and_without_replacement(&bytes[bom_len..])
            .map(|text| text.into_owned());
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Some(text.to_string());
    }
    encoding_rs::GBK
        .decode_without_bom_handling_and_without_replacement(bytes)
        .map(|text| text.into_owned())
}

/// 读取全部文件为 (标题, 内容)；有读不出来的文件时返回列出文件名的错误
fn read_import_files(files: &[PathBuf]) -> Result<Vec<(String, String)>, String> {
    let mut items = Vec::with_capacity(files.len());
    let mut failures: Vec<String> = Vec::new();
    for file in files {
        let display_name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.display().to_string());
        let title = file
            .file_stem()
            .map(|s| s.to_string_lossy().trim().to_string())
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        let content = match std::fs::metadata(file) {
            Ok(meta) if meta.len() > MAX_IMPORT_FILE_BYTES => {
                Err(format!("超过 {}MB", MAX_IMPORT_FILE_BYTES / 1024 / 1024))
            }
            Ok(_) => std::fs::read(file)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    decode_text(&bytes).ok_or_else(|| "不是 UTF-8 / UTF-16 / GBK 文本".to_string())
                }),
            Err(e) => Err(e.to_string()),
        };
        match content {
            Ok(content) => items.push((title, content)),
            Err(reason) => failures.push(format!("{}（{}）", display_name, reason)),
        }
    }
    if failures.is_empty() {
        return Ok(items);
    }
    const SHOWN: usize = 10;
    let mut message = format!(
        "以下 {} 个文件无法读取，已取消导入：{}",
        failures.len(),
        failures
            .iter()
            .take(SHOWN)
            .cloned()
            .collect::<Vec<_>>()
            .join("、")
    );
    if failures.len() > SHOWN {
        message.push_str(" 等");
    }
    Err(message)
}

fn insert_imported_subtasks(
    conn: &Connection,
    parent_id: i64,
    items: &[(String, String)],
) -> rusqlite::Result<Vec<SubTask>> {
    let mut max_order: i32 = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM todos WHERE id = ?1),
                (SELECT COALESCE(MAX(sort_order), -1) FROM subtasks WHERE parent_id = ?1)",
        [parent_id],
        |row| {
            if row.get::<_, i64>(0)? == 0 {
                Err(rusqlite::Error::QueryReturnedNoRows)
            } else {
                row.get(1)
            }
        },
    )?;
    let mut created = Vec::with_capacity(items.len());
    for (title, content) in items {
        max_order += 1;
        conn.execute(
            "INSERT INTO subtasks (parent_id, title, content, sort_order) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![parent_id, title, content, max_order],
        )?;
        created.push(load_subtask(conn, conn.last_insert_rowid())?);
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Database {
        Database::new_in_memory().expect("内存库")
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "mini-todo-todo-test-{}-{}-{}",
            tag,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn create_request(title: &str) -> CreateTodoRequest {
        CreateTodoRequest {
            title: title.to_string(),
            description: None,
            color: "#EF4444".to_string(),
            quadrant: 1,
            notify_at: None,
            notify_before: None,
            start_time: None,
            end_time: None,
        }
    }

    fn update_request() -> UpdateTodoRequest {
        serde_json::from_value(serde_json::json!({})).unwrap()
    }

    fn seq(db: &Database) -> i64 {
        db.with_connection(sync_store::local_seq).unwrap()
    }

    // ---- 时间字段规范化 ----

    #[test]
    fn create_and_update_store_canonical_times() {
        let db = db();
        let mut req = create_request("a");
        req.notify_at = Some("2026-05-01T09:30".to_string());
        req.start_time = Some("2026-05-01".to_string());
        req.end_time = Some("2026-05-02".to_string());
        let todo = create_todo_inner(&db, &req).unwrap();
        assert_eq!(todo.notify_at.as_deref(), Some("2026-05-01 09:30:00"));
        assert_eq!(todo.start_time.as_deref(), Some("2026-05-01 00:00:00"));
        assert_eq!(todo.end_time.as_deref(), Some("2026-05-02 23:59:00"));

        let mut upd = update_request();
        upd.notify_at = Some("2026-06-01".to_string());
        upd.end_time = Some("2026-06-03T18:00:00".to_string());
        let todo = update_todo_inner(&db, todo.id, &upd).unwrap();
        assert_eq!(todo.notify_at.as_deref(), Some("2026-06-01 09:00:00"));
        assert_eq!(todo.end_time.as_deref(), Some("2026-06-03 18:00:00"));
        assert_eq!(todo.start_time.as_deref(), Some("2026-05-01 00:00:00"));
    }

    #[test]
    fn unparsable_times_are_rejected_and_empty_means_unset() {
        let db = db();
        let mut req = create_request("a");
        req.notify_at = Some("下周二".to_string());
        let err = create_todo_inner(&db, &req).unwrap_err();
        assert!(err.contains("提醒时间"), "{err}");
        assert!(db
            .with_connection(
                |c| c.query_row("SELECT COUNT(*) FROM todos", [], |r| r.get::<_, i64>(0))
            )
            .map(|n| n == 0)
            .unwrap());

        req.notify_at = Some("  ".to_string());
        let todo = create_todo_inner(&db, &req).unwrap();
        assert_eq!(todo.notify_at, None);

        let mut upd = update_request();
        upd.start_time = Some("2026-13-01".to_string());
        assert!(update_todo_inner(&db, todo.id, &upd)
            .unwrap_err()
            .contains("开始时间"));
        assert_eq!(
            update_todo_inner(&db, 999, &{
                let mut u = update_request();
                u.title = Some("x".to_string());
                u
            })
            .unwrap_err(),
            "待办不存在"
        );
    }

    // ---- 单条读取 / 变更计数 ----

    #[test]
    fn load_todo_includes_ordered_subtasks_and_missing_is_none() {
        let db = db();
        let todo = create_todo_inner(&db, &create_request("a")).unwrap();
        db.with_connection(|c| {
            c.execute_batch(&format!(
                "INSERT INTO subtasks (id, parent_id, title, sort_order) VALUES
                 (11, {id}, 's1', 1), (12, {id}, 's0', 0);",
                id = todo.id
            ))
        })
        .unwrap();
        let loaded = db
            .with_connection(|c| load_todo(c, todo.id))
            .unwrap()
            .unwrap();
        let ids: Vec<i64> = loaded.subtasks.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![12, 11]);
        assert!(db.with_connection(|c| load_todo(c, 999)).unwrap().is_none());

        let all = db
            .with_connection(|c| sync_store::load_todos_with_subtasks(c, TODO_LIST_ORDER))
            .unwrap();
        assert_eq!(all[0].subtasks, loaded.subtasks);
    }

    #[test]
    fn change_seq_moves_on_every_write() {
        let db = db();
        let s0 = seq(&db);
        let todo = create_todo_inner(&db, &create_request("a")).unwrap();
        let s1 = seq(&db);
        assert!(s1 > s0);
        let mut upd = update_request();
        upd.completed = Some(true);
        update_todo_inner(&db, todo.id, &upd).unwrap();
        assert!(seq(&db) > s1);
    }

    // ---- updated_at 严格递增（记录级 LWW 的前提）----

    fn updated_at_of(db: &Database, table: &str, id: i64) -> String {
        db.with_connection(|c| {
            c.query_row(
                &format!("SELECT updated_at FROM {table} WHERE id = ?1"),
                [id],
                |r| r.get(0),
            )
        })
        .unwrap()
    }

    /// 回归（e2e 实测丢更新）：同一条记录在同一秒内改两次，以前两次的 updated_at 相同；
    /// 另一端在两次之间同步过第一版的话，平局保留本地，第二版永远传不过去。
    #[test]
    fn two_updates_in_the_same_second_get_strictly_increasing_updated_at() {
        let db = db();
        let todo = create_todo_inner(&db, &create_request("a")).unwrap();
        let mut stamps = vec![updated_at_of(&db, "todos", todo.id)];
        for title in ["b", "c", "d"] {
            let mut upd = update_request();
            upd.title = Some(title.to_string());
            update_todo_inner(&db, todo.id, &upd).unwrap();
            stamps.push(updated_at_of(&db, "todos", todo.id));
        }
        for pair in stamps.windows(2) {
            assert!(pair[1] > pair[0], "{stamps:?}");
        }
    }

    /// 同步来的版本带着比本机时钟还新的时间戳（对端时钟偏快 / cloud timezone 配错）时，
    /// 本机的修改仍要晚于它，否则下次同步就被它要取代的旧版本覆盖
    #[test]
    fn edits_on_future_dated_records_still_win() {
        let db = db();
        db.with_connection(|c| {
            c.execute_batch(
                "INSERT INTO todos (id, title, sort_order, updated_at) VALUES
                 (1, 'a', 0, '2099-01-01 08:00:00'), (2, 'b', 1, '2099-01-01 08:00:00');
                 INSERT INTO subtasks (id, parent_id, title, sort_order, updated_at) VALUES
                 (11, 1, 's', 0, '2099-01-01 08:00:00'), (12, 1, 't', 1, 'not a time');",
            )
        })
        .unwrap();

        let mut upd = update_request();
        upd.completed = Some(true);
        update_todo_inner(&db, 1, &upd).unwrap();
        assert_eq!(updated_at_of(&db, "todos", 1), "2099-01-01 08:00:01");

        db.with_transaction(|tx| apply_order(tx, OrderTable::Todos, &[2, 1]))
            .unwrap();
        assert_eq!(updated_at_of(&db, "todos", 1), "2099-01-01 08:00:02");
        assert_eq!(updated_at_of(&db, "todos", 2), "2099-01-01 08:00:01");

        let sub = update_subtask_inner(
            &db,
            11,
            &UpdateSubTaskRequest {
                title: None,
                content: Some("x".to_string()),
                completed: None,
                sort_order: None,
            },
        )
        .unwrap();
        assert_eq!(sub.updated_at, "2099-01-01 08:00:01");

        // 原值是乱码：按"现在"写，不是 NULL（MAX 遇 NULL 会撞 NOT NULL）
        let before = now_local();
        let sub = update_subtask_inner(
            &db,
            12,
            &UpdateSubTaskRequest {
                title: None,
                content: None,
                completed: Some(true),
                sort_order: None,
            },
        )
        .unwrap();
        assert!(sub.updated_at >= before && sub.updated_at.len() == 19);
    }

    // ---- 排序 ----

    #[test]
    fn reorder_only_touches_rows_whose_order_changes() {
        let db = db();
        db.with_connection(|c| {
            c.execute_batch(
                "INSERT INTO todos (id, title, sort_order, updated_at) VALUES
                 (10, 'a', 0, '2026-01-01 00:00:00'),
                 (11, 'b', 1, '2026-01-01 00:00:00'),
                 (12, 'c', 2, '2026-01-01 00:00:00');",
            )
        })
        .unwrap();
        let before = seq(&db);

        let changed = db
            .with_transaction(|tx| apply_order(tx, OrderTable::Todos, &[11, 10, 12]))
            .unwrap();
        assert_eq!(changed, 2);
        assert_eq!(seq(&db), before + 2, "只有变化的两行计入本地变更");

        let rows: Vec<(i64, i32, String)> = db
            .with_connection(|c| {
                let mut stmt =
                    c.prepare("SELECT id, sort_order, updated_at FROM todos ORDER BY id")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                rows.collect()
            })
            .unwrap();
        assert_eq!((rows[0].0, rows[0].1), (10, 1));
        assert_eq!((rows[1].0, rows[1].1), (11, 0));
        assert!(rows[0].2.as_str() > "2026-01-01 00:00:00");
        assert!(rows[1].2.as_str() > "2026-01-01 00:00:00");
        assert_eq!(
            rows[2].2, "2026-01-01 00:00:00",
            "位置没变的行不刷新 updated_at"
        );

        // 再次提交同样的顺序：什么都不改
        let changed = db
            .with_transaction(|tx| apply_order(tx, OrderTable::Todos, &[11, 10, 12]))
            .unwrap();
        assert_eq!(changed, 0);
    }

    #[test]
    fn reorder_subtasks_uses_the_same_rule() {
        let db = db();
        let todo = create_todo_inner(&db, &create_request("a")).unwrap();
        db.with_connection(|c| {
            c.execute_batch(&format!(
                "INSERT INTO subtasks (id, parent_id, title, sort_order) VALUES
                 (21, {id}, 'x', 0), (22, {id}, 'y', 1);",
                id = todo.id
            ))
        })
        .unwrap();
        let changed = db
            .with_transaction(|tx| apply_order(tx, OrderTable::Subtasks, &[22, 21]))
            .unwrap();
        assert_eq!(changed, 2);
        let loaded = db
            .with_connection(|c| load_todo(c, todo.id))
            .unwrap()
            .unwrap();
        let ids: Vec<i64> = loaded.subtasks.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![22, 21]);
    }

    // ---- 图片 ----

    #[test]
    fn image_extension_whitelist_is_case_insensitive() {
        assert_eq!(image_extension(Some("PNG")), Ok("png"));
        assert_eq!(image_extension(Some(".Jpeg")), Ok("jpeg"));
        assert_eq!(image_extension(Some(" webp ")), Ok("webp"));
        for bad in [None, Some(""), Some("svg"), Some("png/../x"), Some("exe")] {
            assert!(image_extension(bad).is_err(), "{bad:?}");
        }
        assert!(check_image_size(0, MAX_IMAGE_BYTES).is_err());
        assert!(check_image_size(MAX_IMAGE_BYTES, MAX_IMAGE_BYTES).is_ok());
        assert!(check_image_size(MAX_IMAGE_BYTES + 1, MAX_IMAGE_BYTES).is_err());
    }

    /// 原始字节（自定义协议 IPC）与数字数组（退回 postMessage 时 `Uint8Array` 的序列化形式）
    /// 都接受；数组里只要有一项不是 0–255 的整数就整体拒绝；大小上限对两种形式都生效
    #[test]
    fn image_bytes_accept_raw_bodies_and_postmessage_byte_arrays() {
        use serde_json::json;
        use tauri::ipc::InvokeBody;

        assert_eq!(
            image_bytes(&InvokeBody::Raw(vec![1, 2, 3]), 8),
            Ok(vec![1, 2, 3])
        );
        assert_eq!(
            image_bytes(&InvokeBody::Json(json!([0, 127, 255])), 8),
            Ok(vec![0, 127, 255])
        );

        for bad in [
            json!([256]),
            json!([-1]),
            json!([1.5]),
            json!([1.0]),
            json!(["1"]),
            json!([null]),
            json!([[1]]),
            json!([1, 2, 300]),
        ] {
            let err = image_bytes(&InvokeBody::Json(bad.clone()), 8).unwrap_err();
            assert!(err.contains("0–255"), "{bad}: {err}");
        }

        // 空、超限：两种形式一样处理
        assert!(image_bytes(&InvokeBody::Raw(Vec::new()), 8).is_err());
        assert!(image_bytes(&InvokeBody::Json(json!([])), 8).is_err());
        assert!(image_bytes(&InvokeBody::Raw(vec![0; 9]), 8)
            .unwrap_err()
            .contains("超过"));
        assert!(
            image_bytes(&InvokeBody::Json(json!([0, 0, 0, 0, 0, 0, 0, 0, 0])), 8)
                .unwrap_err()
                .contains("超过")
        );
        assert!(image_bytes(&InvokeBody::Raw(vec![0; 8]), 8).is_ok());

        // 其它 JSON（对象 / base64 串 / null）不是图片数据
        for other in [json!({"0": 1}), json!("AAEC"), json!(null)] {
            assert!(
                image_bytes(&InvokeBody::Json(other.clone()), 8).is_err(),
                "{other}"
            );
        }
    }

    #[test]
    fn stored_images_get_safe_unique_names_without_temp_leftovers() {
        let dir = temp_dir("images");
        let a = store_image(&dir, "png", b"one").unwrap();
        let b = store_image(&dir, "png", b"two").unwrap();
        assert_ne!(a, b);
        for path in [&a, &b] {
            let name = path.file_name().unwrap().to_str().unwrap();
            assert!(is_safe_image_name(name), "{name}");
            assert!(name.ends_with(".png"));
            let (millis, rest) = name.split_once('_').unwrap();
            assert!(millis.bytes().all(|c| c.is_ascii_digit()));
            assert_eq!(rest.len(), "abcdef.png".len());
            assert_eq!(path.parent().unwrap(), dir.as_path());
        }
        assert_eq!(std::fs::read(&a).unwrap(), b"one");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- 导入子任务 ----

    #[test]
    fn decode_text_falls_back_through_encodings() {
        assert_eq!(
            decode_text("中文 md".as_bytes()).as_deref(),
            Some("中文 md")
        );
        let mut bom8 = vec![0xEF, 0xBB, 0xBF];
        bom8.extend_from_slice("带 BOM".as_bytes());
        assert_eq!(decode_text(&bom8).as_deref(), Some("带 BOM"));

        let mut le = vec![0xFF, 0xFE];
        le.extend("宽字符".encode_utf16().flat_map(|u| u.to_le_bytes()));
        assert_eq!(decode_text(&le).as_deref(), Some("宽字符"));
        let mut be = vec![0xFE, 0xFF];
        be.extend("宽字符".encode_utf16().flat_map(|u| u.to_be_bytes()));
        assert_eq!(decode_text(&be).as_deref(), Some("宽字符"));

        let (gbk, _, _) = encoding_rs::GBK.encode("记事本保存的中文");
        assert!(std::str::from_utf8(&gbk).is_err(), "前提：不是合法 UTF-8");
        assert_eq!(decode_text(&gbk).as_deref(), Some("记事本保存的中文"));

        assert_eq!(decode_text(&[0xFF, 0xFF, 0xFF]), None);
        assert_eq!(decode_text(b"").as_deref(), Some(""));
    }

    #[test]
    fn import_reads_nested_files_in_one_transaction() {
        let db = db();
        let parent = create_todo_inner(&db, &create_request("p")).unwrap();
        let dir = temp_dir("import");
        std::fs::write(dir.join("b.md"), "# B").unwrap();
        std::fs::write(dir.join("a.TXT"), encoding_rs::GBK.encode("甲").0).unwrap();
        std::fs::write(dir.join("skip.pdf"), "x").unwrap();
        let mut deep = dir.clone();
        for level in 1..=7 {
            deep = deep.join(format!("d{level}"));
            std::fs::create_dir_all(&deep).unwrap();
            std::fs::write(deep.join(format!("level{level}.md")), "x").unwrap();
        }

        let created =
            import_subtasks_inner(&db, parent.id, &[dir.to_string_lossy().into_owned()]).unwrap();
        let titles: Vec<&str> = created.iter().map(|s| s.title.as_str()).collect();
        // 按路径排序（与以前一致）；所选文件夹为第 0 层，最多递归到第 5 层
        assert_eq!(
            titles,
            vec!["a", "b", "level5", "level4", "level3", "level2", "level1"]
        );
        assert_eq!(created[0].content.as_deref(), Some("甲"));
        let orders: Vec<i32> = created.iter().map(|s| s.sort_order).collect();
        assert_eq!(orders, (0..7).collect::<Vec<_>>());

        // 单个文件也行；父待办不存在时报错
        let single = dir.join("b.md").to_string_lossy().into_owned();
        assert_eq!(
            import_subtasks_inner(&db, parent.id, std::slice::from_ref(&single)).unwrap()[0]
                .sort_order,
            7
        );
        assert_eq!(
            import_subtasks_inner(&db, 999, &[single]).unwrap_err(),
            "待办不存在"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_files_abort_the_whole_import_with_their_names() {
        let db = db();
        let parent = create_todo_inner(&db, &create_request("p")).unwrap();
        let dir = temp_dir("import-bad");
        std::fs::write(dir.join("good.md"), "ok").unwrap();
        std::fs::write(dir.join("binary.txt"), [0xFF, 0xFF, 0xFF, 0xFE]).unwrap();

        let err = import_subtasks_inner(&db, parent.id, &[dir.to_string_lossy().into_owned()])
            .unwrap_err();
        assert!(err.contains("binary.txt"), "{err}");
        assert!(!err.contains("good.md"), "{err}");
        let count: i64 = db
            .with_connection(|c| c.query_row("SELECT COUNT(*) FROM subtasks", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(count, 0, "整体取消，不留半截导入");

        let empty = temp_dir("import-empty");
        assert_eq!(
            import_subtasks_inner(&db, parent.id, &[empty.to_string_lossy().into_owned()])
                .unwrap_err(),
            "未找到 .md 或 .txt 文件"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_folders_are_not_followed_during_recursion() {
        let dir = temp_dir("import-link");
        std::fs::write(dir.join("a.md"), "a").unwrap();
        // 指回自身的符号链接：跟随的话会无限递归
        std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();
        std::os::unix::fs::symlink(dir.join("a.md"), dir.join("link.md")).unwrap();

        let files = collect_import_files(&[dir.to_string_lossy().into_owned()]).unwrap();
        assert_eq!(files, vec![dir.join("a.md")]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
