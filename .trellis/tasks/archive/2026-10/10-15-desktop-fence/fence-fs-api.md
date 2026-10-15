# 收纳箱文件层 API（`services::fence_fs`）

> 与收尾后的代码一致（2026-10-15）。代码：`pc/src-tauri/src/services/fence_fs/`（`mod.rs` 汇总重新导出）。
> 跨层契约（窗口、数据库、命令、事件、对账与登记规则）见 `.trellis/spec/backend/desktop-fences.md`，本文件只做文件层逐函数说明。
> 图标 spike 结论见 `research/spike-icons.md`；spike 测试代码已移出 crate，存档在 `research/spike-icons-tests.rs`。

## 0. 总览

`mod.rs` 里的子模块都是私有的（`mod folder;` 等）。**只有重新导出的项**能在 `fence_fs` 之外（即 `commands::fence` /
`commands::fence_items`）使用；其余 `pub` 项只在 `fence_fs` 内部可见（主要供本模块与单测使用），集成层需要时再加一行 `pub use`。

| 分组 | 重新导出（`fence_fs::…`） | 仅模块内部 |
|---|---|---|
| 路径 | `fence_root_dir` `icon_cache_dir` `user_desktop_dir` | `fence_root_path` `public_desktop_dir` `FENCE_ROOT_DIR_NAME` `FENCES_ROOT_ENV` |
| 路径比较 | `path_starts_with_ci` `same_path_ci` | `normalize_for_compare`（`pub(crate)`） |
| 命名 | `is_valid_item_name` | `sanitize_folder_name` `dedupe_name` `is_valid_folder_name` `unique_file_name` `suffixed_file_name` `split_extension` `restore_file_name` `display_name` `is_shortcut_name` `is_shell_metadata_file` `DEFAULT_FOLDER_NAME` `MAX_FOLDER_NAME_CHARS` |
| 文件夹 | `create_fence_folder` `ensure_fence_folder` `rename_fence_folder` `remove_fence_folder_if_empty` `fence_folder_path` `list_fence_items` `list_fence_folders` `FenceEntry` | 各 `*_at(root, …)` 版本（单测在临时目录里调用） |
| 移动 | `move_into_fence` `restore_item` `move_between_fences` `restore_all` `ItemOpResult` `ItemOpStatus` `RestoreAllResult` | `choose_restore_dir` `plan_move_in` `plan_restore` `classify` |
| 判定 | `desktop_is_onedrive_redirected` | `is_under_onedrive` |
| 打开 | `launch_item` `reveal_in_explorer` | |
| 图标 | `icon_png` `prune_icon_cache` | `icon_bucket` `icon_cache_key` `path_hash` `ICON_SIZE_BUCKETS` |
| 监听 | `watch_fence_root` `FenceWatcher` `FenceChange` | `WATCH_DEBOUNCE` |

调用方式：`use crate::services::fence_fs;` 然后 `fence_fs::move_into_fence(...)`。

**已删除 / 已变更（不要再引用）**：

- `invalidate_icon`（"刷新图标"没有入口，已删除；缓存键含 mtime 与大小，文件变化后自动换键，旧文件由 `prune_icon_cache` 清理）
- `is_in_public_desktop`（原预留给"拖入前提示需要管理员确认"，未使用，已删除；公共桌面项由系统管理员权限对话框 / UAC 提示，取消后按逐项结果提示）
- PR0 spike 测试模块（原 `icon.rs` 末尾的 `mod spike`）→ `research/spike-icons-tests.rs`，不在 crate 内编译
- 集成前的 `#![allow(dead_code)]`、`#[allow(unused_imports)]` 与 `// INTEGRATION:` 标记已移除；`mod.rs` 只重新导出集成层用到的项
- `FENCES_ROOT_ENV` 不再经 `fence_fs::` 重新导出（`location::FENCES_ROOT_ENV`，值仍为 `"MINI_TODO_FENCES_ROOT"`）
- `move_into_fence` 新增 **`protected`** 拒绝条件（见 3.3）

## 1. 通用约定

### 1.1 错误

