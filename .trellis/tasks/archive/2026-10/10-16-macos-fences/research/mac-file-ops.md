# Research：macOS 收纳箱的文件操作与 AppKit API（移动 / TCC / iCloud / 图标 / 打开 / 垃圾文件 / 取色 / mac-check）

- **Query**：把桌面收纳箱的文件层（`fence_fs`：移入 / 移回 / 跨箱 / 删除收纳箱全部移回、打开、在访达中显示、图标、隐藏）
  与外观里的「自定义颜色」移植到 macOS。硬约束：绝不丢文件、绝不覆盖；core 函数阻塞、只在后台线程调用。
- **Scope**：mixed。来源包括本机 cargo registry 里的 objc2 生成源码、仓库代码、Apple 文档（developer.apple.com 的 JSON 文档接口）、
  Apple 开源的 xnu / swift-foundation 源码（raw.githubusercontent.com）、Apple 开发者论坛（DTS 回答），以及社区资料。
  eclecticlight.co、hacktricks.wiki、lapcatsoftware.com、bdash.net.nz、brunerd.com 被本会话的出口代理拒绝，只看到了搜索摘要，相关结论都标了出处等级。
- **Date**：2026-10-16
- **路径简写**：`REG` = `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`，`CORE` = `pc-gpui/crates/core/src/`，
  `FFS` = `CORE/services/fence_fs/`，`APP` = `pc-gpui/crates/app/src/`
- **验证方式**：文中所有 objc2 草案都在临时 crate 里用 `cargo clippy --offline --target aarch64-apple-darwin -- -D warnings`
  编译通过，edition 2021（core 用）与 2024（mac-check 用）两种都试过。版本钉死为 objc2 0.6.5、objc2-foundation 0.3.2、objc2-app-kit 0.3.2、
  block2 0.6.2、libc 0.2.180，直接沿用 `pc-gpui/scripts/mac-check/Cargo.lock`，没有联网解析。
  这只证明**类型和特性名正确**，运行时行为要靠 mac CI 或真机确认（第 10 节列了清单）。
- **标记**：「未验证」= 没找到可靠出处，或者只有社区说法，需要在 mac 上实测。

---

## 结论先行

1. **移动方案：`renamex_np(src, dst, RENAME_EXCL)` 为主，跨卷（EXDEV）才交给 `NSFileManager moveItemAtURL:toURL:error:`。**
   - RENAME_EXCL 由 xnu 在 VFS 层、持 rename 锁时判断目标是否存在，存在就返回 `EEXIST`，原子且不覆盖。
   - NSFileManager 在同卷时的实现（swift-foundation 源码）是先 `lstat(dst)` 再普通 `rename()`，中间有竞争窗口。窗口内出现的同名文件会被覆盖，满足不了"绝不覆盖"。
   - 跨卷时 NSFileManager 用 `copyfile(COPYFILE_CLONE|ALL|EXCL|NOFOLLOW)` 复制，复制本身不覆盖。复制失败会删掉目标里的半成品；删源失败不回滚，结果是两边各一份，但不会丢文件。
   - EEXIST 时按 `naming::suffixed_file_name` 顺延改名重试，等价于 Windows 的 `FOF_RENAMEONCOLLISION`。草案见 1.5，已编译验证。
2. **errno 能区分大部分失败原因。**
   - `EPERM` 来自 sandbox / TCC（Quinn："App Sandbox 或 MAC 拒绝都是 EPERM，BSD 权限 / ACL 是 EACCES"），也可能来自访达「已锁定」（uchg / schg，xnu `vnode_authorize_checkimmutable`）和 sticky 目录。
   - 先看源文件的 `st_flags` 排除"锁定"，剩下的按 TCC 处理，提示用户去「文件与文件夹」授权。
3. **TCC（桌面文件夹）对收纳箱的影响比预想小，但有一个必须改的坑。**
   - **拖放就是 user intent**（WWDC19-701 原话："dragging and dropping from another application … your app gets access to the file or files that the user selected without the need for a consent prompt"）。
   - **往受保护目录里新建不需要同意**（同一场演讲："user consent is not required for an app to create new documents in any of these protected locations, just to read existing contents"）。所以移入（把拖进来的项改名移出桌面）和移回（在桌面新建目录项）预计都不弹框。
   - **坑**：规划阶段 `Reserved::pick` → `existing_names_lower(dest_dir)` 会对目标目录做 `read_dir`。移回桌面时这就是"列出桌面内容"，会触发「想访问桌面文件夹中的文件」弹框。
   - ad-hoc 签名的包每次升级 cdhash 都变，授权会失效、重新弹框（Quinn："Unsigned or ad hoc-signed builds cause repeated prompts"）。
   - 所以 mac 上不能列举受保护的目标目录：改为靠 RENAME_EXCL 撞名重试，或逐个 lstat 探测候选名。
4. **TCC 弹框会阻塞调用线程，直到用户点按钮**（WWDC19-701："the calling thread is actually blocked"）。移动必须在后台线程做，现在本来就是。
   CI 的 macOS runner 有 GUI 会话但没人点，**e2e 绝不能碰 `~/Desktop` / `~/Documents` / `~/Downloads`**，否则线程会一直卡住。还要确保移回不会回落到真实桌面。
5. **iCloud「桌面与文稿」**
   - **判定**：对拖进来的项目本身取 `NSURLIsUbiquitousItemKey`。Apple DTS 的说法是"该对象受 iCloud Drive 或某个 File Provider 控制"；而拖放已经授权了这个项目，不会额外弹框。
     读 `~/Desktop` 目录本身的 xattr（`com.apple.file-provider-domain-id` / `com.apple.icloud.desktop`）也能判断，但会不会触发 TCC 未验证。
   - **安全**：rename 一个只在云端的 dataless 文件到同步域之外时，内核先经 filecoordinationd 让 File Provider 下载（xnu `vfs_materialize_reparent`），下载完再改名。这一步会阻塞；被禁止时返回 `EDEADLK`（`rename(2)` 手册明文）。**不会留下坏文件**，也不需要 NSFileCoordinator。
   - macOS ≤13 的 iCloud 还没换成 File Provider，未下载的文件在磁盘上只有 `.名字.icloud` 占位文件，按真实名字移动会得到 ENOENT（也是安全失败），应提示"请先下载"。
6. **图标**
   - `NSWorkspace iconForFile:` 文档写明 "You can safely call this method from any thread"。
   - NSImage 的线程契约同 NSMutableDictionary（10.6 AppKit 发布说明）：不调用变更方法就能跨线程用。
   - 做法：把图标画进**自己新建**的 N×N `NSBitmapImageRep`（预乘 RGBA8，rep 逻辑尺寸 = 像素尺寸，即 1x），再 `representationUsingType:PNG`，全程包在 `autoreleasepool` 里。替身和符号链接的图标会带小箭头，与访达一致；要去掉就先解析目标。草案已编译验证。
7. **打开 / 定位**
   - 推荐 `NSWorkspace openURL:configuration:completionHandler:`：任意线程可调，completion 里能拿到 NSError；`activateFileViewerSelectingURLs:` 也是任意线程可调，但没有错误返回，所以先 lstat。
   - 打开前用 `URLByResolvingAliasFileAtURL:options:`（WithoutUI | WithoutMounting）检查替身是否失效。
   - `/usr/bin/open [-R] --` 也行，GPUI 自己的 `open_with_system` 就这么做，零依赖，但只有英文 stderr。
8. **垃圾文件**
   - mac 上必须把 `.DS_Store`、`Icon\r`（加上 `.localized`）并入 `SHELL_METADATA_FILES`。
   - 否则用户在访达里打开过收纳文件夹后（访达会写 `.DS_Store`），会出现三个问题：(a) 打开「显示隐藏的图标」后它作为半透明项目出现；(b) 删除收纳箱时 `restore_all` 会把 `.DS_Store` **搬到桌面**；(c) 文件夹删不掉，`remaining` 里列出它。
   - `UF_HIDDEN`（`chflags hidden`）对应 Windows 的隐藏属性，用 `std::os::macos::fs::MetadataExt::st_flags() & libc::UF_HIDDEN` 判断。
9. **取色**：`NSColorPanel` 继承 NSResponder，objc2 标成 MainThreadOnly，`sharedColorPanel(mtm)` 需要 `MainThreadMarker`，所以只能在 GPUI 前台调用（mac 上前台就是主线程）。
   - 不需要定义 ObjC 类：每 150 ms 轮询 `color()` / `isVisible()`。
   - 要先 `setHidesOnDeactivate(false)`。NSPanel 默认是 YES，用户点一下桌面（访达被激活）面板就藏起来，会被误判成关闭。
   - 换成 sRGB 要先 `colorUsingColorSpace(sRGBColorSpace)`，再确认颜色模型是 RGB，然后才读 `redComponent`。对非 RGB 颜色读分量会抛 ObjC 异常，在 Rust 里就是 abort。
   - 更优雅的做法是用 block 订阅 `NSColorPanelColorDidChangeNotification` / `NSWindowWillCloseNotification`，同样不需要 `define_class!`。
10. **mac-check 能覆盖全部草案**。libc、block2、objc2 都没有 C 构建脚本，草案已在 Linux 上对 aarch64-apple-darwin 通过 clippy。条件是：
    - mac 源文件是"叶子模块"：不 `use super::…` / `crate::…`，入参出参都是 `Path` / 普通结构。
    - 同时兼容 edition 2021（core）与 2024（mac-check）：用 `unsafe extern "C"`，unsafe fn 体内显式写 `unsafe {}`。
    - mac-check 的 `Cargo.toml` 补上 libc、block2 和对应特性（第 8 节）。

---

## 0. 现有抽象与 mac 的插入点（读源码）

