# 发布流水线与 Windows 安装器

> 建立于任务 `10-16-gpui-release-pipeline`（2026-10-16，首个版本 v3.0.0-pre）。写法参考 mini-term 的
> `.github/workflows/release.yml` / `cache-warm.yml` / `scripts/windows-installer.nsi`。
> 文件：`.github/workflows/release.yml`、`.github/workflows/cache-warm.yml`、`pc-gpui/scripts/windows-installer.nsi`。
> 3.0 起只发 GPUI 版：旧 Tauri 版（`pc/`）连同 `build.yml` / `pc-ci.yml` 已删除，`cloud-release.yml` 并入 `release.yml`。

---

## 1. 版本号（唯一来源）

- 唯一来源：`pc-gpui/Cargo.toml` 的 `[workspace.package] version`；两个 crate 都是 `version.workspace = true`
- 改完必须 `cargo update --workspace`（只改 `Cargo.lock` 里 `mini-todo` / `mini-todo-core` 两条），CI 一律 `--locked`
- tag 必须等于 `v<version>`：`release.yml` 的 create-release 第一步校验，不等直接失败（防止发出 exe 版本与 tag 对不上的包）
- exe 内的版本信息由 `crates/app/build.rs` 从 `CARGO_PKG_VERSION` 生成：数字四段取 `-` / `+` 之前的部分，字符串版本原样
- cloud 有自己的版本（`cloud/Cargo.toml`），与 tag 无关，只进 cloud 资产名

## 2. 预发布

- tag 含 `-`（如 `v3.0.0-pre`）→ GitHub pre-release；不含 → 正式版
- 应用内更新检查走 `/releases/latest`（`crates/app/src/services/update_check.rs`），**它不返回 pre-release**：
  发预发布版不会提示已装正式版的用户升级
- `compare_versions`（`crates/app/src/windows/main_window/logic.rs`）按 semver 认预发布：`3.0.0-pre < 3.0.0`、
  `2.5.0 < 3.0.0-pre`；装了预发布版的用户在正式版发布后能看到更新红点。改这个函数必须保留 `versions_compare_prerelease` 单测

## 3. release.yml 流程

```
push v* tag
  ├─ create-release   校验 tag = v<pc-gpui 版本> → changelog 占位 → gh release create --draft [--prerelease]
  │                   （release 已存在则原样沿用：不重建、不改 draft、不覆盖 notes）
  ├─ plan             push = windows + cloud-x86_64 + cloud-aarch64；dispatch = inputs.platforms 点名的
  ├─ build-windows    cargo build --release -p mini-todo --locked → 便携 zip + NSIS 安装包 → gh release upload --clobber
  ├─ build-cloud ×2   cloud cargo build --release --locked → tar.gz + .sha256 → gh release upload --clobber
  └─ publish          全部成功后：下载全部资产 → SHA256SUMS.txt 上传 →（仅 push）gh release edit --draft=false --prerelease=<含 ->
```

资产（`<ver>` 不带 `v`）：

| 资产 | 内容 |
|---|---|
| `mini-todo_<ver>_x64-setup.exe` | NSIS 安装包（§5） |
| `mini-todo_<ver>_x64-portable.zip` | 只有 `mini-todo.exe`（图标 / 字体 / SVG 都嵌在 exe 里） |
| `minitodo-cloud-<cloud 版本>-linux-{x86_64,aarch64}.tar.gz` + `.sha256` | 云端 API + config 示例 + deploy + Skill |
| `SHA256SUMS.txt` | release 全部资产的 SHA256（publish 生成，补建后重算） |

约定（违反就出错）：

- **上传一律 `gh release upload`，不用 softprops/action-gh-release**：后者按 tag 找不到 draft，会另建一个公开 release
- 下游作业的 `if` 必须带 `!cancelled()` 再显式放行 `create-release` 的 `skipped`（手动补建时它被跳过；
  不写状态函数时隐含的 `success()` 会把下游一起跳掉）
