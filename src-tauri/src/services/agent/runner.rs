use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use tauri::{Emitter, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::Mutex;

use crate::db::models::{AgentConfig, AgentHealthStatus};
use crate::db::{Database, agent_execution_db};

use super::claude_code::ClaudeCodeRunner;
use super::codex::CodexRunner;

/// 从 Windows 注册表读取完整的系统 + 用户 PATH，解决 Tauri GUI
/// 进程启动时 PATH 不完整导致找不到 CLI 的问题。
#[cfg(target_os = "windows")]
fn get_registry_path() -> Option<String> {
    use winreg::enums::*;
    use winreg::RegKey;

    let mut paths = Vec::new();

    if let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment")
    {
        if let Ok(val) = key.get_value::<String, _>("Path") {
            paths.push(val);
        }
    }

    if let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Environment") {
        if let Ok(val) = key.get_value::<String, _>("Path") {
            paths.push(val);
        }
    }

    if paths.is_empty() {
        None
    } else {
        Some(paths.join(";"))
    }
}

/// 合并当前进程 PATH 与注册表 PATH，返回完整的 PATH 字符串。
#[cfg(target_os = "windows")]
fn get_merged_path() -> String {
    let current = std::env::var("PATH").unwrap_or_default();
    match get_registry_path() {
        Some(reg) if !reg.is_empty() => {
            if current.is_empty() {
                reg
            } else {
                format!("{};{}", current, reg)
            }
        }
        _ => current,
    }
}

/// 在合并后的 PATH 目录中搜索可执行文件。
/// 返回找到的完整路径和文件类型（exe 或 cmd）。
#[cfg(target_os = "windows")]
enum ResolvedProgram {
    Exe(String),
    CmdScript { node_exe: String, script: String },
    NotFound,
}

#[cfg(target_os = "windows")]
fn resolve_in_path(program: &str) -> ResolvedProgram {
    let merged = get_merged_path();

    for dir in merged.split(';') {
        let dir = dir.trim();
        if dir.is_empty() {
            continue;
        }
        let base = Path::new(dir);

        let exe = base.join(format!("{}.exe", program));
        if exe.exists() {
            return ResolvedProgram::Exe(exe.to_string_lossy().to_string());
        }
        let com = base.join(format!("{}.com", program));
        if com.exists() {
            return ResolvedProgram::Exe(com.to_string_lossy().to_string());
        }

        let cmd_file = base.join(format!("{}.cmd", program));
        if cmd_file.exists() {
            if let Some((node, script)) = parse_npm_cmd_file(&cmd_file) {
                return ResolvedProgram::CmdScript {
                    node_exe: node,
                    script,
                };
            }
        }
    }

    ResolvedProgram::NotFound
}

/// 解析 npm 生成的 .cmd 包装脚本，提取底层的 node.exe 和 JS
/// 脚本路径，使得我们可以直接通过 node.exe 调用而不经过 cmd.exe，
/// 避免 Rust CVE-2024-24576 安全限制导致的参数校验失败。
///
/// npm .cmd 文件典型结构：
///   IF EXIST "%dp0%\node.exe" ( SET "_prog=%dp0%\node.exe" )
///   "%_prog%"  "%dp0%\node_modules\...\script.js" %*
///
/// 解析策略：
/// 1. 从 IF EXIST 或 SET 行提取 node.exe 的相对路径
/// 2. 从包含 %* 的执行行提取 .js/.mjs 脚本路径
#[cfg(target_os = "windows")]
fn parse_npm_cmd_file(cmd_path: &Path) -> Option<(String, String)> {
    let content = std::fs::read_to_string(cmd_path).ok()?;
    let cmd_dir = cmd_path.parent()?;

    let mut node_path: Option<String> = None;
    let mut script_path: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();

        if line.contains("node.exe") && node_path.is_none() {
            for part in line.split('"') {
                if part.contains("node.exe") && !part.contains("node_modules") {
                    let resolved = part
                        .replace("%dp0%\\", "")
                        .replace("%~dp0\\", "")
                        .replace("%dp0%/", "")
                        .replace("%~dp0/", "");
                    let full = cmd_dir.join(&resolved);
                    if full.exists() {
                        node_path = Some(full.to_string_lossy().to_string());
                        break;
                    }
                }
            }
        }

        if line.contains("%*") && (line.contains(".js") || line.contains(".mjs")) && script_path.is_none() {
            for part in line.split('"') {
                if part.ends_with(".js") || part.ends_with(".mjs") {
                    let resolved = part
                        .replace("%dp0%\\", "")
                        .replace("%~dp0\\", "")
                        .replace("%dp0%/", "")
                        .replace("%~dp0/", "");
                    let full = cmd_dir.join(&resolved);
                    if full.exists() {
                        script_path = Some(full.to_string_lossy().to_string());
                        break;
                    }
                }
            }
        }

        if node_path.is_some() && script_path.is_some() {
            break;
        }
    }

    match (node_path, script_path) {
        (Some(node), Some(script)) => Some((node, script)),
        _ => None,
    }
}