所有可能失败的函数返回 `Result<_, String>`，错误信息是给用户看的中文。移动类函数的 `Err` **只**表示整次调用无法进行
（文件夹名不合法、根目录不可用、桌面路径未知、非 Windows），**逐项**成功 / 取消 / 失败放在 `ItemOpResult.status` 里。

### 1.2 线程（重要）

- 所有函数都可从任意线程调用；需要 COM STA 的操作在内部线程执行：
  - 移动 / 启动 / 打开所在位置：每次新开一个 STA 线程（`fence-move` / `fence-launch` / `fence-reveal`），互不排队
  - 图标：常驻 STA 线程 `fence-icon`，按提交顺序串行
- 函数都是**阻塞**的。集成层统一经 `commands::fence_items::blocking()`（`tauri::async_runtime::spawn_blocking` 包装）调用
- **绝不能在主线程**（同步 `#[tauri::command]`、`run_on_main_thread` 闭包、窗口事件回调、托盘事件回调）里调用**带 `owner_hwnd` 的移动函数**：
  系统对话框以该窗口为父窗口，会同步给主线程发消息（如 `EnableWindow` 的 `WM_ENABLE`），主线程在等结果时会死锁
- `owner_hwnd: Option<isize>`：收纳箱窗口的 HWND 数值，集成层用 `commands::fence_items::owner_hwnd(&window)`
  （`window.hwnd().ok().map(|h| h.0 as isize)`，非 Windows 恒 `None`）在 async 命令里取。传 `None` 时系统对话框没有父窗口（仍会弹出）

### 1.3 平台

| 能力 | Windows | 其他平台 |
|---|---|---|
| 命名、路径判定、文件夹管理、列举、监听 | ✓ | ✓（`public_desktop_dir` 为 `None`，OneDrive 判定按路径名） |
| 移动、启动、打开所在位置、图标 | ✓ | `Err("桌面收纳箱仅支持 Windows")`（`fence_fs::UNSUPPORTED`） |

### 1.4 开发专用：`MINI_TODO_FENCES_ROOT`

- **仅 debug 构建**（`cfg(debug_assertions)`）读取：去空白后非空时收纳根目录改为该目录（不存在由 `fence_root_dir` 创建）
- **release 构建完全忽略**：`location::fence_root_override` 的 `cfg(not(debug_assertions))` 分支恒为 `None`，有单测
  `release_build_ignores_override`（只在 `cargo test --release` 时编译执行）
- 图标缓存目录跟随 `db::paths::app_data_dir()`，所以 `MINI_TODO_DATA_DIR` 隔离对图标缓存同样生效
- 实测时两个变量都要设，指向会话临时目录

### 1.5 名字比较

文件 / 文件夹名一律**不区分大小写**比较（NTFS 语义）；去重时按目录实际列表（小写）判断，与文件系统本身是否区分大小写无关。

## 2. 数据类型（均 `Serialize`，字段 camelCase，可直接作为命令返回值）

```rust
pub struct FenceEntry {              // list_fence_items 的一项
    pub file_name: String,           // 收纳文件夹内的实际文件名（= fence_items.file_name）
    pub display_name: String,        // .lnk / .url 去掉后缀（去掉后为空则保留原名）；文件夹与其他文件原样
    pub path: String,                // 完整路径
    pub is_dir: bool,                // 文件夹（含指向文件夹的符号链接 / 联接点）
    pub is_shortcut: bool,           // .lnk / .url（文件夹恒 false）
    pub modified_ms: Option<i64>,    // 修改时间，Unix 毫秒
}

pub enum ItemOpStatus { Moved, Cancelled, Failed, Skipped }   // JSON: "moved" | "cancelled" | "failed" | "skipped"

pub struct ItemOpResult {
    pub source: String,              // 源完整路径（移入时 = 桌面原路径 → fence_items.original_path）
    pub status: ItemOpStatus,
    pub file_name: Option<String>,   // Moved 时：移动后的实际文件名（重名时带 " (2)" 后缀）
    pub dest_path: Option<String>,   // Moved 时：移动后的完整路径
    pub error: Option<String>,       // 失败 / 取消 / 跳过原因；Moved 时偶尔带提示（见 3.3）
}

pub struct RestoreAllResult {
    pub items: Vec<ItemOpResult>,
    pub folder_removed: bool,        // 收纳文件夹已删除（或本来就不存在）
    pub remaining: Vec<String>,      // 未能移走、仍留在文件夹里的文件名（不含 desktop.ini / Thumbs.db），按小写排序
}

pub struct FenceChange {             // 监听回调参数
    pub folders: Vec<String>,        // 受影响的收纳文件夹名（去重、排序；改名时新旧都在）
    pub rescan: bool,                // 监听出错 / 事件溢出：对所有收纳箱重新对账
}

pub struct FenceWatcher { .. }       // 监听句柄，drop 即停止；fn root(&self) -> &Path
```