| 位置 | 现状 | mac 要做的 |
|---|---|---|
| `FFS/ops.rs` `execute()` | `cfg(windows)` 新开 STA + `shell::perform_moves`；非 Windows 返回 `Err(UNSUPPORTED)` | 加 `cfg(target_os = "macos")` 分支：在当前后台线程逐项调 `mac::move_item`，转成 `BatchOutcome` |
| `MoveOutcome { hr, new_path, error }` | `hr` 是 `PostMoveItem` 的 HRESULT，`classify` 用 `hr >= 0` 判成功，并用 3 个取消码判 Cancelled | mac 成功填 `hr = Some(0), new_path = Some(最终路径)`；失败填 `hr = Some(-errno)`（不会撞上取消码）、`error = Some(中文说明)`。mac 没有"取消"（不弹 UAC） |
| `ItemOpResult` | 只有 `error: Option<String>` | 想给"去系统设置授权"按钮的话，加一个 `#[serde(default)] needs_privacy_access: bool`，由 `MoveOutcome` 带上来（可选） |
| `plan_move_in` / `plan_restore` → `Reserved::pick` → `folder::existing_names_lower(dir)` | 对目标目录 `read_dir`，用于不区分大小写的去重 | **移回桌面时会列出桌面内容，触发 TCC**（第 2 节）。mac 上对受保护目录不要 `read_dir`：返回空集合，把撞名交给 RENAME_EXCL 重试；或者改成按候选名逐个 lstat |
| `FFS/open.rs` `launch_item` / `reveal_in_explorer` | Windows 走 STA；非 Windows 返回 Err | mac 用 NSWorkspace（第 5 节） |
| `FFS/icon.rs` `icon_png_in` | `cfg(windows)` 提取后写缓存；非 Windows 返回 Err | mac 用 `mac::icon_png(path, bucket)`，产出 PNG 字节后复用 `write_atomically`（第 4 节） |
| `FFS/folder.rs` `has_hidden_or_system_attribute` / `is_hidden_or_system` | 非 Windows 恒 false / 只看是否以 `.` 开头 | mac 加 `UF_HIDDEN`（第 6 节） |
| `FFS/naming.rs` `SHELL_METADATA_FILES` | `["desktop.ini", "thumbs.db"]` | 并入 `.ds_store`、`icon\r`、`.localized`（第 6 节） |
| `FFS/location.rs` `desktop_is_onedrive_redirected` | 看 OneDrive 环境变量 | mac 上改成"移入前对拖入项判断 `NSURLIsUbiquitousItemKey`"（第 3 节） |
| `APP/platform/fence_win.rs` `choose_color` | 非 Windows 返回 `Task::ready(None)` | mac 用 NSColorPanel 轮询（第 7 节） |

**建议的文件布局**：
- 新增 `FFS/mac.rs`，只依赖 std、libc、objc2、objc2-foundation、objc2-app-kit、block2，可以把第 1、3、4、5、6 节的草案合进去。
- ops.rs / open.rs / icon.rs / folder.rs 里各加一个很薄的 `cfg(target_os = "macos")` 适配。
- mac-check 用 `#[path = "../../../crates/core/src/services/fence_fs/mac.rs"]` 原样编译它。
- `CORE/Cargo.toml` 加 `[target.'cfg(target_os = "macos")'.dependencies]`，特性清单见第 8 节。需要的 crate 已全部在 `pc-gpui/Cargo.lock` 里，不会引入新 crate，但 core 的依赖列表变了，`Cargo.lock` 要随之更新（CI 用 `--locked`）。

---

## 1. 移动：NSFileManager vs `renamex_np`

### 1.1 `NSFileManager moveItemAtURL:toURL:error:`

**Apple 文档原文**（`foundation/filemanager/moveitem(at:to:)`）：
> "When moving items, the current process must have permission to read the item at srcURL and write the parent directory of dstURL. … **If an item with the same name already exists at dstURL, this method stops the move attempt and returns an appropriate error.** If the last component of srcURL is a symbolic link, only the link is moved … If the source and destination of the move operation are not on the same volume, this method copies the item first and then removes it from its current location."

**线程安全**（`foundation/filemanager` 的 Threading considerations）：
> "The methods of the shared FileManager object can be called from multiple threads safely. However, if you use a delegate … you should create a unique instance of the file manager object …"

我们不设 delegate，`defaultManager` 和 `NSFileManager::new()` 都可以。objc2 里 `NSFileManager` 不是 `Send` / `Sync`，`Retained<NSFileManager>` 不能跨线程传递，每次在工作线程上现取即可。

**实现细节**（swift-foundation 的 `Sources/FoundationEssentials/FileManager/FileOperations.swift` `_moveItem`；其中 `FOUNDATION_FRAMEWORK` 分支就是编进 Apple Foundation.framework 的代码。按我的理解 macOS 15 起 FileManager 用的是这份实现，更早的 ObjC 实现没开源，**未验证**是否相同）：
- 先 `lstat(dstPath)`，存在就抛 `moveFileError(EEXIST)`，即 `NSFileWriteFileExistsError = 516`。然后调**普通 `rename(srcPath, dstPath)`**。
  **lstat 与 rename 之间有竞争窗口**：窗口内别人在 dst 建了同名文件，rename(2) 会原子地**替换**它（目标是非空目录时才会 ENOTEMPTY）。
- rename 返回 `EXDEV` 时走 `copyFile` 然后 `removeFile(src)`：
  - 复制用 fts 遍历。文件用 `copyfile(COPYFILE_CLONE | COPYFILE_ALL | COPYFILE_EXCL | COPYFILE_NOFOLLOW)`，目录用 `COPYFILE_DATA | COPYFILE_EXCL | COPYFILE_NOFOLLOW`，然后补目录元数据。**每一项都带 EXCL，复制阶段不会覆盖**。
  - 复制失败时 `try? removeFile(dst)`，即删除目标里的半成品（delegate 忽略了 overwrite 错误的情况除外）。抛出的 CocoaError 的 underlying 是复制错误。
  - 复制成功、删源失败时只抛错，**不回滚**。递归删除可能已经删了源树的一部分，但目标里有完整副本，所以不丢数据。WebKit 的评审意见也指出了这一点（bugs.webkit.org 225307）。
- rename 返回 `ENOENT` 时，私有的 `_handleFaultedOutCloudDoc` 会尝试处理"已逐出的 iCloud 文稿"，对应 macOS ≤13 的 `.icloud` 占位文件（第 3 节）。
- errno 到 Cocoa 错误码的映射（`CocoaError+FilePath.swift`，写入 / 移动方向）：
  - `EPERM`、`EACCES` → `NSFileWriteNoPermissionError = 513`
  - `ENOENT` → `NSFileNoSuchFileError = 4`
  - `EEXIST` → `516`
  - `ENOSPC` → `640`
  - `EROFS` → `642`
  - `ENAMETOOLONG` → `514`
  - userInfo 里 `NSUnderlyingErrorKey` 是 `NSPOSIXErrorDomain` 的原始 errno。Quinn 在论坛里也教用这个 key 区分 EPERM（1）和 EACCES（13）。
- **面向用户的文案**：`-[NSError localizedDescription]` 跟随系统语言（中文系统上是中文），格式形如「"a.txt"不能被移到"箱"，因为……」。也可以取 `localizedFailureReason`。我们自己按 errno 给固定中文文案更稳定，`localizedDescription` 只在未知错误时兜底。
- objc2 的 `NSError.underlyingErrors()` 需要 macOS 11.3+，最低 macOS 11 用不了，改读 `userInfo().objectForKey(NSUnderlyingErrorKey)`，草案里就是这么做的。

### 1.2 `renamex_np(from, to, RENAME_EXCL)`

- **libc 0.2.180 有**：`libc::renamex_np(*const c_char, *const c_char, c_uint) -> c_int`、`libc::RENAME_EXCL = 0x4`、`RENAME_SWAP`，位置在 `REG/libc-0.2.180/src/unix/bsd/apple/mod.rs:4652 / 3268`。macOS 10.12 起可用，最低 macOS 11 没有问题。
- **手册**（xnu `bsd/man/man2/rename.2`）：
  > "RENAME_EXCL — On file systems that support it (see getattrlist(2) VOL_CAP_INT_RENAME_EXCL), it will cause EEXIST to be returned if the destination already exists."

  不支持的文件系统返回 `ENOTSUP`（"flags has a value that is not supported by the file system"）；跨文件系统返回 `EXDEV`。
- **原子性**（xnu `bsd/vfs/vfs_syscalls.c` renameat_internal，约 9538 行）：`if (tvp && ISSET(flags, VFS_RENAME_EXCL))` 在 VFS 层、持有 rename 锁时判断，只要目标存在就返回 `EEXIST`。
  唯一的例外是同一目录、同一文件、只改大小写、卷不区分大小写。不区分大小写的 APFS 上，`a.txt` 撞 `A.txt` 也会得到 EEXIST。
  APFS 不区分 Unicode 规范化形式，NFC 和 NFD 的同名也会被内核挡住。这比按字符串比较的 `Reserved::pick` 更可靠。
- **EPERM 的来源**（手册加 xnu）：
  - 源目录或目标目录带 sticky 位，且不属于当前用户。
  - 源带 `UF_IMMUTABLE` / `SF_IMMUTABLE` / `*_APPEND`。`vfs_subr.c` 的 `vnode_authorize_checkimmutable` 对 rename 需要的 DELETE 权限返回 EPERM，访达「显示简介 → 已锁定」设的就是 uchg。
  - sandbox / TCC 拒绝（第 2 节）。
- **EDEADLK**（手册）：
  > "The from pathname refers to a 'dataless' file or directory that must be materialized before being moved to its new location and the I/O policy of the current thread or process disallows file or directory materialization"

  详见第 3 节。

### 1.3 对比

| | `renamex_np(RENAME_EXCL)` | `NSFileManager moveItemAtURL` |
|---|---|---|
| 同卷不覆盖 | **原子**（内核持锁判断） | lstat 后 rename，**有竞争窗口**，窗口内会覆盖 |
| 跨卷 | 返回 EXDEV，要自己处理 | 自动 copyfile（EXCL）+ 删源 |
| 错误信息 | 精确 errno，能区分 EPERM 和 EACCES | NSError（513 / 516…）+ underlying POSIX，`localizedDescription` 已本地化 |
| 依赖 | 只要 libc | objc2-foundation（NSFileManager / NSURL / NSError），需要 autoreleasepool |
| 元数据 | 同卷是同一个 inode：xattr、资源叉、FinderInfo、ACL、标签、创建时间全部保留 | 跨卷时 `COPYFILE_ALL` 复制数据、stat（权限、标志、时间）、ACL、xattr（资源叉和 FinderInfo 都是 xattr）。目标卷不支持 xattr（FAT / exFAT / 部分 SMB）时由 copyfile 写 `._` AppleDouble；创建时间是否保留**未验证** |
| 本场景用到的频率 | 桌面和 `~/MiniTodo收纳箱` 同在用户的 Data 卷上，**几乎全部是同卷 rename** | 只有从外接盘 / 网络卷拖进来，或移回到外接盘上的原目录时才会用到 |

### 1.4 推荐

每一项按以下顺序处理：
1. `renamex_np(src, dest_dir/name, RENAME_EXCL)`。
2. `EEXIST` 时用 `suffixed_file_name(name, n, is_dir)` 顺延（n = 2, 3, …，上限 64 次）重试。
3. `EXDEV` 时交给 NSFileManager；它返回 516 时同样顺延重试；复制完成但删源失败时，按"已移动但源还在"返回。
4. `ENOTSUP`（卷不支持 RENAME_EXCL，比如部分网络卷）时用 lstat 探测加普通 rename。这和 NSFileManager 的保证相同，只在这类卷上有竞争窗口。
5. 其它 errno 按第 1.5 节分类。

移动开始前，在本线程打开 dataless 按需下载（`setiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_THREAD, …_ON)`，见第 3 节），结束后还原。

