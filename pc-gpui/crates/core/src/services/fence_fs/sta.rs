//! COM STA 线程（仅 Windows）。
//!
//! Shell 对象（IFileOperation / IShellItemImageFactory / ShellExecuteExW / SHOpenFolderAndSelectItems）
//! 要求在 STA 中调用，而且不能放在 tao 主线程（会卡界面）或 tokio worker（可能已被初始化成 MTA，
//! 再调 `CoInitializeEx` 返回 `RPC_E_CHANGED_MODE`）上。这里提供两种执行方式：
//!
//! * [`run_in_new_sta`]：每次调用新开一个 STA 线程执行后退出。用于移动文件 / 启动 / 定位这类
//!   低频但可能长时间阻塞的操作（UAC 对话框、慢速启动），彼此之间互不排队。
//! * [`icon_worker`]：常驻 STA 线程按顺序处理图标提取任务，避免每个图标都初始化一次 COM。
//!
//! 两者对调用方都是**阻塞**的：调用线程会等待结果返回。

use std::sync::mpsc;
use std::sync::OnceLock;
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};

/// 当前线程的 COM 初始化守卫：`CoInitializeEx` 成功（含 `S_FALSE`）时在 drop 时 `CoUninitialize`
struct ComGuard;

impl ComGuard {
    fn init_sta() -> Result<Self, String> {
        // COINIT_DISABLE_OLE1DDE 是 MS 对 ShellExecuteEx 的推荐写法
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if hr.is_err() {
            return Err(format!(
                "初始化 COM 失败: {}",
                windows::core::Error::from(hr)
            ));
        }
        Ok(ComGuard)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

/// 新开一个 STA 线程执行 `f`，阻塞等待其返回。`f` panic 时返回错误而不是让调用方崩溃。
pub(crate) fn run_in_new_sta<T, F>(name: &str, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let handle = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            let _com = ComGuard::init_sta()?;
            f()
        })
        .map_err(|e| format!("创建 Shell 线程失败: {}", e))?;
    handle
        .join()
        .map_err(|_| "Shell 操作线程异常退出".to_string())?
}

type Job = Box<dyn FnOnce() + Send + 'static>;

/// 常驻 STA 线程：任务按提交顺序串行执行
pub(crate) struct StaWorker {
    name: &'static str,
    tx: mpsc::Sender<Job>,
}

impl StaWorker {
    fn spawn(name: &'static str) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || {
                let _com = match ComGuard::init_sta() {
                    Ok(guard) => guard,
                    Err(e) => {
                        log::error!("[fence] {} 线程 COM 初始化失败: {}", name, e);
                        // 直接退出：后续提交的任务会因为接收端已关闭而立即报错
                        return;
                    }
                };
                while let Ok(job) = rx.recv() {
                    // 单个任务 panic 不能带走整个工作线程；结果通道随之关闭，调用方收到错误
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                }
            })
            .map_err(|e| format!("创建 {} 线程失败: {}", name, e))?;
        Ok(StaWorker { name, tx })
    }

    /// 把 `f` 放到工作线程执行并阻塞等待结果
    pub(crate) fn run<T, F>(&self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let (result_tx, result_rx) = mpsc::channel::<T>();
        self.tx
            .send(Box::new(move || {
                let _ = result_tx.send(f());
            }))
            .map_err(|_| format!("{} 线程不可用", self.name))?;
        result_rx
            .recv()
            .map_err(|_| format!("{} 线程任务异常中止", self.name))
    }
}

/// 图标提取专用的常驻 STA 线程（首次使用时创建）
pub(crate) fn icon_worker() -> Result<&'static StaWorker, String> {
    static WORKER: OnceLock<Result<StaWorker, String>> = OnceLock::new();
    WORKER
        .get_or_init(|| StaWorker::spawn("fence-icon"))
        .as_ref()
        .map_err(|e| e.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_sta_returns_value_and_error() {
        assert_eq!(run_in_new_sta("t-ok", || Ok(42)).unwrap(), 42);
        let err = run_in_new_sta::<(), _>("t-err", || Err("坏了".to_string())).unwrap_err();
        assert_eq!(err, "坏了");
    }

    #[test]
    fn new_sta_survives_panic() {
        let err = run_in_new_sta::<(), _>("t-panic", || panic!("boom")).unwrap_err();
        assert!(err.contains("异常"));
    }

    #[test]
    fn worker_runs_jobs_in_order_and_survives_panic() {
        let worker = StaWorker::spawn("t-worker").unwrap();
        assert_eq!(worker.run(|| 1 + 1).unwrap(), 2);
        assert!(worker.run(|| -> i32 { panic!("boom") }).is_err());
        // panic 之后线程仍可用
        assert_eq!(worker.run(|| "ok").unwrap(), "ok");
    }
}
