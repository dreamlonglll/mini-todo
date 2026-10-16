# Research: macOS 打包与 CI（GPUI 版 mini-todo）

- **Query**：GPUI 版恢复 macOS 安装包——产物形态、.app 组装、签名 / Gatekeeper、dmg 制作、CI 集成、runner 上能自动化验证什么；重点参考 mini-term
- **Scope**：mixed（本仓 + `D:\Git\mini-term` + v2.5.0 发布资产实物 + GitHub / Apple 一手资料）
- **Date**：2026-10-16

---

## 结论速览

1. **产物：建议出一个 universal dmg，资产名 `mini-todo_<ver>_universal.dmg`**，与 2.5.0 的 `mini-todo_2.5.0_universal.dmg` 同一个名字格式，
   也与 `mini-todo_<ver>_x64-setup.exe` 一致（不带 v、下划线分段）。在**一台 arm64 runner（`macos-15`）**上一次 `cargo build` 两个
   `--target`，`lipo` 合并。不用 Intel runner：`macos-15-intel` 官方只承诺到 2027-08，而且它上面 Metal 多重采样会卡死 GPU
   （runner-images#14836），GPUI 画 path 用的正是 4x MSAA。mini-term 出两个 dmg 是历史原因：它先只出 arm64，后来按 issue #77 原生补 Intel；
   它有 4 个二进制、C 依赖也重，交叉编译风险大。注释里它自己也把 lipo universal 列为后路。mini-todo 只有 1 个二进制，C/ObjC 依赖只有
   bundled sqlite 和 mac-notification-sys，交叉编译风险低。
2. **.app：照 2.5.0 的实物来**：`mini-todo.app/Contents/{MacOS/mini-todo, Resources/icon.icns, Info.plist}`，`CFBundleIdentifier = com.tauri-app.mini-todo`
   （从 2.5.0 实物里取出来核对过，和 `APP_IDENTIFIER` 常量一致），`CFBundleExecutable = mini-todo`。Apple 规定
   `CFBundleShortVersionString` 只能是"三段整数"，所以填 `3.0.1-pre` 去掉 `-` 之后的部分 `3.0.1`。`LSMinimumSystemVersion = 11.0`，
   `NSHighResolutionCapable = true`，**`LSUIElement` 先不设**：2.5.0 没设；目前非 Windows 平台关掉最后一个窗口就退出，藏掉 Dock 图标后就没地方唤起了。
   **图标直接用旧版的 `icon.icns`**（`git show 749405a:pc/src-tauri/icons/icon.icns`，109 KB，16 到 1024 各档都有）。CI 上不必再跑 iconutil：
   现有的 `icon.ico` 最大只有 256 px。
3. **签名：没有开发者账号时，最后一步对整个 bundle 做 ad-hoc 签名**（`codesign --force --sign - mini-todo.app`）。2.5.0 实物只有链接器自动加的
   ad-hoc 签名（arm64 切片的 flags 是 `adhoc|linker-signed`，x86_64 切片没签，也没有 `_CodeSignature`），用户下载后会看到"已损坏"，只能靠 `xattr -cr`。
   整包 ad-hoc 签名以后，提示会变成"无法验证开发者"一类，可以走「系统设置 → 隐私与安全性 → 仍要打开」放行（社区经验，见 §3）。
   macOS 15 起不能再用「右键 → 打开」绕过（Apple 官方公告）。发布说明里两种放行方法都写上（§3.4 有现成文案）。
4. **dmg**：先建一个暂存目录（放 `mini-todo.app` 和指向 `/Applications` 的软链），再
   `hdiutil create -volname mini-todo -srcfolder <暂存目录> -fs HFS+ -format UDZO -size <给足> -ov`。重试直接照抄 mini-term
   （`release.yml:342-362`：显式 `-size`，失败后 detach 残留的卷、尺寸翻倍、sleep 15，最多 5 次）。之后挂载 dmg 校验一遍：`codesign --verify` 加 `lipo -archs`。
5. **CI**：
   - `release.yml`：plan 加平台 key `macos`，新建作业 `build-macos`（结构与 `build-windows` 相同），publish 的 `needs` / `if` 加上它。
     SHA256SUMS 本来就按全部资产计算，自动包含 dmg。
   - `cache-warm.yml`：加一个逐字对齐的 `gpui-macos` 作业。
   - `pc-gpui-ci.yml`：建议加 macOS 作业（clippy + test + universal 构建并上传 `.app` 产物）。mac 专属代码在 Windows / Linux 上都没法交叉检查
     （C 依赖需要 Apple SDK），用户又没有 Mac，这是唯一的编译门。
   - **构建期不需要 Metal 编译器**：gpui-kit 0.7.3 给 `gpui_platform` 开了 `runtime_shaders`（同时开了 `font-kit`），着色器源码嵌进二进制，
     运行时由系统 Metal 编译。所以 macos-26 / Xcode 26 上"缺 Metal Toolchain"的问题碰不到（已用 `cargo tree` 实测，§5.4）。
   - 缓存预算要注意：本仓 Actions 缓存现在 8.57 GB / 10 GB，加上 mac 的缓存会开始被淘汰（§5.2）。
6. **runner 上能做 GUI 冒烟**：macOS runner 自动登录 GUI 会话。镜像预先给 `/bin/bash`、`osascript`，以及 runner agent 这个负责进程
   授予了屏幕录制、辅助功能、AppleEvents 和 PostEvent（`configure-tccdb-macos.sh`）。macOS 15+ 还预先批准了"绕过私有窗口选择器"的提醒。
   所以 `screencapture`、`CGWindowListCopyWindowInfo`、CGEvent 点击 / 输入、`osascript` 操作 System Events 原则上都能用。
   - **测不了**：系统通知横幅（镜像卸载了 `notificationcenterui`）、Gatekeeper 首次打开的交互、真实 Intel GPU、多显示器、重启后的开机自启。
   - x86_64 切片可以在 arm64 runner 上用 `arch -x86_64` 走 Rosetta 跑冒烟（镜像装了 Rosetta）。
   - 最小方案见 §6.3。教训来自 mini-term issue #82：mac 包连续 6 个版本**一个字都不画**，没人发现。冒烟必须**断言界面上有文字**（OCR）。

---

## 0. mini-term 是不是同一套栈？可以复用什么

**是同一个家族（Zed 的 gpui-pre 每周快照），版本和接入方式不同：**

| | mini-term | mini-todo |
|---|---|---|
| 依赖 | 直接依赖 `gpui-pre 0.3.5` + `gpui-pre-platform 0.3.5`（手动开 `font-kit`）+ `gpui-component 0.6.2`（`D:\Git\mini-term\Cargo.toml` 的 `[workspace.dependencies]`） | 只依赖一个 `gpui-kit = "=0.7.3"`（= gpui-pre 0.3.9 + gpui-base / gpui-component 0.7.x），见 `pc-gpui/crates/app/Cargo.toml:20` |
| mac 字体后端 | 漏开 `font-kit` 时 gpui_macos 会退回 `NoopTextSystem`，**界面一个字都不画**（mini-term issue #82，v1.13.6-pre 到 v1.13.11-pre）。后来补开，并加了守门测试 `tests/gpuiMacFontKit.test.cjs` | gpui-kit 0.7.3 的 `Cargo.toml` 对所有非 iOS / Android 目标都给 `gpui_platform` 开了 `["font-kit","x11","wayland","runtime_shaders"]`，**不会踩这个坑**（§5.4 有 `cargo tree` 实测） |
| 着色器 | gpui-pre 0.3.5 默认在构建期 `xcrun metal` 预编译 | `runtime_shaders`：构建期只拼出着色器源码，运行时编译 |
| 二进制 | 主程序 + 3 个 sidecar | 只有 `mini-todo` |
| bundle id | `com.mini-term.app`（沿用它旧 Tauri 版的值） | `com.tauri-app.mini-todo`（沿用 2.5.0 的值） |

**可以直接复用的做法（逐条，带出处）：**

