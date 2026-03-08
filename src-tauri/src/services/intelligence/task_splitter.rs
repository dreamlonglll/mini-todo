use serde::{Deserialize, Serialize};

use super::context_builder;
use crate::services::agent::runner::create_command;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitResult {
    pub tasks: Vec<SplitTask>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitTask {
    pub title: String,
    pub description: String,
    pub prompt: String,
    #[serde(default = "default_complexity")]
    pub complexity: String,
    #[serde(default = "default_agent")]
    pub recommended_agent: String,
    #[serde(default)]
    pub dependencies: Vec<usize>,
}

fn default_complexity() -> String {
    "medium".to_string()
}

fn default_agent() -> String {
    "claude_code".to_string()
}

const SPLIT_PROMPT_TEMPLATE: &str = r#"你是一个任务拆分专家。请将以下需求拆分为多个独立的、可由 AI 编程助手执行的开发子任务。

【项目上下文】
{context}

【需求描述】
{requirement}

【拆分原则】
1. 每个子任务必须是单个 Agent 可独立完成的，修改范围明确
2. 子任务之间不应有代码冲突
3. 正确标注依赖关系，被依赖的任务排在前面
4. prompt 字段应包含足够的细节让 Agent 直接执行
5. dependencies 数组中的数字是该任务在 tasks 数组中的 0-based 索引

【输出要求】
请严格以 JSON 格式输出，不要包含任何 markdown 代码块标记或额外文字：
{"tasks":[{"title":"简洁的任务标题","description":"任务的详细描述","prompt":"给 Agent 的具体执行指令","complexity":"low|medium|high","recommended_agent":"claude_code|codex","dependencies":[]}],"summary":"整体执行策略说明"}"#;

const SPLIT_TIMEOUT_SECS: u64 = 180;

pub async fn split_task(
    project_path: &str,
    requirement: &str,
    agent_cli: &str,
) -> Result<SplitResult, String> {
    let ctx = context_builder::build_context(project_path).await?;
    let context_text = context_builder::render_context_text(&ctx);

    let prompt = SPLIT_PROMPT_TEMPLATE
        .replace("{context}", &context_text)
        .replace("{requirement}", requirement);

    let output = call_agent_cli(agent_cli, &prompt, project_path).await?;
    let result = extract_json_from_text(&output)?;

    // 验证依赖索引
    let task_count = result.tasks.len();
    let mut validated = result;
    for task in &mut validated.tasks {
        task.dependencies.retain(|&idx| idx < task_count);
    }

    if validated.tasks.is_empty() {
        return Err("AI 未能生成有效的子任务列表".to_string());
    }

    Ok(validated)
}

async fn call_agent_cli(
    cli_name: &str,
    prompt: &str,
    working_dir: &str,
) -> Result<String, String> {
    let mut cmd = create_command(cli_name);
    cmd.current_dir(working_dir);
    cmd.args(["-p", prompt]);
    cmd.args(["--output-format", "json"]);

    let mut tokio_cmd: tokio::process::Command = cmd.into();
    tokio_cmd.stdout(std::process::Stdio::piped());
    tokio_cmd.stderr(std::process::Stdio::piped());

    let child = tokio_cmd
        .spawn()
        .map_err(|e| format!("启动 Agent CLI 失败: {}", e))?;

    let result = tokio::time::timeout(
        tokio::time::Duration::from_secs(SPLIT_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();

            if !output.status.success() && stdout.trim().is_empty() {
                return Err(format!(
                    "Agent CLI 退出码 {}: {}",
                    output.status.code().unwrap_or(-1),
                    if stderr.is_empty() { "unknown error" } else { &stderr }
                ));
            }

            if stdout.trim().is_empty() {
                return Err("Agent CLI 未返回任何输出".to_string());
            }

            Ok(stdout)
        }
        Ok(Err(e)) => Err(format!("Agent CLI 执行失败: {}", e)),
        Err(_) => Err(format!(
            "AI 响应超时（{}秒），请重试",
            SPLIT_TIMEOUT_SECS
        )),
    }
}

fn extract_json_from_text(text: &str) -> Result<SplitResult, String> {
    let trimmed = text.trim();

    // 策略 1：直接解析
    if let Ok(result) = serde_json::from_str::<SplitResult>(trimmed) {
        return Ok(result);
    }

    // 策略 1.5：Claude --output-format json 会输出 {"type":"result","result":"..."}
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(result_str) = json["result"].as_str() {
            if let Ok(result) = serde_json::from_str::<SplitResult>(result_str.trim()) {
                return Ok(result);
            }
            // result 字段可能包含 markdown 代码块
            if let Some(extracted) = extract_from_code_block(result_str) {
                if let Ok(result) = serde_json::from_str::<SplitResult>(&extracted) {
                    return Ok(result);
                }
            }
            // 从 result 文本中提取 JSON
            if let Some(extracted) = extract_outermost_json(result_str) {
                if let Ok(result) = serde_json::from_str::<SplitResult>(&extracted) {
                    return Ok(result);
                }
            }
        }
    }

    // 策略 2：从 ```json ... ``` 代码块中提取
    if let Some(extracted) = extract_from_code_block(trimmed) {
        if let Ok(result) = serde_json::from_str::<SplitResult>(&extracted) {
            return Ok(result);
        }
    }

    // 策略 3：查找最外层 { ... }
    if let Some(extracted) = extract_outermost_json(trimmed) {
        if let Ok(result) = serde_json::from_str::<SplitResult>(&extracted) {
            return Ok(result);
        }
    }

    Err(format!(
        "无法从 AI 输出中提取有效的 JSON。原始输出:\n{}",
        if trimmed.chars().count() > 500 {
            let truncated: String = trimmed.chars().take(500).collect();
            format!("{}...", truncated)
        } else {
            trimmed.to_string()
        }
    ))
}

fn extract_from_code_block(text: &str) -> Option<String> {
    let start_markers = ["```json\n", "```json\r\n", "```\n", "```\r\n"];
    for marker in &start_markers {
        if let Some(start) = text.find(marker) {
            let content_start = start + marker.len();
            if let Some(end) = text[content_start..].find("```") {
                return Some(text[content_start..content_start + end].trim().to_string());
            }
        }
    }
    None
}

fn extract_outermost_json(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let mut depth = 0;
    let mut end = start;
    let bytes = text.as_bytes();
    let mut in_string = false;
    let mut escape = false;

    for i in start..bytes.len() {
        if escape {
            escape = false;
            continue;
        }
        match bytes[i] {
            b'\\' if in_string => escape = true,
            b'"' => in_string = !in_string,
            b'{' if !in_string => depth += 1,
            b'}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }

    if depth == 0 && end > start {
        Some(text[start..=end].to_string())
    } else {
        None
    }
}