适配层（ops.rs，示意，未编译；`MoveJob` 最好带上去重前的原名 `base_name`，否则二次撞名会得到 `X (2) (2)`）：

```rust
#[cfg(target_os = "macos")]
{
    let old = super::mac::allow_materialize_on_this_thread();
    let items = jobs
        .iter()
        .map(|job| {
            let is_dir = std::fs::metadata(&job.source).map(|m| m.is_dir()).unwrap_or(false);
            let r = super::mac::move_item(&job.source, &job.dest_dir, &job.dest_name, &|n| {
                super::naming::suffixed_file_name(&job.dest_name, n, is_dir)
            });
            let ok = r.final_path.is_some();
            MoveOutcome {
                hr: Some(if ok { 0 } else { -r.errno.unwrap_or(1) }),
                new_path: r.final_path,
                error: r.message, // ok 且 source_left 时是"源未能删除"的提示，classify 会走 Moved + note
            }
        })
        .collect();
    super::mac::restore_materialize_policy(old);
    Ok(BatchOutcome { items, aborted: false, perform_error: None })
}
```

`classify` 不用改：成功时 `new_path` 存在且源已不在，得到 Moved；跨卷删源失败时目标在、源也在、`hr_ok`，得到 Moved 加提示；失败时得到 Failed 加 `error`。

### 1.5 草案（已编译验证）：`mac_fs.rs` 里与移动相关的部分

```rust
use std::ffi::CString;
use std::io;
use std::os::macos::fs::MetadataExt;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_foundation::{
    NSCocoaErrorDomain, NSError, NSFileManager, NSFileWriteFileExistsError, NSNumber,
    NSPOSIXErrorDomain, NSString, NSURL, NSURLIsUbiquitousItemKey, NSUnderlyingErrorKey,
};

/// `sys/stat.h` 的 `SF_DATALESS`（libc 0.2.180 没有导出）
pub const SF_DATALESS: u32 = 0x4000_0000;

// `sys/resource.h`（libc 0.2.180 没有导出 setiopolicy_np 及其常量）
const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: libc::c_int = 3;
const IOPOL_SCOPE_THREAD: libc::c_int = 1;
const IOPOL_MATERIALIZE_DATALESS_FILES_ON: libc::c_int = 2;

unsafe extern "C" {
    fn setiopolicy_np(iotype: libc::c_int, scope: libc::c_int, policy: libc::c_int) -> libc::c_int;
    fn getiopolicy_np(iotype: libc::c_int, scope: libc::c_int) -> libc::c_int;
}

const MAX_NAME_ATTEMPTS: u32 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveFailure {
    PrivacyDenied, // EPERM 且没有锁定标志：多半是 TCC
    Locked,        // EPERM 且带 uchg/schg/uappnd/sappnd
    NoPermission,  // EACCES
    NotFound,      // ENOENT（含 ≤13 的 .icloud 占位）
    CloudOnly,     // EDEADLK / ETIMEDOUT
    NameExhausted,
    Other,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoveResult {
    pub final_path: Option<PathBuf>,
    pub failure: Option<MoveFailure>,
    pub message: Option<String>,
    pub errno: Option<i32>,
    pub source_left: bool, // 跨卷：已完整复制，但删源失败
}

impl MoveResult {
    fn moved(dest: PathBuf) -> Self {
        MoveResult { final_path: Some(dest), ..Default::default() }
    }
    fn failed(failure: MoveFailure, errno: Option<i32>, message: String) -> Self {
        MoveResult { failure: Some(failure), errno, message: Some(message), ..Default::default() }
    }
}

fn cpath(path: &Path) -> Option<CString> {
    CString::new(path.as_os_str().as_bytes()).ok()
}

pub fn is_locked(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| {
            m.st_flags() & (libc::UF_IMMUTABLE | libc::SF_IMMUTABLE | libc::UF_APPEND | libc::SF_APPEND)
                != 0
        })
        .unwrap_or(false)
}

fn classify_errno(source: &Path, errno: i32) -> (MoveFailure, String) {
    match errno {
        libc::EPERM if is_locked(source) => (
            MoveFailure::Locked,
            "项目已锁定（访达「显示简介」里勾选了「已锁定」），请先解锁".to_string(),
        ),
        libc::EPERM => (
            MoveFailure::PrivacyDenied,
            "macOS 不允许访问该位置：请在「系统设置 → 隐私与安全性 → 文件与文件夹」中允许 mini-todo 访问「桌面」文件夹"
                .to_string(),
        ),
        libc::EACCES => (MoveFailure::NoPermission, "没有权限".to_string()),
        libc::ENOENT => (MoveFailure::NotFound, "不存在".to_string()),
        libc::EDEADLK | libc::ETIMEDOUT => (
            MoveFailure::CloudOnly,
            "该项目只在云端（iCloud 等），下载失败，请先在访达中下载后再试".to_string(),
        ),
        libc::ENOSPC => (MoveFailure::Other, "磁盘空间不足".to_string()),
        libc::EROFS => (MoveFailure::Other, "目标位置是只读的".to_string()),
        libc::ENAMETOOLONG => (MoveFailure::Other, "路径过长".to_string()),
        libc::EBUSY => (MoveFailure::Other, "正被系统占用".to_string()),
        code => (
            MoveFailure::Other,
            format!("{}（errno {}）", io::Error::from_raw_os_error(code), code),
        ),
    }
}

pub fn allow_materialize_on_this_thread() -> libc::c_int {
    // SAFETY: 只读写调用线程的 I/O 策略，参数是头文件常量
    unsafe {
        let old = getiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_THREAD);
        setiopolicy_np(
            IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
            IOPOL_SCOPE_THREAD,
            IOPOL_MATERIALIZE_DATALESS_FILES_ON,
        );
        old
    }
}

pub fn restore_materialize_policy(old: libc::c_int) {
    if old >= 0 {
        // SAFETY: 同上
        unsafe {
            setiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_THREAD, old);
        }
    }
}

pub fn move_item(
    source: &Path,
    dest_dir: &Path,
    first_name: &str,
    next_name: &dyn Fn(u32) -> String,
) -> MoveResult {
    let Some(src) = cpath(source) else {
        return MoveResult::failed(MoveFailure::Other, None, "路径含有非法字符".to_string());
    };
    let mut name = first_name.to_string();
    for attempt in 1..=MAX_NAME_ATTEMPTS {
        let dest = dest_dir.join(&name);
        let Some(dst) = cpath(&dest) else {
            return MoveResult::failed(MoveFailure::Other, None, "路径含有非法字符".to_string());
        };
        // SAFETY: 两个指针都是有效的 NUL 结尾 C 字符串，调用期间存活
        let rc = unsafe { libc::renamex_np(src.as_ptr(), dst.as_ptr(), libc::RENAME_EXCL) };
        if rc == 0 {
            return MoveResult::moved(dest);
        }
        let errno = io::Error::last_os_error().raw_os_error().unwrap_or(0);
        match errno {
            libc::EEXIST => name = next_name(attempt + 1),
            libc::EXDEV => match move_across_volumes(source, &dest) {
                CrossVolume::Exists => name = next_name(attempt + 1),
                CrossVolume::Done(result) => return result,
            },
            libc::ENOTSUP => {
                if std::fs::symlink_metadata(&dest).is_ok() {
                    name = next_name(attempt + 1);
                    continue;
                }
                // SAFETY: 同上
                if unsafe { libc::rename(src.as_ptr(), dst.as_ptr()) } == 0 {
                    return MoveResult::moved(dest);
                }
                let errno = io::Error::last_os_error().raw_os_error().unwrap_or(0);
                let (failure, message) = classify_errno(source, errno);
                return MoveResult::failed(failure, Some(errno), message);
            }
            _ => {
                let (failure, message) = classify_errno(source, errno);
                return MoveResult::failed(failure, Some(errno), message);
            }
        }
    }
    MoveResult::failed(MoveFailure::NameExhausted, Some(libc::EEXIST), "目标位置同名项目过多".to_string())
}

enum CrossVolume {
    Exists,
    Done(MoveResult),
}

fn move_across_volumes(source: &Path, dest: &Path) -> CrossVolume {
    autoreleasepool(|_| {
        let (Some(src_url), Some(dst_url)) = (NSURL::from_file_path(source), NSURL::from_file_path(dest)) else {
            return CrossVolume::Done(MoveResult::failed(MoveFailure::Other, None, "路径含有非法字符".to_string()));
        };
        let manager = NSFileManager::new();
        match manager.moveItemAtURL_toURL_error(&src_url, &dst_url) {
            Ok(()) => CrossVolume::Done(MoveResult::moved(dest.to_path_buf())),
            Err(error) => {
                if is_cocoa_code(&error, NSFileWriteFileExistsError) {
                    return CrossVolume::Exists;
                }
                let errno = underlying_posix_code(&error);
                let dest_exists = std::fs::symlink_metadata(dest).is_ok();
                let source_exists = std::fs::symlink_metadata(source).is_ok();
                if dest_exists && source_exists {
                    // 复制已完成、删源失败（Foundation 不回滚）：绝不删目标
                    return CrossVolume::Done(MoveResult {
                        final_path: Some(dest.to_path_buf()),
                        failure: None,
                        message: Some("已复制到目标位置，但原位置的项目未能删除".to_string()),
                        errno,
                        source_left: true,
                    });
                }
                let (failure, message) = match errno {
                    Some(code) => classify_errno(source, code),
                    None => (MoveFailure::Other, error.localizedDescription().to_string()),
                };
                CrossVolume::Done(MoveResult::failed(failure, errno, message))
            }
        }
    })
}

fn is_cocoa_code(error: &NSError, code: isize) -> bool {
    // SAFETY: Foundation 导出的常量 NSString
    let cocoa = unsafe { NSCocoaErrorDomain };
    error.code() == code && &*error.domain() == cocoa
}

pub fn underlying_posix_code(error: &NSError) -> Option<i32> {
    // SAFETY: Foundation 导出的常量 NSString
    let posix = unsafe { NSPOSIXErrorDomain };
    if &*error.domain() == posix {
        return i32::try_from(error.code()).ok();
    }
    // SAFETY: 同上
    let key = unsafe { NSUnderlyingErrorKey };
    let info = error.userInfo();
    let underlying: Retained<AnyObject> = info.objectForKey(key)?;
    let underlying = underlying.downcast::<NSError>().ok()?;
    if &*underlying.domain() == posix {
        i32::try_from(underlying.code()).ok()
    } else {
        None
    }
}
```

（`is_ubiquitous_item` / `has_legacy_icloud_stub` / `has_hidden_flag` / `is_dataless` 在第 3、6 节。）

---

## 2. TCC（隐私与安全性 → 文件与文件夹 → 桌面文件夹）

### 2.1 哪些操作会弹框