- `matrix.include` 为空数组会让作业报错，所以 `build-cloud` 先判 `needs.plan.outputs.cloud != '[]'`
- `concurrency: release-<tag>`、`cancel-in-progress: false`：同一 tag 重复投递的运行排队而不是并行抢传
- 手动补建（workflow_dispatch：`tag` + `platforms`）从 tag 的代码构建，`--clobber` 覆盖同名资产，publish 只重算
  SHA256SUMS，不改 draft / prerelease
- 某平台失败时 release 停在 draft（匿名用户看不到）：修好后在该次运行上 Re-run failed jobs

## 4. 安全与缓存

- 顶层 `permissions: contents: read`，写 release 的作业各自提到 `contents: write`；token 只出现在 gh 那一步的 env
- `actions/checkout` 一律 `persist-credentials: false`（构建期依赖的 build.rs 读得到 `.git/config`）
- 所有 action 钉 commit SHA，行尾注释写原 tag / 分支；升级时按 tag 重新查：
  `gh api repos/<owner>/<repo>/git/ref/tags/<tag>`（annotated tag 还要再 `git/tags/<sha>` 解一层）
- workflow 里引用 `inputs.*` 一律经 `env:` 传进脚本，不直接 `${{ }}` 拼进 run（防注入）
- 缓存：`release.yml` 的 rust-cache 一律 `save-if: false`，由 `cache-warm.yml` 在 main 上预热（tag 之间看不见彼此的缓存）。
  **两边的 workflow 级 env（`CARGO_TERM_COLOR`）、装工具链步骤、`shared-key`（`release-gpui` / `release-cloud`）、
  `workspaces`、编译命令必须逐字一致**，否则键对不上、发版冷编。只改 `[profile.*]` 或编译命令时键不变 →
  四处 rust-cache 一起加同一个 `prefix-key` 强制换代

## 5. Windows NSIS 安装器（`pc-gpui/scripts/windows-installer.nsi`）

### 5.1 安装身份 = 旧 Tauri 版（tauri-bundler 2.9.x NSIS 模板，currentUser）

| 项 | 值 |
|---|---|
| 卸载键 | `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\mini-todo`（`InstallDirRegKey` 读 `InstallLocation`；Tauri 写的是带引号的路径，NSIS 会去引号） |
| 默认目录 | `$LOCALAPPDATA\mini-todo`，无 UAC（`RequestExecutionLevel user`） |
| 主程序 | `mini-todo.exe`（新旧同名；清场按进程名 `mini-todo` + 路径前缀匹配） |
| 快捷方式 | `$SMPROGRAMS\mini-todo.lnk`（必建）、`$DESKTOP\mini-todo.lnk`（可选；升级时沿用旧版有无） |
| AppUserModelID | 快捷方式登记 `com.tauri-app.mini-todo`（= `crates/app/src/lib.rs` 的 `APP_IDENTIFIER`，Toast 通知归属；没登记时系统通知可能弹不出来） |
| 开机自启 | `HKCU\...\Run` 值名 `mini-todo`（应用自己写，`services/autostart.rs`）；升级保留，真卸载删除 |
| 旧厂商键 | `HKCU\Software\tauri-app\mini-todo` 默认值 = 安装目录：**本安装器也照写**——回退到 2.x 时 Tauri 安装器用 `"<UninstallString>" _?=<这个默认值>` 卸载已装版本，没有它 `_?=` 为空、卸载失败。本卸载器真卸载时删（`/ifempty` 删父键） |

### 5.2 ⚠️ 默认安装目录就是数据目录