`ItemOpStatus` 语义与集成层（`commands/fence_items.rs`、`commands/fence.rs`）的处理：

| 状态 | 含义 | 集成层动作 |
|---|---|---|
| `Moved` | 已移动到目标位置 | 移入：upsert 行（`original_path` = 源路径，跨箱来源则搬运旧行的）；移回：删行；跨箱：删旧行、在目标 upsert 并搬 `original_path`；删除收纳箱：计入 `restored` |
| `Cancelled` | 用户在管理员权限对话框 / UAC 点了取消（"用户取消了操作"），或在错误对话框里选"跳过"（"已跳过"） | 不动库；前端提示"已取消"；删除收纳箱时计入 `failures` → 收纳箱保留 |
| `Failed` | 文件被占用、不存在、权限不足、校验不通过等（`error` 有中文原因） | 不动库；前端提示原因；删除收纳箱时计入 `failures` → 收纳箱保留 |
| `Skipped` | 未执行：已在该收纳箱、`desktop.ini` / `Thumbs.db`、隐藏或系统文件、同一批里重复的源路径 | 不动；前端不提示 |

## 3. API 详解

### 3.1 路径

| 签名 | 说明 |
|---|---|
| `fn fence_root_dir() -> Result<PathBuf, String>` | 收纳根目录，并确保存在（`create_dir_all`）。默认 `%USERPROFILE%\MiniTodo收纳箱`（`dirs::home_dir()` + `FENCE_ROOT_DIR_NAME`），debug 下可被 `MINI_TODO_FENCES_ROOT` 覆盖。取不到用户目录 → `Err("无法确定用户目录，不能创建收纳文件夹")`。所有对外文件函数内部都先调它，因此**第一次调用就会建出根目录**——集成层只在有收纳箱之后才调用 |
| `fn icon_cache_dir() -> PathBuf` | `<app_data_dir>\fence-icons`（默认 `%LOCALAPPDATA%\mini-todo\fence-icons`），不创建 |
| `fn user_desktop_dir() -> Option<PathBuf>` | `dirs::desktop_dir()`，已处理 OneDrive / 组策略重定向 |
| `fn path_starts_with_ci(path, prefix) -> bool` | `path` 等于 `prefix` 或在其下（按路径段、不区分大小写、忽略 `\\?\` 前缀与末尾分隔符；空 `prefix` 恒 false） |
| `fn same_path_ci(a, b) -> bool` | 两路径指向同一位置（同上规则） |
| 内部：`fence_root_path()` | 同 `fence_root_dir` 但不创建 |
| 内部：`public_desktop_dir()` | `SHGetKnownFolderPath(FOLDERID_PublicDesktop)`，失败退回 `%PUBLIC%\Desktop`；非 Windows `None`。`move_into_fence` 用它组成 `protected` |

### 3.2 文件夹

#### `fn create_fence_folder(title: &str, taken: &[String]) -> Result<String, String>`

新建收纳箱时调用，返回实际文件夹名，写入 `fences.folder_name`。集成层**先建文件夹再插库**，插库 / 建窗失败时用
`remove_fence_folder_if_empty` 回滚。
- `title` 经 `sanitize_folder_name` 清洗：去首尾空白；控制字符与 `<>:"/\|?*` 换成 `_`；去尾部 `.` 与空格；超过 48 字符截断；
  空则 `收纳箱`；主干是保留设备名（`CON`、`com1.txt`…）时插入 `_`（`CON` → `CON_`）
