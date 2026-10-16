# 发布流水线、Windows 安装器与 macOS 打包

> 建立于任务 `10-16-gpui-release-pipeline`（2026-10-16，首个版本 v3.0.0-pre）。写法参考 mini-term 的
> `.github/workflows/release.yml` / `cache-warm.yml` / `scripts/windows-installer.nsi`。
> 文件：`.github/workflows/release.yml`、`.github/workflows/cache-warm.yml`、`pc-gpui/scripts/windows-installer.nsi`、
> `pc-gpui/scripts/macos/{bundle.sh,make-dmg.sh}`。
> 3.0 起只发 GPUI 版：旧 Tauri 版（`pc/`）连同 `build.yml` / `pc-ci.yml` 已删除，`cloud-release.yml` 并入 `release.yml`。
> macOS 包（universal dmg，§6）由任务 `10-16-macos-package` 恢复，首个版本 v3.0.2-pre；mac 是**预览平台**，构建失败不挡发布（§3）。

---

## 1. 版本号（唯一来源）

- 唯一来源：`pc-gpui/Cargo.toml` 的 `[workspace.package] version`；两个 crate 都是 `version.workspace = true`
- 改完必须 `cargo update --workspace`（只改 `Cargo.lock` 里 `mini-todo` / `mini-todo-core` 两条），CI 一律 `--locked`
- tag 必须等于 `v<version>`：`release.yml` 的 create-release 第一步校验，不等直接失败（防止发出 exe 版本与 tag 对不上的包）
- exe 内的版本信息由 `crates/app/build.rs` 从 `CARGO_PKG_VERSION` 生成：数字四段取 `-` / `+` 之前的部分，字符串版本原样
- mac 的 `Info.plist` 由 `pc-gpui/scripts/macos/bundle.sh` 从同一个 `[workspace.package] version` 读（awk），`CFBundleShortVersionString` /
  `CFBundleVersion` 取 `-` / `+` 之前的部分（`3.0.2-pre` → `3.0.2`，§6.2）；完整版本号由应用内关于页显示
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
  ├─ create-release   校验 tag = v<pc-gpui 版本> → 发布说明 → gh release create --draft [--prerelease]
  │                   发布说明：tag 的代码里有 `.github/release-notes/<tag>.md` 就用它，否则按上一个 v* tag 以来的提交生成占位
  │                   （release 已存在则原样沿用：不重建、不改 draft、不覆盖 notes）
  ├─ plan             push = windows + macos + cloud-x86_64 + cloud-aarch64；dispatch = inputs.platforms 点名的
  ├─ build-windows    cargo build --release -p mini-todo --locked → 便携 zip + NSIS 安装包 → gh release upload --clobber
  ├─ build-macos      macos-15（arm64）：cargo build --release -p mini-todo --locked --target aarch64-apple-darwin --target x86_64-apple-darwin
  │                   → 再比一次 tag 与 Cargo.toml 版本 → bundle.sh（lipo + .app + ad-hoc 签名）→ make-dmg.sh → gh release upload --clobber
  ├─ build-cloud ×2   cloud cargo build --release --locked → tar.gz + .sha256 → gh release upload --clobber
  └─ publish          构建结束后（Windows / cloud 没有失败或取消；mac 失败只出 warning）：下载全部资产 → SHA256SUMS.txt 上传 →
                      （仅 push）gh release edit --draft=false --prerelease=<含 ->
