# WebDAV 服务端互操作实测（2026-10-02，本地 Apache 2.4 mod_dav + nginx 1.24 dav/dav_ext）

实测方法：本机起 Apache（mod_dav_fs）与 nginx（ngx_http_dav_module + dav_ext），curl 逐项探测；
随后用真实 PC 应用（Xvfb + tauri-driver）与 cloud 二进制跑同步 e2e 复现。

## Apache mod_dav

| 行为 | 结果 |
|---|---|
| 文件写入后 1 秒内的 ETag | 弱 `W/"<size>-<mtime-usec>"`，约 1 秒后变为强 `"<同一不透明值>"` |
| `If-Match` + 强 ETag | 204（可用） |
| `If-Match` + 去掉 `W/` 的不透明值，写入后 1 秒内 | 412；约 1.1 秒后同一请求 204 |
| `If-Unmodified-Since: <文件自己的 Last-Modified>` | **对一个 2 秒前写入、此后未改的文件也返回 412**（亚秒 mtime 与秒级日期比较）→ IUS 不可用 |
| PUT 响应头 | 201/204，**不带 ETag 也不带 Last-Modified** |
| PROPFIND Depth:0 getetag | 返回强形式 |
| `If-None-Match`（弱比较） | 未变化时 304，正常 |

## nginx dav + dav_ext

| 行为 | 结果 |
|---|---|
| PUT 前置条件 | `If-Match` / `If-Unmodified-Since` 都被忽略（204） |
| PUT 响应头 | 不带校验器 |
| PROPFIND Depth:0 | 有 getlastmodified、getcontentlength，**没有 getetag** |
| HEAD / GET | 强 ETag `"<mtime-hex>-<size-hex>"` + Last-Modified |
| `If-Modified-Since` | 按整秒比较 → 与基准同一秒内的写入被报告为 304（漏检变化） |

## 由此确定的客户端规则（PC 与 cloud 一致）

1. PUT 只用 `If-Match: "<不透明值>"`（去掉 `W/`）；**永不发送 If-Unmodified-Since**；没有 ETag 时不带前置条件，
   由"写前总是 GET + 合并"兜底（nginx/Caddy 本来就忽略前置条件）。
2. 412 后等待 ≥ 1.1 秒再重试（Apache 写入后 1 秒内 ETag 为弱），重试 = 无条件 GET → 合并 → PUT，次数有上限。
3. 条件 GET 只用 `If-None-Match`；**不用 If-Modified-Since**；没有 ETag 时无条件 GET。
4. PUT 成功后的基准：PUT 响应的 ETag → HEAD → PROPFIND getetag，依次回退；Last-Modified 只记录、不用于前置条件。
5. 任何一端发现远端文档缺少自己本地有的记录或墓碑（被忽略前置条件的服务端覆盖），重新上传补齐（自愈）。

## e2e 中期验证暴露的问题（已派修复）

- Apache：另一写入方 1 秒内刚写过 → 弱 ETag → 旧逻辑改用 IUS → 连续 412 → PC 同步失败；cloud push 同样失败，
  导致云端删除的墓碑传不出去。
- nginx：基准只有 Last-Modified → 条件 GET 用 IMS → 同秒写入 304 → cloud 拉不到 PC 新建的记录、PC 拉不到远端设置变化。
- PC `webdav_force_push` 从上传文档中移除了远端独有记录但没写墓碑 → cloud 下次 push 会把它们加回来。