- `taken` 传数据库里**全部**已有的 `folder_name`（即使对应文件夹已被用户删掉，也要避开，防 UNIQUE 冲突）
- 与 `taken` 及磁盘上已有项（含同名文件）不区分大小写去重：`收纳箱` → `收纳箱 (2)` → `收纳箱 (3)`…
- 去重与创建之间被抢先建了同名目录会自动重试（最多 16 次）
- 错误：根目录不可写、创建失败

#### `fn ensure_fence_folder(folder_name: &str) -> Result<PathBuf, String>`

确保文件夹存在（不存在按原名重建），返回完整路径。启动恢复、以及对账时发现文件夹被删时调用。

#### `fn rename_fence_folder(old_folder: &str, new_title: &str, taken: &[String]) -> Result<String, String>`

改标题时同步改文件夹名，返回新文件夹名（与 `old_folder` 相同表示无需改动，调用方照常写库）。
- `taken`：数据库里所有收纳箱的 `folder_name`，可以包含 `old_folder`（内部排除）
- 只改大小写也会真正改名；去重后落回旧名则不动
- 旧文件夹不存在：按新标题新建一个（里面没有可丢的文件）
- 失败（文件夹或其中文件被占用等）返回 `Err`，旧文件夹保持原样 → 集成层保留旧 `folder_name`，`RenameFenceResult.folderError` 带原因
- 集成层在数据库锁内调用，且有文件正在移进 / 移出时**不调用**（只改标题），正在删除时直接拒绝改名

#### `fn remove_fence_folder_if_empty(folder_name: &str) -> Result<bool, String>`

- `Ok(true)`：已删除，或本来就不存在
- `Ok(false)`：里面还有东西，保留
- 只剩 `desktop.ini` / `Thumbs.db` 视为空，一并删除（先去只读属性）；**绝不递归删除用户文件**
- `restore_all` 内部已调用；集成层只在新建失败回滚时单独调用

#### `fn fence_folder_path(folder_name: &str) -> Result<PathBuf, String>`

校验文件夹名（`is_valid_folder_name`：非空、非 `.`/`..`、无分隔符与非法字符、首尾无空白、尾部无 `.`、非保留名、≤ 64 字符）后拼出
完整路径；不检查存在。名字不合法 → `Err`（防 `..` 逃逸）。注意它会经 `fence_root_dir` 创建根目录。

#### `fn list_fence_items(folder_name: &str) -> Result<Vec<FenceEntry>, String>`

列举收纳文件夹内容，按文件名不区分大小写排序；跳过 `desktop.ini`、`Thumbs.db`、隐藏 / 系统属性的文件、名字含非法 UTF-16 的项。
文件夹不存在 → `Err("收纳文件夹「x」不存在")`。集成层只在数据库锁内（`reconcile_fence` 的事务里）调用，失败时 `ensure_fence_folder`
后重试一次。

#### `fn list_fence_folders() -> Result<Vec<String>, String>`

根目录下现有子文件夹名（排序，跳过名字不合法的目录；根目录不存在返回空）。集成层启动对账后用它记录不属于任何收纳箱的文件夹（只记日志）。

### 3.3 移动

共同规则：
- 执行用一个 `IFileOperation`（`FOF_RENAMEONCOLLISION | FOF_NOCONFIRMMKDIR | FOFX_NOMINIMIZEBOX`），Shell 会发变更通知，桌面立即刷新
- **目标文件名在执行前已去重**（按目标目录实际列表 + 本批已预订的名字；` (2)` 后缀插在扩展名前，文件夹整体加后缀），
  `FOF_RENAMEONCOLLISION` 只作兜底，**绝不覆盖**
- 不带 `FOF_NOERRORUI`：权限不足时弹系统"需要管理员权限才能移动此文件"→ UAC；文件被占用时弹"文件正在使用"（重试 / 跳过 / 取消）。
  行为与资源管理器拖放一致；用户做出选择前调用一直阻塞
