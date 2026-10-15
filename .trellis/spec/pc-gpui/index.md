# pc-gpui（PC 端）开发规范

> 适用于 `pc-gpui/`：`crates/core`（package `mini-todo-core`，业务核心，不依赖 GPUI / Tauri / tokio）+
> `crates/app`（package `mini-todo`，GPUI 界面、窗口与平台集成，二进制 `mini-todo`）。
> 建立于任务 `10-15-gpui-rewrite`（2026-10-15）：用 Rust + GPUI（`gpui-kit =0.7.3` = gpui-pre 0.3.9 + gpui-component 0.7.3）
> 重写旧 Tauri 2 + Vue 3 + WebView2 版，元素 / 样式 / 功能与旧版一致。2026-10-16（任务 `10-16-gpui-release-pipeline`）起旧版
> （`pc/`）已删除，`pc-gpui/` 是唯一的 PC 实现（目录名暂未改），3.0 起只发布它。
>
> 模块地图与接口表以 crate README 为准，本目录只写"不遵守就出错"的约定、数据兼容规则、验证门槛与发布流水线：
> `pc-gpui/crates/app/README.md`、`pc-gpui/crates/core/README.md`、`pc-gpui/scripts/README.md`。
> 设计依据：`.trellis/tasks/archive/2026-10/10-15-gpui-rewrite/{prd.md,info.md}`；GPUI API 速查 `research/gpui-api.md`；
> UI 规格（颜色 / 尺寸 / 文案的唯一来源）`research/ui-main-window.md`、`ui-editor-windows.md`、`ui-settings-fence.md`
> （同一归档目录下）。

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [GPUI Conventions](./gpui-conventions.md) | app crate 的可执行约定：后台执行与 `WeakEntity`、App 借用期间禁止同步 Win32 调用（`platform` 延后）、拖动窗口、透明窗口 Root 背景、z 序靠绘制顺序、弹层画在窗口内、`img(PathBuf)`、字体与 theme token 唯一来源、`ui/` 控件复用、跨窗口状态、debug 开关只在 debug 构建生效、`gpui-kit` 精确钉版本与升级步骤 | Active |
| [Data Compat](./data-compat.md) | 数据契约与跨版本兼容：同一份数据的读写方（core / cloud / 已安装的旧版本）、迁移 / settings 键 / 记录字段 / 同步规则 / 导出格式改动必须一起改的地方与自查命令（取代已失效的 `core-parity.md` 双改规则） | Active |
| [Verification](./verification.md) | 单测、Windows 交叉 clippy、`regress.py` 视觉回归、Linux / Windows e2e 的入口，以及"改什么必须跑什么" | Active |
| [Release](./release.md) | 发布流水线与 Windows 安装器契约：版本号唯一来源、tag 校验、预发布、`release.yml` / `cache-warm.yml`、NSIS 安装身份沿用旧 Tauri 版、安装目录即数据目录 | Active |

---

## 现有规范对 pc-gpui 的适用范围

| 规范 | 对 pc-gpui 的适用部分 |
|---|---|
| [backend/sync-protocol.md](../backend/sync-protocol.md) | **全部**。实现在 `crates/core/src/sync/` + `db/sync_store.rs` + `db/time.rs` + `services/webdav.rs`（另一端是 cloud） |
| [backend/window-modes.md](../backend/window-modes.md) | **全部**：主窗口三种模式。实现在 `crates/app/src/services/window_mode.rs`、`platform/embed.rs`、`platform/subclass.rs`，纯逻辑在 `crates/core/src/window_logic.rs` |
| [backend/desktop-fences.md](../backend/desktop-fences.md) | **全部**：桌面收纳箱。文件层与几何在 `crates/core/src/fence/`、`services/fence_fs/`；窗口在 `crates/app/src/services/fences.rs`、`windows/fence/` |
| [guides/desktop-window-e2e-guide.md](../guides/desktop-window-e2e-guide.md) | "开发版与安装版并存"与"输入安全"规则；Windows 验证用 `pc-gpui/scripts/e2e-windows/` |

---

## Pre-Development Checklist

改 `crates/core`：

- [ ] 读 [Data Compat](./data-compat.md)：数据库迁移、settings 键、记录字段、同步 / 合并规则、导入导出格式要同时考虑 cloud 与已安装的旧版本
- [ ] 遵守 [backend/index.md](../backend/index.md) 的 checklist（`db::settings_kv`、`SYNCED_SETTING_KEYS`、`CLAUDE.md`「维护检查清单」、`log::*`）
- [ ] core 不引入 GPUI / Tauri / tokio；函数保持阻塞、错误保持面向用户的中文 `String`；`reqwest::blocking` 不能在 tokio runtime 里调用
- [ ] 进程级静态量（同步互斥、`FenceActivity`、通知槽位……）留在 core，窗口状态（固定 / 嵌入标志、Progman owner、贴边状态实例）
      留在 app（`crates/core/README.md`「线程与调用约束」）

改 `crates/app`：

- [ ] 读 [GPUI Conventions](./gpui-conventions.md)，尤其 §2.1（后台执行）、§2.2（App 借用与 Win32）、§2.5（z 序与弹层）、§2.7（theme token）
- [ ] 改主窗口模式 / 收纳箱窗口：先读 `backend/window-modes.md` / `backend/desktop-fences.md` 的契约
- [ ] 颜色 / 尺寸 / 文案照抄 UI 规格（`research/ui-*.md`），不要凭感觉"近似"
- [ ] 新窗口种类 / 新控件 / 新颜色 / 新平台能力 / 新依赖 / 新 debug 开关：按 `crates/app/README.md` §2.1「维护规则」登记
- [ ] 会移动用户文件的收纳箱改动：只在隔离环境（`MINI_TODO_DATA_DIR` + `MINI_TODO_FENCES_ROOT`）验证

改版本号 / 发布流程 / 安装器：

- [ ] 读 [Release](./release.md)

升级 `gpui-kit` / gpui-pre：

- [ ] 按 [gpui-conventions.md §3](./gpui-conventions.md) 的升级步骤逐项核对，跑 [verification.md](./verification.md) 的全部门槛

---

## Quality Check

在 `pc-gpui/` 下（详见 [Verification](./verification.md)）：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
# 改了 cfg(windows) 代码（platform/、托盘、单实例、开机自启、Toast、core 的 DPAPI / fence_fs / 字体枚举）时，在 Linux 上开发必跑
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
# 改了可见界面：相关场景截图并与改动前对照（--before），逐张看过
scripts/regress.py --only 'main-*' --before <改动前的输出> --out /tmp/regress
# 改了跨窗口流程：端到端冒烟
scripts/regress.py --smoke --no-build
```

- 改 core 的同步 / 导入导出 / 迁移：再跑 `pc-gpui/scripts/e2e-linux/run_e2e.py`（S 组 Apache + nginx），并按
  [Data Compat](./data-compat.md) 确认 cloud 侧也改了、`cloud/` 下 `cargo test --locked` 也绿。
- 改 Windows 窗口行为（窗口模式、嵌入桌面、透明、通知定位、收纳箱、托盘、单实例、开机自启）：跑
  `.github/workflows/pc-gpui-e2e-windows.yml`（或本机 `pc-gpui/scripts/e2e-windows/run_e2e.py`），交付说明里列出需要用户实机验收的项。
