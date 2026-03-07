use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::{mpsc, Mutex};

use crate::db::models::{AgentConfig, AgentHealthStatus, SandboxConfig};

use super::claude_code::ClaudeCodeRunner;
use super::codex::CodexRunner;
use super::crypto;
use super::worktree::WorktreeManager;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
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

pub struct ExecutionHandle {
    cancel_sender: tokio::sync::oneshot::Sender<()>,
    #[allow(dead_code)]
    child_pid: u32,
}

impl ExecutionHandle {
    pub fn cancel(self) {
        let _ = self.cancel_sender.send(());
    }
}

#[async_trait::async_trait]
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

pub struct AgentManager {
    runners: HashMap<String, Box<dyn AgentRunner>>,
    active_executions: Arc<Mutex<HashMap<String, ExecutionHandle>>>,
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
        }
    }

    pub async fn execute(
        &self,
        config: &AgentConfig,
        prompt: &str,
        project_path: &str,
        task_id: &str,
        event_sender: mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<AgentOutput, String> {
        let runner = self
            .runners
            .get(&config.agent_type)
            .ok_or_else(|| format!("不支持的 Agent 类型: {}", config.agent_type))?;

        let api_key = crypto::decrypt_api_key(&config.api_key_encrypted)?;
        let sandbox: SandboxConfig =
            serde_json::from_str(&config.sandbox_config).unwrap_or_default();

        let work_dir = if sandbox.enable_worktree_isolation {
            let wt = WorktreeManager::create(project_path, task_id)?;
            wt.path().to_string_lossy().to_string()
        } else {
            project_path.to_string()
        };

        let mut cmd = runner.build_command(
            &config.cli_path,
            prompt,
            Path::new(&work_dir),
            Some(&config.default_model),
            &sandbox.allowed_tools,
        );

        cmd.env(self.api_key_env_var(&config.agent_type), &api_key);

        if let Ok(env_map) = serde_json::from_str::<HashMap<String, String>>(&config.env_vars) {
            for (key, value) in &env_map {
                cmd.env(key, value);
            }
        }

        let output = self
            .run_with_streaming(cmd, runner.as_ref(), config.timeout_seconds as u64, event_sender)
            .await?;

        Ok(output)
    }

    async fn run_with_streaming(
        &self,
        cmd: std::process::Command,
        runner: &dyn AgentRunner,
        timeout_secs: u64,
        event_sender: mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<AgentOutput, String> {
        let start = Instant::now();
        let mut collected_events: Vec<AgentEvent> = Vec::new();

        let mut tokio_cmd: TokioCommand = cmd.into();
        tokio_cmd.stdout(std::process::Stdio::piped());
        tokio_cmd.stderr(std::process::Stdio::piped());

        let mut child = tokio_cmd
            .spawn()
            .map_err(|e| format!("启动 Agent 进程失败: {}", e))?;

        let stdout = child.stdout.take().ok_or("无法获取 stdout")?;
        let mut reader = BufReader::new(stdout).lines();

        let timeout = tokio::time::Duration::from_secs(timeout_secs);

        let result = tokio::time::timeout(timeout, async {
            while let Ok(Some(line)) = reader.next_line().await {
                if let Some(event) = runner.parse_event_line(&line) {
                    let _ = event_sender.send(event.clone());
                    collected_events.push(event);
                }
            }
            child.wait().await
        })
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(status)) => {
                let exit_code = status.code().unwrap_or(-1);
                let output = runner.extract_output(&collected_events, exit_code, duration_ms);
                Ok(output)
            }
            Ok(Err(e)) => Err(format!("Agent 进程异常: {}", e)),
            Err(_) => {
                let _ = child.kill().await;
                Err(format!("Agent 执行超时（{}秒）", timeout_secs))
            }
        }
    }

    fn api_key_env_var(&self, agent_type: &str) -> &str {
        match agent_type {
            "claude_code" => "ANTHROPIC_API_KEY",
            "codex" => "OPENAI_API_KEY",
            _ => "API_KEY",
        }
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

        let version_compatible = if let Some(ref ver) = detected_version {
            if config.min_cli_version.is_empty() {
                true
            } else {
                ver >= &config.min_cli_version
            }
        } else {
            false
        };

        let api_key_configured = !config.api_key_encrypted.is_empty();

        let status = if !cli_found {
            "unavailable"
        } else if !version_compatible {
            "outdated"
        } else if !api_key_configured {
            "no_key"
        } else {
            "healthy"
        };

        let message = match status {
            "unavailable" => format!("CLI 未安装或不在 PATH 中: {}", config.cli_path),
            "outdated" => format!(
                "版本过低，当前 {}，要求 >= {}",
                detected_version.as_deref().unwrap_or("?"),
                config.min_cli_version
            ),
            "no_key" => "API Key 未配置".to_string(),
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
            Ok(())
        } else {
            Err("未找到执行中的任务".to_string())
        }
    }
}