- 结果按每项的 `PostMoveItem` 回调 + 文件系统实际状态判定：`COPYENGINE_E_USER_CANCELLED` / `HRESULT_FROM_WIN32(ERROR_CANCELLED)` →
  `Cancelled`（"用户取消了操作"）；`COPYENGINE_S_USER_IGNORED` → `Cancelled`（"已跳过"）。极少数情况下 `Moved` 会附带 `error` 提示：
  "已放入目标位置，但源文件未能删除，原位置可能仍保留一份"（跨卷复制后删除源失败）或"已移动，但无法确定新的文件名，请刷新收纳箱"（`file_name` 为 `None`）

#### `fn move_into_fence<P: AsRef<Path>>(sources: &[P], folder_name: &str, owner_hwnd: Option<isize>) -> Result<Vec<ItemOpResult>, String>`

拖入收纳箱（集成层 `fence_move_in`，`sources` 来自前端 `onDragDropEvent` 的 `paths`）。
- 返回与 `sources` **一一对应、顺序相同**的结果；收纳文件夹不存在时自动重建
- 内部以 `protected = [user_desktop_dir(), public_desktop_dir()]` 调规划函数 `plan_move_in(sources, root, protected, fence_dir)`，逐项校验：

  | 条件 | 状态与原因 |
  |---|---|
  | 不是绝对路径 | `Failed`："需要完整路径" |
  | 没有文件名（磁盘根） | `Failed`："无法收纳磁盘根目录" |
  | 无法访问（不存在等） | `Failed`："无法访问: …" |
  | 同一批里重复的源路径 | `Skipped`："重复的项目" |
  | `desktop.ini` / `Thumbs.db` | `Skipped`："desktop.ini 等系统文件不收纳" |
  | 隐藏或系统属性 | `Skipped`："隐藏或系统文件不收纳" |
  | 收纳根目录本身或其上级目录 | `Failed`："不能收纳收纳目录本身或其上级目录" |
  | **用户桌面 / 公共桌面本身或其上级目录（`protected`）** | `Failed`："不能收纳桌面文件夹本身或其上级目录"——经 Shell 移动已知文件夹会让系统把"桌面"重定向到收纳文件夹里。桌面**上的**项照常可以收纳 |
  | 已在本收纳箱中 | `Skipped`："已在该收纳箱中" |
  | 别的收纳箱文件夹本身（父目录是根目录） | `Failed`："不能把收纳箱文件夹放进收纳箱" |

- 来自**另一个收纳箱文件夹里**的项允许（等同跨箱移动）：集成层在移动前取旧行的 `original_path` 搬过来，而不是用 `source`
- 公共桌面上的项会弹管理员权限对话框 / UAC，取消 → `Cancelled`

#### `fn restore_item(folder_name: &str, file_name: &str, original_path: Option<&str>, owner_hwnd: Option<isize>) -> Result<ItemOpResult, String>`

右键「移回桌面」（集成层 `fence_restore_item`）。
- 目标目录（`choose_restore_dir`）：`original_path` 的上级目录是绝对路径、不在收纳根目录内、且仍存在 → 原目录；否则（含 `None`）→ 用户桌面
- 目标文件名（`restore_file_name`）：收纳时因重名被改成 `X (2).lnk` 的恢复原名 `X.lnk`；用户在资源管理器里改过的名字保留；
  目标处已有同名项再加后缀
- 移回公共桌面需要管理员权限，取消 → `Cancelled`（项目留在收纳箱）
- `Moved` 时 `dest_path` 为最终完整路径；项目不在文件夹里 → `Failed("收纳文件夹中找不到该项目")`
- `Err`：文件夹名 / 文件名不合法、`Err("无法确定桌面路径")`

#### `fn move_between_fences(from_folder: &str, file_name: &str, to_folder: &str, owner_hwnd: Option<isize>) -> Result<ItemOpResult, String>`

