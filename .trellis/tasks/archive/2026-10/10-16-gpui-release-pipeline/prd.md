# GPUI 版发布流水线、删除 Tauri 版、发布 v3.0.0-pre

## 背景

`claude/determined-mccarthy-6rfxev`（GPUI 重写，任务 `10-15-gpui-rewrite`）已快进合入 main。用户决定：

- 发布 **v3.0.0-pre**（GitHub pre-release），内容是 GPUI 版
- 发布流水线**参考 `D:\Git\mini-term` 的 CI/CD**（`.github/workflows/release.yml` / `cache-warm.yml`）
- **删除整个 `pc/`（Tauri + Vue）**，以后不再打 Tauri 包；`pc-gpui/` 成为唯一 PC 端（目录名暂不改）

## 用户已确认的决策

| 项 | 决定 |
|---|---|
| Windows 产物 | NSIS 安装包（身份对齐 Tauri 版，原地升级 2.5.0）+ 便携 zip |
| 平台 | 仅 Windows x64（pc-gpui 只为 Windows 做了平台集成） |
| Tauri 版 | 删除 `pc/` 源码、`build.yml`、`pc-ci.yml`，文档 / spec 中 Tauri 描述与双改规则一并清理 |

## 需求

### R1 发布流水线 `.github/workflows/release.yml`（替换 `build.yml` + `cloud-release.yml`）

参照 mini-term：

1. `create-release`（仅 push tag）：生成 changelog（基线 = `git describe --tags --abbrev=0 --match 'v*' "$TAG^"`），
   `gh release create --draft --verify-tag`，tag 含 `-` 加 `--prerelease`；release 已存在则沿用
2. 构建作业只向 draft 上传（`gh release upload --clobber`，不用 softprops —— 它找不到 draft 会另建公开 release）：
   - `windows`：`pc-gpui` `cargo build --release -p mini-todo --locked` → NSIS 安装包 `mini-todo_<ver>_x64-setup.exe`
     + 便携 zip `mini-todo_<ver>_x64-portable.zip`
   - `cloud`（x86_64 / aarch64 两个 runner）：沿用旧 `cloud-release.yml` 的打包（`minitodo-cloud-<cloud 版本>-linux-<arch>.tar.gz` + `.sha256`）
3. `publish`：全部构建成功后对 release 全部资产算 `SHA256SUMS.txt` 上传，再 `--draft=false --prerelease=<tag 含 ->`
4. `workflow_dispatch`（tag + platforms）：给已有 release 补建产物，不新建 release、不改 draft / prerelease，只重算 SHA256SUMS
5. 安全：顶层 `contents: read`，写 release 的作业各自提权；checkout `persist-credentials: false`；action 钉 commit SHA；
   token 只在需要的步骤 env 里
6. 版本门槛：tag 必须等于 `v<pc-gpui 工作区版本>`，否则 create-release 失败（防止忘改版本号发出错版本的包）

### R2 Windows NSIS 安装器 `pc-gpui/scripts/windows-installer.nsi`

身份对齐 Tauri 2.9.6 NSIS（currentUser）：

- 产品名 `mini-todo`；卸载键 `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\mini-todo`；默认目录 `$LOCALAPPDATA\mini-todo`；
  主程序 `mini-todo.exe`；开始菜单 `$SMPROGRAMS\mini-todo.lnk`；桌面 `$DESKTOP\mini-todo.lnk`（可选）
- 升级：`.onInit` 读 `UninstallString` 认出旧版 → 告知会先卸载（取消即退出）→ Section 里先关运行中的实例，再
  `ExecWait '"<旧卸载器>" /S /UPDATE _?=<旧目录>'`（Tauri 卸载器认 `/UPDATE`：保留开机自启值与快捷方式，不删数据）
- **⚠️ 默认安装目录 `%LOCALAPPDATA%\mini-todo` 就是数据目录（data.db / images / backups / holidays）**：卸载只删自己装的文件，
  `RMDir "$INSTDIR"` 不递归；绝不 `RMDir /r $INSTDIR`
- 真卸载（非 `/UPDATE`）删除 `HKCU\...\Run` 的 `mini-todo` 值（与 Tauri 卸载器一致）；数据目录、日志目录不动
- 运行中实例：只处理路径在安装目录（及旧目录）下的 `mini-todo.exe`；交互模式先询问，再 CloseMainWindow → 等待 → 强杀

### R3 缓存 `.github/workflows/cache-warm.yml`

main 上预热 release 用的 Rust 缓存（Windows pc-gpui release 构建、cloud 两个架构），`release.yml` 一律 `save-if: false`；
shared-key / workspaces / 工具链安装步骤 / workflow env 两边逐字一致。

### R4 版本号

- `pc-gpui/Cargo.toml` `[workspace.package] version = "3.0.0-pre"`，`Cargo.lock` 同步
- `compare_versions` 认预发布：`3.0.0-pre < 3.0.0`（否则装了 3.0.0-pre 的用户在 3.0.0 正式版发布后看不到更新红点）

### R5 删除 `pc/` 与文档清理

- 删除 `pc/`、`.github/workflows/build.yml`、`pc-ci.yml`、`cloud-release.yml`（并入 release.yml）
- `CLAUDE.md` / `AGENTS.md` / `README.md` / `docs/开发文档/` / `cloud/README.md` / `.trellis/spec/**`（不含 tasks 归档与 journal）：
  PC 端改为 GPUI 版唯一实现；去掉"两边一起改 / diff -r 自查"的双改规则；Tauri / Vue / WebView 描述删除或改为 GPUI 对应物
- 代码注释里指向 `pc/...` 的出处说明改成"旧 Tauri 版（已删除）"之类的历史说明，或直接去掉
- `.trellis/spec/frontend/`（Vue 规范）删除，Trellis 配置的 spec 层随之调整

## 验收

- `pc-gpui`：`cargo fmt --check`、`cargo clippy --workspace --all-targets --locked -D warnings`、`cargo test --workspace --locked` 通过
- `cloud`：`cargo test --locked` 通过（删除 pc/ 不影响）
- workflow YAML 语法正确（actionlint 如可用）
- NSIS 本地 makensis 能编译（如本机无 makensis 则依赖 CI）
- 推 main → `PC (GPUI) CI` 绿 → 推 tag `v3.0.0-pre` → release.yml 全绿、release 为 pre-release、资产齐全
  （setup.exe、portable.zip、cloud ×2 + sha256、SHA256SUMS.txt）→ 改写中文 release notes
