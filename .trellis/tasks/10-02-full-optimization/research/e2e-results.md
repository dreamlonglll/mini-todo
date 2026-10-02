# 端到端验证结果（2026-10-02，本环境 Linux / Xvfb）

套件：`pc/scripts/e2e-linux/`（真实应用 debug 构建 + tauri-driver + Apache mod_dav / nginx dav + minitodo-cloud + Skill）。
时区 `Asia/Shanghai`，GDK_SCALE=2 用于 F09 / C2。

## 修复前后对照（审查发现的关键缺陷）

旧版二进制（v28 迁移之前的构建）上用 `repro_base.py` 复现，新版在 `run_e2e.py` 对应用例中验证：

| 缺陷 | 旧版（repro_base.py，PASS = 缺陷复现） | 新版（run_e2e.py） |
|---|---|---|
| A1 当天本地新建的待办被自动同步删除 | 复现：同步后本地只剩 `A-synced`、`Y-from-cloud`，新建的 X 被删 | S02[apache/nginx] PASS：合并后三条都在 |
| B1 只读详情里 `javascript:` 链接可执行 | 复现：点击后 `document.title == 'PWNED'` | F03 PASS：javascript: / data: / vbscript: 三种链接点击后标题不变、无导航 |
| A6 空格格式时间的重复提醒永不推进（每分钟重复触发） | 复现：调度后 notify_at 不变 | F07 PASS：`00:59:00` → 次日 `00:59:00` |
| E1 日历不显示只有截止时间的待办 | 复现：日历条为空 | F06 PASS：`cal-no-start` 显示 |
| B2 `save_subtask_image` 路径穿越 | 复现：`../escaped.png` 写到 images 目录之外 | F05 PASS：文件名由后端生成，穿越与旧 JSON 形式被拒 |
| C2 缩放 200% 下应用内通知出屏 | 复现：窗口在 1600×1000 屏幕的 +2520+1720 | F09 PASS：3 个通知全部在屏内、叠放 / 换列不重叠 |

旧版复现结果：6/6 复现成功。

## 新版全量结果（HEAD b73b6ea）