右键「移动到…」（集成层 `fence_move_item`）。目标收纳箱文件夹不存在时自动创建；重名加后缀；`Moved` 时 `file_name` 为目标收纳箱里的新文件名。
`from_folder == to_folder`（不区分大小写）→ `Err("源收纳箱与目标收纳箱相同")`。不涉及数据库：`original_path` 由集成层从旧行搬到新行。

#### `fn restore_all(folder_name: &str, originals: &HashMap<String, String>, owner_hwnd: Option<isize>) -> Result<RestoreAllResult, String>`

删除收纳箱时调用（集成层 `delete_fence`）。
- `originals`：`file_name → original_path`（集成层 `fence_originals`：`fence_items` 中 `original_path` 非空的行），按文件名不区分大小写匹配；
  不在表里的项回用户桌面
- 处理文件夹里的**所有**项（含隐藏文件；`desktop.ini` / `Thumbs.db` 除外），规则同 `restore_item`，所有项在同一个 `IFileOperation` 里执行
  （公共桌面多项时，系统对话框可勾选"对所有当前项目执行此操作"）
- 然后调用 `remove_fence_folder_if_empty`：全部移走则 `folder_removed = true`；否则保留文件夹，`remaining` 列出剩余文件名
- 文件夹本来就不存在 → `{ items: [], folderRemoved: true, remaining: [] }`
- 集成层据此（纯函数 `plan_delete`）决定：`folder_removed` 才删收纳箱；否则整个收纳箱保留（PRD R1）

### 3.4 判定

| 签名 | 说明 |
|---|---|
| `fn desktop_is_onedrive_redirected() -> bool` | 用户桌面位于 OneDrive 下：在 `OneDrive` / `OneDriveConsumer` / `OneDriveCommercial` 环境变量所指目录内，或路径某一级以 `OneDrive` 开头（覆盖 `OneDrive - 公司名`）。集成层在本次有项从用户桌面移入时据此决定是否给一次性提示（标记存 settings `fence_onedrive_notice_shown`） |
| 内部：`is_under_onedrive(path, roots)` | 上面的纯函数核心（有单测） |

### 3.5 打开

#### `fn launch_item(path: &str) -> Result<(), String>`

双击打开（集成层 `fence_open_item`，路径由 `is_valid_item_name` 校验后的文件名拼出）：`ShellExecuteExW` 默认动作，
`SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC`。
- exe / com / bat / cmd 把工作目录设为所在目录；`.lnk` 用快捷方式自带的参数与工作目录；`.url` 用默认浏览器；文件夹在资源管理器打开
- 目标失效的 `.lnk`：不弹系统对话框，通常**约 3 秒**后返回 `Err("快捷方式指向的目标不存在，可能已被移动或卸载")`
  （Shell 先静默尝试解析；之后用 `IShellLinkW::GetPath` 只读目标路径判断），前端显示"正在打开"
- 其他错误：`"文件不存在"`、`"已取消"`（UAC 取消）、`"没有与该类型文件关联的程序"`、`"没有权限打开"`、`"不是有效的程序"`、`"打开失败：…"`
- 阻塞到 Shell 完成启动
- 前台：收纳箱窗口必须可激活（不要 `focusable(false)`），否则新程序可能拿不到前台

#### `fn reveal_in_explorer(path: &str) -> Result<(), String>`

右键「打开所在位置」（集成层 `fence_reveal_item`）：`SHOpenFolderAndSelectItems`（复用已打开的资源管理器窗口），失败时退回
`explorer.exe /select,"<路径>"`。路径不存在 → `Err("文件不存在")`。

### 3.6 图标

#### `fn icon_png(path: &str, size_px: u32) -> Result<PathBuf, String>`

取图标 PNG 的缓存文件路径（集成层 `fence_item_icon` 返回其字符串），前端 `convertFileSrc(path)` 加载。
assetProtocol scope 为 `["$LOCALDATA/mini-todo/images/**", "$LOCALDATA/mini-todo/fence-icons/**"]`（`tauri.conf.json`，只为收纳箱新增了
`fence-icons` 一项）；debug 构建设了 `MINI_TODO_DATA_DIR` 时，`lib.rs` setup 另用 `asset_protocol_scope().allow_directory(icon_cache_dir(), true)`
放行隔离目录。
- `size_px`：**物理像素**（前端 `ceil(图标逻辑尺寸 × devicePixelRatio)`），向上取整到档位 `ICON_SIZE_BUCKETS = [16, 24, 32, 48, 64, 96, 128, 256]`
  （超过 256 按 256；前端 `FENCE_ICON_BUCKETS` 与之一致）
