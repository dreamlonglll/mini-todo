use tauri::State;

use crate::db::agent_db;
use crate::db::agent_execution_db;
use crate::db::models::{AgentConfig, AgentHealthStatus, CreateAgentRequest, UpdateAgentRequest};
use crate::db::Database;
use crate::services::agent::AgentManager;
use crate::services::agent::runner::{create_command, CachedLog, ExecutionState};

#[tauri::command]
pub fn get_agents(db: State<'_, Database>) -> Result<Vec<AgentConfig>, String> {
    db.with_connection(|conn| agent_db::get_all_agents(conn))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_agent(db: State<'_, Database>, id: i64) -> Result<AgentConfig, String> {
    db.with_connection(|conn| agent_db::get_agent_by_id(conn, id))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_agent(
    db: State<'_, Database>,
    request: CreateAgentRequest,
) -> Result<i64, String> {
    db.with_connection(|conn| agent_db::create_agent(conn, &request))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_agent(
    db: State<'_, Database>,
    id: i64,
    request: UpdateAgentRequest,
) -> Result<(), String> {
    db.with_connection(|conn| agent_db::update_agent(conn, id, &request))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_agent(db: State<'_, Database>, id: i64) -> Result<(), String> {
    db.with_connection(|conn| agent_db::delete_agent(conn, id))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn check_agent_health(
    db: State<'_, Database>,
    agent_manager: State<'_, AgentManager>,
    id: i64,
) -> Result<AgentHealthStatus, String> {
    let config = db
        .with_connection(|conn| agent_db::get_agent_by_id(conn, id))
        .map_err(|e| e.to_string())?;

    Ok(agent_manager.check_health(&config).await)
}

#[tauri::command]
pub async fn check_all_agents_health(
    db: State<'_, Database>,
    agent_manager: State<'_, AgentManager>,
) -> Result<Vec<AgentHealthStatus>, String> {
    let agents = db
        .with_connection(|conn| agent_db::get_all_agents(conn))
        .map_err(|e| e.to_string())?;

    let mut results = Vec::new();
    for agent in &agents {
        results.push(agent_manager.check_health(agent).await);
    }
    Ok(results)
}

#[tauri::command]
pub async fn start_agent_execution(
    app: tauri::AppHandle,
    db: State<'_, Database>,
    agent_manager: State<'_, AgentManager>,
    agent_id: i64,
    prompt: String,
    project_path: String,
    task_id: String,
    subtask_id: Option<i64>,
) -> Result<(), String> {
    let config = db
        .with_connection(|conn| agent_db::get_agent_by_id(conn, agent_id))
        .map_err(|e| e.to_string())?;

    if !config.enabled {
        return Err("Agent 已禁用".to_string());
    }

    agent_manager
        .start_background_execution(config, prompt, project_path, task_id, subtask_id, app)
        .await
}

#[tauri::command]
pub async fn get_agent_execution_state(
    agent_manager: State<'_, AgentManager>,
    task_id: String,
) -> Result<Option<ExecutionState>, String> {
    Ok(agent_manager.get_execution_state(&task_id).await)
}

#[tauri::command]
pub async fn get_agent_execution_by_subtask(
    db: State<'_, Database>,
    agent_manager: State<'_, AgentManager>,
    subtask_id: i64,
) -> Result<Option<ExecutionState>, String> {
    if let Some(state) = agent_manager.get_execution_by_subtask(subtask_id).await {
        return Ok(Some(state));
    }

    let record = db
        .with_connection(|conn| agent_execution_db::get_latest_by_subtask(conn, subtask_id))
        .map_err(|e| e.to_string())?;

    if let Some(rec) = record {
        let logs: Vec<CachedLog> = serde_json::from_str(&rec.logs).unwrap_or_default();
        return Ok(Some(ExecutionState {
            task_id: rec.task_id,
            subtask_id: rec.subtask_id,
            agent_type: rec.agent_type,
            status: rec.status,
            logs,
            result: None,
            error: rec.error,
            start_time_ms: rec.start_time_ms as u64,
            duration_ms: Some(rec.duration_ms as u64),
            input_tokens: rec.input_tokens as u64,
            output_tokens: rec.output_tokens as u64,
        }));
    }

    Ok(None)
}

#[tauri::command]
pub async fn cancel_agent_execution(
    agent_manager: State<'_, AgentManager>,
    task_id: String,
) -> Result<(), String> {
    agent_manager.cancel_execution(&task_id).await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedAgent {
    pub agent_type: String,
    pub cli_path: String,
    pub version: Option<String>,
    pub available: bool,
}

#[tauri::command]
pub async fn auto_detect_agents(
    db: State<'_, Database>,
) -> Result<Vec<DetectedAgent>, String> {
    let known = vec![
        ("claude_code", "claude", "Claude Code"),
        ("codex", "codex", "Codex"),
    ];

    let mut detected = Vec::new();

    for (agent_type, cli_name, display_name) in &known {
        let output = create_command(cli_name)
            .arg("--version")
            .output();

        let (available, version) = match output {
            Ok(out) if out.status.success() => {
                let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let ver = raw
                    .rsplit_once(' ')
                    .map(|(_, v)| v.to_string())
                    .unwrap_or(raw.clone());
                (true, Some(ver))
            }
            _ => (false, None),
        };

        detected.push(DetectedAgent {
            agent_type: agent_type.to_string(),
            cli_path: cli_name.to_string(),
            version: version.clone(),
            available,
        });

        if available {
            let existing = db.with_connection(|conn| {
                agent_db::get_all_agents(conn)
            }).unwrap_or_default();

            let already_exists = existing.iter().any(|a| a.agent_type == *agent_type);
            if !already_exists {
                let request = CreateAgentRequest {
                    name: display_name.to_string(),
                    agent_type: agent_type.to_string(),
                    cli_path: cli_name.to_string(),
                };
                let _ = db.with_connection(|conn| {
                    agent_db::create_agent(conn, &request)
                });
            }
        }
    }

    Ok(detected)
}
