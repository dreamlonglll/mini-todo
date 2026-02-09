use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 节假日信息
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HolidayInfo {
    pub date: String,
    pub name: String,
    pub is_off_day: bool,
}

/// NateScarlet/holiday-cn 数据格式
/// 数据源: https://raw.githubusercontent.com/NateScarlet/holiday-cn/master/{year}.json
#[derive(Debug, Deserialize)]
struct HolidayCnResponse {
    #[serde(rename = "$schema")]
    #[allow(dead_code)]
    schema: Option<String>,
    #[serde(rename = "$id")]
    #[allow(dead_code)]
    id: Option<String>,
    #[allow(dead_code)]
    year: i32,
    #[allow(dead_code)]
    papers: Option<Vec<serde_json::Value>>,
    days: Vec<HolidayCnDay>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HolidayCnDay {
    name: String,
    date: String,
    is_off_day: bool,
}

/// 获取节假日缓存目录
fn get_cache_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mini-todo")
        .join("holidays")
}

/// 获取指定年份的缓存文件路径
fn get_cache_path(year: i32) -> PathBuf {
    get_cache_dir().join(format!("{}.json", year))
}

/// 从本地缓存读取节假日数据
fn read_cache(year: i32) -> Option<Vec<HolidayInfo>> {
    let path = get_cache_path(year);
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

/// 将节假日数据写入本地缓存
fn write_cache(year: i32, holidays: &[HolidayInfo]) {
    let cache_dir = get_cache_dir();
    let _ = std::fs::create_dir_all(&cache_dir);
    if let Ok(data) = serde_json::to_string(holidays) {
        let _ = std::fs::write(get_cache_path(year), data);
    }
}

/// 从网络获取节假日数据（带超时）
async fn fetch_from_network(year: i32) -> Result<Vec<HolidayInfo>, String> {
    let url = format!(
        "https://raw.githubusercontent.com/NateScarlet/holiday-cn/master/{}.json",
        year
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch holidays: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("API returned status: {}", response.status()));
    }

    let api_response: HolidayCnResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let holidays: Vec<HolidayInfo> = api_response
        .days
        .into_iter()
        .map(|day| HolidayInfo {
            date: day.date,
            name: day.name,
            is_off_day: day.is_off_day,
        })
        .collect();

    Ok(holidays)
}

/// 获取指定年份的节假日数据
/// 策略：优先从网络获取（带超时），失败时回退到本地缓存
/// 网络获取成功后自动更新本地缓存
#[tauri::command]
pub async fn fetch_holidays(year: i32) -> Result<Vec<HolidayInfo>, String> {
    // 尝试从网络获取
    match fetch_from_network(year).await {
        Ok(holidays) => {
            // 网络获取成功，更新本地缓存
            write_cache(year, &holidays);
            Ok(holidays)
        }
        Err(network_err) => {
            // 网络获取失败，尝试从本地缓存读取
            eprintln!("Network fetch failed for year {}: {}", year, network_err);
            if let Some(cached) = read_cache(year) {
                eprintln!("Using cached holiday data for year {}", year);
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