`%LOCALAPPDATA%\mini-todo` 同时是 `db::paths` 的数据目录（`data.db`、`images/`、`backups/`、`holidays/`、`fence-icons/`）。
**安装器 / 卸载器只增删自己的两个文件（`mini-todo.exe`、`uninstall.exe`），所有 `RMDir` 都不带 `/r`**。
Tauri 2.9.6 的卸载器同样只删自己的文件、`RMDir "$INSTDIR"` 不递归（已对照模板确认）；它的"删除应用数据"复选框删的是
`$LOCALAPPDATA\com.tauri-app.mini-todo`（日志目录）与 `$APPDATA\com.tauri-app.mini-todo`，且只在交互卸载勾选且非 `/UPDATE` 时。

### 5.3 升级流程

1. `.onInit`：读 `UninstallString`（去引号）且卸载器文件存在 → 记下旧卸载器 / 旧目录（卸载器的父目录）/ 旧版本，
   交互模式弹框告知会先卸载（取消即退出）
2. SecMain：清场（安装目录 + 改选新目录时的旧目录下运行中的 `mini-todo.exe`）→
   `ExecWait '"<旧卸载器>" /S /UPDATE _?=<旧目录>'`（`_?=` 必须最后，否则 ExecWait 等的是复制到 %TEMP% 的壳）→
   补删旧卸载器、`RMDir` 旧目录（不递归）→ 铺新文件、重建快捷方式与卸载键
3. `/UPDATE`：Tauri 卸载器据此保留开机自启值与快捷方式；本卸载器据此保留开机自启值与旧厂商键（两边同名参数，互相兼容）

清场口径：交互模式先探测（probe）再询问（重试 / 取消），静默模式直接关；**直接 `Stop-Process -Force`**，不发 WM_CLOSE ——
关主窗口只有在没有别的窗口时才退出进程（收纳箱 / 编辑窗口开着时继续），`CloseMainWindow` 也发不到有 owner（固定 / 嵌入）
或不可见（贴边隐藏）的主窗口；数据随改随存进 SQLite，强制结束不丢数据。PowerShell 脚本受
`NSIS_MAX_STRLEN`（1024）限制，改完核对长度。

### 5.4 编码与编译

- **文件必须是 UTF-8 带 BOM**：makensis 靠 BOM 认 UTF-8，没有 BOM 报 `Bad text encoding`（编辑器另存时别丢 BOM）
- 编译参数（全部 `/D`）：`VERSION`（semver）、`VERSION_NUM`（纯数字四段，`VIProductVersion` 只收这个）、`SOURCE_DIR`、
  `ICON_FILE`、`OUT_FILE`
- 本地验证：下载 NSIS 便携包（`https://downloads.sourceforge.net/project/nsis/NSIS%203/3.10/nsis-3.10.zip`）解压到临时目录，
  用占位 `mini-todo.exe` 编译：
  `makensis /V2 /DVERSION=3.0.0-pre /DVERSION_NUM=3.0.0.0 /DSOURCE_DIR=<dir> /DICON_FILE=<pc-gpui\crates\app\assets\icon.ico> /DOUT_FILE=<out.exe> pc-gpui\scripts\windows-installer.nsi`
- ⚠️ 不要在装着正式版的机器上实跑安装器做测试：它会真的卸载现有安装（数据不会丢，但开发机上的安装版会被换掉）

## 6. 发版步骤

1. 改 `pc-gpui/Cargo.toml` 版本 → `cargo update --workspace` → 提交
2. 推 main，等 `PC (GPUI) CI` 绿；`Cache warm` 会因 `Cargo.lock` 变化触发，等它跑完发版能命中缓存
3. `git tag v<ver> && git push origin v<ver>` → `Release` 运行：`gh run list --workflow Release --limit 3`
   （同一 tag 出现多份运行时它们会排队，必要时 `gh run cancel` 多余的）
4. 全绿后核对资产（setup.exe、portable.zip、cloud ×2 + .sha256、SHA256SUMS.txt）与 pre-release 标记
5. `gh release edit v<ver> --notes-file -` 改写中文 release notes（自动生成的只是 commit 列表占位）

验证工具：`actionlint`（`gh release download -R rhysd/actionlint --pattern '*_windows_amd64.zip'`）检查 workflow。