```

资产（`<ver>` 不带 `v`）：

| 资产 | 内容 |
|---|---|
| `mini-todo_<ver>_x64-setup.exe` | NSIS 安装包（§5） |
| `mini-todo_<ver>_x64-portable.zip` | 只有 `mini-todo.exe`（图标 / 字体 / SVG 都嵌在 exe 里） |
| `mini-todo_<ver>_universal.dmg` | macOS 拖放安装镜像：`mini-todo.app`（arm64 + x86_64，ad-hoc 签名）+ `/Applications` 软链（§6）。名字格式同 2.5.0 的 `mini-todo_2.5.0_universal.dmg`；不再出 2.5.0 那个 `mini-todo_universal.app.tar.gz`（tauri-action 的副产物，没有 updater 用它） |
| `minitodo-cloud-<cloud 版本>-linux-{x86_64,aarch64}.tar.gz` + `.sha256` | 云端 API + config 示例 + deploy + Skill |
| `SHA256SUMS.txt` | release 全部资产的 SHA256（publish 生成，补建后重算，补上的 dmg 也在里面） |

约定（违反就出错）：

- **上传一律 `gh release upload`，不用 softprops/action-gh-release**：后者按 tag 找不到 draft，会另建一个公开 release
- 下游作业的 `if` 必须带 `!cancelled()` 再显式放行 `create-release` 的 `skipped`（手动补建时它被跳过；
  不写状态函数时隐含的 `success()` 会把下游一起跳掉）
- `matrix.include` 为空数组会让作业报错，所以 `build-cloud` 先判 `needs.plan.outputs.cloud != '[]'`
- `concurrency: release-<tag>`、`cancel-in-progress: false`：同一 tag 重复投递的运行排队而不是并行抢传
- 手动补建（workflow_dispatch：`tag` + `platforms`，可选 `windows` / `macos` / `cloud-x86_64` / `cloud-aarch64`）从 tag 的代码构建
  （**包括打包脚本**），`--clobber` 覆盖同名资产，publish 只重算 SHA256SUMS，不改 draft / prerelease。所以补建只能救偶发失败
  （runner / 网络 / hdiutil）；代码或打包脚本本身的问题要修好后发新版本
- Windows 或 cloud 失败（或被取消）时 release 停在 draft（匿名用户看不到）：修好后在该次运行上 Re-run failed jobs
- **macOS 失败不挡发布**（PRD：mac 是预览平台）：`build-macos` 照常标红，publish 的 `if` 只排除 `build-windows` / `build-cloud` 的
  `failure` / `cancelled`，不看 `build-macos`；publish 第一步对"选了 macos 但 `build-macos` 不是 success"出一条 warning（并写进
  Job Summary），其余照常转正，release 里暂时没有 dmg。之后在该次运行上 Re-run failed jobs，或 `workflow_dispatch`
  （`tag=<tag>`、`platforms=macos`）补建，publish 随之重跑、SUMS 补上 dmg
- publish 的 `if` 还要求至少一个平台本次成功（`build-windows` / `build-macos` / `build-cloud` 任一 success）：只补建 mac 且失败时什么都不做。
  改 publish 的 `if` 时不要换回 `!contains(needs.*.result, 'failure')`——那样 mac 失败会挡住整个发布

## 4. 安全与缓存

- 顶层 `permissions: contents: read`，写 release 的作业各自提到 `contents: write`；token 只出现在 gh 那一步的 env
- `actions/checkout` 一律 `persist-credentials: false`（构建期依赖的 build.rs 读得到 `.git/config`）
- 所有 action 钉 commit SHA，行尾注释写原 tag / 分支；升级时按 tag 重新查：
  `gh api repos/<owner>/<repo>/git/ref/tags/<tag>`（annotated tag 还要再 `git/tags/<sha>` 解一层）
- workflow 里引用 `inputs.*` 一律经 `env:` 传进脚本，不直接 `${{ }}` 拼进 run（防注入）
- 缓存：`release.yml` 的 rust-cache 一律 `save-if: false`，由 `cache-warm.yml` 在 main 上预热（tag 之间看不见彼此的缓存）。
  **两边的 workflow 级 env（`CARGO_TERM_COLOR`）、装工具链步骤、`shared-key`（`release-gpui` / `release-cloud`）、
  `workspaces`、编译命令必须逐字一致**，否则键对不上、发版冷编。只改 `[profile.*]` 或编译命令时键不变 →
  release 与 cache-warm 的所有 rust-cache（各三处：gpui Windows、gpui macOS、cloud）一起加同一个 `prefix-key` 强制换代
- 作业对照：`gpui-windows` ↔ `build-windows`、`gpui-macos` ↔ `build-macos`、`cloud` ↔ `build-cloud`（表在 `cache-warm.yml` 文件头）。
  `gpui-windows` 与 `gpui-macos` 共用 `shared-key: release-gpui`：键里带 runner OS / 架构，两份缓存不会互相覆盖。mac 额外要逐字一致的有
  `runs-on: macos-15`（换标签 = 换 OS 版本 / SDK，键也变）、装工具链那一步的 `targets: aarch64-apple-darwin,x86_64-apple-darwin`、
  两个 `--target` 的编译命令（顺序也算）。要设 `MACOSX_DEPLOYMENT_TARGET` 之类的环境变量时两边写在同一层——它不在 rust-cache 的环境哈希前缀
  （`CARGO` / `CC` / `CFLAGS` / `CXX` / `CMAKE` / `RUST`）里，键看不出两边不一致（目前不设：Rust 默认 arm64 11.0、x86_64 10.12，都不高于 plist 的 11.0）

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

## 6. macOS 打包（`pc-gpui/scripts/macos/bundle.sh`、`make-dmg.sh`）

> 不用 Tauri bundler：workflow 里手工组装 `.app`（照 mini-term 的做法），身份照抄 2.5.0 mac 包的**实物** Info.plist
> （`mini-todo_universal.app.tar.gz` 解包核对）。两个脚本都只能在 macOS 上运行，路径按脚本位置解析，可从任何目录调用；
> release、`pc-gpui-ci.yml` 的 macos 作业、mac e2e 共用同一份 `bundle.sh`，Info.plist 的逻辑在发版前就被 CI 跑到。

### 6.1 产物形态与构建机

- **一个 universal dmg**：一台 arm64 runner（`macos-15`）上一次 `cargo build --release -p mini-todo --locked --target aarch64-apple-darwin
  --target x86_64-apple-darwin`（x86_64 是交叉编译），`bundle.sh` 用 `lipo -create` 合并并断言 `arm64` / `x86_64` 两个切片都在
- **不用 Intel runner**：`macos-15-intel` 只承诺到 2027-08，且上面的 Metal 多重采样会卡死（runner-images#14836，GPUI 画 path 用 4x MSAA）
- **构建期不需要 Metal 工具链**：gpui-kit 0.7.3 给 `gpui_platform` 强制开了 `runtime_shaders`，gpui-pre-apple 的 build.rs 只用 cbindgen
  拼出 `stitched_shaders.metal`，运行时 `new_library_with_source` 编译。构建只要 Xcode Command Line Tools（clang + macOS SDK：bundled SQLite
  等 C 编译与链接）；Xcode 26 "Metal Toolchain 要另外下载"的问题碰不到
- 不设 `MACOSX_DEPLOYMENT_TARGET`（Rust 默认 arm64 11.0、x86_64 10.12，都不高于 plist 的 11.0）；要设时见 §4 的缓存键说明

### 6.2 `.app` 结构与 Info.plist

```
mini-todo.app/Contents/
├── Info.plist
├── MacOS/mini-todo        # universal（release / pc-gpui-ci）或单架构（e2e：bundle.sh <二进制> - <输出目录>）
├── Resources/icon.icns    # crates/app/assets/icon.icns（检回自 749405a:pc/src-tauri/icons/icon.icns，16~1024 全档；icon.ico 最大只有 256）
└── _CodeSignature/        # §6.3
```

| 键 | 值 | 依据 |
|---|---|---|
| `CFBundleIdentifier` | `com.tauri-app.mini-todo` | 与 2.5.0 一致、= `crates/app/src/lib.rs` 的 `APP_IDENTIFIER`：通知授权 / 显示设置、LaunchServices 识别都挂在它上面，换 id 等于换一个应用（拖进「应用程序」不替换旧版而是并存）。GPUI 的系统通知没有 bundle id 时直接禁用。`bundle.sh` 读回校验 |
| `CFBundleExecutable` | `mini-todo` | cargo bin 名，与 2.5.0 一致：LaunchAgent 里的 `/Applications/mini-todo.app/Contents/MacOS/mini-todo` 覆盖安装后仍有效 |
| `CFBundleName` / `CFBundleDisplayName` | `mini-todo` | 与 2.5.0 一致；菜单栏应用菜单的标题与 Dock 悬停提示显示它（应用菜单项文案里的 "Mini Todo" 来自 `APP_NAME`） |
| `CFBundleIconFile` | `icon.icns` | 与 2.5.0 一致 |
| `CFBundlePackageType` / `CFBundleInfoDictionaryVersion` | `APPL` / `6.0` | |
| `CFBundleShortVersionString` / `CFBundleVersion` | `${VERSION%%[-+]*}`（`3.0.2-pre` → `3.0.2`） | Apple 规定只能是 1~3 段整数（数字与点），`bundle.sh` 不合格直接报错；口径同 Windows 的 `VIProductVersion`。预发布与正式版同号，不影响非 App Store 分发 |
| `CFBundleDevelopmentRegion` | `zh_CN` | 2.5.0 是 `English`；没有 `.lproj` 时系统面板按开发区域的语言显示（效果待真机确认） |
| `LSMinimumSystemVersion` | `11.0` | arm64 的下限本来就是 11（2.5.0 的 arm64 切片 minos 也是 11.0）；CI 只能测 15 / 26。代价：2.5.0 写的是 10.13，10.13~10.15 的用户会被挡 |
| `LSApplicationCategoryType` | `public.app-category.productivity` | 只影响分类展示 |
| `NSHighResolutionCapable` | `true` | 与 2.5.0 一致 |
| `LSUIElement` | **不设** | 2.5.0 没设，Dock 图标照常显示；而且 GPUI 在 `applicationDidFinishLaunching` 里按自身状态重设激活策略（默认 Regular），写了也会被覆盖——要隐藏 Dock 图标只能用 `cx.set_activation_policy(Accessory)` |
| `NSDesktopFolderUsageDescription` / `NSDocumentsFolderUsageDescription` / `NSDownloadsFolderUsageDescription` | 中文用途说明（"mini-todo 需要访问桌面文件夹，才能把桌面上的项目收进收纳箱或移回桌面。"等） | 桌面收纳箱移入 / 移回时万一触发「文件与文件夹」授权框，框里显示的说明（正常路径不列举这些受保护目录，不会弹框，见 `backend/desktop-fences.md`「场景：macOS 上的收纳箱」）。**不是身份键**，加 / 改不影响覆盖升级与已有授权的归属 |

不写：`LSRequiresCarbon` / `CSResourcesFileMapped`（Tauri 模板遗留）、`NSPrincipalClass`（GPUI 自己建 NSApplication）、上表以外的 `*UsageDescription`
（没用到别的受保护资源）。`plutil -lint` 校验；版本号只从 `pc-gpui/Cargo.toml` 读（§1）。

### 6.3 签名：整包 ad-hoc，不公证（没有 Apple 开发者账号）

- **最后一步** `codesign --force --sign - --timestamp=none mini-todo.app`：封 Info.plist / Resources，并给 lipo 合并后的每个切片重签。
  链接器自带的 linker-signed ad-hoc 签名不封 bundle，lipo / strip 之后也不可靠；Apple Silicon 上没有签名的 arm64 代码会被直接杀掉
- **签完不要再改 bundle 里的任何文件**：顺序固定为 lipo → 复制图标 → 写 Info.plist → 签名；之后搬运一律 `ditto`（保留签名与扩展属性），
  不用 `cp`。`bundle.sh` 签完 `codesign --verify --deep --strict`，`make-dmg.sh` 打包前、挂载后各再校验一次
- 不开 hardened runtime（`--options runtime`）、不打时间戳：只有公证才要求。`spctl --assess` 对 ad-hoc 必然 `rejected`，只作信息输出
- 为什么一定要整包签：GPUI 的 `UNUserNotificationCenter` 对未签名的 bundle 拒绝授权（UNErrorDomain code 1），ad-hoc 可以；
  整包签名后带隔离属性的应用首次打开的提示从"已损坏"变成"无法验证开发者"，可以在「隐私与安全性 → 仍要打开」放行（社区经验，未真机验证；
  macOS 15 起「右键 → 打开」不能再绕过）
- ad-hoc 没有稳定身份（designated requirement 就是 cdhash）：**每次升级都要重新放行**；以后若申请辅助功能等 TCC 权限，每次升级也要重新授权；
  通知授权按 bundle id 记，不受影响；钥匙串项绑定代码签名，所以 WebDAV 密码维持明文（每次升级都会弹"允许访问钥匙串"）
- 以后有 Developer ID：`codesign --options runtime --timestamp -s "Developer ID Application: …"` → `xcrun notarytool submit --wait` →
  `xcrun stapler staple <dmg>`，需要 p12 证书与 App Store Connect API key 两个 secret

### 6.4 dmg

- 暂存目录：`ditto` 一份 `mini-todo.app` + `ln -s /Applications Applications`（拖放安装）；卷名 `mini-todo`；`-format UDZO`（zlib、只读，
  与 2.5.0 相同）+ `-fs HFS+`
- **显式 `-size`**（内容 × 2 + 64 MB）：`-srcfolder` 自动估算的临时卷偏小时会随机报 `No space left on device` / `could not access /Volumes/…`
  （mini-term 踩过三次；同类随机失败 runner-images#4288 / #7522 / #12323）。失败时 detach 残留的 `/Volumes/mini-todo*`、尺寸翻倍、等 15 秒，最多 5 次
- 做完 `hdiutil verify`，再挂载回读校验：签名、`lipo -archs`、`/Applications` 软链指向、dmg 里的可执行文件与 bundle 内的 `cmp` 一致
- 不做窗口背景 / 图标位置美化：要在挂载状态下用 AppleScript 操作访达，在 runner 上不稳定

### 6.5 发布说明里的 mac 段落（每个带 dmg 的版本都写）

- 预览版、**未经真机验收**、反馈方式（Issue + 截图 + 日志 `~/Library/Logs/com.tauri-app.mini-todo/mini-todo.log`）；macOS 11+、Apple 芯片 / Intel 通用
- 安装：打开 dmg，把 mini-todo 拖进「应用程序」（装过 2.5.0 的直接覆盖，数据保留在 `~/Library/Application Support/mini-todo`）；
  先拖进去再打开（直接在 dmg / 下载文件夹里打开会被 App Translocation 挪到临时路径，开机自启拒绝写入）
- 放行（二选一）：`xattr -cr /Applications/mini-todo.app`；或先双击一次、点「完成」，到「系统设置 → 隐私与安全性」点「仍要打开」并输入密码
  （按钮只在尝试打开后约一小时内出现）；提示"已损坏"时只能用前者。每次升级都要重新放行
- 范例见 `.github/release-notes/v3.0.2-pre.md`（§3：create-release 直接用它作为发布说明）

## 7. 发版步骤

1. 改 `pc-gpui/Cargo.toml` 版本 → `cargo update --workspace` → 写好 `.github/release-notes/v<ver>.md`（中文，版式照上一版）→ 提交
2. 推 main，等 `PC (GPUI) CI` 绿（含 macOS 作业）；`Cache warm` 会因 `Cargo.lock` 变化触发，等它跑完发版能命中缓存
3. `git tag v<ver> && git push origin v<ver>` → `Release` 运行：`gh run list --workflow Release --limit 3`
   （同一 tag 出现多份运行时它们会排队，必要时 `gh run cancel` 多余的）
4. 跑完后核对资产（setup.exe、portable.zip、universal.dmg、cloud ×2 + .sha256、SHA256SUMS.txt）与 pre-release 标记。
   mac 失败时 release 已经转正但没有 dmg：publish 的 warning / Job Summary 写着原因；偶发失败就 Re-run failed jobs 或
   `gh workflow run Release -f tag=v<ver> -f platforms=macos`，代码问题修好后发新版本，并在 release notes 里写明本版暂无 mac 包
5. `gh release edit v<ver> --notes-file -` 改写中文 release notes（自动生成的只是 commit 列表占位；mac 段落按 §6.5）

验证工具：`actionlint`（`gh release download -R rhysd/actionlint --pattern '*_windows_amd64.zip'`）检查 workflow。