| # | 做法 | mini-term 位置 | 能否照搬到 mini-todo |
|---|---|---|---|
| 1 | 不用 Tauri bundler，在 workflow 里手工组装 `.app`（mkdir、cp、heredoc 写 Info.plist） | `release.yml:306-339` | 能。Info.plist 改用 §2 的键 |
| 2 | 资产名不带 v、不带 gpui 字样，沿用旧 Tauri 版的形态 | `release.yml:27-29` | 能：`mini-todo_<ver>_universal.dmg` |
| 3 | hdiutil 显式给 `-size`（2 倍内容 + 64 MB），失败时 detach 残留卷、尺寸翻倍、sleep 15，最多 5 次 | `release.yml:342-362` | 能，原样抄。卷名改成 `mini-todo` |
| 4 | 先 `df -h` 打印磁盘，方便排查 | `release.yml:349` | 能 |
| 5 | `gh release upload --clobber` 只传文件，用 `nullglob` 收集 | `release.yml:463-476` | 能。mini-todo 现有写法会断言文件个数（`release.yml:271-278`），mac 作业断言 `==1` |
| 6 | plan 作业算平台，`workflow_dispatch` 可以只补建某个平台 | `.github/scripts/release_matrix.mjs` | mini-todo 用 bash 版 plan（`release.yml:123-160`），加一个 key 就行 |
| 7 | `fail-fast: false`，mac 失败不打断 Windows 上传；但 publish 要**全部**成功才转正，修好后 Re-run failed jobs | `release.yml:30-35` | 与 mini-todo 现有约定一致（`.trellis/spec/pc-gpui/release.md` §3） |
| 8 | rust-cache 只读，由 cache-warm 在 main 上预热，release 各平台全部预热（"publish 要等最慢的那个"） | `cache-warm.yml:32-36、205-260` | 能。mac 作业的步骤和编译命令要逐字对齐 |
| 9 | 显式 `rustup target add "$MATRIX_TARGET"`，给以后在 arm64 上交叉编 Intel 留口子 | `release.yml:193-201` | mini-todo 用 universal，直接装两个 target |
| 10 | 未签名包的用户指引：`xattr -cr /Applications/<App>.app`，每次升级要再执行一次 | `README.md:119-125`、`docs/features.zh-CN.md:240-251` | 能，并补上"仍要打开"这条路（§3.4） |
| 11 | 守门测试：`cargo tree --target aarch64-apple-darwin -e features -i <font-kit 包>`，断言 mac 字体后端已开 | `tests/gpuiMacFontKit.test.cjs` | 建议移植成 Rust / Python 测试，同时断言 `font-kit` 和 `runtime_shaders`（gpui-kit 升级时把关） |
| 12 | 无真机时在 issue 里写明"没有真机验证，请反馈" | issue #77 / #82 的回复 | 发布说明照写（PRD 已决定写明"未经真机验收"） |

**不照搬的：**
- mini-term 的 Info.plist 把带 `-pre` 的完整版本号写进了 `CFBundleShortVersionString`。实测能启动，但不符合 Apple 的格式要求（§2.3）。
- mini-term 用 `sips` 从 `docs/icon.png` 现做 icns，只生成了 16 到 512 的单倍图、没有 @2x。mini-todo 直接用旧 icns，质量更好。
- mini-term 没有做 ad-hoc 整包签名。
- mini-term 的 dmg 里没有 Applications 软链。

---

## 1. 产物形态

### 1.1 GitHub 免费 macOS runner 现状（2026-10，public 仓库免费且不限分钟；本仓 `dreamlonglll/mini-todo` 是 PUBLIC）

| 标签 | 架构 | 规格（public 仓库的标准 runner） | 状态 | 出处 |
|---|---|---|---|---|
| `macos-latest` | arm64 | 3 核 M1 / 7 GB | 2026-06-15 到 07-15 期间切到 macos-26 | runner-images#14167 |
| `macos-26` | arm64 | 同上 | GA（2026-02-26），默认 Xcode 26.6 | #13739；`images/macos/macos-26-arm64-Readme.md` |
| `macos-15` | arm64 | 同上 | 在支持期，默认 Xcode 16.4（另装 26.x），Rust 1.98.1，Rosetta 已装 | `macos-15-arm64-Readme.md`；`templates/macOS-15.arm64.anka.pkr.hcl:170` |
| `macos-14` | arm64 | 同上 | **淘汰中**：2026-11-02 退役；10 月有多次 brownout（包括今天 10-16 14:00 到 24:00 UTC） | #13518 |
| `macos-15-intel` | x86_64 | 4 核 / 14 GB | "available from now until **August 2027** … the last available x86_64 image" | #13045 |
| `macos-26-intel` | x86_64 | 4 核 / 14 GB | 2026-02-26 GA，出现在标准 runner 表里；**没有公布退役日期** | #13739；docs「GitHub-hosted runners」标准 runner 表 |

- 免费计划同时最多跑 **5 个 macOS 作业**（docs「Actions limits」）。
- Intel 镜像已知的问题：`macos-15-intel` 上 Metal 多重采样纹理会卡死 GPU 或给出错误结果（#14836，2026-10 仍未关闭，arm64 不受影响）；
  `macos-26`（Intel）上 `iconservicesagent` 崩溃循环，任何 NSWorkspace 取文件图标的调用都会永远挂住（#14751）。
  GPUI 的 Metal 渲染器画 path 用 4x MSAA（`gpui-pre-apple-0.3.9/src/metal_renderer.rs:37` `PATH_SAMPLE_COUNT: u32 = 4`），
  **GUI e2e 不要放在 Intel runner 上跑**。

### 1.2 universal（lipo）单个 dmg 与 arm64 / x64 两个 dmg 的对比

| 维度 | universal 单 dmg（推荐） | 两个 dmg（mini-term 的做法） |
|---|---|---|
| 用户 | 不用分芯片；2.5.0 用户找的就是 `universal.dmg` | 要先分清 Apple Silicon 和 Intel；Intel 用户下错包打不开（mini-term issue #77 就是这样来的） |
| runner | 只要 arm64（`macos-15`），x86_64 交叉编译 | Intel 包原生编译依赖 `macos-15-intel`（2027-08 之后得改成交叉编译或 lipo）或 `macos-26-intel`（退役日期未知） |
| 构建时间 | 一个作业编两个 target（可以用 cargo 多 `--target` 一次调度）。约等于单 target 的 2 倍，再减去并行收益 | 两个作业并行；Intel runner 本身慢（mini-term 有缓存时 Intel 15.5 分钟，arm64 7 分钟，§5.5） |
| 体积 | 约 2 倍（2.5.0 universal dmg 13.6 MB；`.app` 里的二进制 40 MB = x64 20.4 MB + arm64 20.0 MB，实测） | 各占一半 |
| 测试覆盖 | arm64 原生跑；x86_64 只能在 arm64 上用 `arch -x86_64` 走 Rosetta 跑 | x64 包可以在 Intel runner 原生跑，但那里 Metal MSAA 不可靠（#14836） |
| 交叉编译风险 | mini-todo 的 C/ObjC 依赖只有 `libsqlite3-sys`（bundled）、`mac-notification-sys`；TLS 走 `native-tls`（Security.framework，没有 OpenSSL）。都是 Xcode clang 开 `-arch x86_64` 就能编（`cargo tree --target x86_64-apple-darwin -i cc` 实测） | 无 |
| 先例 | 2.5.0 的旧 `build.yml`：`macos-latest` + `--target universal-apple-darwin`（`git show 749405a:.github/workflows/build.yml`） | mini-term `release.yml:11-15` |

**建议**：universal，资产名 **`mini-todo_<ver>_universal.dmg`**（例如 `mini-todo_3.0.2-pre_universal.dmg`）。
- 卷名、`.app` 文件夹名都用 `mini-todo`。`.app` 名必须和 2.5.0 一样，用户把新包拖进「应用程序」时才会直接替换旧包，路径才能保持不变（见 §2.1）。
- 不再出 `mini-todo_universal.app.tar.gz`：那是 tauri-action 为了上传 .app 目录生成的副产物，2.5.0 没有用 updater 插件（`git show 749405a:pc/src-tauri/src/lib.rs` 的插件列表里没有 updater）。
- 如果以后觉得构建太慢，可以改成"两个并行构建作业上传二进制 → 第三个 mac 作业 lipo、组装、出 dmg"，资产名不变。

---

## 2. .app 组装

### 2.1 2.5.0 实物（下载 `mini-todo_universal.app.tar.gz` 解包，2026-10-16 实测）

```
mini-todo.app/Contents/Info.plist
mini-todo.app/Contents/MacOS/mini-todo        # fat：x86_64 20,365,808 B + arm64 19,988,336 B
mini-todo.app/Contents/Resources/icon.icns
（没有 _CodeSignature）
```

