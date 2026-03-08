use std::path::Path;
use std::process::Command;

use super::runner::{create_command, AgentEvent, AgentOutput, AgentRunner};

pub struct ClaudeCodeRunner;

impl ClaudeCodeRunner {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl AgentRunner for ClaudeCodeRunner {
    fn build_command(
        &self,
        cli_path: &str,
        prompt: &str,
        working_dir: &Path,
        model: Option<&str>,
        allowed_tools: &[String],
    ) -> Command {
        let mut cmd = create_command(cli_path);
        cmd.current_dir(working_dir);
        cmd.args(["-p", prompt]);
        cmd.args(["--output-format", "stream-json"]);
        cmd.arg("--include-partial-messages");
        cmd.arg("--verbose");
        cmd.arg("--dangerously-skip-permissions");

        if let Some(m) = model {
            if !m.is_empty() {
                cmd.args(["--model", m]);
            }
        }
        if !allowed_tools.is_empty() {
            cmd.args(["--allowedTools", &allowed_tools.join(",")]);
        }
        cmd
    }

    fn parse_event_line(&self, line: &str) -> Option<AgentEvent> {
        let json: serde_json::Value = serde_json::from_str(line).ok()?;

        match json["type"].as_str()? {
            "stream_event" => {
                let text = json["event"]["delta"]["text"].as_str()?;
                Some(AgentEvent::Log {
                    content: text.to_string(),
                    level: "stdout".to_string(),
                })
            }
            "result" => {
                let result_text = json["result"].as_str().unwrap_or("").to_string();
                let cost = json["cost"]["total_cost_usd"].as_f64();
                let session_id = json["session_id"].as_str().map(String::from);
                let model_used = json["model"].as_str().map(String::from);
                Some(AgentEvent::Completed {
                    exit_code: 0,
                    result: serde_json::json!({
                        "text": result_text,
                        "cost_usd": cost,
                        "session_id": session_id,
                        "model": model_used,
                    })
                    .to_string(),
                })
            }
            _ => None,
        }
    }

    fn extract_output(
        &self,
        events: &[AgentEvent],
        exit_code: i32,
        duration_ms: u64,
    ) -> AgentOutput {
        let mut text_response = String::new();
        let mut cost: Option<f64> = None;
        let mut session_id: Option<String> = None;
        let mut model_used: Option<String> = None;

        for event in events {
            match event {
                AgentEvent::Log { content, .. } => text_response.push_str(content),
                AgentEvent::Completed { result, .. } => {
                    if let Ok(data) = serde_json::from_str::<serde_json::Value>(result) {
                        if let Some(text) = data["text"].as_str() {
                            if !text.is_empty() {
                                text_response = text.to_string();
                            }
                        }
                        cost = data["cost_usd"].as_f64();
                        session_id = data["session_id"].as_str().map(String::from);
                        model_used = data["model"].as_str().map(String::from);
                    }
                }
                _ => {}
            }
        }

        AgentOutput {
            text_response,
            input_tokens: None,
            output_tokens: None,
            estimated_cost_usd: cost,
            model_used,
            session_id,
            exit_code,
            duration_ms,
        }
    }

    async fn get_version(&self, cli_path: &str) -> Result<String, String> {
        let output = create_command(cli_path)
            .arg("--version")
            .output()
            .map_err(|e| format!("无法执行 {}: {}", cli_path, e))?;

        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let version = raw
            .split_whitespace()
            .next()
            .unwrap_or(&raw)
            .to_string();
        Ok(version)
    }

    fn agent_type(&self) -> &str {
        "claude_code"
    }
}
