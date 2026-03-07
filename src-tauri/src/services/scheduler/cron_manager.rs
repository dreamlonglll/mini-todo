use std::str::FromStr;
use chrono::{Local, DateTime, NaiveDateTime, Utc};
use cron::Schedule;

/// 将 5 段标准 cron 自动转为 6 段（加秒前缀 0）
fn normalize_cron(expression: &str) -> String {
    let parts: Vec<&str> = expression.split_whitespace().collect();
    if parts.len() == 5 {
        format!("0 {}", expression)
    } else {
        expression.to_string()
    }
}

/// 验证 Cron 表达式是否合法
pub fn validate_cron(expression: &str) -> Result<(), String> {
    let expr = normalize_cron(expression);
    Schedule::from_str(&expr)
        .map(|_| ())
        .map_err(|e| format!("无效的 Cron 表达式: {}", e))
}

/// 计算下一次执行时间
pub fn next_execution_time(expression: &str) -> Result<DateTime<Local>, String> {
    let expr = normalize_cron(expression);
    let schedule = Schedule::from_str(&expr)
        .map_err(|e| format!("无效的 Cron 表达式: {}", e))?;

    schedule
        .upcoming(Local)
        .next()
        .ok_or_else(|| "无法计算下次执行时间".to_string())
}

/// 检查当前时间是否应该触发定时任务
/// last_run: 上次运行时间字符串 (ISO 格式)
pub fn should_trigger(
    expression: &str,
    last_run: Option<&str>,
) -> Result<bool, String> {
    let expr = normalize_cron(expression);
    let schedule = Schedule::from_str(&expr)
        .map_err(|e| format!("无效的 Cron 表达式: {}", e))?;

    let now = Local::now();

    let check_from = if let Some(last) = last_run {
        NaiveDateTime::parse_from_str(last, "%Y-%m-%d %H:%M:%S")
            .map(|naive| naive.and_local_timezone(Local).unwrap())
            .or_else(|_| {
                DateTime::parse_from_rfc3339(last)
                    .map(|dt| dt.with_timezone(&Local))
            })
            .unwrap_or_else(|_| now - chrono::Duration::hours(24))
    } else {
        now - chrono::Duration::hours(24)
    };

    // 查找 last_run 之后、now 之前的触发时间
    for next_time in schedule.after(&check_from.with_timezone(&Utc)) {
        let next_local = next_time.with_timezone(&Local);
        if next_local > now {
            return Ok(false);
        }
        if next_local > check_from && next_local <= now {
            return Ok(true);
        }
    }

    Ok(false)
}

/// 获取 Cron 表达式的人类可读描述
pub fn describe_cron(expression: &str) -> String {
    let normalized = normalize_cron(expression);
    let parts: Vec<&str> = normalized.split_whitespace().collect();
    if parts.len() < 6 {
        return expression.to_string();
    }

    let (sec, min, hour, dom, mon, dow) = (parts[0], parts[1], parts[2], parts[3], parts[4], parts[5]);

    if sec == "0" && min != "*" && hour != "*" && dom == "*" && mon == "*" && dow == "*" {
        return format!("每天 {}:{} 执行", hour, min);
    }
    if sec == "0" && min != "*" && hour != "*" && dom == "*" && mon == "*" && dow != "*" {
        let day_name = match dow {
            "1" => "周一",
            "2" => "周二",
            "3" => "周三",
            "4" => "周四",
            "5" => "周五",
            "6" => "周六",
            "0" | "7" => "周日",
            "1-5" => "工作日",
            "0,6" | "6,0" => "周末",
            _ => dow,
        };
        return format!("每{} {}:{} 执行", day_name, hour, min);
    }
    if sec == "0" && dom == "*" && mon == "*" && dow == "*" {
        if let Ok(interval) = min.strip_prefix("*/").unwrap_or("").parse::<u32>() {
            return format!("每 {} 分钟执行", interval);
        }
        if let Ok(interval) = hour.strip_prefix("*/").unwrap_or("").parse::<u32>() {
            return format!("每 {} 小时执行", interval);
        }
    }

    expression.to_string()
}