| 操作（mini-todo 是非沙盒、ad-hoc 签名、从 /Applications 启动的 .app） | 结论 | 依据 |
|---|---|---|
| **列出 `~/Desktop` 的内容**（`read_dir`、`contentsOfDirectoryAtURL`） | **会弹框**，拒绝后 EPERM | 读取已有内容就是受保护的操作（WWDC19-701）；社区普遍复现"终端没权限时 `ls ~/Desktop` → Operation not permitted"；论坛 663889 的日志是 `Sandbox: … System Policy: deny(1) file-read-data` |
| 读已有文件的内容 | 会弹框（拖进来的项目除外） | WWDC19-701 |
| **在桌面新建项目**（移回时 rename 进桌面） | **预计不弹框** | WWDC19-701："user consent is not required for an app to create new documents in any of these protected locations, just to read existing contents"。rename 进目录按"新建目录项"算是推断，**未验证** |
| **把拖进来的项目改名移出桌面**（移入） | **预计不弹框** | user intent："dragging and dropping from another application … your app gets access to the file or files that the user selected without the need for a consent prompt"。对非沙盒应用，授权记在文件自身的 `com.apple.macl` xattr 里（Quinn，论坛 124121）。授权是否覆盖"从桌面目录删除这个目录项"（rename 源端需要的权限）**未验证**；失败时 EPERM → 第 1.5 节的 `PrivacyDenied` 引导授权 |
| lstat 拖进来的项目 | 不弹框（user intent） | 同上 |
| `stat ~/Desktop` 目录本身、`dir.is_dir()` | 预计不弹框（只读元数据，不读内容） | 推断，**未验证**。现有 `choose_restore_dir` 用的就是这个 |
| 读 `~/Desktop` 目录本身的 xattr | **未验证** | 第 3 节判断 iCloud 时会用到 |
| `~/MiniTodo收纳箱/` 下的任何操作、FSEvents 监听 | 不受保护 | home 根目录不在 Files & Folders 范围内；Apple DTS 在论坛 794845 里也建议"换到不受保护的位置，比如 home 目录" |
| 外接盘 / 网络卷（「可移除的宗卷」「网络宗卷」） | 规则同桌面：拖放有 user intent，新建不需要同意 | WWDC19-701 把 removable volumes 也列为受保护类别 |

**弹框会阻塞调用线程**（WWDC19-701："rather than denying the access and displaying an alert to the user, the calling thread is actually blocked, waiting for the user to approve or deny"）：
- 移动只在后台线程做（现有约定），千万不能放到 GPUI 主线程上。
- 同意是按类别记录的："all files on your desktop"，一次就够。

**拒绝时的返回值**：EPERM（1，"Operation not permitted"）。Quinn 在 "On File System Permissions"（论坛 678819）里说：BSD 权限 / ACL 失败是 EACCES，App Sandbox / MAC（TCC 用的就是这套）失败是 EPERM。经 NSFileManager 时是 513 + underlying `NSPOSIXErrorDomain` 1。
控制台会有 `Sandbox: mini-todo(pid) System Policy: deny(1) file-read-data …`（WWDC 建议去控制台找 "Sandbox violation" 定位是哪次访问触发的）。

### 2.2 必须改的地方：不要列举受保护的目标目录

`plan_restore` → `Reserved::pick(dest_dir)` → `existing_names_lower(dest_dir)` = `fs::read_dir(dest_dir)`。移回的目标是 `~/Desktop`，或原目录在 `~/Documents` / `~/Downloads` 时，就是"列出受保护目录内容"，会触发弹框：
- 用户同意：之后可用，但 ad-hoc 包每次升级都要再同意一次（2.3）。
- 用户拒绝：`read_dir` 失败 → `unwrap_or_default()` 得到空集合 → 去重失效。靠 RENAME_EXCL 兜底仍然不会覆盖，但多弹了一次莫名其妙的框。

建议在 mac 上任选一种：
- (a) `existing_names_lower` 对受保护目录直接返回空集合，撞名完全交给第 1.4 节的 EEXIST 顺延。
- (b) 把 `Reserved` 的判重改成按候选名逐个 `symlink_metadata`。lstat 指定路径预计不受 TCC 限制（**未验证**）；APFS 默认不区分大小写，lstat 自然也不区分。

建议采用 (a)：代码最少，而且内核判重本来就比字符串比较准（大小写、Unicode 规范化）。

### 2.3 ad-hoc 签名与升级

- Quinn（论坛 663889）："TCC needs a stable signature for it to be able to record which app was granted permission by the user." 在 678819 里的大意是：记住用户的决定需要稳定的签名身份，unsigned 或 ad hoc 签名的构建会反复弹框。
- ad-hoc 签名的 designated requirement 就是 cdhash，每次构建都会变。多个开源项目的 issue 报告升级后授权失效：系统设置里开关还开着，但新二进制不满足旧的 csreq。
  对「文件与文件夹」这类可以弹框的服务，预期是升级后第一次访问再弹一次；可能残留旧条目，可以用 `tccutil reset SystemPolicyDesktopFolder com.tauri-app.mini-todo` 清理（**未在真机验证**具体表现）。
- 结论：**设计上尽量不需要「桌面文件夹」授权**。拖放授权加"新建不需要同意"，再加 2.2 的修改，正常路径应该一次框都不弹。

### 2.4 打开系统设置的对应面板

- `x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders`（旧的 prefpane 写法）。社区维护的列表（bvanpeski/SystemPreferences，Ventura 版）显示 Ventura 及以后仍然支持，但"能用多久不确定"。
- 隐私与安全性主面板的新写法是 `x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension`，可以作为兜底。
- 打开方式：`NSWorkspace openURL:`（任意线程）或 `/usr/bin/open "<url>"`。
- 「完全磁盘访问」是 `?Privacy_AllFiles`。不建议引导用户给它，范围太大。

### 2.5 其它

- **用途说明**：bundle.sh 的 Info.plist 建议加 `NSDesktopFolderUsageDescription`（例如「mini-todo 需要访问桌面，以便把桌面上的项目收纳进收纳箱或移回桌面」）。WWDC："purpose strings … highly recommended"。没有它时弹框只显示通用文案。
- **"负责进程"**：`cargo run` 从终端启动的裸二进制，TCC 的负责进程是终端，套用的是终端的授权（终端常常有完全磁盘访问权限），所以本地测不出问题。要验证 TCC 行为必须用 `bundle.sh` 打包出的 .app、经 `open` 启动。
- **CI**：macOS runner 有登录的 GUI 会话，弹框会出现但没人点，线程永久阻塞。e2e 必须用 `MINI_TODO_FENCES_ROOT` 加临时目录作为"桌面"源，并保证移回的原目录存在，不让 `choose_restore_dir` 回落到真实的 `~/Desktop`。可以考虑加一个 debug-only 的 `MINI_TODO_DESKTOP_DIR`。
- **桌面上的特殊项目**：
  - **符号链接**：rename 移动的是链接本身，目标不动。**相对路径的符号链接**从 `~/Desktop`（深度 1）移到 `~/MiniTodo收纳箱/<箱>/`（深度 2）后会失效，移回后恢复，不丢数据。访达做的是"替身"，桌面上的符号链接少见。
  - **替身（alias）**：普通文件，内容是 bookmark 数据，加 FinderInfo 里的 alias 标志（xattr）。rename 后照常可用；跨卷复制带 `COPYFILE_ALL` 也会保留 xattr。
  - **.app**：本质是目录，`path_is_dir` 会判成文件夹。显示和打开时应该当成文件处理：用 `NSWorkspace isFilePackageAtPath:` 或 `NSURLIsPackageKey` 判断；显示名可以用 `NSFileManager displayNameAtPath:`，它会按访达设置隐藏 `.app`。
    移动 .app 可能触发 Ventura 起的「App 管理」提示：社区报告"提示出现了但移动成功"，**未验证**。如果得到 EPERM，提示里可以顺带提一下「App 管理」。
    从未打开过的隔离（quarantine）app 被非访达移动后，启动时仍会被 App Translocation，只是表面问题。
  - **已锁定的文件**（uchg）：rename 得到 EPERM，第 1.5 节按 `Locked` 区分提示。
  - **正被其它 App 打开的文件**：macOS 没有共享冲突，rename 照常成功（这一点与 Windows "正被占用" 不同）。

---

## 3. iCloud「桌面与文稿」

### 3.1 判断是否开启（尽量不触发 TCC）

| 方法 | 是否触发 TCC | 可靠性 | 依据 |
|---|---|---|---|
| **对拖进来的项目本身取 `NSURLIsUbiquitousItemKey`** | 否（拖放授权已覆盖该项目） | Apple DTS（论坛 781427）：这个 key 表示"an object is under the 'control' of iCloud Drive or a File Provider"，同时提醒 "reliable" 很难保证 | **推荐**。还能覆盖 OneDrive、Dropbox 等第三方 File Provider 的"桌面备份" |
| 读 `~/Desktop` 目录的 xattr：`com.apple.file-provider-domain-id`（Sequoia）/ `com.apple.icloud.desktop`（更早） | **未验证**（读的是受保护文件夹自身的 xattr） | 社区观察，不是文档行为。brunerd 的 macAdminTools 脚本（以 root 身份经 MDM 运行）就是这样判断的 | 可以作为文件夹级判断的补充 |
| `~/Library/Preferences/MobileMeAccounts.plist` 里 `MOBILE_DOCUMENTS` 服务 Enabled | 否（不在受保护目录） | 只能说明 iCloud Drive 开着，**区分不了**「桌面与文稿」 | brunerd 博客（只看到摘要） |
| `brctl` / `fileproviderctl` | 要起子进程，输出格式不稳定 | 不推荐 | — |

**语义**：把一个文件移出 iCloud 管理的桌面，等于把它从 iCloud 同步域里删除，其它 Mac 的桌面和 iCloud 云端都会少这个文件（推断，与 OneDrive 同理）。
建议照搬 Windows 的"首次收纳时提示一次"，但判断时机要改到**移动之前**：项目移走后就不在 iCloud 里了，事后再取这个 key 是 false。
做法是在 `fence::items::move_in` 调 `move_into_fence` 之前，对来源在桌面下的项目取 `is_ubiquitous_item`。

```rust
pub fn is_ubiquitous_item(path: &Path) -> bool {
    autoreleasepool(|_| {
        let Some(url) = NSURL::from_file_path(path) else {
            return false;
        };
        let mut value: Option<Retained<AnyObject>> = None;
        // SAFETY: NSURLIsUbiquitousItemKey 的值类型是 NSNumber（BOOL）
        let ok = unsafe { url.getResourceValue_forKey_error(&mut value, NSURLIsUbiquitousItemKey) };
        ok.is_ok()
            && value
                .and_then(|v| v.downcast::<NSNumber>().ok())
                .is_some_and(|n| n.boolValue())
    })
}
```

### 3.2 移动"只在云端"的文件安全吗

