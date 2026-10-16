//! 移入 / 移回 / 跨箱移动 / 批量还原。
//!
//! 分两层：
//! * **规划**（全平台、可单测）：校验源路径、选目标目录、预先算好不冲突的目标文件名
//!   （macOS 上不列举收纳根目录以外的目标目录，见 [`Reserved`]）
//! * **执行**：
//!   - Windows：在新开的 STA 线程里用一个 `IFileOperation` 批量 `MoveItem`，
//!     Shell 会发变更通知让桌面立即刷新，权限不足时弹系统"需要管理员权限"对话框（UAC）
//!   - macOS：在调用线程（后台线程）上逐项 `renamex_np(RENAME_EXCL)`，目标已存在就换下一个后缀名重试，
//!     跨卷交给 `NSFileManager`（`mac.rs`）；没有提权对话框，也就没有"用户取消"
//!   - 其他平台：不支持
//!
//! 执行完后按每项的执行反馈（Windows 是 `PostMoveItem` 回调）与文件系统实际状态归类为 移动成功 / 用户取消 / 失败 / 跳过。

use super::folder::{
    describe_io_error, ensure_folder_at, entry_exists, existing_names_lower, folder_path_at,
    is_hidden_or_system, path_is_dir, remove_folder_if_empty_at,
};
use super::location::{
    fence_root_dir, path_starts_with_ci, public_desktop_dir, same_path_ci, user_desktop_dir,
};
use super::naming::{
    is_shell_metadata_file, is_valid_item_name, restore_file_name, unique_file_name,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// 单项操作结果的状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemOpStatus {
    /// 已移动到目标位置
    Moved,
    /// 用户取消（UAC / 管理员权限对话框点了取消，或在错误对话框里选择跳过）
    Cancelled,
    /// 失败（文件被占用、不存在、权限等），见 `error`
    Failed,
    /// 未执行：不需要或不允许移动（已在该收纳箱、desktop.ini、隐藏 / 系统文件、重复的源路径）
    Skipped,
}

/// 单项操作结果
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemOpResult {
    /// 源完整路径（移入时是桌面上的原路径 → 存入 `fence_items.original_path`）
    pub source: String,
    pub status: ItemOpStatus,
    /// 移动后的实际文件名（`Moved` 时有值；移入 / 跨箱时即 `fence_items.file_name`）
    pub file_name: Option<String>,
    /// 移动后的完整路径（`Moved` 时有值）
    pub dest_path: Option<String>,
    /// 失败 / 取消 / 跳过的原因；`Moved` 时偶尔带提示（例如源文件未能删除）
    pub error: Option<String>,
}

impl ItemOpResult {
    fn new(source: &Path, status: ItemOpStatus, error: Option<String>) -> Self {
        ItemOpResult {
            source: source.to_string_lossy().into_owned(),
            status,
            file_name: None,
            dest_path: None,
            error,
        }
    }

    fn moved(source: &Path, dest: &Path, note: Option<String>) -> Self {
        ItemOpResult {
            source: source.to_string_lossy().into_owned(),
            status: ItemOpStatus::Moved,
            file_name: dest.file_name().map(|n| n.to_string_lossy().into_owned()),
            dest_path: Some(dest.to_string_lossy().into_owned()),
            error: note,
        }
    }
}

/// 批量还原（删除收纳箱）的结果
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreAllResult {
    /// 逐项结果（顺序不保证与列举顺序一致以外的任何约定）
    pub items: Vec<ItemOpResult>,
    /// 收纳文件夹是否已删除（全部移走后自动删除；本来就不存在也为 true）
    pub folder_removed: bool,
    /// 未能移走、仍留在收纳文件夹里的文件名（`folder_removed` 为 false 时提示用户）
    pub remaining: Vec<String>,
}

/// 一次移动任务：`source` → `dest_dir\dest_name`
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MoveJob {
    pub source: PathBuf,
    pub dest_dir: PathBuf,
    /// 规划阶段已去重的目标文件名；执行时仍有兜底（Windows `FOF_RENAMEONCOLLISION`、macOS `RENAME_EXCL` 撞名顺延），绝不覆盖
    pub dest_name: String,
}

impl MoveJob {
    fn expected_path(&self) -> PathBuf {
        self.dest_dir.join(&self.dest_name)
    }
}

/// 执行层对单个任务的原始反馈
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MoveOutcome {
    /// Windows：`PostMoveItem` 回调里的 HRESULT，没收到回调为 `None`。
    /// macOS：成功为 `Some(0)`，失败为 `Some(-errno)`（小负数，不会与取消码相撞）
    pub hr: Option<i32>,
    /// 移动后的实际路径（Windows：回调里 `psiNewlyCreated` 的文件系统路径；macOS：撞名顺延后的最终路径）
    pub new_path: Option<PathBuf>,
    /// 失败说明（入队失败，或 `hr` 失败码的中文描述）
    pub error: Option<String>,
}

/// 执行层对整批任务的反馈
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct BatchOutcome {
    /// 与任务一一对应
    pub items: Vec<MoveOutcome>,
    /// `GetAnyOperationsAborted`：用户是否取消了（部分）操作
    pub aborted: bool,
    /// `PerformOperations` 整体返回的错误说明
    pub perform_error: Option<String>,
}

/// `COPYENGINE_E_USER_CANCELLED`
pub(crate) const HR_COPYENGINE_USER_CANCELLED: i32 = 0x8027_0000_u32 as i32;
/// `HRESULT_FROM_WIN32(ERROR_CANCELLED)`：UAC 提示被取消
pub(crate) const HR_ERROR_CANCELLED: i32 = 0x8007_04C7_u32 as i32;
/// `COPYENGINE_S_USER_IGNORED`：用户在对话框里选择了"跳过"
pub(crate) const HR_COPYENGINE_USER_IGNORED: i32 = 0x0027_0005;

fn is_cancel_hr(hr: i32) -> bool {
    matches!(
        hr,
        HR_COPYENGINE_USER_CANCELLED | HR_ERROR_CANCELLED | HR_COPYENGINE_USER_IGNORED
    )
}

/// 根据执行反馈与文件系统现状归类单项结果。`exists` 由调用方注入（测试时可模拟）。
pub(crate) fn classify(
    job: &MoveJob,
    outcome: &MoveOutcome,
    aborted: bool,
    perform_error: Option<&str>,
    exists: &dyn Fn(&Path) -> bool,
) -> ItemOpResult {
    let source = job.source.as_path();
    let expected = job.expected_path();
    let final_path = outcome
        .new_path
        .clone()
        .filter(|p| exists(p))
        .or_else(|| exists(&expected).then_some(expected));
    let source_gone = !exists(source);
    let hr_ok = outcome
        .hr
        .is_some_and(|hr| hr >= 0 && hr != HR_COPYENGINE_USER_IGNORED);

    match (&final_path, source_gone) {
        (Some(dest), true) => return ItemOpResult::moved(source, dest, None),
        (Some(dest), false) if hr_ok => {
            return ItemOpResult::moved(
                source,
                dest,
                Some("已放入目标位置，但源文件未能删除，原位置可能仍保留一份".to_string()),
            )
        }
        (None, true) if hr_ok => {
            // 回调报告成功、源文件已不在，但找不到新文件（罕见：目标路径取不到）
            return ItemOpResult {
                source: source.to_string_lossy().into_owned(),
                status: ItemOpStatus::Moved,
                file_name: None,
                dest_path: None,
                error: Some("已移动，但无法确定新的文件名，请刷新收纳箱".to_string()),
            };
        }
        _ => {}
    }

    match outcome.hr {
        Some(hr) if is_cancel_hr(hr) => {
            let reason = if hr == HR_COPYENGINE_USER_IGNORED {
                "已跳过"
            } else {
                "用户取消了操作"
            };
            ItemOpResult::new(source, ItemOpStatus::Cancelled, Some(reason.to_string()))
        }
        Some(hr) => {
            let error = outcome
                .error
                .clone()
                .unwrap_or_else(|| format!("移动失败（0x{:08X}）", hr as u32));
            ItemOpResult::new(source, ItemOpStatus::Failed, Some(error))
        }
        None => {
            if let Some(error) = &outcome.error {
                ItemOpResult::new(source, ItemOpStatus::Failed, Some(error.clone()))
            } else if aborted {
                ItemOpResult::new(
                    source,
                    ItemOpStatus::Cancelled,
                    Some("用户取消了操作".to_string()),
                )
            } else {
                let error = perform_error.unwrap_or("操作未执行").to_string();
                ItemOpResult::new(source, ItemOpStatus::Failed, Some(error))
            }
        }
    }
}

