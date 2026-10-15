//! 版本检查（旧 `appStore.checkForUpdates`）：GitHub 最新 release 的 `tag_name` 比当前版本新 →
//! `AppState.has_update / latest_version / release_url`，标题栏版本号旁出现红点。失败静默（只记日志）。
//!
//! debug 构建可用 `MINI_TODO_DEBUG_LATEST_VERSION=v2.6.0` 跳过网络、直接当作最新版本（截图 / 冒烟用；release 忽略）。

use std::cmp::Ordering;
use std::time::Duration;

use gpui_kit::{App, Task};
use serde::Deserialize;

use crate::state::{APP_VERSION, AppState, GITHUB_OWNER, GITHUB_REPO};
use crate::windows::main_window::logic::compare_versions;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    html_url: Option<String>,
}

/// 检查结果：有新版本时 `Some((版本, 发布页))`
pub type UpdateInfo = Option<(String, Option<String>)>;

fn latest_release_url() -> String {
    format!("https://api.github.com/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest")
}

/// Release 页面（有检查结果时用它的 `html_url`，否则仓库的 releases 列表；旧 `getReleasesUrl`）
pub fn releases_url(cx: &App) -> String {
    AppState::global(cx)
        .read(cx)
        .release_url
        .clone()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| format!("https://github.com/{GITHUB_OWNER}/{GITHUB_REPO}/releases"))
}

fn debug_override() -> Option<String> {
    #[cfg(debug_assertions)]
    {
        std::env::var("MINI_TODO_DEBUG_LATEST_VERSION")
            .ok()
            .filter(|v| !v.trim().is_empty())
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

fn fetch_latest() -> Result<Release, String> {
    if let Some(tag) = debug_override() {
        return Ok(Release {
            tag_name: tag,
            html_url: None,
        });
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(format!("mini-todo/{APP_VERSION}"))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(latest_release_url())
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("GitHub 返回 {}", response.status()));
    }
    response.json::<Release>().map_err(|e| e.to_string())
}

/// 后台检查一次；有新版本时写进 `AppState`（返回值同时给设置页「检查更新」用）
pub fn check(cx: &mut App) -> Task<Result<UpdateInfo, String>> {
    let task = cx
        .background_executor()
        .spawn(async move { fetch_latest() });
    cx.spawn(async move |cx| {
        let release = task.await?;
        if compare_versions(&release.tag_name, APP_VERSION) != Ordering::Greater {
            return Ok(None);
        }
        let info = (release.tag_name.clone(), release.html_url.clone());
        cx.update(|cx| {
            AppState::global(cx).update(cx, |state, cx| {
                state.has_update = true;
                state.latest_version = Some(release.tag_name);
                state.release_url = release.html_url;
                cx.notify();
            })
        });
        Ok(Some(info))
    })
}

/// 启动时检查（不阻塞、失败静默）
pub fn check_for_updates(cx: &mut App) {
    let task = check(cx);
    cx.spawn(async move |_| match task.await {
        Ok(Some((version, _))) => log::info!("[update] 发现新版本 {version}"),
        Ok(None) => log::info!("[update] 已是最新版本 v{APP_VERSION}"),
        Err(e) => log::info!("[update] 检查更新失败（忽略）: {e}"),
    })
    .detach();
}