- **macOS 14 Sonoma 起**，iCloud Drive 改用 File Provider，逐出（evict）后的文件变成 APFS **dataless** 文件。名字、属性、xattr 都在本地，只是没有数据 extent，`st_flags` 带 `SF_DATALESS = 0x40000000`（TN3150；`sys/stat.h`）。
  macOS 13 及以前用的是隐藏的 `.名字.扩展名.icloud` 占位文件（Howard Oakley 等人的文章，只看到搜索摘要）。
- **rename dataless 文件到新位置时**（xnu `vfs_syscalls.c` 约 9930 行）：
  - 文件系统返回内部错误 `EDATALESS` 后，内核释放 rename 锁，调用 `vfs_materialize_reparent(fvp, tdvp)`。这会以 `NAMESPACE_HANDLER_RENAME_OP` 经 filecoordinationd 请求 File Provider 下载，成功后带 `VFS_RENAME_DATALESS` 重试 rename。
  - 下载被禁止（线程或进程的 I/O 策略关掉了 materialize）时返回 **EDEADLK**（`rename(2)`："The from pathname refers to a 'dataless' file or directory that must be materialized before being moved to its new location and the I/O policy … disallows"）。
  - 目录里有 dataless 后代时，同样先 materialize（源码注释："The object we're renaming is dataless (or has a dataless descendent)"）。
  - **结论：不会产生坏文件**，要么先下载完再移动，要么失败、原样不动。
- **会阻塞**：下载大文件或网络差时可能很久，失败时可能是 ETIMEDOUT（TN3150 列出的现象之一）或 EIO。
  - 必须在后台线程做。
  - 可以先对来源 lstat 看 `SF_DATALESS`，在界面上提示"正在从 iCloud 下载 N 个项目"。
- **I/O 策略**：xnu 里进程的默认策略是 OFF（`kern_resource.c` 的 `set_proc_materialize_policy`："The default policy is to NOT materialize dataless files"）。但 TN3150 描述的现象（普通 App 访问时被 materialize、卡住）说明用户态通常会打开它。
  为了行为确定，在移动线程上显式 `setiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_THREAD, IOPOL_MATERIALIZE_DATALESS_FILES_ON)`，完成后还原。草案是 `allow_materialize_on_this_thread` / `restore_materialize_policy`。libc 0.2.180 没有这个函数，草案自己声明了 `unsafe extern "C"`，常量取自 `sys/resource.h`：
  - `IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES = 3`
  - `IOPOL_SCOPE_THREAD = 1`
  - `…_ON = 2`
  - 如果改成 fail-fast：设 `…_OFF = 1`，之后会得到 EDEADLK，提示"请先在访达中下载"。
- **macOS ≤13**：访达显示的是 `名字`，磁盘上只有 `.名字.icloud`。拖放给出的大概率是不存在的 `名字` 路径（**未验证**）。`plan_move_in` 的 lstat 会得到 ENOENT → Failed "无法访问: 不存在"，是安全失败。
  可以用下面的函数把文案换成"该文件还没从 iCloud 下载，请先在访达中下载"。不要去移动占位文件本身。NSFileManager 的私有 `_handleFaultedOutCloudDoc` 会特殊处理这种情况，但我们不走它。

```rust
pub fn has_legacy_icloud_stub(path: &Path) -> bool {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    let stub = parent.join(format!(".{}.icloud", name.to_string_lossy()));
    std::fs::symlink_metadata(stub).is_ok()
}

pub fn is_dataless(meta: &std::fs::Metadata) -> bool {
    meta.st_flags() & SF_DATALESS != 0
}
```

- **NSFileCoordinator 不需要**：
  - 对非沙盒应用，`item(at:willMoveTo:)` 的文档原文是 "If your macOS app is not sandboxed, this method serves no purpose"。
  - dataless 的下载由内核与 File Provider 完成，与我们是否协调无关。
  - 协调写入（`.forMoving`）只是让其它进程里打开该文件的 NSFilePresenter（比如正在编辑它的 Pages）及时更新 URL。NSDocument 本来就按文件引用追踪，可以以后再加。

---

## 4. 图标

### 4.1 线程安全

- `-[NSWorkspace iconForFile:]`（`appkit/nsworkspace/icon(forfile:)`）："The returned image has an initial size of 32 pixels by 32 pixels. **You can safely call this method from any thread of your app.**"
- NSImage / NSImageRep 的线程契约（Mac OS X 10.6 AppKit 发布说明，"NSImage: Clarifying the contract for threading"）：
  > "NSImage, NSImageRep, and all of its subclasses have the same threading contract as NSMutableDictionary … If you aren't calling mutating methods, an image can be safely used from arbitrary threads … By 'mutating method' we mean … -setSize: or -addRepresentation:. We do _not_ mean methods like -drawInRect:fromRect:operation:fraction: … Locking focus on an image and drawing in it is a mutation, however, as is modifying the bitmapData of an NSBitmapImageRep."

  → **不对 iconForFile: 返回的图片调 setSize:**（它可能是共享的缓存对象），只往**自己新建**的位图里画。
- NSGraphicsContext（Thread Safety Summary）："If you do any drawing from a secondary thread, a new instance of NSGraphicsContext is created specifically for that thread … **you must flush your drawing calls manually** … call the flushGraphics method"。
- objc2 0.3.2 的标记：`NSWorkspace`、`NSImage`、`NSImageRep`、`NSBitmapImageRep`、`NSGraphicsContext` **都没有** `MainThreadOnly`，可以在后台线程调；它们也都不是 `Send`，所以在同一线程里创建、使用、释放。`NSColorSpace` 是 `Send + Sync`。
- **autoreleasepool**：Rust 的 std 线程没有自动释放池，AppKit 内部 autorelease 的临时对象会泄漏，每次调用都要包在 `objc2::rc::autoreleasepool(|_| …)` 里。

### 4.2 渲染成恰好 N×N 像素

- 用 `initWithBitmapDataPlanes:NULL pixelsWide:N pixelsHigh:N bitsPerSample:8 samplesPerPixel:4 hasAlpha:YES isPlanar:NO colorSpaceName:NSDeviceRGBColorSpace bitmapFormat:0 bytesPerRow:0 bitsPerPixel:0`，由 AppKit 分配缓冲。
  - `bitmapFormat` 必须是 0，即**预乘** alpha、alpha 在后。CoreGraphics 的位图上下文只支持预乘格式，用 `AlphaNonpremultiplied` 时 `graphicsContextWithBitmapImageRep:` 会返回 nil。这是 CG 的公知限制，没有单独找到文档出处。
  - PNG 编码时 AppKit 自己反预乘。
- `bitmapImageRepByRetaggingWithColorSpace:sRGB`：在画之前把位图标成 sRGB，只改标签、不动像素，之后画进来的颜色会换算到 sRGB。
- **HiDPI**：`rep.setSize(N×N points)`，让 1 point = 1 像素。图标的 NSImage 由 IconServices 支撑，分辨率无关；`drawInRect:` 会按目标像素挑最合适的 rep（16…1024）。我们已经按物理像素档位（16..256）请求，结果清晰，不用再乘 backingScale。
- `NSGraphicsContext saveGraphicsState`（objc2 名 `saveGraphicsState_class()`）→ `setCurrentContext` → `drawInRect:fromRect:NSZeroRect operation:SourceOver fraction:1` → `flushGraphics` → `restoreGraphicsState_class()`。10.6 发布说明里的标准写法就是这样。
- `representationUsingType:NSBitmapImageFileType::PNG properties:@{}` 得到 NSData，`to_vec()` 后交给现有的 `write_atomically`。
  另一种做法是读 `bitmapData()`（预乘 RGBA），反预乘后复用 core 的 `encode_png`，与 Windows 共用编码；但这要处理行跨距（bytesPerRow）和反预乘，不如直接用 AppKit 编码。
- **替身 / 符号链接**：iconForFile: 返回的是**目标的图标加左下角小箭头**，与访达一致（行为描述，**未验证**是否总是带箭头）。Windows 版刻意去掉了 .lnk 的箭头；mac 想对齐，就先用第 5 节的 `resolve_link` 取得目标路径，再取图标。建议 v1 保留箭头，符合 mac 用户习惯。
- **其它**：
  - 路径不存在时 iconForFile: 也会返回通用文稿图标，所以先 lstat（现有 `file_stamp` 会报错）。
  - .app 返回的是应用图标。
  - 首次取某类图标可能要和 iconservicesd 走 IPC、比较慢，现有按 mtime / 大小 / 档位的磁盘缓存照用。

### 4.3 草案（已编译验证）

```rust
use std::path::Path;
use std::ptr;

use objc2::AnyThread;
use objc2::rc::autoreleasepool;
use objc2_app_kit::{
    NSBitmapFormat, NSBitmapImageFileType, NSBitmapImageRep, NSColorSpace, NSCompositingOperation,
    NSDeviceRGBColorSpace, NSGraphicsContext, NSImageInterpolation, NSWorkspace,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

pub fn icon_png(path: &Path, px: u32) -> Result<Vec<u8>, String> {
    let px = px.clamp(16, 1024) as isize;
    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let image = workspace.iconForFile(&NSString::from_str(&path.to_string_lossy()));

        // SAFETY: planes 传 NULL → AppKit 自己分配并持有像素缓冲；8 bit × 4 通道、非平面是合法组合
        let rep = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(),
                ptr::null_mut(),
                px,
                px,
                8,
                4,
                true,
                false,
                NSDeviceRGBColorSpace,
                NSBitmapFormat::empty(), // 预乘、alpha 在后：CG 位图上下文只支持预乘
                0,
                0,
            )
        }
        .ok_or("创建位图失败")?;
        let rep = rep
            .bitmapImageRepByRetaggingWithColorSpace(&NSColorSpace::sRGBColorSpace())
            .unwrap_or(rep);
        rep.setSize(NSSize::new(px as f64, px as f64)); // 1 point = 1 像素

        let ctx = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep).ok_or("创建绘图上下文失败")?;
        NSGraphicsContext::saveGraphicsState_class();
        NSGraphicsContext::setCurrentContext(Some(&ctx));
        ctx.setImageInterpolation(NSImageInterpolation::High);
        image.drawInRect_fromRect_operation_fraction(
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(px as f64, px as f64)),
            NSRect::ZERO, // NSZeroRect = 整张图
            NSCompositingOperation::SourceOver,
            1.0,
        );
        ctx.flushGraphics();
        NSGraphicsContext::restoreGraphicsState_class();

        // SAFETY: 空属性字典的类型正确
        let data = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }
        .ok_or("编码 PNG 失败")?;
        Ok(data.to_vec())
    })
}
```

接到 `icon.rs` 时：在 `icon_png_in` 里加 `#[cfg(target_os = "macos")]` 分支，`create_dir_all(cache_dir)` → `let png = mac::icon_png(path, bucket)?` → `write_atomically(&target, &png)`。
不需要专用线程：Windows 用 STA 是因为 COM，mac 在调用方所在的后台线程直接画即可。多个线程同时画也安全，每个线程有自己的 NSGraphicsContext。