/// 规划结果：要执行的任务，或已经有结论的项
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Plan {
    Job(MoveJob),
    Done(ItemOpResult),
}

/// 同一批任务的目标名字分配器：按目标目录缓存已有项的名字（小写），
/// 并记录本批已经"预订"的名字，避免两项算出同一个名字；比较一律不区分大小写。
///
/// **macOS 只列举收纳根目录内的目标目录**：列举桌面 / 文稿 / 下载等受隐私保护的目录（读取已有内容）会弹
/// 「想访问桌面文件夹中的文件」授权框（ad-hoc 签名每次升级都会再弹；拒绝后读不到），而在里面新建项目不需要授权。
/// 这些目录只按本批已预订的名字去重，与磁盘上已有项的重名交给执行层的 `renamex_np(RENAME_EXCL)` 撞名顺延，同样绝不覆盖
pub(crate) struct Reserved {
    root: PathBuf,
    /// 是否也列举收纳根目录以外的目标目录（macOS 为 false）
    list_outside_root: bool,
    taken: HashMap<String, HashSet<String>>,
}

impl Reserved {
    /// 按当前平台的列举规则（macOS 不列举收纳根目录以外的目录）
    pub(crate) fn new(root: &Path) -> Self {
        Self::with_policy(root, !cfg!(target_os = "macos"))
    }

    pub(crate) fn with_policy(root: &Path, list_outside_root: bool) -> Self {
        Reserved {
            root: root.to_path_buf(),
            list_outside_root,
            taken: HashMap::new(),
        }
    }

    /// 在 `dir` 下为 `name` 选一个磁盘上（可列举时）和本批都不冲突的名字，并预订它
    fn pick(&mut self, dir: &Path, name: &str, is_dir: bool) -> String {
        let listable = self.list_outside_root || path_starts_with_ci(dir, &self.root);
        let taken = self
            .taken
            .entry(dir.to_string_lossy().to_lowercase())
            .or_insert_with(|| {
                if listable {
                    existing_names_lower(dir)
                } else {
                    HashSet::new()
                }
            });
        let chosen = unique_file_name(name, is_dir, |candidate| {
            taken.contains(&candidate.to_lowercase())
        });
        taken.insert(chosen.to_lowercase());
        chosen
    }
}

/// 源路径读不到时的原因。macOS 上隐私权限被拒（`EPERM`）与只在 iCloud 里的旧式占位文件给出可操作的提示
fn describe_access_error(source: &Path, fence_dir: &Path, err: &std::io::Error) -> String {
    #[cfg(target_os = "macos")]
    match err.raw_os_error() {
        Some(errno @ libc::EPERM) => return super::mac::describe_errno(source, fence_dir, errno),
        Some(errno @ libc::ENOENT) if super::mac::has_legacy_icloud_stub(source) => {
            return super::mac::describe_errno(source, fence_dir, errno)
        }
        _ => {}
    }
    let _ = (source, fence_dir);
    format!("无法访问: {}", describe_io_error(err))
}

/// 移入规划：逐个校验源路径，生成移动到 `fence_dir` 的任务。
///
/// `protected`：不能被整体收纳的目录（用户桌面、公共桌面）。拖入的是它们本身或其上级目录时拒绝：
/// 经 Shell 移动已知文件夹会让系统把"桌面"重定向到收纳文件夹里
pub(crate) fn plan_move_in(
    sources: &[PathBuf],
    root: &Path,
    protected: &[PathBuf],
    fence_dir: &Path,
) -> Vec<Plan> {
    let mut reserved = Reserved::new(root);
    let mut seen: HashSet<String> = HashSet::new();
    sources
        .iter()
        .map(|source| {
            let skip = |reason: &str| {
                Plan::Done(ItemOpResult::new(
                    source,
                    ItemOpStatus::Skipped,
                    Some(reason.to_string()),
                ))
            };
            let fail = |reason: String| {
                Plan::Done(ItemOpResult::new(
                    source,
                    ItemOpStatus::Failed,
                    Some(reason),
                ))
            };
            if !source.is_absolute() {
                return fail("需要完整路径".to_string());
            }
            let Some(name) = source.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                return fail("无法收纳磁盘根目录".to_string());
            };
            let meta = match fs::symlink_metadata(source) {
                Ok(meta) => meta,
                Err(e) => return fail(describe_access_error(source, fence_dir, &e)),
            };
            if !seen.insert(source.to_string_lossy().to_lowercase()) {
                return skip("重复的项目");
            }
            if is_shell_metadata_file(&name) {
                return skip("desktop.ini 等系统文件不收纳");
            }
            if is_hidden_or_system(&meta, &name) {
                return skip("隐藏或系统文件不收纳");
            }
            if path_starts_with_ci(root, source) {
                return fail("不能收纳收纳目录本身或其上级目录".to_string());
            }
            if protected.iter().any(|dir| path_starts_with_ci(dir, source)) {
                return fail("不能收纳桌面文件夹本身或其上级目录".to_string());
            }
            let parent = source.parent().unwrap_or(Path::new(""));
            if same_path_ci(parent, fence_dir) {
                return skip("已在该收纳箱中");
            }
            if same_path_ci(parent, root) {
                return fail("不能把收纳箱文件夹放进收纳箱".to_string());
            }
            let is_dir = path_is_dir(source, &meta);
            Plan::Job(MoveJob {
                source: source.clone(),
                dest_dir: fence_dir.to_path_buf(),
                dest_name: reserved.pick(fence_dir, &name, is_dir),
            })
        })
        .collect()
}

/// 选择移回的目标目录：原路径的上级目录仍存在且不在收纳根目录内 → 原目录；否则用户桌面
pub fn choose_restore_dir(
    original_path: Option<&Path>,
    desktop: &Path,
    fence_root: &Path,
    dir_exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    original_path
        .and_then(Path::parent)
        .filter(|parent| parent.is_absolute())
        .filter(|parent| !path_starts_with_ci(parent, fence_root))
        .filter(|parent| dir_exists(parent))
        .map(Path::to_path_buf)
        .unwrap_or_else(|| desktop.to_path_buf())
}

/// 移回规划：决定目标目录与文件名（重名加后缀，不覆盖）
pub(crate) fn plan_restore(
    entry: &Path,
    is_dir: bool,
    original_path: Option<&Path>,
    desktop: &Path,
    root: &Path,
    reserved: &mut Reserved,
) -> MoveJob {
    let current = entry
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dest_dir = choose_restore_dir(original_path, desktop, root, |p| p.is_dir());
    let original_name = original_path
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned());
    let wanted = restore_file_name(&current, original_name.as_deref(), is_dir);
    MoveJob {
        source: entry.to_path_buf(),
        dest_name: reserved.pick(&dest_dir, &wanted, is_dir),
        dest_dir,
    }
}

/// 执行器：真实环境下是 [`execute`]（Windows IFileOperation / macOS renamex_np）；测试里可换成 `std::fs` 实现
pub(crate) type Executor<'a> = &'a dyn Fn(&[MoveJob]) -> Result<BatchOutcome, String>;

/// 执行一批移动任务（Windows：新开 STA 线程 + IFileOperation；macOS：本线程逐项 renamex_np）
fn execute(jobs: &[MoveJob], owner_hwnd: Option<isize>) -> Result<BatchOutcome, String> {
    if jobs.is_empty() {
        return Ok(BatchOutcome::default());
    }
    #[cfg(windows)]
    {
        let jobs = jobs.to_vec();
        super::sta::run_in_new_sta("fence-move", move || {
            Ok(super::shell::perform_moves(&jobs, owner_hwnd))
        })
    }
    #[cfg(target_os = "macos")]
    {
        // macOS 没有提权 / "文件正在使用"对话框，用不到父窗口
        let _ = owner_hwnd;
        Ok(execute_mac(jobs))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = owner_hwnd;
        Err(super::UNSUPPORTED.to_string())
    }
}

