//! 法定节假日 / 调休数据（原 `commands/holiday.rs`，日历视图用）。
//!
//! 策略：先联网取 NateScarlet/holiday-cn 的 `{year}.json`（超时 10s、连接 5s），成功则写本地缓存
//! `<数据目录>/holidays/{year}.json`；联网失败时回退缓存，缓存也没有才报错。
//!
//! 旧版用 async reqwest（依赖 tokio）；这里改为 `reqwest::blocking`，阻塞调用，放后台线程执行
//! （不能在 tokio runtime 上下文里调用）。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 节假日信息
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HolidayInfo {
    /// `YYYY-MM-DD`
    pub date: String,
    pub name: String,
    /// true = 放假，false = 调休上班
    pub is_off_day: bool,
}

/// NateScarlet/holiday-cn 数据格式
/// 数据源: https://raw.githubusercontent.com/NateScarlet/holiday-cn/master/{year}.json
#[derive(Debug, Deserialize)]
struct HolidayCnResponse {
    days: Vec<HolidayCnDay>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HolidayCnDay {
    name: String,
    date: String,
    is_off_day: bool,
}

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

fn source_url(year: i32) -> String {
    format!(
        "https://raw.githubusercontent.com/NateScarlet/holiday-cn/master/{}.json",
        year
    )
}

/// 节假日缓存目录
fn cache_dir() -> PathBuf {
    crate::db::paths::app_data_dir().join("holidays")
}

fn cache_path(dir: &Path, year: i32) -> PathBuf {
    dir.join(format!("{}.json", year))
}

/// 从本地缓存读取节假日数据
fn read_cache(dir: &Path, year: i32) -> Option<Vec<HolidayInfo>> {
    let data = std::fs::read_to_string(cache_path(dir, year)).ok()?;
    serde_json::from_str(&data).ok()
}

/// 将节假日数据写入本地缓存（失败只是少了离线兜底，不报错）
fn write_cache(dir: &Path, year: i32, holidays: &[HolidayInfo]) {
    let _ = std::fs::create_dir_all(dir);
    if let Ok(data) = serde_json::to_string(holidays) {
        let _ = std::fs::write(cache_path(dir, year), data);
    }
}

/// 从网络获取节假日数据（带超时）
fn fetch_from_network(url: &str) -> Result<Vec<HolidayInfo>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("Failed to fetch holidays: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("API returned status: {}", response.status()));
    }

    let api_response: HolidayCnResponse = response
        .json()
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    Ok(api_response
        .days
        .into_iter()
        .map(|day| HolidayInfo {
            date: day.date,
            name: day.name,
            is_off_day: day.is_off_day,
        })
        .collect())
}

/// 指定年份的节假日（`fetch_holidays`）：网络优先，成功后更新缓存；失败回退本地缓存。阻塞。
pub fn fetch(year: i32) -> Result<Vec<HolidayInfo>, String> {
    fetch_with(year, &source_url(year), &cache_dir())
}

fn fetch_with(year: i32, url: &str, cache: &Path) -> Result<Vec<HolidayInfo>, String> {
    match fetch_from_network(url) {
        Ok(holidays) => {
            write_cache(cache, year, &holidays);
            Ok(holidays)
        }
        Err(network_err) => {
            log::warn!("Network fetch failed for year {}: {}", year, network_err);
            if let Some(cached) = read_cache(cache, year) {
                log::info!("Using cached holiday data for year {}", year);
                Ok(cached)
            } else {
                Err(format!(
                    "Failed to fetch holidays and no local cache: {}",
                    network_err
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "mini-todo-holiday-test-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 起一个只回应一次的本地 HTTP 服务，返回 URL
    fn serve_once(status: u16, body: &'static str) -> String {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}/2026.json", server.server_addr());
        std::thread::spawn(move || {
            if let Ok(request) = server.recv() {
                let _ = request
                    .respond(tiny_http::Response::from_string(body).with_status_code(status));
            }
        });
        url
    }

    const SAMPLE: &str = r#"{
        "$schema": "https://raw.githubusercontent.com/NateScarlet/holiday-cn/master/schema.json",
        "$id": "https://raw.githubusercontent.com/NateScarlet/holiday-cn/master/2026.json",
        "year": 2026,
        "papers": ["https://www.gov.cn/x"],
        "days": [
            {"name": "元旦", "date": "2026-01-01", "isOffDay": true},
            {"name": "春节", "date": "2026-02-14", "isOffDay": false}
        ]
    }"#;

    #[test]
    fn network_result_is_returned_and_cached_then_used_offline() {
        let cache = temp_dir("cache");
        let got = fetch_with(2026, &serve_once(200, SAMPLE), &cache).unwrap();
        assert_eq!(
            got,
            vec![
                HolidayInfo {
                    date: "2026-01-01".to_string(),
                    name: "元旦".to_string(),
                    is_off_day: true
                },
                HolidayInfo {
                    date: "2026-02-14".to_string(),
                    name: "春节".to_string(),
                    is_off_day: false
                },
            ]
        );
        // 缓存格式与旧版一致（camelCase 数组）
        let cached = std::fs::read_to_string(cache.join("2026.json")).unwrap();
        assert!(cached.contains("\"isOffDay\":true"), "{cached}");

        // 网络失败（服务端 500）：回退缓存
        let offline = fetch_with(2026, &serve_once(500, "oops"), &cache).unwrap();
        assert_eq!(offline, got);
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn failure_without_cache_is_an_error() {
        let cache = temp_dir("empty");
        let err = fetch_with(2031, &serve_once(404, "nope"), &cache).unwrap_err();
        assert!(err.contains("no local cache"), "{err}");
        let err = fetch_with(2031, &serve_once(200, "not json"), &cache).unwrap_err();
        assert!(err.contains("parse"), "{err}");
        let _ = std::fs::remove_dir_all(&cache);
    }
}