- 返回正方形 PNG，边长通常等于档位；只有 ≤48 px 图标资源的程序按更大档位请求时返回其原尺寸（如 48×48，已去掉 Shell 加的方框）。
  前端用固定 CSS 宽高显示（会拉伸到格子大小）
- `.lnk` 取目标图标、**不带箭头**；`.url` 取浏览器图标；文件夹取文件夹图标；图片等文件显示类型图标而非缩略图（`SIIGBF_ICONONLY`）
- 缓存键：`{路径哈希 FNV-1a 64:016x}-{mtime 毫秒:x}-{文件大小:x}-{档位}.png`，文件变了键随之变；命中缓存时只做一次 `metadata`
- 写缓存先写临时文件再改名，前端不会读到半个 PNG
- 失败（文件不存在、提取失败）→ `Err`，前端显示占位图标；非 Windows → `Err`
- 阻塞；同时有大量请求时在 `fence-icon` 线程上排队（前端最多 3 个并发、只请求可见项）

#### `fn prune_icon_cache<P: AsRef<Path>>(live_paths: &[P]) -> Result<usize, String>`

只保留 `live_paths`（当前所有收纳箱里所有项目的完整路径）在**当前** mtime / 大小下的缓存，其余（已移走的项、旧版本、残留临时文件）全部删除，
返回删除数。集成层在启动对账（线程 `fence-startup`）完成 20 秒后调用一次；任何收纳箱对账失败时跳过；没有收纳箱时以空列表调用（清空缓存）。

### 3.7 监听

#### `fn watch_fence_root(on_change: impl Fn(FenceChange) + Send + 'static) -> Result<FenceWatcher, String>`

- 用 `notify`（ReadDirectoryChangesW）**递归**监听收纳根目录（不存在则创建）；只关心根目录下第一、二层（收纳文件夹本身与其中的项目），
  被收纳的文件夹内部的变化忽略；`Access` 类事件忽略
- 去抖：最后一个事件后安静 300 ms（`WATCH_DEBOUNCE`）回调一次；持续有变化时最多 1.5 s 必回调一次
- 回调在内部线程 `fence-watch` 上执行，可以直接 `app_handle.emit_to(...)`；回调 panic 不会终止监听
- `folders` 可能包含不对应任何收纳箱的名字（例如用户在根目录里自己建的文件夹），集成层按数据库（不区分大小写）匹配后忽略
- 本应用自己的移入 / 移回 / 改名同样会触发回调，不需要区分来源
- 返回的 `FenceWatcher` 必须一直持有：集成层放在托管状态 `FenceWatcherState`，由 `ensure_fence_watcher` 在有收纳箱之后（启动恢复 / 新建）启动

## 4. 集成层映射（实际命令）

