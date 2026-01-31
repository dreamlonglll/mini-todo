use serde::{Deserialize, Serialize};

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

/// 获取指定年份的节假日数据
/// 使用 NateScarlet/holiday-cn 开源数据，包含休息日和调休上班日
#[tauri::command]
pub async fn fetch_holidays(year: i32) -> Result<Vec<HolidayInfo>, String> {
    let url = format!(
        "https://raw.githubusercontent.com/NateScarlet/holiday-cn/master/{}.json",
        year
    );
    
    let response = reqwest::get(&url)
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
        .map(|day| {
            HolidayInfo {
                date: day.date,
                name: day.name,
                is_off_day: day.is_off_day,
            }
        })
        .collect();
    
    Ok(holidays)
}
