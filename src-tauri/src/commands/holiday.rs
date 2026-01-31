use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 节假日信息
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HolidayInfo {
    pub date: String,
    pub name: String,
    pub is_off_day: bool,
}

/// API 响应格式
/// 接口: https://holiday.ailcc.com/api/holiday/year/{year}
#[derive(Debug, Deserialize)]
struct ApiResponse {
    code: i32,
    holiday: Option<HashMap<String, ApiHolidayData>>,
}

#[derive(Debug, Deserialize)]
struct ApiHolidayData {
    holiday: bool,
    name: String,
    date: String,
    #[allow(dead_code)]
    wage: Option<i32>,
    #[serde(rename = "cnLunar")]
    #[allow(dead_code)]
    cn_lunar: Option<String>,
    #[allow(dead_code)]
    extra_info: Option<String>,
    #[allow(dead_code)]
    rest: Option<i32>,
}

/// 获取指定年份的节假日数据
#[tauri::command]
pub async fn fetch_holidays(year: i32) -> Result<Vec<HolidayInfo>, String> {
    let url = format!("https://holiday.ailcc.com/api/holiday/year/{}", year);
    
    let response = reqwest::get(&url)
        .await
        .map_err(|e| format!("Failed to fetch holidays: {}", e))?;
    
    if !response.status().is_success() {
        return Err(format!("API returned status: {}", response.status()));
    }
    
    let api_response: ApiResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;
    
    if api_response.code != 0 {
        return Err(format!("API returned error code: {}", api_response.code));
    }
    
    let holidays: Vec<HolidayInfo> = api_response
        .holiday
        .unwrap_or_default()
        .into_iter()
        .map(|(_, data)| {
            HolidayInfo {
                date: data.date,
                name: data.name,
                is_off_day: data.holiday,
            }
        })
        .collect();
    
    Ok(holidays)
}
