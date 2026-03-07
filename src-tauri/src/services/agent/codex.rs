use std::path::Path;
use std::process::Command;

use super::runner::{AgentEvent, AgentOutput, AgentRunner};

pub struct CodexRunner;

impl CodexRunner {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl AgentRunner for CodexRunner {
    fn build_command(
        &self,
        cli_path: &str,
        prompt: &str,
        working_dir: &Path,
        model: Option<&str>,
        _allowed_tools: &[String],
    ) -> Command {
        let mut cmd = Command::new(cli_path);
        cmd.current_dir(working_dir);
        cmd.args(["exec", "--json", "--full-auto"]);
        cmd.args(["--sandbox", "workspace-write"]);
        if let Some(m) = model {
            if !m.is_empty() {
                cmd.args(["--model", m]);
            }
        }
        cmd.arg(prompt);
        cmd
    }

    fn parse_event_line(&self, line: &str) -> Option<AgentEvent> {
        let json: serde_json::Value = serde_json::from_str(line).ok()?;

        match json["type"].as_str()? {
            "item.started" | "item.completed" => {
                let item = &json["item"];
                let item_type = item["type"]
                    .as_str()
                    .or_else(|| item["item_type"].as_str())
                    .unwrap_or("unknown");

                let content = match item_type {
                    "agent_message" | "assistant_message" => {
                        item["text"].as_str().unwrap_or("").to_string()
                    }
                    "command_execution" => {
                        format!("$ {}", item["command"].as_str().unwrap_or(""))
                    }
                    _ => return None,
                };

                Some(AgentEvent::Log {
                    content,
                    level: "stdout".to_string(),
                })
            }
            "turn.completed" => {
                let usage = &json["usage"];
                let input = usage["input_tokens"].as_u64().unwrap_or(0);
                let output = usage["output_tokens"].as_u64().unwrap_or(0);
                Some(AgentEvent::TokenUsage {
                    input_tokens: input,
                    output_tokens: output,
                })
            }
            "turn.failed" => Some(AgentEvent::Failed {
                error: json.to_string(),
            }),
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
        let mut total_input: u64 = 0;
        let mut total_output: u64 = 0;

        for event in events {
            match event {
                AgentEvent::Log { content, .. } => {
                    if !text_response.is_empty() {
                        text_response.push('\n');
                    }
                    text_response.push_str(content);
                }
                AgentEvent::TokenUsage {
                    input_tokens,
                    output_tokens,
                } => {
                    total_input += input_tokens;
                    total_output += output_tokens;
                }
                _ => {}
            }
        }

        AgentOutput {
            text_response,
            input_tokens: Some(total_input),
            output_tokens: Some(total_output),
            estimated_cost_usd: None,
            model_used: None,
            session_id: None,
            exit_code,
            duration_ms,
        }
    }

    async fn get_version(&self, cli_path: &str) -> Result<String, String> {
        let output = Command::new(cli_path)
            .arg("--version")
            .output()
            .map_err(|e| format!("无法执行 {}: {}", cli_path, e))?;

        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(version)
    }

    fn agent_type(&self) -> &str {
        "codex"
    }
}
