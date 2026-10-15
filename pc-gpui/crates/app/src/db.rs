//! 数据库全局句柄与"后台执行阻塞的 core 调用"帮助函数。
//!
//! core 的函数全部阻塞（SQLite / 文件 / 网络），一律放 `background_executor` 执行，结果回前台。
//!
//! ```ignore
//! let task = db::spawn_db(cx, move |db| mini_todo_core::todo::get(db, id));
//! cx.spawn(async move |this, cx| {
//!     let todo = task.await?;               // Result<Todo, String>
//!     this.update(cx, |this, cx| { this.todo = Some(todo); cx.notify(); })
//! }).detach();
//! ```

use std::sync::Arc;

use gpui_kit::{App, Global, Task};
use mini_todo_core::Database;

/// 进程内唯一的数据库（core 的 `Database` 内部是 `Mutex<Connection>`）
#[derive(Clone)]
pub struct Db(pub Arc<Database>);

impl Global for Db {}

impl Db {
    pub fn init(cx: &mut App, database: Arc<Database>) {
        cx.set_global(Db(database));
    }

    /// 取数据库（`Arc` 克隆，可以带进后台任务）
    pub fn get(cx: &App) -> Arc<Database> {
        cx.global::<Db>().0.clone()
    }
}

/// 在后台线程池执行阻塞的 core 调用，返回前台可 `await` 的任务。
///
/// `Task` 被 drop 即取消（后台闭包已开始执行时会跑完，但结果被丢弃）；不关心结果时 `.detach()`。
pub fn spawn_db<T, F>(cx: &App, f: F) -> Task<Result<T, String>>
where
    T: Send + 'static,
    F: FnOnce(&Database) -> Result<T, String> + Send + 'static,
{
    let db = Db::get(cx);
    cx.background_executor().spawn(async move { f(&db) })
}

/// 同 [`spawn_db`]，但闭包拿到 `Arc<Database>`（需要再传给 core 的 `watch` 之类函数时用）
pub fn spawn_db_arc<T, F>(cx: &App, f: F) -> Task<Result<T, String>>
where
    T: Send + 'static,
    F: FnOnce(Arc<Database>) -> Result<T, String> + Send + 'static,
{
    let db = Db::get(cx);
    cx.background_executor().spawn(async move { f(db) })
}