Info.plist 原文中的键：`CFBundleDevelopmentRegion=English`、`CFBundleDisplayName=mini-todo`、`CFBundleExecutable=mini-todo`、
`CFBundleIconFile=icon.icns`、**`CFBundleIdentifier=com.tauri-app.mini-todo`**、`CFBundleInfoDictionaryVersion=6.0`、`CFBundleName=mini-todo`、
`CFBundlePackageType=APPL`、`CFBundleShortVersionString=2.5.0`、`CFBundleVersion=2.5.0`、`CSResourcesFileMapped=true`、
`LSMinimumSystemVersion=10.13`、`LSRequiresCarbon=true`、`NSHighResolutionCapable=true`。**没有 `LSUIElement`**（Dock 图标照常显示）。

Mach-O 解析：
- arm64 切片：`LC_BUILD_VERSION minos 11.0 / sdk 26.5`；`LC_CODE_SIGNATURE` 的 CodeDirectory flags 是 `0x20002`（= `CS_ADHOC | CS_LINKER_SIGNED`），identifier 是 `mini_todo-8985ccf14a845afb`（链接器生成）。
- x86_64 切片：没有签名。

`tauri.conf.json`（`git show 749405a:pc/src-tauri/tauri.conf.json`）：`"identifier": "com.tauri-app.mini-todo"`、`"productName": "mini-todo"`，`bundle.icon` 里有 `icons/icon.icns`。

### 2.2 推荐的 Info.plist

| 键 | 值 | 理由 / 出处 |
|---|---|---|
| `CFBundleIdentifier` | `com.tauri-app.mini-todo` | 与 2.5.0 一致：通知授权按 bundle id 记、LaunchServices 视为同一个应用、Dock 固定图标不丢；也等于 `pc-gpui/crates/app/src/lib.rs:31` 的 `APP_IDENTIFIER`。gpui-pre-macos 的系统通知**必须在 bundle 里、有 bundle id** 才会启用（`gpui-pre-macos-0.3.9/src/system_notifications.rs:119-127`，裸二进制运行时会直接禁用，否则 `UNUserNotificationCenter` 会 abort） |
| `CFBundleExecutable` | `mini-todo` | 与 2.5.0 一致，也就是 cargo bin 名 |
| `CFBundleName` | `mini-todo`（备选 `Mini Todo`） | 2.5.0 是 `mini-todo`；它显示在菜单栏应用菜单和 Dock 悬停提示里。属于产品取舍，身份兼容不依赖它 |
| `CFBundleDisplayName` | 同 `CFBundleName` | |
| `CFBundleIconFile` | `icon.icns` | 与 2.5.0 一致 |
| `CFBundlePackageType` | `APPL` | |
| `CFBundleInfoDictionaryVersion` | `6.0` | |
| `CFBundleShortVersionString` | `${VERSION%%[-+]*}`，如 `3.0.1` | Apple："The required format is three period-separated integers … can only contain numeric characters (0-9) and periods"（developer.apple.com `cfbundleshortversionstring`） |
| `CFBundleVersion` | 同上 `3.0.1` | Apple：一到三段整数，只能是数字和点，多出来的段系统会忽略。预发布版和正式版会同号，不影响非 App Store 分发 |
| `LSMinimumSystemVersion` | `11.0` | arm64 的下限本来就是 11（2.5.0 的 arm64 切片 minos 也是 11.0）；mini-term 用 11.0；GPUI 代码按 10.15 带运行时判断（`gpui-pre-macos-0.3.9/src/window.rs:2302`）。CI 只能在 15 / 26 上测，写得更低也测不到。**代价**：2.5.0 写的是 10.13，10.13 到 10.15 的用户会被挡住 |
| `NSHighResolutionCapable` | `true` | 2.5.0、mini-term 都设了；Retina 下按 2x 渲染 |
| `CFBundleDevelopmentRegion` | `zh_CN` | 2.5.0 是 `English`。没有 `.lproj` 时，系统提供的 UI（文件面板按钮等）很可能按开发区域的语言显示。**待实测**；也可以加 `CFBundleAllowMixedLocalizations=true` |
| `LSUIElement` | **不设** | 2.5.0 没设。设了就没有 Dock 图标和应用菜单；目前非 Windows 平台关掉最后一个窗口就退出（`lib.rs:67-73`）。要不要做成托盘常驻，由 gpui-macos-platform 调研决定 |
| `LSApplicationCategoryType` | `public.app-category.productivity`（可选） | 只影响分类展示 |

不需要：`LSRequiresCarbon`、`CSResourcesFileMapped`（Tauri 模板遗留的）、`NSPrincipalClass`（GPUI 自己创建 NSApplication）、各种 UsageDescription（没用到受保护资源）。

### 2.3 版本号

- 唯一来源是 `pc-gpui/Cargo.toml` 的 `[workspace.package] version`。release 里 `create-release` 已经用 awk 校验过 tag 等于版本
  （`release.yml:75-83`），build 作业直接用 `VERSION="${RELEASE_TAG#v}"`，和 build-windows 一样。
- **workflow_dispatch 补建时 create-release 被跳过，这个校验不会跑**。组装脚本最好自己再用同一行 awk 读一次 Cargo.toml，和 tag 比对；
  CI（非 tag）也靠这行 awk 取版本：
  `awk -F'"' '/^\[workspace\.package\]/{f=1;next} /^\[/{f=0} f && /^version *=/{print $2; exit}' pc-gpui/Cargo.toml`
- 数字段：`SHORT="${VERSION%%[-+]*}"`（`3.0.1-pre` 得到 `3.0.1`），口径与 Windows 的 `VIProductVersion` 相同（`release.yml:212-213`）。完整 semver 仍然
  由应用内的 `APP_VERSION`（`CARGO_PKG_VERSION`）显示在关于页。
- mini-term 把 `1.13.6-pre` 这种原样写进两个键，实测 Finder 能启动，但不合 Apple 规范，不跟。

### 2.4 图标

| 来源 | 内容（实测） | 结论 |
|---|---|---|
| `git show 749405a:pc/src-tauri/icons/icon.icns` | 109,417 B；`ic07/08/09/10/11/12/13/14` + `il32/is32` 全档（16 到 512@2x，即 1024） | **推荐**：检回仓库，例如 `pc-gpui/crates/app/assets/icon.icns`，组装时 `cp` 进去，CI 不依赖 iconutil |
| `git show 749405a:pc/src-tauri/icons/icon.png` | 512×512 | 如果想在 CI 现做：`sips` 缩放出 `icon_16x16[@2x].png` 到 `icon_256x256@2x.png`，再 `iconutil -c icns`；缺 512@2x |
| `pc-gpui/crates/app/assets/icon.ico` | 与旧版 `icon.ico` 字节完全相同；最大 256×256 | 不适合做 icns（Retina 下发糊） |

### 2.5 组装脚本骨架（建议放到 `pc-gpui/scripts/macos/bundle.sh`，release 和 CI 共用，Info.plist 的逻辑在发版前就能在 CI 里跑到）

```bash
#!/bin/bash
set -euo pipefail
# 用法：bundle.sh <arm64 二进制> <x86_64 二进制|-> <输出目录>
VERSION=$(awk -F'"' '/^\[workspace\.package\]/{f=1;next} /^\[/{f=0} f && /^version *=/{print $2; exit}' pc-gpui/Cargo.toml)
SHORT="${VERSION%%[-+]*}"
APP="$3/mini-todo.app"
rm -rf "$APP"; mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
if [ "$2" = "-" ]; then cp "$1" "$APP/Contents/MacOS/mini-todo"
else lipo -create -output "$APP/Contents/MacOS/mini-todo" "$1" "$2"; fi
lipo -archs "$APP/Contents/MacOS/mini-todo"          # universal 时断言含 "x86_64 arm64"
cp pc-gpui/crates/app/assets/icon.icns "$APP/Contents/Resources/icon.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>com.tauri-app.mini-todo</string>
  <key>CFBundleExecutable</key><string>mini-todo</string>
  <key>CFBundleName</key><string>mini-todo</string>
  <key>CFBundleDisplayName</key><string>mini-todo</string>
  <key>CFBundleIconFile</key><string>icon.icns</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleShortVersionString</key><string>${SHORT}</string>
  <key>CFBundleVersion</key><string>${SHORT}</string>
  <key>CFBundleDevelopmentRegion</key><string>zh_CN</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
plutil -lint "$APP/Contents/Info.plist"
codesign --force --sign - --timestamp=none "$APP"      # §3：ad-hoc 封整个 bundle（也会重签 lipo 后的每个切片）
codesign --verify --deep --strict --verbose=2 "$APP"
codesign -dv --verbose=4 "$APP" 2>&1 | grep -E 'Identifier|Signature|Sealed Resources|Info.plist'
```