```
RESULT  TEST                                                                DETAIL
PASS    F00 smoke: app boots under Xvfb, IPC works                          0.0s ok
PASS    F01 time inputs (T / no-seconds / date-only) stored canonical       0.0s stored 2026-10-07 10:00:00; garbage rejected
PASS    F02 change counter + tombstones on delete (todo & subtasks)         0.0s seq 3->5->7, tombstones=2
PASS    F03 javascript:/data:/vbscript: links are inert in read-only detail  6.8s 3 malicious links clicked, title='Mini Todo', no navigation
PASS    F04 CSP blocks eval; asset protocol cannot read files outside images dir  0.0s {'eval': 'blocked', 'etc': 'blocked:TypeError: Load failed', 'fn': 'blocked'}
PASS    F05 raw image save: generated safe names, traversal rejected        0.0s saved 1790960470102_2xragl.png; bad exts and legacy JSON form rejected
PASS    F06 calendar shows todo with end date but no start date             3.1s bars=['xss-probe', 'cal-no-start', 'time-norm', '']
PASS    F07 repeat reminder with space-format time advances after firing    48.1s 2026-10-03 00:59:00 -> 2026-10-04 00:59:00
PASS    F09 app notifications on-screen & stacked at scale factor 2         58.2s 3 windows: [(260, 560, 640, 400), (920, 140, 640, 400), (920, 560, 640, 400)]
PASS    S01[apache] first sync uploads, second is no-op; password never returned  0.3s pushed then no_changes
PASS    S02[apache] AI write + same-day local create merge without loss (A1)  2.6s report=merged; Y color=#EF4444 end=2026-10-06 23:59:00
PASS    S03[apache] deletions propagate both ways via tombstones            2.7s both directions propagate, no resurrection after extra rounds
PASS    S04[apache] concurrent edits on different records merge             3.6s edits on different records both survive
PASS    S05[apache] cloud write validation keeps PC model intact            1.3s aliases mapped, bad input rejected (400), PC applies all records
PASS    S06[apache] images: upload, cloud mirror, render, hostile names ignored  6.7s uploaded+mirrored; rendered via asset://localhost/%2F<work>%2F…
PASS    S07[apache] settings LWW + unknown top-level keys preserved         1.4s newer remote settings applied (geometry excluded); unknown keys preserved by PC and cloud
PASS    S08[apache] force pull / force push semantics                       1.3s force_pull=pulled; force_push tombstoned remote-only record
PASS    S09[apache] concurrent PC + cloud writers lose nothing              15.6s 10/10 concurrent creates survived; transient errors=0
PASS    S10[apache] UI sync button shows a result toast                     3.3s toast: 已是最新，没有需要同步的更改
PASS    S11[apache] back-to-back edits get strictly newer versions on both sides  5.1s PC 2026-10-03 01:03:44 -> 2026-10-03 01:03:45; cloud 2026-10-03 01:03:46 -> 2026-10-03 01:03:47; both sides converge on v2
PASS    S01[nginx] first sync uploads, second is no-op; password never returned  0.3s pushed then no_changes
PASS    S02[nginx] AI write + same-day local create merge without loss (A1)  0.3s report=merged; Y color=#EF4444 end=2026-10-06 23:59:00
PASS    S03[nginx] deletions propagate both ways via tombstones             0.4s both directions propagate, no resurrection after extra rounds
PASS    S04[nginx] concurrent edits on different records merge              1.4s edits on different records both survive
PASS    S05[nginx] cloud write validation keeps PC model intact             0.1s aliases mapped, bad input rejected (400), PC applies all records
PASS    S06[nginx] images: upload, cloud mirror, render, hostile names ignored  5.5s uploaded+mirrored; rendered via asset://localhost/%2F<work>%2F…
PASS    S07[nginx] settings LWW + unknown top-level keys preserved          0.3s newer remote settings applied (geometry excluded); unknown keys preserved by PC and cloud
PASS    S08[nginx] force pull / force push semantics                        0.2s force_pull=pulled; force_push tombstoned remote-only record
PASS    S09[nginx] concurrent PC + cloud writers lose nothing               4.5s 10/10 concurrent creates survived; transient errors=0
PASS    S10[nginx] UI sync button shows a result toast                      3.3s toast: 已是最新，没有需要同步的更改
PASS    S11[nginx] back-to-back edits get strictly newer versions on both sides  0.8s PC 2026-10-03 01:04:08 -> 2026-10-03 01:04:09; cloud 2026-10-03 01:04:10 -> 2026-10-03 01:04:11; both sides converge on v2
PASS    L01 upgrade from an old DB: backup + v28 migration + data intact    0.1s backup data-v27-20261003-010413-184.db; legacy data intact and normalized
PASS    L01b tauri-plugin-log records the migration backup in the app log dir  0.0s .local/share/com.tauri-app.mini-todo/logs/mini-todo.log: [2026-10-03][01:04:13][INFO][mini_todo_lib::db::connection] [db] 数据库将从 v27 升级到 v28，升级前备份：/
PASS    L02 export → import keeps ids, tombstones removed records, auto-backup  0.0s ids preserved (1), tombstone for removed, backups=2
PASS    L03 single instance: second launch exits, first keeps running       0.1s second instance exited rc=0; first instance alive
PASS    L04 main list picks up backend changes via change-seq polling       5.1s list 1 -> 2 without manual refresh
PASS    L05 editor create via UI is re-entrancy safe                        4.8s triple-click create -> exactly 1 todo, shown in main list

37/37 passed
```

## 过程中由 e2e 发现并已修复的问题

- Apache：`If-Unmodified-Since` 对未改动文件也 412、写入后 1 秒内 ETag 为弱 → 两端只用 `If-Match`（去 `W/`），412 等 1.1s 重试
- nginx：PROPFIND 无 getetag、`If-Modified-Since` 按整秒比较 → 不用 IMS；基准取 PUT → HEAD → PROPFIND；cloud 每 10 轮全量拉取
- `webdav_force_push` 移除远端独有记录时未写墓碑 → cloud 下次推送会加回来（已改为写墓碑）
- 同一秒内两次编辑（S05[nginx] 首次暴露）：秒级 LWW + 平局保留本地导致第二版丢失 → 两端 `updated_at = max(now, 旧值 + 1s)`（S11 覆盖）
- Linux/GTK 强加通知窗口最小高度导致出屏（F09）→ 按实际尺寸重新定位
- nginx 秒级 ETag 下同秒等长改写被 304 掩盖 → 手动同步与每第 10 轮无条件 GET（Rust mock 测试覆盖）
