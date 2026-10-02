# 端到端验证（Linux / 云端会话）

在 Linux（含 Claude Code 云端容器）里驱动**真实构建的 PC 应用**，跑通"PC ↔ WebDAV ↔ cloud ↔ Skill"
整条链路。与 `../e2e/`（Windows 窗口模式断言）互补：这里覆盖同步、数据、安全、通知、升级迁移等
跨平台行为，Win32 窗口样式仍需在 Windows 上用 `../e2e/` 验收。

```
Xvfb :99 ── tauri-driver ── WebKitWebDriver ── mini-todo（debug 构建，isolated HOME）
                                                     │  webdav_sync
              Apache mod_dav :8081 ◄──────────────────┤  （遵守条件请求）
              nginx dav      :8082 ◄──────────────────┘  （忽略条件请求）
                    ▲
              minitodo-cloud（REST）◄── minitodo.py（Skill CLI）
```

## 一次性准备（Ubuntu 24.04，root）

```bash
apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf \
    xvfb webkit2gtk-driver dbus-x11 x11-utils imagemagick \
    apache2 apache2-utils nginx libnginx-mod-http-dav-ext
pip install selenium requests
cargo install tauri-driver --locked          # 或设置 MT_E2E_TAURI_DRIVER 指向已有二进制
bash pc/scripts/e2e-linux/setup_servers.sh   # 起两个 WebDAV（e2e / e2e-pass）
```

## 构建被测程序

```bash
cd pc && npm ci && npx tauri build --debug --no-bundle   # → src-tauri/target/debug/mini-todo（内嵌前端，CSP 生效）
cd ../cloud && cargo build                              # → target/debug/minitodo-cloud
```

`L01`（旧库升级）需要一个 v28 迁移之前的旧版应用二进制，例如在 `git worktree add /tmp/old a6935e4`
里按同样方式构建。

## 运行

```bash
cd pc/scripts/e2e-linux
python3 run_e2e.py <app> <cloud> <old-app>            # 全量（约 10 分钟）
python3 run_e2e.py <app> <cloud> <old-app> S02 F03    # 只跑指定用例（前缀匹配）
python3 repro_base.py <old-app> A1 B1                 # 在旧版上复现审查发现的缺陷（预期全部 PASS = 复现成功）
```

环境变量：`MT_E2E_WORK`（日志与隔离 HOME，默认 `/tmp/mini-todo-e2e`）、`MT_E2E_TAURI_DRIVER`、
`MT_E2E_DAV_ROOT`（默认 `/srv/mtodo-e2e`）。每个用例在独立 HOME 下运行，结果汇总写入
`$MT_E2E_WORK/run/results.txt`，各进程日志在 `$MT_E2E_WORK/run/*.log`。

## 用例

| 组 | 编号 | 覆盖 |
|---|---|---|
| F 本地 | F00–F07 | 启动与 IPC、时间规范化、变更计数与墓碑、`javascript:`/`data:` 链接无效、CSP 拦截 eval 与越权读文件、图片保存防穿越、日历显示无开始时间的待办、空格格式重复提醒推进 |
| | F09 | 缩放 200%（`GDK_SCALE=2`）下应用内通知窗口位于屏幕内且不重叠 |
| S 同步（Apache / nginx 各一遍） | S01–S10 | 首次同步与无变化、AI 写入与当天本地新建合并不丢（A1）、删除双向传播不复活、不同记录并发编辑、cloud 写入校验与别名映射、图片上传/云端镜像/渲染/恶意文件名、设置 LWW 与未知顶层键保留、强制拉取/推送、PC 与 cloud 并发写入、界面同步按钮 |
| L 生命周期 | L01–L05 | 旧库升级前自动备份 + v28 迁移 + 数据完好、日志落盘、导出导入保留 id 并写墓碑、单实例、变更计数轮询刷新列表、界面新建防重复提交 |

## WebDAV 服务端差异（编写同步代码前必读）

见 `.trellis/tasks/archive/*/10-02-full-optimization/research/webdav-interop.md`：Apache 的
`If-Unmodified-Since` 对未改动文件也会 412、写入后 1 秒内 ETag 为弱；nginx 的 PROPFIND 无 getetag、
`If-Modified-Since` 按整秒比较。两端同步实现只用 `If-Match` / `If-None-Match`。

## 已知环境差异

- Linux/GTK 会给 WebView 窗口强加最小高度（应用内通知请求 320×120，实际 320×200），Windows 上按请求尺寸。
- 系统通知需要桌面通知守护进程；用例统一切到应用内通知（`set_notification_type: app`）。
- 容器默认 UTC，套件对应用与 cloud 统一设置 `TZ=Asia/Shanghai`。