部署目标：Rust 默认 `aarch64-apple-darwin` 是 11.0，`x86_64-apple-darwin` 是 10.12，都不高于 plist 的 11.0，**不需要**设
`MACOSX_DEPLOYMENT_TARGET`。如果要设，必须写在 release 和 cache-warm 两个 workflow 的同一层 env 里。它不在 rust-cache 的环境哈希前缀
（CARGO / CC / CFLAGS / CXX / CMAKE / RUST）之内，缓存键看不出两边不一致。

---

## 3. 签名与 Gatekeeper（没有 Apple Developer 账号）

### 3.1 事实

- **Apple Silicon 上原生 arm64 代码必须至少有 ad-hoc 签名**。Tauri 官方文档："This is useful on ARM (Apple Silicon) devices, where code-signing is required
  for all apps from the Internet"，并且 "Ad-hoc code signing does not prevent MacOS from requiring users to whitelist the installation in their Privacy & Security settings"
  （tauri-docs `src/content/docs/distribute/Sign/macos.mdx` 的「Ad-Hoc Signing」节）。
- 链接器会自动给 arm64 二进制加 ad-hoc 签名（linker-signed），2.5.0 实物就是这样（§2.1）。这个签名不封 Info.plist / Resources，整个 bundle 不算"签过名"。
- **社区经验**（不是 Apple 官方文档）：带 quarantine 的 arm64 应用如果只有链接器签名，会被提示"已损坏，无法打开"，只能 `xattr -cr`；对整包做 ad-hoc 签名后，
  提示变成"无法验证开发者"一类，可以走「隐私与安全性 → 仍要打开」。出处：tauri-apps/tauri#8763（"adhoc code signing the app … you'll get rid of the
  'app is damaged' dialog"）、djfbryant/slugtale#49（2026："avoids the macOS 15 'app is damaged' hard block and the Privacy & Security 'Open Anyway' path works"）。
  mini-term 没有整包签名，它的 README 写的就是"已损坏，执行 xattr -cr"（`D:\Git\mini-term\README.md:119-125`）。
- **macOS 15 Sequoia 去掉了右键打开绕过**。Apple Developer News（2024-08-06，`developer.apple.com/news/?id=saqachfa`）："In macOS Sequoia, users will no longer be able to
  Control-click to override Gatekeeper when opening software that isn't signed correctly or notarized. They'll need to visit System Settings > Privacy & Security…"
- Apple 支持文档 mh40616（「Open a Mac app from an unknown developer」）：系统设置 → 隐私与安全性 → 安全性 → 「Open Anyway」（中文系统是「仍要打开」）。
  按钮只在尝试打开后约 1 小时内出现，确认时要输入登录密码，之后就记为例外。

### 3.2 建议

- 组装的最后一步：`codesign --force --sign - --timestamp=none mini-todo.app`。顺序是 lipo、写 Info.plist、复制图标，最后签名；签完不要再改 bundle 里的任何文件。
  codesign 会给 universal 二进制的每个切片分别签名，也会封 Info.plist 和 Resources。
  这样还能顺带消除一个隐患：cargo `strip = true`（`pc-gpui/Cargo.toml` 的 release profile）是在链接之后剥符号，剥完链接器签名还在不在取决于工具链；
  最后统一重签就不用操心这一点。mini-term 剥完不重签，arm64 包在用户 M3 上也能跑（issue #82 的用户实际跑起来了），不过那是经验，不是保证。
- **不开 hardened runtime**（`--options runtime`），也不打时间戳：只有公证才要求，ad-hoc 签名用不上。
- CI 校验：`codesign --verify --deep --strict`。`spctl --assess --type execute -vv mini-todo.app` 只作信息输出：ad-hoc 签名必然 `rejected`，不要当作失败条件。

### 3.3 ad-hoc 签名的已知限制（写进发布说明或文档）

- ad-hoc 签名没有稳定身份（designated requirement 就是 cdhash），每个版本都是"新应用"：**每次升级都要重新放行一次**；以后如果申请辅助功能等 TCC 权限，每次升级也得重新授权。
  通知授权按 bundle id 记，不受影响。
- 不要在挂载的 dmg 里或"下载"文件夹里直接打开：带 quarantine 的 app 在原地运行会被 **App Translocation** 挪到随机的只读路径。
  到时 `current_exe()` 拿到的是随机路径，开机自启的"自愈刷新路径"（`services/autostart.rs` 的 `refresh_if_enabled`）会写进一个无效路径。
  用户用 Finder 拖进「应用程序」，或者先 `xattr -cr`，就不会被挪。这一点交给 gpui-macos-platform 调研决定要不要在代码里防一下。
- 以后有了 Developer ID（99 美元 / 年）：`codesign --options runtime --timestamp -s "Developer ID Application: …"` → `xcrun notarytool submit --wait` →
  `xcrun stapler staple <dmg>`，需要 p12 证书和 App Store Connect API key 两个 secret。runner 上公证也会有网络问题（runner-images#14924，2026-10：
  `macos-15-intel` 访问不了 `api.apple-cloudkit.com`，staple 报 Error 68）。

### 3.4 发布说明可直接用的用户指引（草稿）

> **macOS 版（预览）**：`mini-todo_<ver>_universal.dmg`，Apple 芯片和 Intel 芯片通用，需要 macOS 11 或更高。**未经真机验收**，遇到问题请提 Issue 并附截图。
>
> 1. 打开 dmg，把 **mini-todo** 拖进「应用程序」文件夹（装过 2.5.0 的直接覆盖，数据保留在 `~/Library/Application Support/mini-todo`）。
> 2. 本应用没有 Apple 开发者签名，第一次打开会被系统拦下，二选一放行：
>    - **方法一（推荐，终端一条命令）**：`xattr -cr /Applications/mini-todo.app`，之后正常双击打开。
>    - **方法二（不用终端）**：先双击打开一次，在弹框里点「完成」；然后打开「系统设置 → 隐私与安全性」，在「安全性」下点「仍要打开」，
>      输入登录密码确认（这个按钮只在尝试打开后约一小时内出现）。macOS 15 起「右键 → 打开」已经不能绕过。
>    - 如果提示「已损坏，无法打开」，只能用方法一。
> 3. 每次升级新版本后需要重新放行一次。

---

## 4. dmg 制作

```bash
STAGE="$RUNNER_TEMP/dmg-root"; rm -rf "$STAGE"; mkdir -p "$STAGE"
ditto dist/mini-todo.app "$STAGE/mini-todo.app"       # 用 ditto 不用 cp：保留签名、扩展属性、软链
ln -s /Applications "$STAGE/Applications"             # 拖放安装的目标
DMG="dist/mini-todo_${VERSION}_universal.dmg"
df -h .
SIZE_MB=$(( $(du -sm "$STAGE" | cut -f1) * 2 + 64 ))
for attempt in 1 2 3 4 5; do
  if hdiutil create -volname "mini-todo" -srcfolder "$STAGE" -fs HFS+ -format UDZO \
       -size "${SIZE_MB}m" -ov "$DMG"; then break; fi
  echo "::warning::hdiutil 第 $attempt 次失败（size=${SIZE_MB}m）"
  [ "$attempt" -lt 5 ] || { echo "::error::hdiutil 连续 5 次失败"; exit 1; }
  hdiutil detach /Volumes/mini-todo -force >/dev/null 2>&1 || true
  rm -f "$DMG"; SIZE_MB=$(( SIZE_MB * 2 )); sleep 15
done
# 挂载回读校验
MNT="$RUNNER_TEMP/dmg-mnt"; mkdir -p "$MNT"
hdiutil attach -nobrowse -readonly -mountpoint "$MNT" "$DMG"
codesign --verify --deep --strict "$MNT/mini-todo.app"
lipo -archs "$MNT/mini-todo.app/Contents/MacOS/mini-todo"
test -L "$MNT/Applications"
hdiutil detach "$MNT"
```

- 参数：`UDZO`（zlib 压缩、只读，与 2.5.0、mini-term 相同）；`-fs HFS+` 显式写出来。create-dmg（Tauri 的 `bundle_dmg` 就是从它 fork 的）默认用
  `FILESYSTEM="HFS+"`（create-dmg 脚本第 26 行）。`-size` 只决定临时卷的容量，UDZO 压缩后的体积只看实际内容。
