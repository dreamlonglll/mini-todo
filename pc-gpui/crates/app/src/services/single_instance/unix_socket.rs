//! Unix domain socket 单实例（macOS 使用），与 2.5.0 mac 版的 tauri-plugin-single-instance 2.3.7（`platform_impl/macos.rs`，
//! 未开 `semver` 特性）**同名同格式**，新旧两版互相识别：
//!
//! - socket：`/tmp/<identifier 中 '.' '-' 换成 '_'>_si.sock`，即 `/tmp/com_tauri_app_mini_todo_si.sock`
//!   （socket 路径不能超过约 100 字节，所以放 `/tmp`）
//! - 第二个实例 `connect` 成功：写 `<cwd>` + `"\0\0"` + `args.join("\0")`，关闭连接后退出
//! - `connect` 得 `NotFound` / `ConnectionRefused`：没有实例在运行（或上次崩溃留下的残留文件）→ 删掉残留 → `bind` 成为首个实例
//! - 其它错误：照常启动，不做单实例
//! - 首个实例对每个连接读到 EOF，按第一个 `"\0\0"` 切出 cwd 与参数；退出时删除 socket 文件
//!
//! 只用 std 与 log，不依赖 GPUI：单测在 Linux 上也跑，`pc-gpui/scripts/mac-check` 按 `aarch64-apple-darwin` 交叉检查。

use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 读一条消息的超时：旧版 / 新版第二个实例都是写完立即关闭，卡住的连接不能挡住后面的唤起请求
const READ_TIMEOUT: Duration = Duration::from_secs(2);

/// socket 路径（与 tauri-plugin-single-instance 2.3.7 mac 的 `socket_path` 一致）
pub fn socket_path(identifier: &str) -> PathBuf {
    let name = identifier.replace(['.', '-'], "_");
    PathBuf::from(format!("/tmp/{name}_si.sock"))
}

/// 第二个实例发送的内容：`<cwd>\0\0<arg0>\0<arg1>…`
pub fn encode_payload(cwd: &str, args: &[String]) -> String {
    format!("{cwd}\0\0{}", args.join("\0"))
}

/// 解析 [`encode_payload`] 的内容 → `(cwd, args)`；没有 `"\0\0"` 分隔（不是本协议的消息，如空连接）返回 `None`
pub fn parse_payload(data: &str) -> Option<(String, Vec<String>)> {
    let (cwd, args) = data.split_once("\0\0")?;
    let args = if args.is_empty() {
        Vec::new()
    } else {
        args.split('\0').map(str::to_string).collect()
    };
    Some((cwd.to_string(), args))
}

/// [`acquire`] 的结果
pub enum Acquired {
    /// 首个实例：已 bind，持有监听 socket
    Primary(Server),
    /// 已有实例在运行，参数已转交：调用方应退出
    Forwarded,
    /// 无法判断（connect / bind 的其它错误）：照常启动，不做单实例
    Standalone(std::io::Error),
}

/// 先尝试把 `payload` 转交给已运行的实例；没有实例在运行时成为首个实例
pub fn acquire(path: &Path, payload: &str) -> Acquired {
    match forward(path, payload) {
        Ok(()) => Acquired::Forwarded,
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => {
            // 残留的 socket 文件（上次崩溃 / 被强制结束）：没有人在监听，删掉重建
            let _ = std::fs::remove_file(path);
            match UnixListener::bind(path) {
                Ok(listener) => Acquired::Primary(Server {
                    identity: file_identity(path),
                    path: path.to_path_buf(),
                    listener,
                }),
                Err(e) => Acquired::Standalone(e),
            }
        }
        Err(e) => Acquired::Standalone(e),
    }
}

/// 连上已运行的实例并写入消息；返回前关闭连接（对方据此读到 EOF）
fn forward(path: &Path, payload: &str) -> std::io::Result<()> {
    let mut stream = UnixStream::connect(path)?;
    stream.write_all(payload.as_bytes())?;
    stream.flush()
}