---

## 5. 打开 / 在访达中显示

| 方式 | 线程 | 错误报告 | 备注 |
|---|---|---|---|
| `NSWorkspace openURL:` | "safe from any thread in macOS 10.6 and later" | 只有 BOOL | GPUI 的 `open_url` 用的是它 |
| **`openURL:configuration:completionHandler:`**（10.15+） | "You may call this method safely from any thread" | completion 带 `NSError*`（文件不存在、没有能打开它的 App 等） | 需要 objc2-app-kit 的特性 `NSRunningApplication` + `block2`；completion 在 LaunchServices 的私有队列上回调 |
| `activateFileViewerSelectingURLs:` | "You can safely call this method from any thread" | 没有 | 现代写法；GPUI 的 `reveal_path` 用的是 `selectFile:inFileViewerRootedAtPath:` |
| `/usr/bin/open [-R] -- <path>` | 任意（子进程） | 退出码 + 英文 stderr | GPUI 的 `open_with_system` 用的是它；零 ObjC 依赖 |

**推荐**：打开用 `openURL:configuration:completionHandler:`，在后台线程用 channel 同步等结果（最多 30 秒，超时按成功处理，应用可能还在启动）；定位用 `activateFileViewerSelectingURLs:`，事先 lstat。
打开前用 `resolve_link` 判断替身 / 符号链接是否失效，对齐 Windows 的「快捷方式指向的目标不存在，可能已被移动或卸载」。
LaunchServices 打开失效替身时会不会弹访达风格的"找不到原身"对话框，**未验证**，所以预检更稳。
隔离（quarantine）应用的 Gatekeeper 首次打开提示由系统负责，与我们无关。

```rust
use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::autoreleasepool;
use objc2_app_kit::{NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration};
use objc2_foundation::{
    NSArray, NSError, NSNumber, NSURL, NSURLBookmarkResolutionOptions, NSURLIsAliasFileKey,
};

pub fn launch(path: &Path) -> Result<(), String> {
    if std::fs::symlink_metadata(path).is_err() {
        return Err("文件不存在".to_string());
    }
    let (tx, rx) = mpsc::channel::<Option<String>>();
    autoreleasepool(|_| {
        let url = NSURL::from_file_path(path).ok_or("路径含有非法字符")?;
        let config = NSWorkspaceOpenConfiguration::configuration();
        config.setActivates(true);
        let handler = RcBlock::new(move |_app: *mut NSRunningApplication, error: *mut NSError| {
            // SAFETY: 回调期间 error 要么为 NULL，要么指向有效的 NSError
            let message = unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string());
            let _ = tx.send(message);
        });
        NSWorkspace::sharedWorkspace().openURL_configuration_completionHandler(&url, &config, Some(&handler));
        Ok::<(), String>(())
    })?;
    match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(None) | Err(mpsc::RecvTimeoutError::Timeout) => Ok(()),
        Ok(Some(message)) => Err(format!("打开失败：{message}")),
        Err(mpsc::RecvTimeoutError::Disconnected) => Ok(()),
    }
}

pub fn reveal(path: &Path) -> Result<(), String> {
    if std::fs::symlink_metadata(path).is_err() {
        return Err("文件不存在".to_string());
    }
    autoreleasepool(|_| {
        let url = NSURL::from_file_path(path).ok_or("路径含有非法字符")?;
        NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
        Ok(())
    })
}

/// 替身 / 符号链接：Some(目标)；失效：None；不是链接：Some(自身)
pub fn resolve_link(path: &Path) -> Option<std::path::PathBuf> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() {
        return std::fs::canonicalize(path).ok();
    }
    autoreleasepool(|_| {
        let url = NSURL::from_file_path(path)?;
        let mut value: Option<objc2::rc::Retained<objc2::runtime::AnyObject>> = None;
        // SAFETY: NSURLIsAliasFileKey 的值是 NSNumber（BOOL）
        let is_alias = unsafe { url.getResourceValue_forKey_error(&mut value, NSURLIsAliasFileKey) }.is_ok()
            && value
                .and_then(|v| v.downcast::<NSNumber>().ok())
                .is_some_and(|n| n.boolValue());
        if !is_alias {
            return Some(path.to_path_buf());
        }
        let options = NSURLBookmarkResolutionOptions::WithoutUI | NSURLBookmarkResolutionOptions::WithoutMounting;
        NSURL::URLByResolvingAliasFileAtURL_options_error(&url, options)
            .ok()
            .and_then(|target| target.to_file_path())
    })
}

/// 备选：`/usr/bin/open [-R] -- <path>`
pub fn open_with_cli(path: &Path, reveal: bool) -> Result<(), String> {
    let mut cmd = Command::new("/usr/bin/open");
    if reveal {
        cmd.arg("-R");
    }
    let output = cmd.arg("--").arg(path).output().map_err(|e| format!("无法调用 open: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}
```

（`activateIgnoringOtherApps` 不需要：`setActivates(true)` 会把被打开的 App 切到前台。）

---

## 6. 垃圾文件与隐藏标志

### 6.1 收纳文件夹里可能出现的系统文件

| 名字 | 来源 | 在 `~/MiniTodo收纳箱/<箱>/`（APFS）里会不会出现 | 建议 |
|---|---|---|---|
| `.DS_Store` | 访达打开过这个文件夹、改了视图设置就会写 | **经常出现**（「在访达中显示」之后） | **并入元数据**：不列举、不算剩余、删收纳箱时随文件夹删掉、`restore_all` 不移动 |
| `Icon\r`（名字末尾是回车） | 给文件夹设了自定义图标；隐藏靠 FinderInfo 的 invisible 位，不一定是 UF_HIDDEN | 用户给收纳文件夹换图标时 | **并入元数据**。它不以 `.` 开头，否则会作为普通可见项出现 |
| `.localized` | 让文件夹名本地化的空标记文件（`~/Desktop` 里就有） | 基本不会 | 并入元数据（无害） |
| `._名字`（AppleDouble） | 在不支持 xattr 的卷（FAT / exFAT / SMB）上存 xattr | 收纳根在 APFS 上时不会出现；只有 `MINI_TODO_FENCES_ROOT` 指到这类卷才会 | **不要删**：可能是孤立的用户内容，以 `.` 开头本来就自动隐藏 |
| `.Trashes`、`.fseventsd`、`.Spotlight-V100`、`.TemporaryItems`、`.DocumentRevisions-V100`、`.VolumeIcon.icns`、`.apdisk` | 只在卷根目录出现 | 不会 | 不处理（都是目录或卷级文件，`remove_folder_if_empty_at` 只删元数据**文件**） |

**不加的后果**（读 `FFS/ops.rs` / `folder.rs` 推出来的）：
- `.DS_Store` 以 `.` 开头，被 `list_items_at` 标成 `auto_hidden`。打开「显示隐藏的图标」后它会半透明地出现在收纳箱里。
- 删除收纳箱时，`restore_all_at` 跳过的只有 `is_shell_metadata_file`，所以 `.DS_Store` 会被移回**桌面**。桌面上已有自己的 `.DS_Store`，于是变成 `.DS_Store (2)`，成了垃圾。
- 文件夹里只剩 `.DS_Store` 时，`remove_folder_if_empty_at` 返回 false（不是元数据），收纳箱删不干净。

**建议**：`SHELL_METADATA_FILES` 改成全平台共用的并集 `["desktop.ini", "thumbs.db", ".ds_store", "icon\r", ".localized"]`。比较已经是先转小写再比，`Icon\r` 转小写后是 `icon\r`。
Windows 上 Mac 用户拷来的 U 盘 / 网络共享里也常见 `.DS_Store`，一起跳过也是好事。

### 6.2 `UF_HIDDEN` ≈ Windows 隐藏属性

- 手册（`chflags(1)`）和 `sys/stat.h`：`UF_HIDDEN 0x00008000 /* hint that this item should not be displayed in a GUI */`。`chflags hidden` 设的就是它，访达默认不显示这类文件。libc 有 `libc::UF_HIDDEN`。
- 读取：`std::os::macos::fs::MetadataExt::st_flags()`（稳定 API），`meta.st_flags() & libc::UF_HIDDEN != 0`。用 `symlink_metadata` 读，不跟随链接。
- 对齐 Windows 语义：
  - `has_hidden_or_system_attribute`（列举）在 mac 上返回 `UF_HIDDEN`，这类文件永不显示。
  - `is_hidden_or_system`（移入校验）在 mac 上返回 `UF_HIDDEN || 以 . 开头`。
  - `restore_all` 照常移回它们（与 Windows "删除收纳箱时隐藏项一并移回"一致）。
- 也可以用 `NSURLIsHiddenKey`，它把"以 . 开头"和 UF_HIDDEN 合在一起，但我们需要分开判断，st_flags 更直接。

```rust
pub fn has_hidden_flag(meta: &std::fs::Metadata) -> bool {
    meta.st_flags() & libc::UF_HIDDEN != 0
}
```

---

## 7. 自定义颜色：NSColorPanel

### 7.1 事实

- `NSColorPanel` 的 objc2 声明是 `#[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]`，`NSResponder` 带 `#[thread_kind = MainThreadOnly]`，所以面板及其方法都只能在主线程调用。
  `NSColorPanel::sharedColorPanel(mtm: MainThreadMarker)`。在 GPUI 前台任务里 `MainThreadMarker::new()` 能拿到，因为 mac 上 GPUI 的前台就是主线程，见 `APP/platform/macos/ns.rs` 的 `main_thread()`。
- 用到的方法：
  - `NSColorPanel`：`setColor`、`color`、`setShowsAlpha`、`setContinuous`、`setTarget` / `setAction`（unsafe）。
  - `NSWindow`：`makeKeyAndOrderFront`、`isVisible`、`setHidesOnDeactivate`、`close`。
- **坑：`hidesOnDeactivate`**（`appkit/nswindow/hidesondeactivate`）："The default value for NSWindow is false; **the default value for NSPanel is true**." 用户点一下桌面让访达成为前台 App，面板就会消失，`isVisible` 变成 false，轮询会误判为"用户关掉了"。所以要 `setHidesOnDeactivate(false)`。
- **取色结果**：面板的 `changeColor:` 会沿响应链发给第一响应者。GPUI 的 NSView 不实现它，所以没有副作用，target / action 保持 nil 即可。
- **转 sRGB**：
  - `colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())`：文档说"Returns nil if conversion is not possible"，例如图案色。
  - `redComponent` 文档："Access this property only for colors in the calibratedRGB or deviceRGB color spaces. **Accessing it for other color types raises an exception.**" ObjC 异常穿过 Rust 帧会直接 abort。
  - 所以转换后要再判断 `colorSpace().colorSpaceModel() == NSColorSpaceModel::RGB`。实际上 sRGB 转换结果的模型就是 RGB；`usingColorSpace(.sRGB)!.redComponent` 是常见写法。