- **为什么要重试**：mini-term 在 v1.2.4、v1.2.6 以及 v1.2.6 的 Intel 补建中三次碰到 `create failed - No space left on device / could not access /Volumes/…`。
  原因是 `-srcfolder` 自动估算的临时卷偏小，不是 runner 磁盘满了（`D:\Git\mini-term\.github\workflows\release.yml:342-348`，引用 runner-images#4288）。
  同类的随机失败还有：#7522（`Resource busy`）、#12323（`resize: Device not configured`）。create-dmg 自己也有 `hdiutil_retry`，专门重试 `Resource busy`、
  指数退避（脚本 49-82 行）。mini-term 的写法（给足 size，失败后 detach、翻倍、重试）能覆盖这几种情况。
- 可选美化（窗口背景、图标位置）需要在挂载状态下用 AppleScript 操作 Finder，在 runner 上不稳定（create-dmg 专门有 `--applescript-sleep-duration` 应对 `-1728`）。**不做**。

---

## 5. CI 集成

### 5.1 `release.yml`

**plan**：在 `release.yml:135-160` 的 bash 里加一个 key。

```bash
keys="windows,macos,cloud-x86_64,cloud-aarch64"          # push 全量
...
macos) macos=true ;;
echo "macos=$macos" >> "$GITHUB_OUTPUT"
# 空平台判断加上 macos；workflow_dispatch 的 platforms 描述改成 windows / macos / cloud-x86_64 / cloud-aarch64
```

`outputs:` 加 `macos: ${{ steps.plan.outputs.macos }}`。如果以后改成两个 dmg，就照 cloud 输出 JSON 数组，交给 matrix（`build-cloud` 的写法），并且同样先判断 `!= '[]'`。

**build-macos**（结构与 `build-windows` 相同；action 一律钉 SHA，下面沿用本仓已经钉好的那几个）：

```yaml
  build-macos:
    name: macOS universal (pc-gpui)
    needs: [create-release, plan]
    if: >-
      !cancelled() && needs.plan.result == 'success' && needs.plan.outputs.macos == 'true' &&
      (needs.create-release.result == 'success' || needs.create-release.result == 'skipped')
    runs-on: macos-15            # ⚠️ 与 cache-warm.yml 的 gpui-macos 同一个标签（键里带 runner OS/arch；SDK 也要一致）
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09 # v5
        with: { ref: '${{ env.RELEASE_TAG }}', persist-credentials: false }
      # ⚠️ 与 cache-warm.yml 的 gpui-macos 一致
      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@e2a55d2ffb04f378e9626c28d38b36d230d1e12f # master
        with:
          toolchain: stable
          targets: aarch64-apple-darwin,x86_64-apple-darwin
      - name: Rust cache
        uses: swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2
        with:
          shared-key: release-gpui        # 键里带 OS/arch，和 Windows 的同名 shared-key 不冲突（mini-term 四个平台共用一个 "release"）
          save-if: false
          workspaces: pc-gpui -> target
      # 不需要 Metal 编译器：gpui-kit 开了 runtime_shaders（见 §5.4）
      - name: Build
        working-directory: pc-gpui
        run: cargo build --release -p mini-todo --locked --target aarch64-apple-darwin --target x86_64-apple-darwin
      - name: Assemble .app + ad-hoc sign + dmg
        shell: bash
        run: |
          VERSION="${RELEASE_TAG#v}"
          bash pc-gpui/scripts/macos/bundle.sh \
            pc-gpui/target/aarch64-apple-darwin/release/mini-todo \
            pc-gpui/target/x86_64-apple-darwin/release/mini-todo dist
          # …§4 的 dmg 步骤…
      - name: Upload to release
        shell: bash
        env: { GH_TOKEN: '${{ github.token }}', GH_REPO: '${{ github.repository }}' }
        run: |
          shopt -s nullglob
          files=(dist/mini-todo_*_universal.dmg)
          [ "${#files[@]}" -eq 1 ] || { echo "::error::dist/ 下应有 1 个 dmg，实际：${files[*]:-<none>}"; exit 1; }
          gh release upload "$RELEASE_TAG" "${files[@]}" --clobber
```

- `gh` 已经预装在 macOS 镜像里（`macos-15-arm64-Readme.md`：GitHub CLI 2.100.0），不需要像 arm64 Linux 那样兜底安装。
- **publish**：`needs` 加 `build-macos`，`if` 里的最后一项改成
  `(needs.build-windows.result == 'success' || needs.build-macos.result == 'success' || needs.build-cloud.result == 'success')`。
  `!contains(needs.*.result, 'failure')` 自动覆盖 mac 失败的情况：mac 挂了 release 就停在 draft，与现有约定一致（`.trellis/spec/pc-gpui/release.md` §3）。
  **需要拍板**：mac 是"预览"平台，要不要让它挂了也照样发布 Windows？mini-term 选的是"全部成功才转正"；只发部分平台时手动
  `gh release edit --draft=false`，之后补建一次就会补上 SHA256SUMS（mini-term `release.yml:30-35`）。
- **SHA256SUMS**：publish 先 `gh release download` 拿到全部资产再算（`release.yml:371-392`），dmg 自动进入，不用改。
- 文档：要同步更新 `.trellis/spec/pc-gpui/release.md` §3 的流程图和资产表、文件头注释、`CLAUDE.md` 的「CI 与发布」与 README 下载说明
  （目前写的是"3.0 起只提供 Windows x64 版本"，`README.md:7`）。

### 5.2 `cache-warm.yml`

```yaml
  gpui-macos:
    name: release cache (pc-gpui, macOS universal)
    if: github.ref == 'refs/heads/main'
    runs-on: macos-15
    steps:
      - uses: actions/checkout@fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09 # v5
        with: { persist-credentials: false }
      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@e2a55d2ffb04f378e9626c28d38b36d230d1e12f # master
        with:
          toolchain: stable
          targets: aarch64-apple-darwin,x86_64-apple-darwin
      - name: Rust cache
        uses: swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2
        with:
          shared-key: release-gpui
          workspaces: pc-gpui -> target
      - name: Build
        working-directory: pc-gpui
        run: cargo build --release -p mini-todo --locked --target aarch64-apple-darwin --target x86_64-apple-darwin
```

- **键一致性**（`cache-warm.yml:9-21` 的约束同样适用）：workflow 级 env（`CARGO_TERM_COLOR: always`）、装工具链那一步（包括 `targets` 输入）、
  `shared-key`、`workspaces`、`runs-on`、**编译命令（两个 `--target` 的写法和顺序）**都要与 build-macos 逐字一致。文件头的作业对照表要加一行。
  mini-term 的做法是只要缓存精确命中就跳过编译（`if: steps.rust-cache.outputs.cache-hit != 'true'`，`cache-warm.yml:254-260`）；本仓现在没这么做，可以顺手加上。
- **缓存预算（实测）**：`gh api repos/dreamlonglll/mini-todo/actions/cache/usage` 返回 **8,569,667,453 B（约 8.57 GB）/ 15 条**，额度 10 GB。
  里面各有两代：`release-gpui` Windows 676 MB ×2、`pc-gpui-ci` windows 1250 MB ×2、linux 786 MB ×2、e2e 分支缓存约 0.7 GB ×3 等。
  按 Windows release 一个 target 676 MB 粗算，mac universal 两个 target 约 1.2 到 1.5 GB，再加一个 mac CI 作业约 1 GB，**会超出 10 GB**。
  超出后按最近访问时间从旧到新淘汰；也可以由仓库管理员调高上限，超出部分计费（docs「Dependency caching → Usage limits and eviction policy」）。
  缓解办法：CI 的 rust-cache 只在 main 上保存（`save-if: ${{ github.ref == 'refs/heads/main' }}`），e2e 分支的缓存不保存或保存后清理。

### 5.3 `pc-gpui-ci.yml`：建议加 macOS 作业

理由：
- mac 专属的 `cfg` 分支（objc 依赖、auto-launch / notify-rust 的 mac 实现、以后加的 mac 平台代码）在 Linux 上编不到。
- 也没法在 Windows / Linux 上做交叉 `cargo check --target aarch64-apple-darwin`：bundled sqlite、mac-notification-sys 的 cc 编译需要 Apple SDK。
- 用户没有 Mac，这是唯一的编译门。public 仓库的 macOS runner 免费。

