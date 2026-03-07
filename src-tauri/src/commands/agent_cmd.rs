use tauri::State;

use crate::db::agent_db;
use crate::db::models::{AgentConfig, AgentHealthStatus, CreateAgentRequest, UpdateAgentRequest};
use crate::db::Database;
use crate::services::agent::{encrypt_api_key, AgentManager};

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
    let encrypted_key = if let Some(ref key) = request.api_key {
        encrypt_api_key(key)?
    } else {
        String::new()
    };

    db.with_connection(|conn| agent_db::create_agent(conn, &request, &encrypted_key))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_agent(
    db: State<'_, Database>,
    id: i64,
    request: UpdateAgentRequest,
) -> Result<(), String> {
    let encrypted_key = match &request.api_key {
        Some(key) if !key.is_empty() => Some(encrypt_api_key(key)?),
        Some(_) => Some(String::new()),
        None => None,
    };

    db.with_connection(|conn| agent_db::update_agent(conn, id, &request, encrypted_key.as_deref()))
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
