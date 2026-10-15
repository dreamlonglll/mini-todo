# 本机 Windows e2e 结果（2026-10-16，发布 v3.0.0-pre 前）

- 环境：用户的 Windows 11 开发机，debug 构建 `pc-gpui/target/debug/mini-todo.exe`（3.0.0-pre），与正在运行的安装版 2.5.0 并存（隔离数据目录）
- 命令：`run_e2e.py --only W,S,N,I,D,F --skip I06`（I06 开机自启由用户自测；不加 `--allow-real-profile`，P 组跳过；未跑 `set-display.ps1`）

## 结论：无产品缺陷

| 轮次 | 结果 | 说明 |
|---|---|---|
| 首轮（中文输入法"中"模式） | PASS 35 / FAIL 12 / SKIP 1 | 12 个 FAIL 全部是环境问题 |
| 重跑 I / D / F（键盘布局切 en-US，`kbd=0x04090409`） | 只剩 I05 FAIL | 输入法类全部通过 |

- 输入法类（I01–I04、D01–D04、F03、F10）：中文输入法篡改模拟键入（`E2E created todo` → `eEcreatedtodo`、数字被吞），并连锁导致后续用例失败；切 en-US 后全部通过
- W01、I05：取色判据拿"桌面众数色"对照，照片壁纸下无效；截图人工核对：主窗口四角与深色半透明主窗口都清楚透出壁纸
- 次要观察（不阻碍发布）：关闭子窗口时日志有 `window not found` / `无效的窗口句柄` 的 ERROR 噪声（GPUI 对已关闭窗口 update_window）；W05 示例数据图片在隔离目录不存在的 asset 加载错误

## 发现并已修的测试脚本问题

- I 组收尾"见 `HKCU Run\mini-todo` 就删"，`--skip I06` 也会删掉安装版的自启项 → 改为记下原值、收尾恢复原样（提交 8117b1f）；本次被删的值已按备份恢复
- 新增 `--skip`；README 写明键盘布局要切 en-US、照片壁纸下 W01 / I05 无效

## 发版前检查（trellis-check）

- pc-gpui fmt / clippy（-D warnings）/ test 通过（app 150、core 294）；修了一个 Windows 上确定性失败的测试
  `reminder::tests::next_minute_wait_aligns_to_the_unix_minute`（SystemTime 精度 100ns，提交 b0f26d6）
- cloud fmt / clippy / test 通过（264 + 1 ignored）
- NSIS 本地编译通过（UTF-8 BOM、/V4 无警告）；actionlint 全部 workflow 通过

## 发布结果

- main（5e09540）：PC (GPUI) CI 18m44s、Cache warm 9m43s、Cloud CI 全绿
- tag `v3.0.0-pre` → Release run 38779291797 全绿（create-release / plan / Windows / cloud ×2 / publish），只投递了一份
- release：pre-release、已转正；`/releases/latest` 仍是 v2.5.0（2.5.0 用户不会被提示升级）
- 资产 7 个：`mini-todo_3.0.0-pre_x64-setup.exe`（8.8 MB）、`mini-todo_3.0.0-pre_x64-portable.zip`（13.2 MB，内含单个 mini-todo.exe）、
  cloud aarch64 / x86_64 的 tar.gz + .sha256、`SHA256SUMS.txt`；下载后 `sha256sum -c` 通过，exe ProductVersion = 3.0.0-pre
- 中文 release notes 已改写；安装包未在本机实跑（会卸载用户的正式版），升级路径待用户实机验收