```yaml
  macos:
    name: clippy / tests / universal bundle (pc-gpui, macOS)
    runs-on: macos-15
    defaults: { run: { working-directory: pc-gpui } }
    steps:
      - uses: actions/checkout@v4                         # 本文件现有风格用 tag；要钉 SHA 就整份一起改
      - uses: dtolnay/rust-toolchain@stable
        with: { components: clippy, targets: 'aarch64-apple-darwin,x86_64-apple-darwin' }
      - uses: swatinem/rust-cache@v2
        with: { workspaces: './pc-gpui -> target', save-if: "${{ github.ref == 'refs/heads/main' }}" }
      - run: cargo clippy --workspace --all-targets --locked -- -D warnings
      - run: cargo test --workspace --locked --no-fail-fast
      - run: cargo build --release -p mini-todo --locked --target aarch64-apple-darwin --target x86_64-apple-darwin
      - name: Bundle
        working-directory: .
        run: |
          bash pc-gpui/scripts/macos/bundle.sh pc-gpui/target/{aarch64,x86_64}-apple-darwin/release/mini-todo dist
          ditto -c -k --keepParent dist/mini-todo.app dist/mini-todo-macos-universal.zip   # upload-artifact 会丢软链和权限，先用 ditto 打包
      - uses: actions/upload-artifact@v4
        with: { name: mini-todo-gpui-macos-universal, path: dist/mini-todo-macos-universal.zip, if-no-files-found: error }
```

- `cargo fmt --check` 与平台无关，Windows / Linux 作业已经在跑，mac 不用重复。
- 首轮大概率要处理 mac 下的 `dead_code` / `unused` 警告（`-D warnings`），以及测试里假设 Windows / Linux 路径的地方。这些属于实现工作，不在本调研范围。
- 想在推 `claude/**` 分支时就拿到 mac 反馈，可以像 `pc-gpui-e2e-windows.yml` 那样加 `push: branches: ['claude/**']`（同时注意缓存预算）。

### 5.4 gpui-pre-macos 构建期需要什么工具（已核实）

- `cargo tree --offline --locked --target aarch64-apple-darwin -e features -i gpui-pre-apple`（在 `pc-gpui/` 下，2026-10-16 实测）的结果是：
  `gpui-pre-platform feature "font-kit"` 和 `feature "runtime_shaders"` 都由 **gpui-kit v0.7.3** 打开，传递到 `gpui-pre-macos` / `gpui-pre-apple`。
  来源：`gpui-kit-0.7.3/Cargo.toml` 的 `[target.'cfg(not(any(target_os = "ios", target_os = "android")))'.dependencies.gpui_platform]`，
  `features = ["font-kit","x11","wayland","runtime_shaders"]`。
- `gpui-pre-apple-0.3.9/build.rs`：开了 `runtime_shaders` 时只做两件事，`cbindgen` 生成 `scene.h`，再把它和 `src/shaders.metal` 拼成 `stitched_shaders.metal`；
  **不调用 `xcrun metal` / `metallib`**。`metal_renderer.rs:33-34,253` 用 `include_str!` 把源码嵌进去，运行时 `new_library_with_source` 编译。
  这依赖系统 Metal 框架自带的编译服务，用户机器不需要装 Xcode。
- 所以：
  - 构建只需要 Xcode Command Line Tools 和 macOS SDK（链接、cc 编 sqlite / ObjC）。runner 镜像自带，`macos-15` 默认 Xcode 16.4，`macos-26` 默认 Xcode 26.6。
  - **碰不到** Xcode 26 "Metal Toolchain 要另外下载"的坑：runner-images#13014（"cannot execute tool 'metal' due to missing Metal Toolchain"）、#14013、#14717。
  - 代价：每次启动在运行时编译一次着色器，具体耗时没有测过。
- 不同于 Windows：Windows debug 构建会按构建机上的绝对路径在运行时编译 HLSL，拷到别的机器跑不起来；mac 这边着色器源码已经嵌进二进制，debug 构建也能拷到别的机器跑。
- 守门建议：参照 mini-term 的 `tests/gpuiMacFontKit.test.cjs`，加一条断言，确认 mac 目标下 `gpui-pre-macos feature "font-kit"` 和 `"runtime_shaders"` 都开着，防止 gpui-kit 升级时悄悄变了。

### 5.5 构建耗时参考（mini-term 实测，`gh api …/runs/<id>/jobs`）

| 运行 | macos-latest (arm64) | macos-15-intel | windows |
|---|---|---|---|
| mini-term v1.13.15（缓存命中） | 作业 9 分钟（`Build GPUI app` 416 s） | 22 分钟（`Build GPUI app` 930 s，sidecar 220 s） | 14 分钟 |
| mini-term v1.13.13 | 8 分钟 | 15 分钟 | 15 分钟 |
| mini-todo v3.0.0-pre（Windows，本仓） | — | — | 16 分钟（`Build` 629 s，NSIS 184 s） |

mini-term 注释里说 macos-15-intel 冷编约 40 分钟（`cache-warm.yml:32-33`）。mini-todo 的依赖比 mini-term 少（没有 tree-sitter、russh 等），
universal 要编两遍，冷编估计和 Windows 是一个量级（十几到二十几分钟），**没有实测**。

---

## 6. runner 上能自动化验证什么

### 6.1 runner 环境事实（一手出处：actions/runner-images 的 `images/macos/scripts/build/*.sh`；arm64 模板 `templates/macOS-15.arm64.anka.pkr.hcl:170,178,179,295` 都引用了这些脚本）

| 事实 | 出处 |
|---|---|
| **自动登录 GUI 会话**：`autoLoginUser $USERNAME`、`autoLoginUserScreenLocked false` | `configure-autologin.sh` |
| 屏幕、磁盘、系统**都不休眠**；全局关闭 App Nap | `configure-machine.sh`（`pmset -a sleep 0 disksleep 0 displaysleep 0`） |
| 桌面是**纯黑壁纸**；"Close all finder windows because they can interfere with UI tests"；`reduceMotion`、`reduceTransparency` = true | `configure-system.sh` |
| **通知中心 UI 被卸载**（`launchctl unload … com.apple.notificationcenterui.plist`），`apsd` 也被卸载 | `configure-system.sh` |
| TCC（系统和用户两个 TCC.db）已授予：**辅助功能**给 `/bin/bash`、`/usr/bin/osascript`、`hosted-compute-agent`、provisioner；**AppleEvents**（目标 System Events / Finder）给 bash、osascript；**PostEvent** 给 `/bin/bash`；**屏幕录制**给 `/bin/bash`、`/usr/bin/osascript`、`hosted-compute-agent`、provisioner | `configure-tccdb-macos.sh` |
| macOS 15+："<app> is requesting to bypass the system private window picker" 的提醒会抢焦点、"breaks headed UI tests"。镜像给负责进程（"on a hosted runner that is the agent, whatever tool the job actually runs"）写了远期批准，压掉这个提醒 | `configure-tccdb-macos.sh` 末尾（`ScreenCaptureApprovals.plist`） |
| **Rosetta 已安装**（arm64 镜像） | `install-rosetta.sh`；`macOS-15.arm64.anka.pkr.hcl:170` |
| 已开启开发者模式、XCTest 的 automation mode 免认证 | `configure-machine.sh` |
| 维护者说过镜像允许**运行时修改 TCC.db**（2024，针对 macOS 13 / 14） | runner-images#9529 的维护者回复 |
| 没有屏幕录制权限时 `screencapture` **照样输出 PNG，但只有壁纸、没有任何窗口**（这个坑非常隐蔽） | runner-images#8782 |
| arm64 runner 的 GPU 是半虚拟化的 "Apple Paravirtual device"，普通 Metal 工作正常；Intel runner 上 MSAA 不可靠 | #14836（作者在 `macos-15` / `macos-26` arm64 上对照测试，结果正常） |
| 系统 Python 3.14 是 Homebrew 版（PEP 668 外部管理）。e2e 用 `actions/setup-python` 装独立版本再 `pip install` | `macos-15-arm64-Readme.md` |

### 6.2 能力矩阵