/// socket 文件的身份：退出时只删自己 bind 的那个文件。
///
/// 只比 (设备, inode) 不够：文件被删后立刻新建的文件常常复用同一个 inode 号（tmpfs 上实测），所以再比较文件的
/// 状态改变时间 / 修改时间（秒 + 纳秒；新建文件时取当时的时间，此后没人改它）与创建时间（macOS APFS 的 birthtime，
/// Linux statx 的 btime，文件系统不支持时为 `None`）。新文件晚于我们 bind，只要时钟走过了一个刻度，这几项就不同。
/// 判断失误的方向是保守的：我们的文件元数据被改过时认不出来、不删，残留文件由下次启动 connect 得
/// ConnectionRefused 时清理
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
    ctime: (i64, i64),
    mtime: (i64, i64),
    created: Option<SystemTime>,
}

fn file_identity(path: &Path) -> Option<FileIdentity> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    Some(FileIdentity {
        dev: meta.dev(),
        ino: meta.ino(),
        ctime: (meta.ctime(), meta.ctime_nsec()),
        mtime: (meta.mtime(), meta.mtime_nsec()),
        created: meta.created().ok(),
    })
}

/// 首个实例的监听 socket
pub struct Server {
    path: PathBuf,
    identity: Option<FileIdentity>,
    listener: UnixListener,
}

impl Server {
    /// 拆出监听 socket（交给接收线程）与退出时的清理句柄
    pub fn split(self) -> (UnixListener, SocketFile) {
        (
            self.listener,
            SocketFile {
                path: self.path,
                identity: self.identity,
            },
        )
    }
}

/// 首个实例 bind 的 socket 文件
pub struct SocketFile {
    path: PathBuf,
    identity: Option<FileIdentity>,
}

