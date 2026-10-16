# 本机验证结果（2026-10-16，Win11 26H2 build 26300，双屏 144 / 120 DPI）

debug 构建 `pc-gpui/target/debug/mini-todo.exe`（含本任务修复），隔离数据目录，与安装版 3.0.0-pre（未含修复）并存。

## 实机核对（PrintWindow `PW_RENDERFULLCONTENT` + `WM_NCACTIVATE(TRUE)` 模拟激活）

- 收纳箱、深色半透明主窗口：样式无 `WS_SYSMENU`，`DWMWA_CAPTION_BUTTON_BOUNDS` 宽度 0；右上角无 ×、激活态无红底
- 客户区几何与修复前一致：主窗口 570×900 @ (1004,632)，收纳箱 552×504 @ (3252,36)

## Windows e2e：`--only W,S,N,F --skip F03,F10`（F03 / F10 需 en-US 键盘布局，与本改动无关）

| 结果 | 说明 |
|---|---|
| W02–W12、S01–S08、N01、N02、F01、F02、F06、F07 PASS | 含新断言：所有窗口无 `WS_SYSMENU`、按钮区宽度 0；S01 真实双击最大化 / 还原；**S08 真实单击任务栏按钮最小化 / 还原通过** |
| W01 FAIL | 照片壁纸下取色判据无效（已知环境问题，同 10-16 发版前那次） |
| F04 / F08 / F09 / F11 / F05 FAIL | **环境冲突**：测试收纳箱建在安装版收纳箱的位置，拖放落进了安装版的收纳箱（安装版日志 7 条"移入"，测试文件进了真实的 `%USERPROFILE%\MiniTodo收纳箱\收纳箱\`，已移到会话临时目录，收纳文件夹恢复为空）；F05 的 Win+D 命中了安装版收纳箱 |

重跑 F 组（临时 `ShowWindow(SW_HIDE)` 安装版收纳箱，`--skip F03,F10,F06`，跑完恢复显示）：F01 / F02 / F04 / F05 / F07 / F08 / F09 / F11 **全部 PASS**。

## 补测（脚本，修复后状态）

| 项 | 结果 |
|---|---|
| Win+↑ 最大化 / Win+↓ 还原 / 再 Win+↓ 最小化 | PASS |
| 拖标题栏到屏幕顶边贴靠最大化 | PASS |
| 任务栏右键「关闭窗口」 | SKIP（UIA 找不到跳出菜单项），待用户手动确认 |
| Alt+F4（`WM_SYSKEYDOWN` 模拟） | 仍关闭窗口（实现前实验） |

## 门槛

- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`（150 + 294）全绿
- `python -m py_compile winlib.py run_e2e.py`、`taskbar-button.ps1` 解析通过

## 已记入 spec 的教训

- 本机跑 F 组时安装版收纳箱会与测试收纳箱重叠 → `guides/desktop-window-e2e-guide.md`「开发版与安装版并存」第 7 条、e2e README