| 能力 | 结论 | 怎么做 / 备注 |
|---|---|---|
| 启动 GUI 应用 | **能** | 直接执行 `mini-todo.app/Contents/MacOS/mini-todo`：环境变量能传进去，stdout / stderr 可以接住，`NSBundle.mainBundle` 仍然解析到 `.app`（bundle id 有效）。用 `open -a` 就需要 `open --env` |
| 窗口出现、位置、尺寸 | **能** | `CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly, kCGNullWindowID)` 按 `kCGWindowOwnerPID` 过滤，读 `kCGWindowBounds`（单位 point）、`kCGWindowNumber`、`kCGWindowLayer`。`kCGWindowName` 需要屏幕录制权限（有）。也可以用 `osascript -e 'tell application "System Events" to get {position,size} of windows of (first process whose unix id is <pid>)'`，不用 pip 装任何东西 |
| 截图 | **能（预期）** | `screencapture -x full.png`；只截某个窗口用 `screencapture -x -o -l <windowNumber> win.png`。**一定要断言截图里确实有窗口**（壁纸是纯黑，检查窗口区域里有没有非黑像素），否则权限失效时会得到一张假的"成功"截图（#8782） |
| 界面上有字（防 mini-term #82 那种回归） | **能** | 用 macOS Vision OCR（`pyobjc-framework-Vision` 的 `VNRecognizeTextRequest`，`recognitionLanguages=["zh-Hans","en-US"]`），断言种子数据里的待办标题能认出来；或者 `brew install tesseract tesseract-lang`，与 Linux e2e 同一个 OCR 引擎 |
| 点击、拖动 | **能（预期，首轮实测确认）** | pyobjc Quartz 的 `CGEventCreateMouseEvent` + `CGEventPost(kCGHIDEventTap, …)`，或者 `brew install cliclick`。TCC 按负责进程 = agent 判定，而 agent 有辅助功能权限。万一被拒，就 `sudo sqlite3` 往 TCC.db 补授权（#9529） |
| 键盘输入，**包括中文** | **能（预期）** | `CGEventKeyboardSetUnicodeString` 直接注入 Unicode，不经过输入法；快捷键用 keycode 加 `kCGEventFlagMaskCommand`（例如 Cmd+Enter：keycode 36）。备选：`pbcopy` + Cmd+V |
| 把窗口切到前台 | **能** | `osascript -e 'tell application "System Events" to set frontmost of (first process whose unix id is <pid>) to true'`。从 runner 的 shell 拉起的进程不一定会自己激活 |
| 读菜单栏 status item | **原则上能，待实测** | `tell application "System Events" to tell (first process whose unix id is <pid>) to get menu bar items of menu bar 2`（menu bar 2 是该进程的状态栏项），可以 `click`。**mac 版目前没有托盘**（`tray-icon` 是 `cfg(windows)` 依赖），以后加了才用得上；macOS 26 的菜单栏 / 控制中心改版有没有影响，未知 |
| 按名字找控件（AX 树） | **有限** | gpui-pre 0.3.9 自带 AccessKit（`window/a11y.rs`；gpui-pre-macos 依赖 `accesskit_macos`），但只有设了 `.id()` 的元素才进树，要有名字还得 `.aria_label()`（`elements/div.rs:1310,1332`）；文本元素只有带 id 时才算 Label（`elements/text.rs:189-198`）。mini-todo 现在的自绘控件大多没有 label，**基本还是要按布局常量和颜色识别来定位**（与 Windows / Linux e2e 相同）。要按名字定位，得先给关键控件补 label，这是代码改动 |
| 读 `data.db` 断言 | **能** | debug 构建：`MINI_TODO_DATA_DIR=<dir>`（只在 debug 构建生效）；release：`~/Library/Application Support/mini-todo/data.db`（每次都是全新 VM，可以放心用真实目录）。日志在 `~/Library/Application Support/com.tauri-app.mini-todo/logs/mini-todo.log`（`services/logging.rs:30-37`），隔离时在 `<dir>/logs/` |
| 切换深色外观 | **能** | `osascript -e 'tell application "System Events" to tell appearance preferences to set dark mode to true'` |
| 换纯色壁纸（检查透明窗口时对照用） | **能** | 用 Python 生成一张纯色 PNG，`osascript -e 'tell application "System Events" to tell every desktop to set picture to "<png>"'`（与 Windows `set-display.ps1` 的作用相同） |
| x86_64 切片 | **只能在 Rosetta 下跑** | `arch -x86_64 mini-todo.app/Contents/MacOS/mini-todo`；跑的是 Rosetta 转译，不是原生 Intel |
| 系统通知横幅 | **测不了** | 通知中心 UI 被卸载；`UNUserNotificationCenter` 首次申请授权要人点。只能断言"不崩溃、日志里有投递记录"；应用内通知窗口（GPUI 窗口）可以测 |
| Gatekeeper / quarantine 首次打开 | **测不了交互** | 可以给 app 加 quarantine 属性后跑 `spctl --assess` 看评估结果，但弹框和"仍要打开"没法自动化 |
| 开机自启真实生效 | **测不了** | 只能断言登录项 / LaunchAgent 写进去了（具体机制由 gpui-macos-platform 调研决定） |
| 多显示器、真机 Retina、真实 Intel 或独显 | **测不了** | 显示分辨率和缩放先用 `system_profiler SPDisplaysDataType` 记进日志；缩放比 = 截图像素尺寸 ÷ CGWindow 的 point 尺寸 |
| 从 Finder 拖文件进来 | 难 | 可以用 CGEvent 模拟拖动，但 Finder 那一侧不好控制；mac 不做收纳箱，优先级低 |

不依赖 TCC 的兜底：gpui 0.3.9 有 `Window::render_to_image()`，在 mac 上是 Metal 离屏渲染（`gpui-pre-macos-0.3.9/src/window.rs:2362`），
但只在 `test-support` feature 下才有（`gpui-pre-0.3.9/src/window.rs:2868-2873`）。要用的话得单独做一个 e2e 构建、加调试开关，是代码改动，作为最后的备选。

### 6.3 mac 冒烟 e2e：最小可行方案

workflow `pc-gpui-e2e-macos.yml`（触发条件与 `pc-gpui-e2e-windows.yml` 相同：`workflow_dispatch`，以及 push 到 `claude/**` 且改动了 `pc-gpui/**`），
跑在 `macos-15` 上，可以再加一条 `macos-26` 覆盖 Tahoe。**不用 Intel runner。**

1. **准备**：checkout → Rust stable（加上 x86_64 target）→ rust-cache → `setup-python 3.12` →
   `pip install pyobjc-framework-Quartz pyobjc-framework-Vision pillow`。
2. **构建**：`cargo build -p mini-todo --locked`、`cargo build -p mini-todo-core --example seed --locked`（debug，host arm64）；
   release 冒烟另外 `cargo build --release … --target aarch64-apple-darwin --target x86_64-apple-darwin`。
   然后用 `pc-gpui/scripts/macos/bundle.sh` 包成 `.app`：bundle id 才有效，与发版走同一份 Info.plist 逻辑。
3. **环境记录**：`sw_vers`、`csrutil status`、`system_profiler SPDisplaysDataType`、
   `sudo sqlite3 "/Library/Application Support/com.apple.TCC/TCC.db" "select service,client,auth_value from access"`；换纯色壁纸。
4. **W 组（窗口）**：seed 种子数据 →
   `MINI_TODO_DATA_DIR=$W/a MINI_TODO_DEBUG_OPEN=main "$APP/Contents/MacOS/mini-todo" >$W/a.out 2>&1 &` →
   CGWindowList 在 30 s 内等到这个 PID 的 layer 0 窗口，逻辑尺寸约 380×600 →
   `screencapture -l` 截窗口，断言有非壁纸像素 → **OCR 认出种子待办的标题**。
   再依次用 `editor:1:view`、`settings` 打开其它窗口，重复以上检查（`MINI_TODO_DEBUG_OPEN` 的格式见 `pc-gpui/scripts/README.md`）。
5. **I 组（最小交互）**：切到前台 → 按布局常量 / 颜色识别点 FAB（复用 Windows / Linux e2e 的坐标逻辑，Windows 用像素、mac 用 point）→
   `CGEventKeyboardSetUnicodeString` 输入"E2E 新建待办" → Cmd+Enter → **读 `$W/a/data.db`**，断言多了这一条、`updated_at` 是规范格式 →
   截图，OCR 认出新标题。
6. **退出**：Cmd+Q，或者 `osascript -e 'tell application id "com.tauri-app.mini-todo" to quit'`，或者关掉最后一个窗口（非 Windows 平台关掉最后一个窗口就退出）→
   断言进程退出、日志里没有 panic / `ERROR`。
