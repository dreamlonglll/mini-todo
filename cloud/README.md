# minitodo-cloud

mini-todo 的云端 HTTP API（Rust + Axum），与 PC 端共用同一个 WebDAV
`sync-data.json.gz` 做底层数据通道。AI 客户端（Claude Code Skill / 任意带
HTTP 客户端的工具）通过本服务读写用户待办，数据最终回写 WebDAV，PC 端下次
同步拉到。

> 当前进度：**PR1 骨架（只读 + 后台拉同步）**。
> CRUD API（/todos /subtasks /images）与 dirty flag push 在 PR2 实现，
> PC 端的条件 PUT / 412 重试 / per-record merge 在 PR3 实现。

## 总览

```
┌────────┐  HTTPS+Bearer   ┌──────────────┐ pull 60s   ┌─────────┐
│   AI   │ ──────────────► │ minitodo-    │ ─────────► │ WebDAV  │
│ Skill  │ ◄────────────── │ cloud (本服) │ ◄───────── │ server  │
└────────┘    JSON / files └──────┬───────┘ push 1s    └────┬────┘
                                  │ rusqlite                │
                                  ▼                         ▼
                          /var/lib/minitodo/         /mini-todo/
                          data.db + images/          sync-data.json.gz
                                                     images/
```

* **WebDAV 是 source of truth**。云端 SQLite 是缓存，重启会重新从 WebDAV 灌满。
* **时间格式与 PC SQLite 完全一致**：`YYYY-MM-DD HH:MM:SS` 无时区后缀，按
  config.toml `timezone` 取墙钟时间。
* **per-record LWW**：远端 record.updated_at ≥ 本地 → upsert；本地比远端新 → 保留。

## 构建

```bash
cd cloud
cargo build --release
# 产物：target/release/minitodo-cloud
```

无需任何 C 工具链（rusqlite bundled、reqwest rustls）。

## 配置

复制 `config.example.toml` 为 `/etc/minitodo/config.toml`（路径自定），按注释填
WebDAV 凭据、`api_key`、`timezone` 等。

`api_key` 建议至少 32 字符随机串：

```bash
openssl rand -hex 32
```

## 部署（systemd + Caddy）

```bash
# 1. 创建 service 账户与数据目录
sudo useradd --system --no-create-home minitodo
sudo mkdir -p /opt/minitodo-cloud /var/lib/minitodo /etc/minitodo
sudo chown -R minitodo:minitodo /var/lib/minitodo

# 2. 拷贝二进制与配置
sudo cp target/release/minitodo-cloud /opt/minitodo-cloud/
sudo cp config.example.toml /etc/minitodo/config.toml
sudo $EDITOR /etc/minitodo/config.toml      # 填实际值
sudo chown root:minitodo /etc/minitodo/config.toml
sudo chmod 640 /etc/minitodo/config.toml

# 3. 安装 systemd unit
sudo cp deploy/minitodo-cloud.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now minitodo-cloud

# 4. 反向代理（任选其一）
#    Caddy：将 deploy/Caddyfile.example 中的 server block 并入 /etc/caddy/Caddyfile
#           然后 `sudo systemctl reload caddy`，自动签 Let's Encrypt 证书
#    Nginx：自行配置反代到 127.0.0.1:8787

# 5. 健康检查
curl -H "Authorization: Bearer $API_KEY" https://minitodo.example.com/health
```

输出示例：

```json
{"status":"healthy","sync":"healthy","lastPullAt":"2026-05-13 12:34:56"}
```

并附 HTTP 响应头：

```
X-Sync-Status: healthy
X-Last-Sync-At: 2026-05-13 12:34:56
```

## WebDAV server 选型

| 选项 | 说明 |
|------|------|
| **Caddy + caddy-webdav 插件** | 单二进制，与反代共用 TLS；适合自托管个人 |
| **nginx-dav-ext-module** | 通过 nginx `dav_methods PUT DELETE MKCOL COPY MOVE` 开放；需要重新编译 nginx 或用 openresty |
| **NextCloud** | 重量级，但自带 Web UI；本服务通过 `/remote.php/dav/files/USER` 接入 |
| **坚果云** | 国内体验稳定；WebDAV 限速明显，仅适合个人 |

最简单的自托管：

```caddy
:8443 {
    tls /etc/ssl/cert.pem /etc/ssl/key.pem
    basicauth /mini-todo/* {
        webdavuser <bcrypt-hash>
    }
    webdav /mini-todo/* {
        root /srv/webdav
    }
}
```

PC 端与 cloud 同时填写 `https://yourhost:8443`、`webdavuser`、原始密码。

## 配置字段速查

| 字段 | 必填 | 默认 | 说明 |
|---|---|---|---|
| `webdav_url` | ✓ | — | WebDAV 服务器根 URL，不带尾部 `/mini-todo` |
| `webdav_username` | ✓ | — | WebDAV 账号 |
| `webdav_password` | ✓ | — | WebDAV 密码 |
| `api_key` | ✓ | — | Bearer Token；≥ 16 字符 |
| `bind` | × | `127.0.0.1:8787` | HTTP 监听地址 |
| `timezone` | × | `Asia/Shanghai` | IANA 时区，**必须与 PC 端一致** |
| `pull_interval` | × | `60` | Pull worker 间隔（秒） |
| `data_dir` | × | `/var/lib/minitodo` | SQLite 与 meta 数据目录 |
| `images_dir` | × | `/var/lib/minitodo/images` | 镜像图片目录 |

缺任意必填字段 → 进程启动直接退出并打印清晰错误。

## 当前能力（PR1）

- [x] `GET /health` 返回 `{status, sync, lastPullAt}` 与 `X-Sync-Status` header
- [x] Bearer token 鉴权（错/缺 → 401）
- [x] 启动同步拉一次 WebDAV `sync-data.json.gz`，per-record LWW merge 进 SQLite
- [x] 60s 后台 pull 轮询（自动用 `If-None-Match` 跳过 304）
- [x] 启动时一次性图片镜像（缺什么下什么）

## 还没有（留给后续 PR）

- [ ] `/todos` `/subtasks` `/images` CRUD（PR2）
- [ ] dirty flag + 1s 后台 push 回写 WebDAV（PR2）
- [ ] PC 端条件 PUT + 412 重试 + per-record merge（PR3）
- [ ] Claude Code Skill（`cloud/skill/minitodo/`）（PR2）

## 开发

```bash
cd cloud
# 单元测试
cargo test
# Lint
cargo clippy --all-targets
# Format
cargo fmt --check
```

本子项目独立于 `pc/`，不在同一 Cargo workspace 中，互不影响。
