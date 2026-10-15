//! 「关于 → 检查更新」用的版本检查（旧 `appStore.checkForUpdates` / `compareVersions` 的移植）：
//! `GET https://api.github.com/repos/{owner}/{repo}/releases/latest`，`tag_name` 比当前版本新则
//! `AppState.has_update = true`、记下 `latest_version` / `release_url`。网络错误 / 非 2xx / 解析失败只记日志、
//! 不改状态（与旧版相同）。
//!
//! 合并说明：W1 在 `services::update_check` 里实现了同一件事（启动检查 + 标题栏红点）。合并后可把 `about.rs` 改成
//! 调用 `services::update_check::check(cx)`（`Ok(Some(_))` = 有新版本）、发布页用它的 `releases_url(cx)`，并删除本文件。

use gpui_kit::{App, Task};

use crate::state::{APP_VERSION, AppState, GITHUB_OWNER, GITHUB_REPO};

/// 旧 `compareVersions`：去掉前缀 `v`，按 `.` 分段数值比较（非数字段与缺段按 0）
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let (x, y) = (
            pa.get(i).copied().unwrap_or(0),
            pb.get(i).copied().unwrap_or(0),
        );
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// GitHub 最新发布（tag、页面地址）；失败返回 `Err`（只记日志用）
fn fetch_latest_release() -> Result<(String, String), String> {
    let url = format!("https://api.github.com/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest");
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(5))
        .user_agent(format!("mini-todo/{APP_VERSION}"))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("GitHub 返回 {}", response.status()));
    }
    let value: serde_json::Value = response.json().map_err(|e| e.to_string())?;
    let tag = value
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or("缺少 tag_name")?
        .to_string();
    let html_url = value
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Ok((tag, html_url))
}

/// 检查更新并写回 `AppState`；任务结果为检查后的 `has_update`
pub fn check_for_updates(cx: &mut App) -> Task<bool> {
    let fetch = cx
        .background_executor()
        .spawn(async { fetch_latest_release() });
    let state = AppState::global(cx);
    cx.spawn(async move |cx| {
        let result = fetch.await;
        state.update(cx, |state, cx| {
            match result {
                Ok((tag, url)) => {
                    if compare_versions(&tag, APP_VERSION) == std::cmp::Ordering::Greater {
                        state.has_update = true;
                        state.latest_version = Some(tag);
                        state.release_url = (!url.is_empty()).then_some(url);
                        cx.notify();
                    }
                }
                Err(e) => log::info!("[update] 检查更新失败（忽略）: {e}"),
            }
            state.has_update
        })
    })
}

/// 发布页地址（缺省为仓库的 releases 页面）
pub fn releases_url(state: &AppState) -> String {
    state
        .release_url
        .clone()
        .unwrap_or_else(|| format!("https://github.com/{GITHUB_OWNER}/{GITHUB_REPO}/releases"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn compares_like_the_frontend() {
        assert_eq!(compare_versions("v2.6.0", "2.5.0"), Ordering::Greater);
        assert_eq!(compare_versions("v2.5.0", "2.5.0"), Ordering::Equal);
        assert_eq!(compare_versions("2.5", "2.5.0"), Ordering::Equal);
        assert_eq!(compare_versions("v2.4.9", "2.5.0"), Ordering::Less);
        assert_eq!(compare_versions("v2.10.0", "2.9.9"), Ordering::Greater);
        // 非数字段按 0（JS `Number('1-beta') || 0`）
        assert_eq!(compare_versions("v2.5.1-beta", "2.5.0"), Ordering::Equal);
    }
}