/// macOS 执行层：在调用线程（后台线程）上逐项移动，绝不覆盖（`mac::move_item`）。
/// 期间允许本线程下载"只在云端"的 iCloud 文件（先下载完再改名，不会留下坏文件）
#[cfg(target_os = "macos")]
fn execute_mac(jobs: &[MoveJob]) -> BatchOutcome {
    let policy = super::mac::allow_materialize_on_this_thread();
    let items = jobs.iter().map(execute_mac_job).collect();
    super::mac::restore_materialize_policy(policy);
    BatchOutcome {
        items,
        aborted: false,
        perform_error: None,
    }
}

#[cfg(target_os = "macos")]
fn execute_mac_job(job: &MoveJob) -> MoveOutcome {
    let is_dir = fs::symlink_metadata(&job.source)
        .map(|meta| path_is_dir(&job.source, &meta))
        .unwrap_or(false);
    // 目标已存在时按收纳的去重规则顺延：`a.txt` → `a (2).txt`，包 `X.app` → `X (2).app`
    let next_name = |n: u32| super::naming::suffixed_file_name(&job.dest_name, n, is_dir);
    let result = super::mac::move_item(&job.source, &job.dest_dir, &job.dest_name, &next_name);
    match result.final_path {
        Some(path) => MoveOutcome {
            hr: Some(0),
            new_path: Some(path),
            error: result.message,
        },
        None => MoveOutcome {
            hr: Some(-result.errno.unwrap_or(1).max(1)),
            new_path: None,
            error: Some(result.message.unwrap_or_else(|| "移动失败".to_string())),
        },
    }
}

/// 执行规划并按原顺序合并结果
fn run_plans(plans: Vec<Plan>, exec: Executor) -> Result<Vec<ItemOpResult>, String> {
    let jobs: Vec<MoveJob> = plans
        .iter()
        .filter_map(|plan| match plan {
            Plan::Job(job) => Some(job.clone()),
            Plan::Done(_) => None,
        })
        .collect();
    let batch = if jobs.is_empty() {
        BatchOutcome::default()
    } else {
        exec(&jobs)?
    };
    let exists = |p: &Path| entry_exists(p);
    let mut outcomes = batch.items.iter();
    let results = plans
        .into_iter()
        .map(|plan| match plan {
            Plan::Done(result) => result,
            Plan::Job(job) => {
                let outcome = outcomes.next().cloned().unwrap_or_default();
                classify(
                    &job,
                    &outcome,
                    batch.aborted,
                    batch.perform_error.as_deref(),
                    &exists,
                )
            }
        })
        .collect();
    Ok(results)
}

pub(crate) fn move_into_at(
    root: &Path,
    protected: &[PathBuf],
    sources: &[PathBuf],
    folder_name: &str,
    exec: Executor,
) -> Result<Vec<ItemOpResult>, String> {
    let fence_dir = ensure_folder_at(root, folder_name)?;
    run_plans(plan_move_in(sources, root, protected, &fence_dir), exec)
}

/// 把多个文件 / 文件夹移入收纳文件夹（拖入收纳箱）。
///
/// * 返回与 `sources` 一一对应、顺序相同的结果；`Moved` 项的 `source` 存为 `original_path`，
///   `file_name` 存为 `fence_items.file_name`
/// * 重名自动加 ` (2)` 后缀；`desktop.ini`、隐藏 / 系统文件、已在该收纳箱中的项返回 `Skipped`
/// * 公共桌面上的项会弹系统"需要管理员权限"对话框 / UAC，用户取消返回 `Cancelled`（Windows）
/// * `owner_hwnd`：系统对话框的父窗口（收纳箱窗口的 HWND；macOS 上不用），`None` 则无父窗口
/// * 收纳文件夹不存在时自动重建
/// * `Err` 只表示整次调用无法进行（文件夹名非法、根目录不可用、既不是 Windows 也不是 macOS）
pub fn move_into_fence<P: AsRef<Path>>(
    sources: &[P],
    folder_name: &str,
    owner_hwnd: Option<isize>,
) -> Result<Vec<ItemOpResult>, String> {
    let sources: Vec<PathBuf> = sources.iter().map(|p| p.as_ref().to_path_buf()).collect();
    let protected: Vec<PathBuf> = [user_desktop_dir(), public_desktop_dir()]
        .into_iter()
        .flatten()
        .collect();
    move_into_at(
        &fence_root_dir()?,
        &protected,
        &sources,
        folder_name,
        &|jobs| execute(jobs, owner_hwnd),
    )
}

/// 校验并取得收纳文件夹内某项的路径与元数据
fn fence_entry(dir: &Path, file_name: &str) -> Result<(PathBuf, Option<fs::Metadata>), String> {
    if !is_valid_item_name(file_name) {
        return Err(format!("文件名不合法: {:?}", file_name));
    }
    let path = dir.join(file_name);
    let meta = fs::symlink_metadata(&path).ok();
    Ok((path, meta))
}

fn not_found(path: &Path) -> ItemOpResult {
    ItemOpResult::new(
        path,
        ItemOpStatus::Failed,
        Some("收纳文件夹中找不到该项目".to_string()),
    )
}

pub(crate) fn restore_item_at(
    root: &Path,
    desktop: &Path,
    folder_name: &str,
    file_name: &str,
    original_path: Option<&str>,
    exec: Executor,
) -> Result<ItemOpResult, String> {
    let dir = folder_path_at(root, folder_name)?;
    let (path, meta) = fence_entry(&dir, file_name)?;
    let Some(meta) = meta else {
        return Ok(not_found(&path));
    };
    let original = original_path.map(PathBuf::from);
    let job = plan_restore(
        &path,
        path_is_dir(&path, &meta),
        original.as_deref(),
        desktop,
        root,
        &mut Reserved::new(root),
    );
    let mut results = run_plans(vec![Plan::Job(job)], exec)?;
    Ok(results.remove(0))
}

/// 把收纳文件夹里的一项移回原位置（右键「移回桌面」）。
///
/// * 目标目录：`original_path` 的上级目录仍存在 → 移回原目录；否则（或 `original_path` 为 `None`，
///   即在资源管理器里直接放进来的）→ 用户桌面 `dirs::desktop_dir()`
/// * 文件名：收纳时因重名被改成 `X (2).lnk` 的会恢复原名；目标位置已有同名项时加后缀，绝不覆盖
/// * 移回公共桌面需要管理员权限，用户取消返回 `Cancelled`（该项留在收纳箱）
/// * `Moved` 时 `dest_path` 是最终完整路径；项目不存在返回 `Failed`
pub fn restore_item(
    folder_name: &str,
    file_name: &str,
    original_path: Option<&str>,
    owner_hwnd: Option<isize>,
) -> Result<ItemOpResult, String> {
    let desktop = user_desktop_dir().ok_or_else(|| "无法确定桌面路径".to_string())?;
    restore_item_at(
        &fence_root_dir()?,
        &desktop,
        folder_name,
        file_name,
        original_path,
        &|jobs| execute(jobs, owner_hwnd),
    )
}

pub(crate) fn move_between_at(
    root: &Path,
    from_folder: &str,
    file_name: &str,
    to_folder: &str,
    exec: Executor,
) -> Result<ItemOpResult, String> {
    let from_dir = folder_path_at(root, from_folder)?;
    folder_path_at(root, to_folder)?;
    if from_folder.to_lowercase() == to_folder.to_lowercase() {
        return Err("源收纳箱与目标收纳箱相同".to_string());
    }
    let (path, meta) = fence_entry(&from_dir, file_name)?;
    let Some(meta) = meta else {
        return Ok(not_found(&path));
    };
    let to_dir = ensure_folder_at(root, to_folder)?;
    let job = MoveJob {
        dest_name: Reserved::new(root).pick(&to_dir, file_name, path_is_dir(&path, &meta)),
        source: path,
        dest_dir: to_dir,
    };
    let mut results = run_plans(vec![Plan::Job(job)], exec)?;
    Ok(results.remove(0))
}

/// 跨箱移动（右键「移动到…」）：把 `from_folder` 里的一项移到 `to_folder`。
///
/// 重名时加后缀；`Moved` 时 `file_name` 是在目标收纳箱里的新文件名。
/// 原始路径（`original_path`）由调用方从旧行搬到新行，本函数不涉及数据库。
pub fn move_between_fences(
    from_folder: &str,
    file_name: &str,
    to_folder: &str,
    owner_hwnd: Option<isize>,
) -> Result<ItemOpResult, String> {
    move_between_at(
        &fence_root_dir()?,
        from_folder,
        file_name,
        to_folder,
        &|jobs| execute(jobs, owner_hwnd),
    )
}