7. **P 组（release bundle，不隔离）**：挂载 CI 构建的 dmg（或者直接用 bundle）→ `ditto` 到 `/Applications/mini-todo.app` → 原生启动：窗口出现、截图、
   `~/Library/Application Support/mini-todo/data.db` 已建好且迁移到 `LATEST_VERSION`（29）、日志路径正确 → 退出 →
   **`arch -x86_64` 再跑一遍**（Rosetta 覆盖 x86_64 切片）。可选：确认 release 构建忽略 `MINI_TODO_DATA_DIR`。
8. **产出**：`summary.md` 写进 Job Summary；截图、日志用 upload-artifact 上传；产物链接打不开时用 `emit_annotations.py` 的方式回传
   （复用 `pc-gpui/scripts/e2e-windows/emit_annotations.py`）。

可选的增量用例：
- 设置里切深色主题（断言落库 + 截图颜色）。
- 本地 WebDAV 同步：复用 `e2e-windows/dav_server.py`（wsgidav），macOS 上能跑。
- 2.5.0 升级：把 `mini-todo_universal.app.tar.gz`（2.5.0）放到 runner 上启动一次（WKWebView 是系统自带的），建出旧库，再启动 3.x，断言迁移成功、数据还在。

### 6.4 测不了的（发布说明里要写明"未经真机验收"）

- 下载后的 Gatekeeper 拦截和放行流程（§3.4 的两种方法都得靠用户反馈）。
- 系统通知横幅、通知授权弹框。
- 原生 Intel 机器（只有 Rosetta）、真机 GPU 和 Retina 下的观感、多显示器。
- 重启 / 重新登录后开机自启是否真的生效；App Translocation 场景。
- 托盘（如果以后加）在真实菜单栏上的显示和点击。

---

## 相关文件

| 文件 | 说明 |
|---|---|
| `.github/workflows/release.yml` | 发布流水线：plan（123-160）、build-windows（162-278）、publish（360-405） |
| `.github/workflows/cache-warm.yml` | 预热；键一致性约束写在 9-21 行 |
| `.github/workflows/pc-gpui-ci.yml` | 现有的 Windows / Linux 作业 |
| `.github/workflows/pc-gpui-e2e-windows.yml` | e2e workflow 的写法（触发条件、注解回传、产物） |
| `.trellis/spec/pc-gpui/release.md` | 发布契约（资产表、约定、缓存） |
| `.trellis/spec/guides/desktop-window-e2e-guide.md` | 桌面窗口 e2e 指南（mac e2e 可以参考） |
| `pc-gpui/Cargo.toml` | 版本唯一来源、release profile（thin LTO、`strip = true`） |
| `pc-gpui/crates/app/Cargo.toml` | `gpui-kit = "=0.7.3"`；`tray-icon`、`rfd` 只在 Windows 编译 |
| `pc-gpui/crates/app/build.rs` | 非 Windows 目标直接 return（mac 不嵌资源，图标靠 bundle） |
| `pc-gpui/crates/app/src/lib.rs:31,57,67-73` | `APP_IDENTIFIER`、`set_app_identity`、非 Windows 下关掉最后一个窗口就退出 |
| `pc-gpui/crates/app/src/services/logging.rs:30-37` | 日志目录（mac 下是 `~/Library/Application Support/com.tauri-app.mini-todo/logs`） |
| `pc-gpui/crates/app/src/services/autostart.rs:14-21` | auto-launch 0.5 没设 `set_use_launch_agent`（见 Caveats） |
| `pc-gpui/scripts/e2e-windows/`、`e2e-linux/` | 现有 e2e 框架（布局常量定位、OCR、数据库断言、注解回传） |
| `D:\Git\mini-term\.github\workflows\release.yml` | mac 组装、dmg 重试、上传（11-15、193-201、306-363、463-476） |
| `D:\Git\mini-term\.github\workflows\cache-warm.yml` | 四个平台的 release 预热、命中即跳过 |
| `D:\Git\mini-term\.github\scripts\release_matrix.mjs` | 平台表（macos-arm64 → `macos-latest`，macos-x64 → `macos-15-intel`） |
| `D:\Git\mini-term\tests\gpuiMacFontKit.test.cjs` | font-kit 守门测试 |

## 外部参考

- actions/runner-images README（标签表）；#13045（macos-15-intel 只到 2027-08）；#13518（macos-14 退役 2026-11-02、brownout）；#13739（macOS 26 GA，包括 `macos-26-intel`）；
  #14167（`macos-latest` 切到 26）；#14836（Intel Metal MSAA 卡死）；#14751（macOS 26 Intel 上 iconservicesagent 的问题）；#13014、#14013、#14717（Xcode 26 缺 Metal Toolchain）；
  #4288、#7522、#12323（hdiutil 随机失败）；#8782、#9529（screencapture 权限、运行时改 TCC.db）；#14924（公证 staple 网络问题）
- runner-images 脚本：`images/macos/scripts/build/configure-tccdb-macos.sh`、`configure-autologin.sh`、`configure-system.sh`、`configure-machine.sh`、`install-rosetta.sh`；
  `images/macos/macos-15-arm64-Readme.md`、`macos-26-arm64-Readme.md`
- GitHub Docs：「GitHub-hosted runners」（标准 runner 表：public 仓库的 Intel / arm64 规格）、「Actions limits」（同时最多 5 个 macOS 作业、10 GB 缓存）、
  「Dependency caching → Usage limits and eviction policy」
- Apple：Developer News「Updates to runtime protection in macOS Sequoia」（`developer.apple.com/news/?id=saqachfa`）；支持文档 mh40616（Open Anyway）；
  Info.plist 键文档 `CFBundleShortVersionString`、`CFBundleVersion`、`LSUIElement`、`LSMinimumSystemVersion`、`NSHighResolutionCapable`
- tauri-docs `distribute/Sign/macos.mdx`（Ad-Hoc Signing）；tauri-apps/tauri#8763；djfbryant/slugtale#49；create-dmg（`FILESYSTEM="HFS+"`、`hdiutil_retry`）
- 本机 crate 源码：`gpui-kit-0.7.3/Cargo.toml`、`gpui-pre-platform-0.3.9/Cargo.toml`、`gpui-pre-apple-0.3.9/{build.rs,src/metal_renderer.rs}`、
  `gpui-pre-macos-0.3.9/src/{system_notifications.rs,platform.rs,window.rs}`、`gpui-pre-0.3.9/src/{window.rs,elements/text.rs,elements/div.rs,window/a11y.rs}`、
  `auto-launch-0.5.0/src/macos.rs`

## Caveats / 未确认

- **"整包 ad-hoc 签名后就不再提示'已损坏'"来自社区经验**（tauri#8763、slugtale#49），不是 Apple 文档的说法；没有真机验证。发布说明里两种放行方法都写上，以防万一。
- **runner 上的 CGEvent 点击、`screencapture` 能否截到窗口**：按镜像的 TCC 配置推断可行，需要首轮 CI 实测；e2e 里一定要有"截图中确实有窗口"的断言。
  SIP 和运行时修改 TCC.db 的说法来自 2024 年针对 13 / 14 的维护者回复，15 / 26 上要先跑一下 `csrutil status` 确认。
- **GPUI 能不能在 runner 的半虚拟化 GPU 上正常渲染**：#14836 说明 arm64 上的普通 Metal 工作（含 MSAA）没问题，但 GPUI 本身在 runner 上没验证过；
  Zed 自己的 mac CI 是自托管的，不能直接类推。
- `CFBundleDevelopmentRegion=zh_CN` 能不能让系统面板显示中文，待实测。
- runtime_shaders 的启动耗时没测。
- 构建耗时、缓存体积是估算（§5.2、§5.5）。
- 属于 gpui-macos-platform 调研的范围，这里只记下与打包相关的部分：
  - 开机自启在 mac 上的机制：2.5.0 用 `MacosLauncher::LaunchAgent`（`git show 749405a:pc/src-tauri/src/lib.rs` 约 214 行）；GPUI 版 `autostart.rs` 没设
    `set_use_launch_agent(true)`，auto-launch 0.5 在 mac 上默认走 AppleScript 登录项（`auto-launch-0.5.0/src/macos.rs:7-24`），两边不一致。
  - 2.5.0 的数据目录是 `dirs::data_local_dir()/mini-todo` = `~/Library/Application Support/mini-todo`（旧 `db/paths.rs`，与现在相同）；
    日志目录 2.5.0 走 tauri-plugin-log 的 `LogDir`，与现在的路径不同，但日志不影响兼容。
  - 是否加托盘或 `LSUIElement`、关掉最后一个窗口时的行为。