/// 创建一个能正确找到 CLI 可执行文件的 Command。
/// 在 Windows 上通过注册表 PATH 搜索可执行文件：
/// - 找到 .exe 文件时直接使用完整路径
/// - 找到 .cmd (npm包装脚本) 时，解析出底层 node.exe + JS 脚本，
///   直接通过 node.exe 调用，绕过 cmd.exe 的参数限制
pub fn create_command(program: &str) -> std::process::Command {
    #[cfg(target_os = "windows")]
    {
        let path = Path::new(program);
        if !path.is_absolute() && !program.contains('\\') && !program.contains('/') {
            match resolve_in_path(program) {
                ResolvedProgram::Exe(exe_path) => {
                    let mut cmd = std::process::Command::new(&exe_path);
                    let merged = get_merged_path();
                    if !merged.is_empty() {
                        cmd.env("PATH", merged);
                    }
                    return cmd;
                }
                ResolvedProgram::CmdScript { node_exe, script } => {
                    let mut cmd = std::process::Command::new(&node_exe);
                    cmd.arg(&script);
                    let merged = get_merged_path();
                    if !merged.is_empty() {
                        cmd.env("PATH", merged);
                    }
                    return cmd;
                }
                ResolvedProgram::NotFound => {}
            }
        }

        let mut cmd = std::process::Command::new(program);
        let merged = get_merged_path();
        if !merged.is_empty() {
            cmd.env("PATH", merged);
        }
        cmd
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new(program)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
#[allow(dead_code)]
pub enum AgentEvent {
    Log { content: String, level: String },
    Progress { message: String },
    TokenUsage { input_tokens: u64, output_tokens: u64 },
    Completed { exit_code: i32, result: String },
    Failed { error: String },
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentOutput {
    pub text_response: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub estimated_cost_usd: Option<f64>,
    pub model_used: Option<String>,
    pub session_id: Option<String>,
    pub exit_code: i32,
    pub duration_ms: u64,
}

/// 将文本中以 `$ ` 开头的行转换为 Markdown 代码块。
/// 连续的命令行会合并到同一个代码块中。
fn format_inline_commands(text: &str) -> String {
    let mut result = Vec::new();
    let mut cmd_block: Vec<&str> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("$ ") {
            cmd_block.push(trimmed);
        } else {
            if !cmd_block.is_empty() {
                result.push(format!("```\n{}\n```", cmd_block.join("\n")));
                cmd_block.clear();
            }
            result.push(line.to_string());
        }
    }
    if !cmd_block.is_empty() {
        result.push(format!("```\n{}\n```", cmd_block.join("\n")));
    }
    result.join("\n")
}

/// 将 Agent CLI 输出的原始 JSON 行解析为人类可读的日志文本。
/// 支持 Codex 和 Claude Code 两种格式。
/// 返回 None 表示该行无需显示（如心跳/内部事件）。
fn format_agent_json(line: &str, agent_type: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let json: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return Some((trimmed.to_string(), "stdout".to_string())),
    };
    let event_type = json["type"].as_str().unwrap_or("");

    match agent_type {
        "codex" => match event_type {
            "item.started" | "item.completed" => {
                let item = &json["item"];
                let item_type = item["type"]
                    .as_str()
                    .or_else(|| item["item_type"].as_str())
                    .unwrap_or("");
                let is_completed = event_type == "item.completed";
                match item_type {
                    "agent_message" | "assistant_message" => {
                        if !is_completed {
                            return None;
                        }
                        let text = item["text"].as_str().unwrap_or("");
                        if text.is_empty() {
                            return None;
                        }
                        let formatted = format_inline_commands(text);
                        Some((formatted, "stdout".to_string()))
                    }
                    "command_execution" => {
                        if !is_completed {
                            return None;
                        }
                        let cmd_str = item["command"].as_str().unwrap_or("");
                        let exit = item["exit_code"].as_i64();
                        let output = item["output"].as_str().unwrap_or("");

                        let mut code_lines = Vec::new();
                        if !cmd_str.is_empty() {
                            code_lines.push(format!("$ {}", cmd_str));
                        }
                        if !output.is_empty() {
                            if output.len() > 2000 {
                                code_lines.push(output[..2000].to_string());
                                code_lines.push("... (truncated)".to_string());
                            } else {
                                code_lines.push(output.to_string());
                            }
                        }

                        if code_lines.is_empty() {
                            return None;
                        }

                        let mut result = format!("```\n{}\n```", code_lines.join("\n"));
                        if let Some(code) = exit {
                            if code != 0 {
                                result.push_str(&format!("\n*exit code: {}*", code));
                            }
                        }
                        Some((result, "stdout".to_string()))
                    }
                    "file_edit" | "file_write" => {
                        if !is_completed {
                            return None;
                        }
                        let path = item["path"].as_str().unwrap_or("");
                        if path.is_empty() {
                            return None;
                        }
                        Some((format!("**[Edited]** `{}`", path), "info".to_string()))
                    }
                    "file_read" => {
                        if !is_completed {
                            return None;
                        }
                        let path = item["path"].as_str().unwrap_or("");
                        if path.is_empty() {
                            return None;
                        }
                        Some((format!("**[Reading]** `{}`", path), "info".to_string()))
                    }
                    _ => None,
                }
            }
            "message.delta" => {
                let text = json["delta"]["text"].as_str().unwrap_or("");
                if text.is_empty() {
                    return None;
                }
                let formatted = format_inline_commands(text);
                Some((formatted, "stdout".to_string()))
            }
            "turn.completed" => {
                let usage = &json["usage"];
                let input = usage["input_tokens"].as_u64().unwrap_or(0);
                let output = usage["output_tokens"].as_u64().unwrap_or(0);
                if input > 0 || output > 0 {
                    Some((
                        format!("[Token] input: {}, output: {}", input, output),
                        "info".to_string(),
                    ))
                } else {
                    None
                }
            }
            "turn.failed" => {
                let msg = json["error"]["message"]
                    .as_str()
                    .unwrap_or("unknown error");
                Some((format!("[Error] {}", msg), "stderr".to_string()))
            }
            _ => None,
        },

        "claude_code" => match event_type {
            "assistant" => {
                let content = &json["message"]["content"];
                if let Some(arr) = content.as_array() {
                    let mut texts = Vec::new();
                    for block in arr {
                        if let Some(text) = block["text"].as_str() {
                            if !text.is_empty() {
                                texts.push(text.to_string());
                            }
                        }
                    }
                    if texts.is_empty() {
                        None
                    } else {
                        let formatted = format_inline_commands(&texts.join("\n"));
                        Some((formatted, "stdout".to_string()))
                    }
                } else if let Some(text) = json["content"].as_str() {
                    if text.is_empty() { None }
                    else { Some((text.to_string(), "stdout".to_string())) }
                } else {
                    None
                }
            }
            "content_block_delta" => {
                let text = json["delta"]["text"].as_str().unwrap_or("");
                if text.is_empty() { None }
                else { Some((text.to_string(), "stdout".to_string())) }
            }
            "tool_use" => {
                let name = json["tool"]["name"]
                    .as_str()
                    .or_else(|| json["name"].as_str())
                    .unwrap_or("tool");
                let input = if json["tool"]["input"].is_object() {
                    &json["tool"]["input"]
                } else {
                    &json["input"]
                };
                let path = input["file_path"]
                    .as_str()
                    .or_else(|| input["path"].as_str())
                    .or_else(|| input["command"].as_str())
                    .unwrap_or("");
                if path.is_empty() {
                    Some((format!("**[Tool]** `{}`", name), "info".to_string()))
                } else {
                    Some((format!("**[Tool]** `{}` `{}`", name, path), "info".to_string()))
                }
            }
            "tool_result" => None,
            "result" => {
                let cost = json["total_cost_usd"].as_f64().or_else(|| json["cost_usd"].as_f64());
                let num_turns = json["num_turns"].as_u64();
                let duration = json["duration_ms"].as_u64();
                let mut info_parts = Vec::new();
                if let Some(c) = cost {
                    info_parts.push(format!("Cost: ${:.4}", c));
                }
                if let Some(t) = num_turns {
                    info_parts.push(format!("Turns: {}", t));
                }
                if let Some(d) = duration {
                    info_parts.push(format!("Duration: {:.1}s", d as f64 / 1000.0));
                }
                if info_parts.is_empty() { None }
                else { Some((format!("---\n*{}*", info_parts.join(" | ")), "info".to_string())) }
            }
            "system" => None,
            "content_block_start" | "content_block_stop" | "message_start" | "message_stop" | "user" => None,
            _ => None,
        },

        _ => Some((line.to_string(), "stdout".to_string())),
    }
}

pub struct ExecutionHandle {
    cancel_sender: tokio::sync::oneshot::Sender<()>,
}

impl ExecutionHandle {
    pub fn cancel(self) {
        let _ = self.cancel_sender.send(());
    }
}

#[async_trait::async_trait]
#[allow(dead_code)]
pub trait AgentRunner: Send + Sync {
    fn build_command(
        &self,
        cli_path: &str,
        prompt: &str,
        working_dir: &Path,
        model: Option<&str>,
        allowed_tools: &[String],
    ) -> std::process::Command;

    fn parse_event_line(&self, line: &str) -> Option<AgentEvent>;

    fn extract_output(
        &self,
        events: &[AgentEvent],
        exit_code: i32,
        duration_ms: u64,
    ) -> AgentOutput;

    async fn get_version(&self, cli_path: &str) -> Result<String, String>;

    fn agent_type(&self) -> &str;
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionState {
    pub task_id: String,
    pub subtask_id: Option<i64>,
    pub agent_type: String,
    pub status: String,
    pub logs: Vec<CachedLog>,
    pub result: Option<AgentOutput>,
    pub error: Option<String>,
    pub start_time_ms: u64,
    pub duration_ms: Option<u64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedLog {
    pub content: String,
    pub level: String,
    pub timestamp_ms: u64,
}

pub struct AgentManager {
    runners: HashMap<String, Box<dyn AgentRunner>>,
    active_executions: Arc<Mutex<HashMap<String, ExecutionHandle>>>,
    execution_states: Arc<Mutex<HashMap<String, ExecutionState>>>,
}

impl AgentManager {
    pub fn new() -> Self {
        let mut runners: HashMap<String, Box<dyn AgentRunner>> = HashMap::new();
        runners.insert(
            "claude_code".to_string(),
            Box::new(ClaudeCodeRunner::new()),
        );
        runners.insert("codex".to_string(), Box::new(CodexRunner::new()));
        Self {
            runners,
            active_executions: Arc::new(Mutex::new(HashMap::new())),
            execution_states: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn start_background_execution(
        &self,
        config: AgentConfig,
        prompt: String,
        project_path: String,
        task_id: String,
        subtask_id: Option<i64>,
        app: tauri::AppHandle,
    ) -> Result<(), String> {
        let runner = self
            .runners
            .get(&config.agent_type)
            .ok_or_else(|| format!("不支持的 Agent 类型: {}", config.agent_type))?;

        let work_dir = project_path.clone();

        let cmd = runner.build_command(
            &config.cli_path,
            &prompt,
            Path::new(&work_dir),
            None,
            &[],
        );

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let state = ExecutionState {
            task_id: task_id.clone(),
            subtask_id,
            agent_type: config.agent_type.clone(),
            status: "running".to_string(),
            logs: Vec::new(),
            result: None,
            error: None,
            start_time_ms: now_ms,
            duration_ms: None,
            input_tokens: 0,
            output_tokens: 0,
        };
        self.execution_states
            .lock()
            .await
            .insert(task_id.clone(), state);

        let states = self.execution_states.clone();
        let timeout_secs = if let Some(sid) = subtask_id {
            let db = app.state::<Database>();
            db.with_connection(|conn| {
                conn.query_row(
                    "SELECT timeout_secs FROM subtasks WHERE id = ?",
                    [sid],
                    |row| row.get::<_, i64>(0),
                )
            })
            .unwrap_or(600) as u64
        } else {
            600u64
        };
        let task_id_clone = task_id.clone();
        let event_name = format!("agent:log:{}", task_id);
        let agent_type_for_task = config.agent_type.clone();
        let agent_id_for_db = Some(config.id);

        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
        {
            let mut executions = self.active_executions.lock().await;
            executions.insert(
                task_id.clone(),
                ExecutionHandle {
                    cancel_sender: cancel_tx,
                },
            );
        }

        let active_execs = self.active_executions.clone();
        tokio::spawn(async move {
            let start = Instant::now();
            let run_result = Self::run_process(
                cmd,
                timeout_secs,
                agent_type_for_task,
                states.clone(),
                &task_id_clone,
                app.clone(),
                &event_name,
                cancel_rx,
            )
            .await;

            let duration_ms = start.elapsed().as_millis() as u64;
            let mut states_lock = states.lock().await;
            if let Some(state) = states_lock.get_mut(&task_id_clone) {
                state.duration_ms = Some(duration_ms);
                let already_cancelled = state.status == "cancelled";
                if !already_cancelled {
                    match run_result {
                        Ok(output) => {
                            state.status = "completed".to_string();
                            state.result = Some(output.clone());
                            let _ = app.emit(
                                &event_name,
                                AgentEvent::Completed {
                                    exit_code: output.exit_code,
                                    result: output.text_response.clone(),
                                },
                            );
                        }
                        Err(err) => {
                            state.status = "failed".to_string();
                            state.error = Some(err.clone());
                            let _ = app.emit(
                                &event_name,
                                AgentEvent::Failed { error: err },
                            );
                        }
                    }
                }

                Self::persist_execution(&app, state, agent_id_for_db);
            }
            active_execs.lock().await.remove(&task_id_clone);
        });

        Ok(())
    }

    async fn run_process(
        cmd: std::process::Command,
        timeout_secs: u64,
        agent_type: String,
        states: Arc<Mutex<HashMap<String, ExecutionState>>>,
        task_id: &str,
        app: tauri::AppHandle,
        event_name: &str,
        cancel_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<AgentOutput, String> {
        use tauri::Emitter;

        let mut tokio_cmd: TokioCommand = cmd.into();
        tokio_cmd.stdout(std::process::Stdio::piped());
        tokio_cmd.stderr(std::process::Stdio::piped());

        let mut child = tokio_cmd
            .spawn()
            .map_err(|e| format!("启动 Agent 进程失败: {}", e))?;

        let stdout = child.stdout.take().ok_or("无法获取 stdout")?;
        let stderr = child.stderr.take().ok_or("无法获取 stderr")?;
        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let states_stderr = states.clone();
        let task_id_stderr = task_id.to_string();
        let app_stderr = app.clone();
        let event_name_stderr = event_name.to_string();
        let stderr_handle = tokio::spawn(async move {
            let mut stderr_lines = Vec::new();
            while let Ok(Some(line)) = stderr_reader.next_line().await {
                stderr_lines.push(line.clone());
                let event = AgentEvent::Log {
                    content: line.clone(),
                    level: "stderr".to_string(),
                };
                let _ = app_stderr.emit(&event_name_stderr, &event);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let mut lock = states_stderr.lock().await;
                if let Some(s) = lock.get_mut(&task_id_stderr) {
                    s.logs.push(CachedLog {
                        content: line,
                        level: "stderr".to_string(),
                        timestamp_ms: now,
                    });
                }
            }
            stderr_lines
        });

        let states_stdout = states.clone();
        let task_id_stdout = task_id.to_string();
        let app_stdout = app.clone();
        let event_name_stdout = event_name.to_string();
        let agent_type_clone = agent_type.clone();

        let timeout = tokio::time::Duration::from_secs(timeout_secs);
        let mut total_input = 0u64;
        let mut total_output = 0u64;
        let mut text_response = String::new();

        let child_id = child.id();

        let process_future = async {
            while let Ok(Some(line)) = stdout_reader.next_line().await {
                if let Some((readable, level)) = format_agent_json(&line, &agent_type_clone) {
                    let log_event = AgentEvent::Log {
                        content: readable.clone(),
                        level: level.clone(),
                    };
                    let _ = app_stdout.emit(&event_name_stdout, &log_event);

                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    let mut lock = states_stdout.lock().await;
                    if let Some(s) = lock.get_mut(&task_id_stdout) {
                        s.logs.push(CachedLog {
                            content: readable.clone(),
                            level: level.clone(),
                            timestamp_ms: now,
                        });
                    }
                    drop(lock);

                    if level == "stdout" {
                        if !text_response.is_empty() {
                            text_response.push('\n');
                        }
                        text_response.push_str(&readable);
                    }
                }

                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                    let event_type = json["type"].as_str().unwrap_or("");
                    if event_type == "turn.completed" {
                        let usage = &json["usage"];
                        let inp = usage["input_tokens"].as_u64().unwrap_or(0);
                        let out = usage["output_tokens"].as_u64().unwrap_or(0);
                        total_input += inp;
                        total_output += out;
                        let evt = AgentEvent::TokenUsage {
                            input_tokens: inp,
                            output_tokens: out,
                        };
                        let _ = app_stdout.emit(&event_name_stdout, &evt);
                        let mut lock = states_stdout.lock().await;
                        if let Some(s) = lock.get_mut(&task_id_stdout) {
                            s.input_tokens = total_input;
                            s.output_tokens = total_output;
                        }
                        drop(lock);
                    }
                }
            }
            child.wait().await
        };

        enum ProcessResult {
            Completed(Result<std::process::ExitStatus, std::io::Error>),
            Timeout,
            Cancelled,
        }

        let result = tokio::select! {
            res = tokio::time::timeout(timeout, process_future) => {
                match res {
                    Ok(wait_result) => ProcessResult::Completed(wait_result),
                    Err(_) => ProcessResult::Timeout,
                }
            }
            _ = cancel_rx => {
                ProcessResult::Cancelled
            }
        };

        // 超时或取消时，杀掉子进程
        if matches!(result, ProcessResult::Timeout | ProcessResult::Cancelled) {
            if let Some(pid) = child_id {
                #[cfg(windows)]
                {
                    let _ = std::process::Command::new("taskkill")
                        .args(["/F", "/T", "/PID", &pid.to_string()])
                        .output();
                }
                #[cfg(not(windows))]
                {
                    unsafe { libc::kill(pid as i32, libc::SIGKILL); }
                }
            }
        }

        let stderr_lines = stderr_handle.await.unwrap_or_default();

        match result {
            ProcessResult::Completed(Ok(status)) => {
                let exit_code = status.code().unwrap_or(-1);
                if exit_code != 0 && text_response.is_empty() && !stderr_lines.is_empty() {
                    return Err(format!(
                        "Agent 进程退出码 {}:\n{}",
                        exit_code,
                        stderr_lines.join("\n")
                    ));
                }
                Ok(AgentOutput {
                    text_response,
                    input_tokens: Some(total_input),
                    output_tokens: Some(total_output),
                    estimated_cost_usd: None,
                    model_used: None,
                    session_id: None,
                    exit_code,
                    duration_ms: 0,
                })
            }
            ProcessResult::Completed(Err(e)) => Err(format!("Agent 进程异常: {}", e)),
            ProcessResult::Timeout => {
                Err(format!("Agent 执行超时（{}秒）", timeout_secs))
            }
            ProcessResult::Cancelled => {
                Err("任务已被用户终止".to_string())
            }
        }
    }

    fn persist_execution(
        app: &tauri::AppHandle,
        state: &ExecutionState,
        agent_id: Option<i64>,
    ) {
        use tauri::Manager;
        let db = app.state::<Database>();
        let logs_json = serde_json::to_string(&state.logs).unwrap_or_else(|_| "[]".to_string());
        let result_text = state
            .result
            .as_ref()
            .map(|o| o.text_response.clone())
            .unwrap_or_default();
        let input_tokens = state
            .result
            .as_ref()
            .and_then(|o| o.input_tokens)
            .unwrap_or(0) as i64;
        let output_tokens = state
            .result
            .as_ref()
            .and_then(|o| o.output_tokens)
            .unwrap_or(0) as i64;

        let _ = db.with_connection(|conn| {
            agent_execution_db::save_execution(
                conn,
                &state.task_id,
                state.subtask_id,
                agent_id,
                &state.agent_type,
                &state.status,
                &logs_json,
                &result_text,
                state.error.as_deref(),
                input_tokens,
                output_tokens,
                state.start_time_ms as i64,
                state.duration_ms.unwrap_or(0) as i64,
            )
        });
    }

    pub async fn get_execution_state(&self, task_id: &str) -> Option<ExecutionState> {
        self.execution_states.lock().await.get(task_id).cloned()
    }

    pub async fn get_execution_by_subtask(&self, subtask_id: i64) -> Option<ExecutionState> {
        let states = self.execution_states.lock().await;
        states
            .values()
            .filter(|s| s.subtask_id == Some(subtask_id))
            .max_by_key(|s| s.start_time_ms)
            .cloned()
    }

    pub async fn check_health(&self, config: &AgentConfig) -> AgentHealthStatus {
        let runner = match self.runners.get(&config.agent_type) {
            Some(r) => r,
            None => {
                return AgentHealthStatus {
                    agent_id: config.id,
                    cli_found: false,
                    detected_version: None,
                    version_compatible: false,
                    status: "error".to_string(),
                    message: Some(format!("不支持的 Agent 类型: {}", config.agent_type)),
                }
            }
        };

        let version_result = runner.get_version(&config.cli_path).await;
        let (cli_found, detected_version) = match version_result {
            Ok(v) => (true, Some(v)),
            Err(_) => (false, None),
        };

        let version_compatible = cli_found;

        let status = if !cli_found {
            "unavailable"
        } else {
            "healthy"
        };

        let message = match status {
            "unavailable" => format!("CLI 未安装或不在 PATH 中: {}", config.cli_path),
            "healthy" => format!(
                "就绪 (v{})",
                detected_version.as_deref().unwrap_or("?")
            ),
            _ => String::new(),
        };

        AgentHealthStatus {
            agent_id: config.id,
            cli_found,
            detected_version,
            version_compatible,
            status: status.to_string(),
            message: Some(message),
        }
    }

    pub async fn cancel_execution(&self, execution_id: &str) -> Result<(), String> {
        let mut executions = self.active_executions.lock().await;
        if let Some(handle) = executions.remove(execution_id) {
            handle.cancel();
            let mut states = self.execution_states.lock().await;
            if let Some(state) = states.get_mut(execution_id) {
                state.status = "cancelled".to_string();
            }
            Ok(())
        } else {
            Err("未找到执行中的任务".to_string())
        }
    }
}