pub(crate) fn restore_all_at(
    root: &Path,
    desktop: &Path,
    folder_name: &str,
    originals: &HashMap<String, String>,
    exec: Executor,
) -> Result<RestoreAllResult, String> {
    let dir = folder_path_at(root, folder_name)?;
    if !entry_exists(&dir) {
        return Ok(RestoreAllResult {
            items: Vec::new(),
            folder_removed: true,
            remaining: Vec::new(),
        });
    }
    let originals: HashMap<String, PathBuf> = originals
        .iter()
        .map(|(name, path)| (name.to_lowercase(), PathBuf::from(path)))
        .collect();
    let mut reserved = Reserved::new(root);
    let mut plans = Vec::new();
    let read =
        fs::read_dir(&dir).map_err(|e| format!("读取收纳文件夹失败: {}", describe_io_error(&e)))?;
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_shell_metadata_file(&name) {
            continue;
        }
        let path = entry.path();
        let is_dir = entry
            .metadata()
            .map(|meta| path_is_dir(&path, &meta))
            .unwrap_or(false);
        let original = originals.get(&name.to_lowercase()).map(PathBuf::as_path);
        plans.push(Plan::Job(plan_restore(
            &path,
            is_dir,
            original,
            desktop,
            root,
            &mut reserved,
        )));
    }
    let items = run_plans(plans, exec)?;
    let folder_removed = match remove_folder_if_empty_at(root, folder_name) {
        Ok(removed) => removed,
        Err(e) => {
            log::warn!("[fence] 删除收纳文件夹「{}」失败: {}", folder_name, e);
            false
        }
    };
    let remaining = if folder_removed {
        Vec::new()
    } else {
        let mut names: Vec<String> = fs::read_dir(&dir)
            .map(|read| {
                read.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|name| !is_shell_metadata_file(name))
                    .collect()
            })
            .unwrap_or_default();
        names.sort_by_key(|name| name.to_lowercase());
        names
    };
    Ok(RestoreAllResult {
        items,
        folder_removed,
        remaining,
    })
}

/// 批量还原（删除收纳箱时）：把收纳文件夹里的所有项移回原位置，然后删除已清空的文件夹。
///
/// * `originals`：`file_name → original_path`（来自 `fence_items`；没有原始路径的项不用放进来，
///   会回用户桌面）。按文件名不区分大小写匹配
/// * 文件夹里的所有项都会处理（包括隐藏文件，`desktop.ini` / `Thumbs.db` / `.DS_Store` 等系统元数据文件除外），规则同 [`restore_item`]
/// * Windows：所有项在同一个 `IFileOperation` 里执行，公共桌面项的管理员权限对话框可"对所有项执行此操作"；
///   macOS：逐项移动
/// * 有项目没移走（取消 / 失败）时保留文件夹，`remaining` 列出剩余文件名
pub fn restore_all(
    folder_name: &str,
    originals: &HashMap<String, String>,
    owner_hwnd: Option<isize>,
) -> Result<RestoreAllResult, String> {
    let desktop = user_desktop_dir().ok_or_else(|| "无法确定桌面路径".to_string())?;
    restore_all_at(
        &fence_root_dir()?,
        &desktop,
        folder_name,
        originals,
        &|jobs| execute(jobs, owner_hwnd),
    )
}

#[cfg(test)]
mod tests {
    use super::super::folder::test_support::{touch, TempDir};
    use super::*;

    fn job(source: &str, dest_dir: &str, dest_name: &str) -> MoveJob {
        MoveJob {
            source: PathBuf::from(source),
            dest_dir: PathBuf::from(dest_dir),
            dest_name: dest_name.to_string(),
        }
    }

    fn fs_with(paths: &[&str]) -> impl Fn(&Path) -> bool {
        let set: HashSet<String> = paths.iter().map(|p| p.to_lowercase()).collect();
        move |p: &Path| set.contains(&p.to_string_lossy().to_lowercase())
    }