impl SocketFile {
    /// 退出时删除 socket 文件；文件已被别人换掉（如 /tmp 被清理后另一个实例重新 bind，哪怕复用了同一个 inode 号）时不删
    pub fn remove(&self) {
        if self.identity.is_some() && file_identity(&self.path) == self.identity {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// 逐个接收第二个实例的消息（阻塞，跑在专用线程里，直到 listener 出错关闭）；每条消息回调 `(cwd, args)`
pub fn serve(listener: UnixListener, mut on_message: impl FnMut(String, Vec<String>)) {
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
                let mut data = Vec::new();
                match stream.read_to_end(&mut data) {
                    Ok(_) => match parse_payload(&String::from_utf8_lossy(&data)) {
                        Some((cwd, args)) => on_message(cwd, args),
                        None => log::warn!(
                            "[app] 单实例收到无法识别的消息（{} 字节），忽略",
                            data.len()
                        ),
                    },
                    Err(e) => log::warn!("[app] 读取单实例消息失败: {e}"),
                }
            }
            Err(e) => {
                log::warn!("[app] 接收单实例连接失败: {e}");
                // 持续出错（如文件描述符耗尽）时不空转
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_matches_tauri_plugin() {
        assert_eq!(
            socket_path("com.tauri-app.mini-todo"),
            PathBuf::from("/tmp/com_tauri_app_mini_todo_si.sock")
        );
    }

    #[test]
    fn payload_round_trip_matches_tauri_format() {
        let args = vec![
            "/Applications/mini-todo.app/Contents/MacOS/mini-todo".to_string(),
            "--autostart".to_string(),
        ];
        let payload = encode_payload("/Users/me", &args);
        assert_eq!(
            payload,
            "/Users/me\0\0/Applications/mini-todo.app/Contents/MacOS/mini-todo\0--autostart"
        );
        assert_eq!(
            parse_payload(&payload),
            Some(("/Users/me".to_string(), args))
        );
        // 2.5.0 发的消息与新版同格式；cwd 取不到时为空串
        assert_eq!(
            parse_payload("\0\0/bin/mini-todo"),
            Some((String::new(), vec!["/bin/mini-todo".to_string()]))
        );
        assert_eq!(
            parse_payload("/tmp\0\0"),
            Some(("/tmp".to_string(), vec![]))
        );
        assert_eq!(parse_payload(""), None);
        assert_eq!(parse_payload("garbage"), None);
    }

    /// 测试用的短路径（socket 路径有长度上限，不用 `temp_dir()` 下的深目录）
    fn test_socket(tag: &str) -> PathBuf {
        PathBuf::from(format!("/tmp/mt_si_test_{}_{tag}.sock", std::process::id()))
    }

    #[test]
    fn second_instance_forwards_to_primary() {
        let path = test_socket("fwd");
        let _ = std::fs::remove_file(&path);
        let Acquired::Primary(server) = acquire(&path, "unused") else {
            panic!("第一个应成为首个实例");
        };
        let (listener, file) = server.split();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || serve(listener, |cwd, args| tx.send((cwd, args)).unwrap()));

        let payload = encode_payload("/Users/me", &["/bin/mini-todo".to_string()]);
        assert!(matches!(acquire(&path, &payload), Acquired::Forwarded));
        let received = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            received,
            ("/Users/me".to_string(), vec!["/bin/mini-todo".to_string()])
        );

        // 空连接（不是本协议的消息）被忽略，后面的消息照常处理
        drop(UnixStream::connect(&path).unwrap());
        assert!(matches!(
            acquire(&path, &encode_payload("/", &["a".to_string()])),
            Acquired::Forwarded
        ));
        let received = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(received, ("/".to_string(), vec!["a".to_string()]));

        file.remove();
        assert!(!path.exists());
    }

    #[test]
    fn stale_socket_file_is_replaced() {
        let path = test_socket("stale");
        let _ = std::fs::remove_file(&path);
        // 上次崩溃留下的 socket 文件：文件在，但没有人监听（ConnectionRefused）
        drop(UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        let Acquired::Primary(server) = acquire(&path, "unused") else {
            panic!("残留文件应被替换");
        };
        let (_listener, file) = server.split();
        assert!(UnixStream::connect(&path).is_ok());
        file.remove();
        assert!(!path.exists());
    }

    #[test]
    fn cleanup_keeps_a_replaced_socket_file() {
        let path = test_socket("replaced");
        let _ = std::fs::remove_file(&path);
        let Acquired::Primary(server) = acquire(&path, "unused") else {
            panic!("第一个应成为首个实例");
        };
        let (listener, file) = server.split();
        drop(listener);
        let ours = file_identity(&path).unwrap();
        // 文件被删后另一个实例重新 bind 了同名 socket：退出清理不能删掉它。
        // 新文件常常复用同一个 inode 号（tmpfs）——正是要覆盖的情况；隔开几个时钟刻度（Linux 粗粒度时钟一刻最长 10ms），
        // 模拟"另一个实例稍后才 bind"，判断靠的是实现里的时间戳比较，与 inode 号是否复用无关
        std::fs::remove_file(&path).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let other = UnixListener::bind(&path).unwrap();
        let theirs = file_identity(&path).unwrap();
        assert_ne!(ours, theirs);
        file.remove();
        assert!(path.exists());
        drop(other);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn identity_differs_when_inode_number_is_reused() {
        let path = test_socket("identity");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let ours = file_identity(&path).unwrap();
        // 同一个文件读两次：身份不变（自己 bind 的文件退出时能删掉）
        assert_eq!(file_identity(&path), Some(ours.clone()));
        // 同设备同 inode 号、时间不同 = 新建的另一个文件
        let reused = FileIdentity {
            ctime: (ours.ctime.0 + 1, ours.ctime.1),
            mtime: (ours.mtime.0 + 1, ours.mtime.1),
            created: None,
            ..ours.clone()
        };
        assert_ne!(ours, reused);
        drop(listener);
        let _ = std::fs::remove_file(&path);
    }
}