- **激活**：收纳箱窗口在桌面层，应用可能不在前台。先 `cx.activate(true)`（GPUI 的封装就是 `activateIgnoringOtherApps:`），或者调 objc2 的 `activateIgnoringOtherApps(true)`（标了 deprecated，要 `#[allow(deprecated)]`）。
  **不要用 `NSApplication::activate()`**：它是 macOS 14+ 才有的选择器，在 11–13 上会因"unrecognized selector"崩溃。

### 7.2 草案（已编译验证；GPUI 胶水部分是示意）

```rust
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSColor, NSColorPanel, NSColorSpace, NSColorSpaceModel};

pub fn show(mtm: MainThreadMarker, initial: (u8, u8, u8)) -> Retained<NSColorPanel> {
    let panel = NSColorPanel::sharedColorPanel(mtm);
    panel.setShowsAlpha(false);
    panel.setHidesOnDeactivate(false); // NSPanel 默认 YES：失焦就藏，会被当成"关闭"
    let (r, g, b) = initial;
    let color = NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
        1.0,
    );
    panel.setColor(&color);
    #[allow(deprecated)] // macOS 14 弃用，但最低支持 11；NSApplication::activate() 在 11–13 上没有
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    panel.makeKeyAndOrderFront(None);
    panel
}

pub fn poll(panel: &NSColorPanel) -> (bool, Option<(u8, u8, u8)>) {
    (panel.isVisible(), to_srgb8(&panel.color()))
}

pub fn to_srgb8(color: &NSColor) -> Option<(u8, u8, u8)> {
    let srgb = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    if srgb.colorSpace().colorSpaceModel() != NSColorSpaceModel::RGB {
        return None;
    }
    let to8 = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some((to8(srgb.redComponent()), to8(srgb.greenComponent()), to8(srgb.blueComponent())))
}
```

GPUI 胶水（示意，未编译；签名沿用 `fence_win::choose_color`）：

```rust
pub fn choose_color(cx: &mut App, _owner: Option<Hwnd>, initial: (u8, u8, u8)) -> Task<Option<(u8, u8, u8)>> {
    let Some(mtm) = objc2::MainThreadMarker::new() else { return Task::ready(None) };
    let panel = ns_color::show(mtm, initial); // Retained<NSColorPanel> 不是 Send：只能留在前台任务里
    cx.spawn(async move |cx| {
        let mut last = Some(initial);
        loop {
            cx.background_executor().timer(std::time::Duration::from_millis(150)).await;
            let (visible, color) = ns_color::poll(&panel); // await 之后回到前台（主线程）
            if color.is_some() {
                last = color; // 想实时预览：这里把颜色写回收纳箱外观
            }
            if !visible {
                break;
            }
        }
        last.filter(|c| *c != initial) // 没改动视为取消（NSColorPanel 没有"取消"按钮）
    })
}
```

### 7.3 更好的做法（可选，同样不需要 `define_class!`）

- 用 `NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(...)` 订阅两个通知，`queue` 传 `NSOperationQueue::mainQueue()`，block 里往 `futures::channel::mpsc::UnboundedSender`（Send）发事件：
  - `NSColorPanelColorDidChangeNotification`（`objc2_app_kit`，特性 `NSColorPanel`）
  - `NSWindowWillCloseNotification`（object 传 panel）
- 结束时 `removeObserver:`。这个方法是 unsafe 的（"block must be sendable"），需要 objc2-foundation 的特性 `NSNotification`、`NSOperation`、`block2`，草案 crate 里已加，类型检查通过。
- 好处是不用轮询，而且用 WillClose 判断"关闭"比 `isVisible` 准，不受 hidesOnDeactivate 影响。
- `define_class!` 定义 target 对象实现 `changeColor:`，再 `setTarget` / `setAction` 也可以，但要多维护一个 ObjC 类，没必要。

---

## 8. mac-check 交叉检查

- 草案 crate 用的依赖：libc 0.2.180、block2 0.6.2、objc2 0.6.5、objc2-foundation 0.3.2、objc2-app-kit 0.3.2。
  - 前三个只有生成 cfg 的 build.rs，**没有 C 编译**。
  - 在 Linux 上 `cargo clippy --offline --target aarch64-apple-darwin -- -D warnings` 编译通过，edition 2021 和 2024 都试过。
  - 用的就是 mac-check 的 `Cargo.lock`，libc 和 block2 已在其中（原本是传递依赖），**不需要联网**。
- **mac-check 要加的东西**：
  - `[target.'cfg(target_os = "macos")'.dependencies]` 加 `libc = "=0.2.180"`、`block2 = "=0.6.2"`。
  - objc2-foundation 特性加 `FoundationErrors`（`NSFileWriteFileExistsError` 等常量在这个特性下）、`NSData`、`NSError`、`NSFileManager`、`NSURL`；用 7.3 的通知方案时再加 `NSNotification`、`NSOperation`。
  - objc2-app-kit 特性加 `block2`、`NSBitmapImageRep`、`NSColor`、`NSColorPanel`、`NSColorSpace`、`NSGraphicsContext`、`NSImage`、`NSImageRep`、`NSPanel`、`NSRunningApplication`、`NSWorkspace`。
  - `lib.rs` 加 `#[path = "../../../crates/core/src/services/fence_fs/mac.rs"] mod fence_fs_mac;`。
- **core 需要的最小特性集**（单独验证通过，不含取色）：
  - objc2-foundation：`std`、`objc2-core-foundation`、`FoundationErrors`、`NSArray`、`NSData`、`NSDictionary`、`NSError`、`NSFileManager`、`NSString`、`NSURL`、`NSValue`
  - objc2-app-kit：`std`、`objc2-core-foundation`、`block2`、`NSBitmapImageRep`、`NSColorSpace`、`NSGraphics`、`NSGraphicsContext`、`NSImage`、`NSImageRep`、`NSRunningApplication`、`NSWorkspace`
  - 再加 `libc`、`block2`、`objc2`。
  - app 侧取色在 app 已有特性的基础上，补 `NSColor`、`NSColorPanel`、`NSColorSpace`、`NSPanel`。
  - 按 mac-check 的注释，mac-check 的特性应等于 core 与 app 的并集。
- **会破坏 mac-check 的写法**：
  - mac 源文件里 `use super::…` / `crate::…`。会把 ops.rs → folder.rs → location.rs → `crate::db::paths` 一路拖进来，而 core 依赖 rusqlite bundled（C 编译需要 Apple SDK），在 Linux 上编不了 mac 目标。所以 `mac.rs` 必须是叶子模块，入参出参只用 `Path` / `PathBuf` / 自己的普通结构。适配（`MoveJob` ↔ `MoveResult`、`suffixed_file_name`）放在 ops.rs 里。
  - 依赖 GPUI（app 侧的取色胶水），放到 mac-check 之外。只把 `show` / `poll` / `to_srgb8` 这种纯 objc2 部分放进可以 `#[path]` 的文件，例如 `APP/platform/macos/ns.rs` 或新建 `ns_color.rs`。
  - **edition 差异**：core 是 2021，mac-check 是 2024，同一个文件要两边都能编译：
    - extern 块写 `unsafe extern "C" { … }`（2021 从 Rust 1.82 起接受）。
    - unsafe fn 体内的 unsafe 操作要包 `unsafe {}`（2024 的 `unsafe_op_in_unsafe_fn` 默认 warn，在 `-D warnings` 下变成错误）。
    - 不要用 `gen` 当标识符。
    - 不要用 `static mut` 的引用。
    - 留意 2024 的 `if let` 临时值作用域变化。
  - 依赖带 C 构建脚本的 crate（cc / bindgen），比如 rusqlite bundled、mac-notification-sys。`png` 是纯 Rust，可以用，但草案用 AppKit 编码，用不到它。

---

## 9. API / 特性 / 线程标记速查（objc2-foundation 0.3.2 / objc2-app-kit 0.3.2，均已在生成源码中核对）