    /// 测试用执行器：用 `fs::rename` 模拟 IFileOperation；文件名在 `fail` 里的项模拟"被占用"失败，
    /// 在 `cancel` 里的项模拟用户取消 UAC
    fn fake_exec<'a>(
        fail: &'a [&'a str],
        cancel: &'a [&'a str],
    ) -> impl Fn(&[MoveJob]) -> Result<BatchOutcome, String> + 'a {
        move |jobs: &[MoveJob]| {
            let mut aborted = false;
            let items = jobs
                .iter()
                .map(|job| {
                    let name = job
                        .source
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    if fail.contains(&name.as_str()) {
                        return MoveOutcome {
                            hr: Some(0x8007_0020_u32 as i32),
                            new_path: None,
                            error: Some("正被其他程序占用".to_string()),
                        };
                    }
                    if cancel.contains(&name.as_str()) {
                        aborted = true;
                        return MoveOutcome {
                            hr: Some(HR_ERROR_CANCELLED),
                            ..Default::default()
                        };
                    }
                    let dest = job.expected_path();
                    assert!(!dest.exists(), "规划出的目标不应已存在: {}", dest.display());
                    fs::create_dir_all(&job.dest_dir).unwrap();
                    fs::rename(&job.source, &dest).unwrap();
                    MoveOutcome {
                        hr: Some(0),
                        new_path: Some(dest),
                        error: None,
                    }
                })
                .collect();
            Ok(BatchOutcome {
                items,
                aborted,
                perform_error: None,
            })
        }
    }

    #[test]
    fn classify_moved_by_callback_path() {
        let j = job(r"C:\D\a.lnk", r"C:\F", "a.lnk");
        // Windows 上即 `C:\F\a (2).lnk`；按平台分隔符拼接，非 Windows 上同样取得到文件名
        let renamed = PathBuf::from(r"C:\F").join("a (2).lnk");
        let renamed_str = renamed.to_string_lossy().into_owned();
        let outcome = MoveOutcome {
            hr: Some(0),
            new_path: Some(renamed),
            error: None,
        };
        let exists = fs_with(&[&renamed_str]);
        let r = classify(&j, &outcome, false, None, &exists);
        assert_eq!(r.status, ItemOpStatus::Moved);
        assert_eq!(r.file_name.as_deref(), Some("a (2).lnk"));
        assert_eq!(r.dest_path.as_deref(), Some(renamed_str.as_str()));
        assert_eq!(r.source, r"C:\D\a.lnk");
        assert_eq!(r.error, None);
    }

    #[test]
    fn classify_moved_by_filesystem_without_callback() {
        let j = job(r"C:\D\a.lnk", r"C:\F", "a.lnk");
        let expected = j.expected_path().to_string_lossy().into_owned();
        let exists = fs_with(&[&expected]);
        let r = classify(&j, &MoveOutcome::default(), false, None, &exists);
        assert_eq!(r.status, ItemOpStatus::Moved);
        assert_eq!(r.file_name.as_deref(), Some("a.lnk"));
    }

    #[test]
    fn classify_cancelled_variants() {
        let j = job(r"C:\P\a.lnk", r"C:\F", "a.lnk");
        let exists = fs_with(&[r"C:\P\a.lnk"]);
        for hr in [
            HR_COPYENGINE_USER_CANCELLED,
            HR_ERROR_CANCELLED,
            HR_COPYENGINE_USER_IGNORED,
        ] {
            let outcome = MoveOutcome {
                hr: Some(hr),
                ..Default::default()
            };
            let r = classify(&j, &outcome, true, None, &exists);
            assert_eq!(r.status, ItemOpStatus::Cancelled, "hr={hr:#x}");
            assert!(r.error.is_some());
        }
        // 没收到回调、整批被取消
        let r = classify(&j, &MoveOutcome::default(), true, None, &exists);
        assert_eq!(r.status, ItemOpStatus::Cancelled);
    }

    #[test]
    fn classify_failures() {
        let j = job(r"C:\D\a.lnk", r"C:\F", "a.lnk");
        let exists = fs_with(&[r"C:\D\a.lnk"]);
        let outcome = MoveOutcome {
            hr: Some(0x8007_0020_u32 as i32),
            new_path: None,
            error: Some("正被其他程序占用".to_string()),
        };
        let r = classify(&j, &outcome, false, None, &exists);
        assert_eq!(r.status, ItemOpStatus::Failed);
        assert_eq!(r.error.as_deref(), Some("正被其他程序占用"));

        // 失败码没有描述时给出十六进制
        let outcome = MoveOutcome {
            hr: Some(0x8007_0005_u32 as i32),
            ..Default::default()
        };
        let r = classify(&j, &outcome, false, None, &exists);
        assert!(r.error.unwrap().contains("0x80070005"));

        // 入队失败
        let outcome = MoveOutcome {
            hr: None,
            new_path: None,
            error: Some("无法解析路径".to_string()),
        };
        let r = classify(&j, &outcome, false, None, &exists);
        assert_eq!(r.status, ItemOpStatus::Failed);

        // 没回调也没取消
        let r = classify(
            &j,
            &MoveOutcome::default(),
            false,
            Some("整体失败"),
            &exists,
        );
        assert_eq!(r.status, ItemOpStatus::Failed);
        assert_eq!(r.error.as_deref(), Some("整体失败"));
    }

    #[test]
    fn classify_copy_without_delete_and_unknown_destination() {
        let j = job(r"C:\D\a.lnk", r"C:\F", "a.lnk");
        let expected = j.expected_path().to_string_lossy().into_owned();
        // 目标到位但源文件还在：算移动成功并提示
        let exists = fs_with(&[r"C:\D\a.lnk", &expected]);
        let outcome = MoveOutcome {
            hr: Some(0),
            ..Default::default()
        };
        let r = classify(&j, &outcome, false, None, &exists);
        assert_eq!(r.status, ItemOpStatus::Moved);
        assert!(r.error.is_some());
        // 取消时即使目标残留了部分内容，也不能算成功
        let outcome = MoveOutcome {
            hr: Some(HR_COPYENGINE_USER_CANCELLED),
            ..Default::default()
        };
        assert_eq!(
            classify(&j, &outcome, true, None, &exists).status,
            ItemOpStatus::Cancelled
        );
        // 回调成功、源已不在、找不到新路径
        let exists = fs_with(&[]);
        let outcome = MoveOutcome {
            hr: Some(0),
            ..Default::default()
        };
        let r = classify(&j, &outcome, false, None, &exists);
        assert_eq!(r.status, ItemOpStatus::Moved);
        assert_eq!(r.file_name, None);
    }

    #[test]
    fn restore_dir_selection() {
        if !Path::new(r"C:\x").is_absolute() {
            // 非 Windows 上反斜杠路径不是绝对路径：规则相同，只是无法用这些样例表达
            return;
        }
        let desktop = Path::new(r"C:\Users\me\Desktop");
        let root = Path::new(r"C:\Users\me\MiniTodo收纳箱");
        let exists = |p: &Path| {
            let s = p.to_string_lossy().to_lowercase();
            s == r"c:\users\public\desktop" || s == r"d:\tools" || s.starts_with(r"c:\users\me")
        };
        // 原目录存在 → 原目录
        assert_eq!(
            choose_restore_dir(
                Some(Path::new(r"C:\Users\Public\Desktop\Chrome.lnk")),
                desktop,
                root,
                exists
            ),
            PathBuf::from(r"C:\Users\Public\Desktop")
        );
        assert_eq!(
            choose_restore_dir(Some(Path::new(r"D:\Tools\x.exe")), desktop, root, exists),
            PathBuf::from(r"D:\Tools")
        );
        // 原目录不存在 → 桌面
        assert_eq!(
            choose_restore_dir(Some(Path::new(r"E:\gone\x.lnk")), desktop, root, exists),
            desktop
        );
        // 没有原始路径 → 桌面
        assert_eq!(choose_restore_dir(None, desktop, root, exists), desktop);
        // 原路径在收纳根目录内（例如来自另一个收纳箱）→ 桌面
        assert_eq!(
            choose_restore_dir(
                Some(Path::new(r"C:\Users\me\MiniTodo收纳箱\其他\a.lnk")),
                desktop,
                root,
                exists
            ),
            desktop
        );
        // 相对路径 → 桌面
        assert_eq!(
            choose_restore_dir(Some(Path::new(r"a.lnk")), desktop, root, exists),
            desktop
        );
    }

    #[test]
    fn plan_move_in_validates_sources() {
        let tmp = TempDir::new("plan-in");
        let root = tmp.path().join("root");
        let fence = root.join("箱");
        let other_fence = root.join("别的箱");
        let desktop = tmp.path().join("desktop");
        fs::create_dir_all(&fence).unwrap();
        fs::create_dir_all(&other_fence).unwrap();
        touch(&desktop.join("a.lnk"));
        touch(&desktop.join("b.lnk"));
        touch(&desktop.join("desktop.ini"));
        touch(&fence.join("b.lnk"));
        touch(&fence.join("in-fence.txt"));
        touch(&other_fence.join("c.lnk"));
        touch(&tmp.path().join("other").join("b.lnk"));

        let sources = vec![
            desktop.join("a.lnk"),
            desktop.join("b.lnk"),
            tmp.path().join("other").join("b.lnk"),
            desktop.join("desktop.ini"),
            desktop.join("missing.lnk"),
            fence.join("in-fence.txt"),
            root.clone(),
            tmp.path().to_path_buf(),
            other_fence.clone(),
            other_fence.join("c.lnk"),
            desktop.join("a.lnk"),
            PathBuf::from("relative.lnk"),
            desktop.clone(),
        ];
        let plans = plan_move_in(&sources, &root, std::slice::from_ref(&desktop), &fence);
        assert_eq!(plans.len(), sources.len());

        let job_name = |plan: &Plan| match plan {
            Plan::Job(job) => {
                assert_eq!(job.dest_dir, fence);
                Some(job.dest_name.clone())
            }
            Plan::Done(_) => None,
        };
        let status = |plan: &Plan| match plan {
            Plan::Done(result) => Some(result.status),
            Plan::Job(_) => None,
        };
        assert_eq!(job_name(&plans[0]).as_deref(), Some("a.lnk"));
        // 收纳箱里已有 b.lnk；同一批里第二个 b.lnk 再顺延
        assert_eq!(job_name(&plans[1]).as_deref(), Some("b (2).lnk"));
        assert_eq!(job_name(&plans[2]).as_deref(), Some("b (3).lnk"));
        assert_eq!(status(&plans[3]), Some(ItemOpStatus::Skipped));
        assert_eq!(status(&plans[4]), Some(ItemOpStatus::Failed));
        assert_eq!(status(&plans[5]), Some(ItemOpStatus::Skipped));
        assert_eq!(status(&plans[6]), Some(ItemOpStatus::Failed));
        assert_eq!(status(&plans[7]), Some(ItemOpStatus::Failed));
        assert_eq!(status(&plans[8]), Some(ItemOpStatus::Failed));
        // 别的收纳箱里的项可以拖进来（跨箱）
        assert_eq!(job_name(&plans[9]).as_deref(), Some("c.lnk"));
        // 重复源路径
        assert_eq!(status(&plans[10]), Some(ItemOpStatus::Skipped));
        assert_eq!(status(&plans[11]), Some(ItemOpStatus::Failed));
        // 桌面文件夹本身（其中的项可以收纳，上面 a.lnk / b.lnk 即是）
        assert_eq!(status(&plans[12]), Some(ItemOpStatus::Failed));
    }

    #[test]
    fn plan_restore_picks_dir_and_name() {
        let tmp = TempDir::new("plan-restore");
        let root = tmp.path().join("root");
        let fence = root.join("箱");
        let desktop = tmp.path().join("desktop");
        let tools = tmp.path().join("tools");
        fs::create_dir_all(&fence).unwrap();
        fs::create_dir_all(&desktop).unwrap();
        fs::create_dir_all(&tools).unwrap();
        touch(&fence.join("Chrome (2).lnk"));
        touch(&fence.join("x.txt"));
        touch(&desktop.join("Chrome.lnk"));

        // 按"所有目标目录都列举"的规则（Windows / Linux）；macOS 的规则见下一个测试
        let mut reserved = Reserved::with_policy(&root, true);
        // 原目录存在，原名可用 → 恢复原名
        let j = plan_restore(
            &fence.join("Chrome (2).lnk"),
            false,
            Some(&tools.join("Chrome.lnk")),
            &desktop,
            &root,
            &mut reserved,
        );
        assert_eq!(j.dest_dir, tools);
        assert_eq!(j.dest_name, "Chrome.lnk");

        // 回桌面且桌面已有同名 → 加后缀
        let j = plan_restore(
            &fence.join("Chrome (2).lnk"),
            false,
            Some(&tmp.path().join("gone").join("Chrome.lnk")),
            &desktop,
            &root,
            &mut reserved,
        );
        assert_eq!(j.dest_dir, desktop);
        assert_eq!(j.dest_name, "Chrome (2).lnk");

        // 同一批再来一个同名（没有原始路径，保持当前名字 "Chrome (2).lnk"）→ 已被预订，再加后缀
        let j = plan_restore(
            &fence.join("Chrome (2).lnk"),
            false,
            None,
            &desktop,
            &root,
            &mut reserved,
        );
        assert_eq!(j.dest_name, "Chrome (2) (2).lnk");

        // 没有原始路径
        let j = plan_restore(
            &fence.join("x.txt"),
            false,
            None,
            &desktop,
            &root,
            &mut reserved,
        );
        assert_eq!(j.dest_dir, desktop);
        assert_eq!(j.dest_name, "x.txt");
    }

    #[test]
    fn protected_targets_outside_the_root_are_not_listed() {
        // macOS 规则：不列举收纳根目录以外的目标目录（列举桌面会弹隐私授权框），只按本批预订的名字去重；
        // 与磁盘上已有项的重名交给执行层撞名顺延
        let tmp = TempDir::new("plan-unlisted");
        let root = tmp.path().join("root");
        let fence = root.join("箱");
        let desktop = tmp.path().join("desktop");
        touch(&fence.join("a.txt"));
        touch(&fence.join("b.txt"));
        touch(&desktop.join("a.txt"));
        touch(&desktop.join("b.txt"));

        let mut reserved = Reserved::with_policy(&root, false);
        let j = plan_restore(
            &fence.join("a.txt"),
            false,
            None,
            &desktop,
            &root,
            &mut reserved,
        );
        assert_eq!(j.dest_dir, desktop);
        assert_eq!(j.dest_name, "a.txt", "桌面没有被列举");
        // 同一批里的同名项仍按预订去重
        let j = plan_restore(
            &fence.join("a.txt"),
            false,
            None,
            &desktop,
            &root,
            &mut reserved,
        );
        assert_eq!(j.dest_name, "a (2).txt");
        // 收纳根目录内的目标照常列举
        assert_eq!(reserved.pick(&fence, "b.txt", false), "b (2).txt");

        // 所有平台的默认规则：只有 macOS 不列举
        let mut reserved = Reserved::new(&root);
        let expected = if cfg!(target_os = "macos") {
            "b.txt"
        } else {
            "b (2).txt"
        };
        assert_eq!(reserved.pick(&desktop, "b.txt", false), expected);
    }

    #[test]
    fn invalid_names_are_rejected_before_touching_files() {
        let tmp = TempDir::new("invalid");
        let root = tmp.path();
        let desktop = tmp.path().join("desktop");
        let exec = fake_exec(&[], &[]);
        assert!(restore_item_at(root, &desktop, "..", "a", None, &exec).is_err());
        fs::create_dir_all(root.join("箱")).unwrap();
        assert!(restore_item_at(root, &desktop, "箱", "../x", None, &exec).is_err());
        assert!(restore_item_at(root, &desktop, "箱", "a\\x", None, &exec).is_err());
        assert!(move_between_at(root, "箱", "a", "箱", &exec).is_err());
        assert!(move_between_at(root, "箱", "a", "../x", &exec).is_err());
        assert!(move_into_at(root, &[], &[desktop.join("a")], "a/b", &exec).is_err());
    }

    #[test]
    fn missing_item_reports_failed() {
        let tmp = TempDir::new("missing");
        let root = tmp.path().join("root");
        let desktop = tmp.path().join("desktop");
        let exec = fake_exec(&[], &[]);
        fs::create_dir_all(root.join("箱")).unwrap();
        let r = restore_item_at(&root, &desktop, "箱", "nope.lnk", None, &exec).unwrap();
        assert_eq!(r.status, ItemOpStatus::Failed);
        let r = move_between_at(&root, "箱", "nope.lnk", "另一个", &exec).unwrap();
        assert_eq!(r.status, ItemOpStatus::Failed);
    }

    #[test]
    fn restore_all_on_missing_folder_is_noop() {
        let tmp = TempDir::new("restore-missing");
        let exec = fake_exec(&[], &[]);
        let r = restore_all_at(
            tmp.path(),
            &tmp.path().join("desktop"),
            "不存在",
            &HashMap::new(),
            &exec,
        )
        .unwrap();
        assert!(r.folder_removed && r.items.is_empty() && r.remaining.is_empty());
    }

    #[test]
    fn move_in_results_keep_input_order() {
        let tmp = TempDir::new("order");
        let root = tmp.path().join("root");
        let desktop = tmp.path().join("desktop");
        touch(&desktop.join("a.lnk"));
        touch(&desktop.join("busy.lnk"));
        touch(&desktop.join("uac.lnk"));
        touch(&desktop.join("desktop.ini"));
        let exec = fake_exec(&["busy.lnk"], &["uac.lnk"]);
        let sources = vec![
            desktop.join("desktop.ini"),
            desktop.join("a.lnk"),
            desktop.join("busy.lnk"),
            desktop.join("uac.lnk"),
        ];
        let results = move_into_at(&root, &[], &sources, "新箱", &exec).unwrap();
        let statuses: Vec<ItemOpStatus> = results.iter().map(|r| r.status).collect();
        assert_eq!(
            statuses,
            vec![
                ItemOpStatus::Skipped,
                ItemOpStatus::Moved,
                ItemOpStatus::Failed,
                ItemOpStatus::Cancelled
            ]
        );
        for (result, source) in results.iter().zip(&sources) {
            assert_eq!(PathBuf::from(&result.source), *source);
        }
        assert_eq!(results[1].file_name.as_deref(), Some("a.lnk"));
        // 收纳文件夹不存在时自动创建
        assert!(root.join("新箱").join("a.lnk").is_file());
        assert!(desktop.join("busy.lnk").is_file());
        assert!(desktop.join("uac.lnk").is_file());
    }

    #[test]
    fn restore_all_keeps_folder_when_items_remain() {
        let tmp = TempDir::new("restore-remain");
        let root = tmp.path().join("root");
        let desktop = tmp.path().join("desktop");
        let origin = tmp.path().join("origin");
        fs::create_dir_all(&desktop).unwrap();
        fs::create_dir_all(&origin).unwrap();
        let fence = root.join("箱");
        touch(&fence.join("ok.lnk"));
        touch(&fence.join("busy.txt"));
        touch(&fence.join("uac.lnk"));
        touch(&fence.join("Desktop.ini"));
        touch(&desktop.join("ok.lnk"));

        let mut originals = HashMap::new();
        // 原始路径按文件名不区分大小写匹配
        originals.insert(
            "OK.LNK".to_string(),
            origin.join("ok.lnk").to_string_lossy().into_owned(),
        );
        let exec = fake_exec(&["busy.txt"], &["uac.lnk"]);
        let r = restore_all_at(&root, &desktop, "箱", &originals, &exec).unwrap();
        assert!(!r.folder_removed);
        assert_eq!(
            r.remaining,
            vec!["busy.txt".to_string(), "uac.lnk".to_string()]
        );
        assert_eq!(r.items.len(), 3);
        assert!(origin.join("ok.lnk").is_file());
        // desktop.ini 不移动、仍在（文件夹没删）
        assert!(fence.join("Desktop.ini").is_file());

        // 剩下的处理掉之后再还原：文件夹被删除
        let exec = fake_exec(&[], &[]);
        let r = restore_all_at(&root, &desktop, "箱", &HashMap::new(), &exec).unwrap();
        assert!(r.folder_removed, "{r:#?}");
        assert!(r.remaining.is_empty());
        assert!(desktop.join("busy.txt").is_file());
        assert!(desktop.join("uac.lnk").is_file());
        assert!(!fence.exists());
    }

    #[test]
    fn cross_fence_move_with_fake_executor() {
        let tmp = TempDir::new("cross");
        let root = tmp.path().join("root");
        touch(&root.join("甲").join("a.lnk"));
        touch(&root.join("乙").join("a.lnk"));
        let exec = fake_exec(&[], &[]);
        let r = move_between_at(&root, "甲", "a.lnk", "乙", &exec).unwrap();
        assert_eq!(r.status, ItemOpStatus::Moved);
        assert_eq!(r.file_name.as_deref(), Some("a (2).lnk"));
        // 目标收纳箱不存在时自动创建
        touch(&root.join("甲").join("b.lnk"));
        let r = move_between_at(&root, "甲", "b.lnk", "丙", &exec).unwrap();
        assert_eq!(r.file_name.as_deref(), Some("b.lnk"));
        assert!(root.join("丙").join("b.lnk").is_file());
    }

    /// 真实执行 IFileOperation（只在临时目录里，不涉及需要提权或会弹对话框的场景）
    #[cfg(windows)]
    mod real_moves {
        use super::*;

        fn shell(jobs: &[MoveJob]) -> Result<BatchOutcome, String> {
            execute(jobs, None)
        }

        #[test]
        fn move_in_restore_and_cross_fence_round_trip() {
            let tmp = TempDir::new("real");
            let root = tmp.path().join("root");
            let desktop = tmp.path().join("desktop");
            let elsewhere = tmp.path().join("elsewhere");
            fs::create_dir_all(&desktop).unwrap();
            fs::create_dir_all(&elsewhere).unwrap();
            fs::create_dir_all(root.join("箱A")).unwrap();
            touch(&desktop.join("App.lnk"));
            touch(&elsewhere.join("App.lnk"));
            touch(&desktop.join("doc.txt"));
            touch(&desktop.join("文件夹").join("sub").join("f.txt"));
            touch(&desktop.join("desktop.ini"));

            // 移入：重名自动加后缀，结果顺序与输入一致
            let sources = vec![
                desktop.join("App.lnk"),
                elsewhere.join("App.lnk"),
                desktop.join("文件夹"),
                desktop.join("desktop.ini"),
                desktop.join("missing.lnk"),
            ];
            let results = move_into_at(&root, &[], &sources, "箱A", &shell).unwrap();
            let statuses: Vec<ItemOpStatus> = results.iter().map(|r| r.status).collect();
            assert_eq!(
                statuses,
                vec![
                    ItemOpStatus::Moved,
                    ItemOpStatus::Moved,
                    ItemOpStatus::Moved,
                    ItemOpStatus::Skipped,
                    ItemOpStatus::Failed
                ],
                "{results:#?}"
            );
            assert_eq!(results[0].file_name.as_deref(), Some("App.lnk"));
            assert_eq!(results[1].file_name.as_deref(), Some("App (2).lnk"));
            assert_eq!(results[2].file_name.as_deref(), Some("文件夹"));
            assert_eq!(PathBuf::from(&results[1].source), elsewhere.join("App.lnk"));
            assert!(!desktop.join("App.lnk").exists());
            assert!(root.join("箱A").join("App (2).lnk").is_file());
            assert!(root
                .join("箱A")
                .join("文件夹")
                .join("sub")
                .join("f.txt")
                .is_file());
            assert!(desktop.join("desktop.ini").is_file());

            // 移回：原目录存在 → 回原目录并恢复原名
            let original = elsewhere.join("App.lnk").to_string_lossy().into_owned();
            let r = restore_item_at(
                &root,
                &desktop,
                "箱A",
                "App (2).lnk",
                Some(&original),
                &shell,
            )
            .unwrap();
            assert_eq!(r.status, ItemOpStatus::Moved, "{r:#?}");
            assert_eq!(
                PathBuf::from(r.dest_path.unwrap()),
                elsewhere.join("App.lnk")
            );

            // 移回：桌面上已有同名 → 不覆盖，加后缀
            fs::write(desktop.join("App.lnk"), b"desktop-copy").unwrap();
            let r = restore_item_at(&root, &desktop, "箱A", "App.lnk", None, &shell).unwrap();
            assert_eq!(r.status, ItemOpStatus::Moved);
            assert_eq!(r.file_name.as_deref(), Some("App (2).lnk"));
            assert_eq!(fs::read(desktop.join("App.lnk")).unwrap(), b"desktop-copy");

            // 跨箱移动：重名加后缀
            let results =
                move_into_at(&root, &[], &[desktop.join("doc.txt")], "箱A", &shell).unwrap();
            assert_eq!(results[0].status, ItemOpStatus::Moved);
            touch(&root.join("箱B").join("doc.txt"));
            let r = move_between_at(&root, "箱A", "doc.txt", "箱B", &shell).unwrap();
            assert_eq!(r.status, ItemOpStatus::Moved, "{r:#?}");
            assert_eq!(r.file_name.as_deref(), Some("doc (2).txt"));
            assert!(root.join("箱B").join("doc (2).txt").is_file());

            // 批量还原：文件夹清空后删除（desktop.ini 不算剩余）
            let mut originals = HashMap::new();
            originals.insert(
                "文件夹".to_string(),
                desktop.join("文件夹").to_string_lossy().into_owned(),
            );
            touch(&root.join("箱A").join("desktop.ini"));
            let r = restore_all_at(&root, &desktop, "箱A", &originals, &shell).unwrap();
            assert!(r.folder_removed, "{r:#?}");
            assert!(r.remaining.is_empty());
            assert_eq!(r.items.len(), 1);
            assert!(desktop.join("文件夹").join("sub").join("f.txt").is_file());
            assert!(!root.join("箱A").exists());

            let r = restore_all_at(&root, &desktop, "箱B", &HashMap::new(), &shell).unwrap();
            assert!(r.folder_removed, "{r:#?}");
            assert_eq!(r.items.len(), 2);
            assert!(desktop.join("doc.txt").is_file());
            assert!(desktop.join("doc (2).txt").is_file());
        }
    }

    /// 真实执行 macOS 的 renamex_np 移动（只在临时目录里；不碰 ~/Desktop 等受隐私保护的目录）
    #[cfg(target_os = "macos")]
    mod real_moves_mac {
        use super::super::super::folder::mac_test_support::{clear_flags, lock, set_hidden_flag};
        use super::super::super::mac;
        use super::*;

        fn exec(jobs: &[MoveJob]) -> Result<BatchOutcome, String> {
            execute(jobs, None)
        }

        #[test]
        fn move_in_restore_and_cross_fence_round_trip() {
            let tmp = TempDir::new("real-mac");
            let root = tmp.path().join("root");
            let desktop = tmp.path().join("desktop");
            let elsewhere = tmp.path().join("elsewhere");
            fs::create_dir_all(&desktop).unwrap();
            fs::create_dir_all(&elsewhere).unwrap();
            fs::create_dir_all(root.join("箱A")).unwrap();
            touch(&desktop.join("App.lnk"));
            touch(&elsewhere.join("App.lnk"));
            touch(&desktop.join("doc.txt"));
            touch(&desktop.join("文件夹").join("sub").join("f.txt"));
            touch(
                &desktop
                    .join("Safari.app")
                    .join("Contents")
                    .join("Info.plist"),
            );
            touch(
                &root
                    .join("箱A")
                    .join("Safari.app")
                    .join("Contents")
                    .join("Info.plist"),
            );
            touch(&desktop.join("desktop.ini"));
            touch(&desktop.join(".DS_Store"));
            touch(&desktop.join(".env"));
            touch(&desktop.join("hidden.txt"));
            set_hidden_flag(&desktop.join("hidden.txt"));

            // 移入：重名自动加后缀（包按文件处理），结果顺序与输入一致；点文件照常移入，UF_HIDDEN 的不收纳
            let sources = vec![
                desktop.join("App.lnk"),
                elsewhere.join("App.lnk"),
                desktop.join("文件夹"),
                desktop.join("Safari.app"),
                desktop.join(".env"),
                desktop.join("hidden.txt"),
                desktop.join("desktop.ini"),
                desktop.join(".DS_Store"),
                desktop.join("missing.lnk"),
            ];
            let results = move_into_at(&root, &[], &sources, "箱A", &exec).unwrap();
            let statuses: Vec<ItemOpStatus> = results.iter().map(|r| r.status).collect();
            assert_eq!(
                statuses,
                vec![
                    ItemOpStatus::Moved,
                    ItemOpStatus::Moved,
                    ItemOpStatus::Moved,
                    ItemOpStatus::Moved,
                    ItemOpStatus::Moved,
                    ItemOpStatus::Skipped,
                    ItemOpStatus::Skipped,
                    ItemOpStatus::Skipped,
                    ItemOpStatus::Failed
                ],
                "{results:#?}"
            );
            assert_eq!(results[0].file_name.as_deref(), Some("App.lnk"));
            assert_eq!(results[1].file_name.as_deref(), Some("App (2).lnk"));
            assert_eq!(results[2].file_name.as_deref(), Some("文件夹"));
            assert_eq!(results[3].file_name.as_deref(), Some("Safari (2).app"));
            assert_eq!(results[4].file_name.as_deref(), Some(".env"));
            assert_eq!(PathBuf::from(&results[1].source), elsewhere.join("App.lnk"));
            assert!(!desktop.join("App.lnk").exists());
            assert!(root.join("箱A").join("App (2).lnk").is_file());
            assert!(root
                .join("箱A")
                .join("文件夹")
                .join("sub")
                .join("f.txt")
                .is_file());
            assert!(root
                .join("箱A")
                .join("Safari (2).app")
                .join("Contents")
                .join("Info.plist")
                .is_file());
            assert!(desktop.join("hidden.txt").is_file());
            assert!(desktop.join("desktop.ini").is_file());
            assert!(desktop.join(".DS_Store").is_file());

            // 移回：原目录存在 → 回原目录并恢复原名
            let original = elsewhere.join("App.lnk").to_string_lossy().into_owned();
            let r = restore_item_at(
                &root,
                &desktop,
                "箱A",
                "App (2).lnk",
                Some(&original),
                &exec,
            )
            .unwrap();
            assert_eq!(r.status, ItemOpStatus::Moved, "{r:#?}");
            assert_eq!(
                PathBuf::from(r.dest_path.unwrap()),
                elsewhere.join("App.lnk")
            );

            // 移回：桌面上已有同名（桌面不列举，靠 RENAME_EXCL 撞名顺延）→ 不覆盖，加后缀
            fs::write(desktop.join("App.lnk"), b"desktop-copy").unwrap();
            let r = restore_item_at(&root, &desktop, "箱A", "App.lnk", None, &exec).unwrap();
            assert_eq!(r.status, ItemOpStatus::Moved, "{r:#?}");
            assert_eq!(r.file_name.as_deref(), Some("App (2).lnk"));
            assert_eq!(fs::read(desktop.join("App.lnk")).unwrap(), b"desktop-copy");

            // 包移回桌面：桌面已有同名包（原来那个已经移走，这里放一个新的）→ `Safari (2).app`
            touch(&desktop.join("Safari.app").join("mine"));
            let original = desktop.join("Safari.app").to_string_lossy().into_owned();
            let r = restore_item_at(
                &root,
                &desktop,
                "箱A",
                "Safari (2).app",
                Some(&original),
                &exec,
            )
            .unwrap();
            assert_eq!(r.status, ItemOpStatus::Moved, "{r:#?}");
            assert_eq!(r.file_name.as_deref(), Some("Safari (2).app"));
            assert!(desktop.join("Safari.app").join("mine").is_file());

            // 跨箱移动：重名加后缀
            let results =
                move_into_at(&root, &[], &[desktop.join("doc.txt")], "箱A", &exec).unwrap();
            assert_eq!(results[0].status, ItemOpStatus::Moved);
            touch(&root.join("箱B").join("doc.txt"));
            let r = move_between_at(&root, "箱A", "doc.txt", "箱B", &exec).unwrap();
            assert_eq!(r.status, ItemOpStatus::Moved, "{r:#?}");
            assert_eq!(r.file_name.as_deref(), Some("doc (2).txt"));
            assert!(root.join("箱B").join("doc (2).txt").is_file());

            // 批量还原：文件夹清空后删除（访达写的 .DS_Store / Icon\r 不算剩余、不移回桌面）
            let mut originals = HashMap::new();
            originals.insert(
                "文件夹".to_string(),
                desktop.join("文件夹").to_string_lossy().into_owned(),
            );
            touch(&root.join("箱A").join(".DS_Store"));
            touch(&root.join("箱A").join("Icon\r"));
            let r = restore_all_at(&root, &desktop, "箱A", &originals, &exec).unwrap();
            assert!(r.folder_removed, "{r:#?}");
            assert!(r.remaining.is_empty());
            // 剩下的：文件夹、Safari.app（原名在箱里）、.env
            assert_eq!(r.items.len(), 3, "{r:#?}");
            assert!(
                r.items.iter().all(|i| i.status == ItemOpStatus::Moved),
                "{r:#?}"
            );
            assert!(desktop.join("文件夹").join("sub").join("f.txt").is_file());
            assert!(desktop.join(".env").is_file());
            assert_eq!(fs::read(desktop.join(".DS_Store")).unwrap(), b"x");
            assert!(!desktop.join(".DS_Store (2)").exists());
            assert!(!root.join("箱A").exists());

            let r = restore_all_at(&root, &desktop, "箱B", &HashMap::new(), &exec).unwrap();
            assert!(r.folder_removed, "{r:#?}");
            assert_eq!(r.items.len(), 2);
            assert!(desktop.join("doc.txt").is_file());
            assert!(desktop.join("doc (2).txt").is_file());
        }

        #[test]
        fn rename_excl_never_overwrites_and_tries_the_next_name() {
            let tmp = TempDir::new("excl-mac");
            let src_dir = tmp.path().join("src");
            let dest = tmp.path().join("dest");
            touch(&src_dir.join("a.txt"));
            fs::create_dir_all(&dest).unwrap();
            fs::write(dest.join("a.txt"), b"keep-1").unwrap();
            fs::write(dest.join("a (2).txt"), b"keep-2").unwrap();
            let next = |n: u32| format!("a ({n}).txt");
            let r = mac::move_item(&src_dir.join("a.txt"), &dest, "a.txt", &next);
            assert_eq!(r.final_path, Some(dest.join("a (3).txt")), "{r:?}");
            assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"keep-1");
            assert_eq!(fs::read(dest.join("a (2).txt")).unwrap(), b"keep-2");
            assert!(!src_dir.join("a.txt").exists());

            // 只差大小写：不区分大小写的卷（APFS 默认）上同样算重名，不覆盖
            let case_insensitive = dest.join("A.TXT").exists();
            touch(&src_dir.join("A.TXT"));
            let r = mac::move_item(&src_dir.join("A.TXT"), &dest, "A.TXT", &|n| {
                format!("A ({n}).TXT")
            });
            let moved_to = r.final_path.expect("应移动成功");
            if case_insensitive {
                assert_ne!(moved_to, dest.join("A.TXT"));
            }
            assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"keep-1");

            // 名字用完：失败，源保留
            touch(&src_dir.join("b.txt"));
            touch(&dest.join("b.txt"));
            let r = mac::move_item(&src_dir.join("b.txt"), &dest, "b.txt", &|_| "b.txt".into());
            assert_eq!(r.final_path, None);
            assert_eq!(r.errno, Some(libc::EEXIST));
            assert!(src_dir.join("b.txt").is_file());

            // 源不存在
            let r = mac::move_item(&src_dir.join("nope"), &dest, "nope", &next);
            assert_eq!(r.errno, Some(libc::ENOENT));
            assert_eq!(r.message.as_deref(), Some("文件不存在"));
        }

        #[test]
        fn locked_items_fail_with_a_clear_reason_and_stay_put() {
            let tmp = TempDir::new("locked-mac");
            let root = tmp.path().join("root");
            let desktop = tmp.path().join("desktop");
            let item = desktop.join("locked.txt");
            touch(&item);
            lock(&item);
            let results =
                move_into_at(&root, &[], std::slice::from_ref(&item), "箱", &exec).unwrap();
            clear_flags(&item);
            assert_eq!(results[0].status, ItemOpStatus::Failed, "{results:#?}");
            let error = results[0].error.as_deref().unwrap_or_default();
            assert!(error.contains("锁定"), "{error}");
            assert!(item.is_file());
        }

        #[test]
        fn icloud_placeholders_are_recognized() {
            let tmp = TempDir::new("icloud-mac");
            touch(&tmp.path().join(".报告.pages.icloud"));
            assert!(mac::has_legacy_icloud_stub(&tmp.path().join("报告.pages")));
            assert!(!mac::has_legacy_icloud_stub(&tmp.path().join("别的.pages")));
            // 临时目录不在 iCloud 里
            touch(&tmp.path().join("local.txt"));
            assert!(!mac::is_ubiquitous_item(&tmp.path().join("local.txt")));
            assert!(!mac::is_ubiquitous_item(&tmp.path().join("missing.txt")));
        }
    }
}