| 命令（`commands/`） | 调用 | 数据库 / 其他 |
|---|---|---|
| `create_fence`（与托盘「新增收纳箱」同一实现，`fence.rs`） | `create_fence_folder(&title, &全部 folder_name)` | 先建文件夹再 `INSERT fences` + `save_layout`；插库 / 建窗失败 → `remove_fence_folder_if_empty` 回滚；之后 `ensure_fence_watcher` |
| `rename_fence`（`fence.rs`） | `rename_fence_folder(&old, &title, &全部 folder_name)`（数据库锁内） | `Err` → 标题照改、`folder_name` 不变、`folderError`；有移动时不调用、只改标题；删除中 → `Err` |
| `restore_fences`（启动，`fence.rs`） | 逐个 `ensure_fence_folder` | 然后建窗、`ensure_fence_watcher`、`spawn_startup_reconcile`（对账 → `list_fence_folders` 记孤儿 → 20s 后 `prune_icon_cache`） |
| `get_fence_items` / 监听回调 / `reconcile_and_notify`（`fence_items.rs`） | `list_fence_items`（失败则 `ensure_fence_folder` 后重试），都在数据库事务内 | 对账 `fence_items`；推 `fence-items-changed` 给 `fence-{id}` |
| `fence_move_in` | `move_into_fence(&paths, &folder, owner)` | `begin_moving(&[id])`；移动前读跨箱来源旧行；`Moved` → upsert（跨箱：删旧行 + 搬 `original_path`）；`desktop_is_onedrive_redirected` + `claim_onedrive_notice` → `onedriveNotice` |
| `fence_restore_item` | `restore_item(&folder, &file_name, original.as_deref(), owner)` | `begin_moving(&[id])`；`Moved` → 删行 |
| `fence_move_item` | `move_between_fences(&from, &file_name, &to, owner)` | 源 = 目标 → `Err`；`begin_moving(&[id, target])`；`Moved` → 删旧行、按新 `file_name` upsert 并搬 `original_path`，通知目标窗口 |
| `delete_fence`（`fence.rs`） | `restore_all(&folder, &originals, owner)` | `begin_deleting(id)`；`plan_delete`：`folder_removed` → 删记录（级联）+ `destroy` 窗口；否则删已移回项的行、对账，返回 `deleted = false` |
| `fence_open_item` / `fence_reveal_item` | `launch_item` / `reveal_in_explorer` | 路径 = `fence_folder_path(folder).join(file_name)`，`file_name` 先过 `is_valid_item_name` |
| `fence_item_icon` | `icon_png(&path, size_px)` | 同上；返回缓存路径字符串 |

所有调用都经 `blocking()` 在阻塞线程池执行；带 owner 的移动命令都是 async，HWND 在 async 上下文里取（见 1.2）。

## 5. 依赖变更（`pc/src-tauri/Cargo.toml`）

- `[dependencies]`：`notify = "7"`、`png = "0.17"`（png 0.17 原本就在依赖树里）
- `[target.'cfg(windows)'.dependencies]`：
  - windows 0.58 新增 features：`implement`、`Win32_Graphics_Gdi`、`Win32_Storage_FileSystem`、`Win32_System_Com`、`Win32_System_Registry`、`Win32_UI_Shell`、`Win32_UI_Shell_Common`
  - `windows-core = "0.58"`：`#[implement]` 宏展开引用 `::windows_core`（实现 `IFileOperationProgressSink`），已在依赖树中，不增加编译量
- `Cargo.lock`：只新增 notify 及其传递依赖（filetime、fsevent-sys、inotify、inotify-sys、instant、kqueue、kqueue-sys、notify-types），
  **没有改动任何已有包的版本**（kqueue 锁在 1.0.8 / kqueue-sys 1.0.4，避免把 bitflags 从 2.10 抬到 2.13）

## 6. 已知限制

- 公共桌面项的管理员权限对话框 / UAC 取消路径无法自动化测试（会弹安全桌面），分类逻辑按 `COPYENGINE_E_USER_CANCELLED` / `ERROR_CANCELLED` /
  `COPYENGINE_S_USER_IGNORED` 判定并有单测，**需人工验证一次**（见 `e2e-progress.md` §4.1）
- "文件正在使用"等错误会弹 Shell 对话框（与资源管理器一致），用户点"取消 / 跳过"前移动调用一直阻塞；删除收纳箱时选"跳过"会让收纳箱整体保留
- 正常启动程序、打开所在位置会在屏幕上开窗口，自动化只覆盖了快捷方式启动、失效快捷方式与不存在的路径
- 收纳的图片 / 视频显示类型图标而非缩略图（如需缩略图，可在 `icon.rs::render_icon` 对图片扩展名改用默认取法）
- `list_fence_items` 跳过隐藏 / 系统文件，因此移入时也拒绝隐藏 / 系统文件（返回 `Skipped`），避免"移走后哪里都看不到"；
  但 `restore_all` 会移回文件夹里的隐藏文件，删除确认框的项目数不计入它们