| API（Rust 名） | 所需特性 | 线程标记 | 生成源码位置 |
|---|---|---|---|
| `NSFileManager::defaultManager()` / `new()` / `moveItemAtURL_toURL_error` | F:`NSFileManager` + `NSURL` + `NSError` | 无 MainThreadOnly；**不是 Send** | `REG/objc2-foundation-0.3.2/src/generated/NSFileManager.rs:277/1052/529` |
| `NSFileWriteFileExistsError = 516`、`NSFileWriteNoPermissionError = 513`、`NSFileNoSuchFileError = 4` | F:`FoundationErrors` | — | `…/FoundationErrors.rs:36/32/8` |
| `NSError::domain()` / `code()` / `userInfo()` / `localizedDescription()`；`NSPOSIXErrorDomain` / `NSCocoaErrorDomain` / `NSUnderlyingErrorKey`（extern static，读取要 unsafe） | F:`NSError` + `NSString` + `NSDictionary` | Send + Sync | `…/NSError.rs:202–216, 18/24/46` |
| `NSURL::from_file_path` / `to_file_path`（Rust 扩展） | F:`NSURL` | Send + Sync | `REG/objc2-foundation-0.3.2/src/url.rs:101/174` |
| `NSURL::getResourceValue_forKey_error`（unsafe）、`NSURLIsUbiquitousItemKey`、`NSURLIsAliasFileKey`、`NSURLIsHiddenKey`、`NSURLIsPackageKey` | F:`NSURL` + `NSError` + `NSString` | — | `…/NSURL.rs:1382, 765/441/97/67` |
| `NSURL::URLByResolvingAliasFileAtURL_options_error`、`NSURLBookmarkResolutionOptions::{WithoutUI, WithoutMounting}` | F:`NSURL` + `NSError` | — | `…/NSURL.rs:1519, 989` |
| `NSWorkspace::sharedWorkspace()` / `iconForFile` / `openURL` / `activateFileViewerSelectingURLs` / `isFilePackageAtPath` | A:`NSWorkspace`（`iconForFile` 另需 `NSImage`） | 无 MainThreadOnly；Apple 文档写明可在任意线程调用 | `REG/objc2-app-kit-0.3.2/src/generated/NSWorkspace.rs:50/124/58/107/119` |
| `openURL_configuration_completionHandler`、`NSWorkspaceOpenConfiguration::configuration()` / `setActivates` | A:`NSWorkspace` + `NSRunningApplication` + `block2` | NSRunningApplication 是 Send + Sync | `…/NSWorkspace.rs:63, 377` |
| `NSBitmapImageRep::initWithBitmapDataPlanes_…_bitmapFormat_bytesPerRow_bitsPerPixel`（unsafe）/ `bitmapImageRepByRetaggingWithColorSpace` / `representationUsingType_properties`（unsafe）、`NSBitmapImageFileType::PNG`、`NSBitmapFormat` | A:`NSBitmapImageRep` + `NSImageRep` + `NSGraphics`（+ `NSColorSpace`） | 无 MainThreadOnly | `…/NSBitmapImageRep.rs:287/494/559, 65, 110` |
| `NSImageRep::setSize` | A:`NSImageRep` | — | `…/NSImageRep.rs:118` |
| `NSImage::drawInRect_fromRect_operation_fraction` | A:`NSImage` + `NSGraphics` | 无 MainThreadOnly | `…/NSImage.rs:322` |
| `NSGraphicsContext::graphicsContextWithBitmapImageRep` / `setCurrentContext` / `saveGraphicsState_class` / `restoreGraphicsState_class` / `flushGraphics` / `setImageInterpolation` | A:`NSGraphicsContext`（前者另需 `NSBitmapImageRep`） | 无 MainThreadOnly | `…/NSGraphicsContext.rs:96/116/124/128/150/203` |
| `NSDeviceRGBColorSpace`（extern static）、`NSCompositingOperation::SourceOver` | A:`NSGraphics` | — | `…/NSGraphics.rs:369, 23` |
| `NSColorSpace::sRGBColorSpace()` / `colorSpaceModel()`、`NSColorSpaceModel::RGB` | A:`NSColorSpace` | Send + Sync | `…/NSColorSpace.rs:124/116/24` |
| `NSColor::colorWithSRGBRed_green_blue_alpha` / `colorUsingColorSpace` / `redComponent`… | A:`NSColor`（+ `NSColorSpace`） | Send + Sync | `…/NSColor.rs:142/312/719` |
| `NSColorPanel::sharedColorPanel(mtm)` / `setColor` / `color` / `setShowsAlpha` / `setContinuous` | A:`NSColorPanel` + `NSPanel` + `NSWindow` + `NSResponder` + `NSColor` | **MainThreadOnly**（经 NSResponder） | `…/NSColorPanel.rs:175/247/239/225/216` |
| `NSWindow::isVisible` / `makeKeyAndOrderFront` / `setHidesOnDeactivate` | A:`NSWindow` + `NSResponder` | **MainThreadOnly** | `…/NSWindow.rs:1153/1092/1073` |
| `NSColorPanelColorDidChangeNotification`、`NSWindowWillCloseNotification`、`NSNotificationCenter::addObserverForName_object_queue_usingBlock`（unsafe）、`NSOperationQueue::mainQueue()` | A:`NSColorPanel` / `NSWindow`；F:`NSNotification` + `NSOperation` + `block2` | — | `…/NSColorPanel.rs:381`、`…/NSWindow.rs:2697`、`F/NSNotification.rs:212`、`F/NSOperation.rs:460` |
| `objc2::rc::autoreleasepool` | objc2 | — | `REG/objc2-0.6.5/src/rc/autorelease.rs:440` |
| `libc::renamex_np` / `RENAME_EXCL` / `UF_HIDDEN` / `UF_IMMUTABLE` / `SF_IMMUTABLE` / `EXDEV` / `EPERM` / `EACCES` / `EDEADLK` | libc | — | `REG/libc-0.2.180/src/unix/bsd/apple/mod.rs:4652/3268/3807/3798/3805/2200/2183/2195/2193` |
| `SF_DATALESS = 0x40000000`、`setiopolicy_np`、`IOPOL_*` | **libc 没有**，要自己声明（xnu `sys/stat.h:520`、`sys/resource.h:570–602/645`） | — | — |

（F = objc2-foundation，A = objc2-app-kit）

---

## 10. 未验证清单（需要 mac CI / 真机确认）

1. 把拖进来的桌面项目 rename 移出 `~/Desktop`，在没有「桌面文件夹」授权时能否成功，即 user intent 是否覆盖源端删除目录项。实测方法：真机，`tccutil reset SystemPolicyDesktopFolder com.tauri-app.mini-todo` 后拖入。
2. rename 进 `~/Desktop`（移回）是否完全不弹框；`stat ~/Desktop`、lstat 桌面里指定路径、读 `~/Desktop` 的 xattr 是否不弹框。
3. ad-hoc 包升级后「文件与文件夹」的具体表现：重新弹框，还是旧开关残留但被静默拒绝。
4. 旧版 NSFileManager（macOS 11–14 的 ObjC 实现）是否同样是 lstat 加 rename。不影响推荐方案。
5. macOS ≤13 从桌面拖出未下载的 iCloud 文件时，拖放板上给的是什么路径。
6. GUI App 进程默认的 materialize I/O 策略。草案显式设成 ON，不依赖默认值。
7. iconForFile: 对替身 / 符号链接是否总带箭头；对 dataless 文件是否触发下载（预计不会，图标不读数据）。
8. LaunchServices 打开失效替身时是否弹系统对话框（草案已预检）。
9. 移动 .app 是否触发「App 管理」，失败时是不是 EPERM。
10. `x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders` 在 macOS 14–26 上能否直接定位到「文件与文件夹」。

可以在 mac CI 上自动验证的（不涉及 TCC，全部在临时目录里做）：
- RENAME_EXCL 撞名顺延；跨卷（`hdiutil create` 一个临时 dmg 挂载作为第二个卷）复制加删源。
- 锁定文件得到 EPERM 并被分类为 `Locked`（`chflags uchg`）。
- UF_HIDDEN 过滤（`chflags hidden`）。
- `.DS_Store` 不阻止删除文件夹。
- iconForFile → PNG 的尺寸与 alpha。
- `/usr/bin/open -R`、`activateFileViewerSelectingURLs` 不报错。

---

## Sources

- Apple 文档（经 `developer.apple.com/tutorials/data/documentation/…json` 读取）：
  [FileManager](https://developer.apple.com/documentation/foundation/filemanager)、
  [moveItem(at:to:)](https://developer.apple.com/documentation/foundation/filemanager/moveitem(at:to:))、
  [copyItem(at:to:)](https://developer.apple.com/documentation/foundation/filemanager/copyitem(at:to:))、
  [NSWorkspace.icon(forFile:)](https://developer.apple.com/documentation/appkit/nsworkspace/icon(forfile:))、
  [open(_:configuration:completionHandler:)](https://developer.apple.com/documentation/appkit/nsworkspace/open(_:configuration:completionhandler:))、
  [open(_:)](https://developer.apple.com/documentation/appkit/nsworkspace/open(_:))、
  [activateFileViewerSelecting(_:)](https://developer.apple.com/documentation/appkit/nsworkspace/activatefileviewerselecting(_:))、
  [NSWindow.hidesOnDeactivate](https://developer.apple.com/documentation/appkit/nswindow/hidesondeactivate)、
  [NSColor.redComponent](https://developer.apple.com/documentation/appkit/nscolor/redcomponent)、
  [NSColor.usingColorSpace(_:)](https://developer.apple.com/documentation/appkit/nscolor/usingcolorspace(_:))、
  [isUbiquitousItemKey](https://developer.apple.com/documentation/foundation/urlresourcekey/isubiquitousitemkey)、
  [NSFileCoordinator](https://developer.apple.com/documentation/foundation/nsfilecoordinator)、
  [item(at:willMoveTo:)](https://developer.apple.com/documentation/foundation/nsfilecoordinator/item(at:willmoveto:))、
  [TN3150: Getting ready for dataless files](https://developer.apple.com/documentation/technotes/tn3150-getting-ready-for-data-less-files)
- [Thread Safety Summary（Threading Programming Guide）](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/Multithreading/ThreadSafetySummary/ThreadSafetySummary.html)
- [AppKit Release Notes（older，含 10.6 "NSImage: Clarifying the contract for threading"）](https://developer.apple.com/library/archive/releasenotes/AppKit/RN-AppKitOlderNotes/index.html)
- [WWDC19 701 Advances in macOS Security（transcript）](https://developer.apple.com/videos/play/wwdc2019/701/)
- Apple 开发者论坛（Quinn / DTS）：
  [On File System Permissions](https://developer.apple.com/forums/thread/678819)、
  [Reset access obtained through inferred user intent（com.apple.macl）](https://developer.apple.com/forums/thread/124121)、
  [Unsandboxed app can't access files（TCC 需要稳定签名）](https://developer.apple.com/forums/thread/663889)、
  [Drop file not found（Desktop/Documents 受 MAC 保护）](https://developer.apple.com/forums/thread/794845)、
  [NSURLIsUbiquitousItemKey / File Provider](https://developer.apple.com/forums/thread/781427)
- Apple 开源：
  [xnu bsd/vfs/vfs_syscalls.c](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/vfs/vfs_syscalls.c)（RENAME_EXCL、EDATALESS → vfs_materialize_reparent、materialize 策略）、
  [xnu bsd/vfs/vfs_subr.c](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/vfs/vfs_subr.c)（immutable → EPERM）、
  [xnu bsd/vfs/kpi_vfs.c](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/vfs/kpi_vfs.c)（VNOP_RENAMEX）、
  [xnu bsd/kern/kern_resource.c](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_resource.c)、
  [xnu bsd/man/man2/rename.2](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/man/man2/rename.2)、
  [xnu bsd/sys/stat.h](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/stat.h)、
  [xnu bsd/sys/resource.h](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/resource.h)、
  [swift-foundation FileOperations.swift](https://github.com/swiftlang/swift-foundation/blob/main/Sources/FoundationEssentials/FileManager/FileOperations.swift)、
  [swift-foundation CocoaError+FilePath.swift](https://github.com/swiftlang/swift-foundation/blob/main/Sources/FoundationEssentials/Error/CocoaError%2BFilePath.swift)
- 社区（只作佐证）：
  [bvanpeski/SystemPreferences（Ventura 深链列表）](https://github.com/bvanpeski/SystemPreferences/blob/main/macos_preferencepanes-Ventura.md)、
  [brunerd/macAdminTools iCloudDriveDesktopSync_func.sh](https://github.com/brunerd/macAdminTools/blob/main/Scripts/iCloudDriveDesktopSync_func.sh)、
  [Michael Tsai：iCloud Drive Switches to Dataless Files](https://mjtsai.com/blog/2023/10/27/icloud-drive-switches-to-dataless-files/)、
  [WebKit bug 225307（moveItem 删源失败不回滚）](https://bugs.webkit.org/show_bug.cgi?id=225307)、
  ad-hoc 签名导致 TCC 授权失效的 issue 若干（如 [MacStats #9](https://github.com/eyupucmaz/MacStats/issues/9)、[parley #75](https://github.com/pathorsAI/parley/issues/75)）、
  [Apple 论坛：Ventura App Management](https://developer.apple.com/forums/thread/715220)
- 本机源码：`REG/objc2-foundation-0.3.2`、`REG/objc2-app-kit-0.3.2`、`REG/objc2-0.6.5`、`REG/libc-0.2.180`、`REG/gpui-pre-macos-0.3.9/src/{window.rs,platform.rs}`（拖放用 `NSFilenamesPboardType`；`open_with_system` = `open --`；`reveal_path` = `selectFile:inFileViewerRootedAtPath:`）
