use rusqlite::{Connection, Result, params};

use super::models::{AgentConfig, CreateAgentRequest, UpdateAgentRequest};

pub fn get_all_agents(conn: &Connection) -> Result<Vec<AgentConfig>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, agent_type, cli_path, cli_version, min_cli_version,
                api_key_encrypted, default_model, max_concurrent, timeout_seconds,
                capabilities, env_vars, sandbox_config, enabled, created_at, updated_at
         FROM agent_configs
         ORDER BY created_at ASC",
    )?;

    let agents = stmt
        .query_map([], |row| {
            let api_key_encrypted: String = row.get(6)?;
            Ok(AgentConfig {
                id: row.get(0)?,
                name: row.get(1)?,
                agent_type: row.get(2)?,
                cli_path: row.get(3)?,
                cli_version: row.get(4)?,
                min_cli_version: row.get(5)?,
                api_key_encrypted: api_key_encrypted.clone(),
                default_model: row.get(7)?,
                max_concurrent: row.get(8)?,
                timeout_seconds: row.get(9)?,
                capabilities: row.get(10)?,
                env_vars: row.get(11)?,
                sandbox_config: row.get(12)?,
                enabled: row.get(13)?,
                created_at: row.get(14)?,
                updated_at: row.get(15)?,
                has_api_key: !api_key_encrypted.is_empty(),
            })
        })?
        .collect::<Result<Vec<_>>>()?;

    Ok(agents)
}

pub fn get_agent_by_id(conn: &Connection, id: i64) -> Result<AgentConfig> {
    conn.query_row(
        "SELECT id, name, agent_type, cli_path, cli_version, min_cli_version,
                api_key_encrypted, default_model, max_concurrent, timeout_seconds,
                capabilities, env_vars, sandbox_config, enabled, created_at, updated_at
         FROM agent_configs WHERE id = ?1",
        params![id],
        |row| {
            let api_key_encrypted: String = row.get(6)?;
            Ok(AgentConfig {
                id: row.get(0)?,
                name: row.get(1)?,
                agent_type: row.get(2)?,
                cli_path: row.get(3)?,
                cli_version: row.get(4)?,
                min_cli_version: row.get(5)?,
                api_key_encrypted: api_key_encrypted.clone(),
                default_model: row.get(7)?,
                max_concurrent: row.get(8)?,
                timeout_seconds: row.get(9)?,
                capabilities: row.get(10)?,
                env_vars: row.get(11)?,
                sandbox_config: row.get(12)?,
                enabled: row.get(13)?,
                created_at: row.get(14)?,
                updated_at: row.get(15)?,
                has_api_key: !api_key_encrypted.is_empty(),
            })
        },
    )
}

pub fn create_agent(
    conn: &Connection,
    req: &CreateAgentRequest,
    encrypted_key: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO agent_configs (name, agent_type, cli_path, api_key_encrypted,
         default_model, max_concurrent, timeout_seconds, capabilities, env_vars, sandbox_config)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            req.name,
            req.agent_type,
            req.cli_path,
            encrypted_key,
            req.default_model.as_deref().unwrap_or(""),
            req.max_concurrent.unwrap_or(1),
            req.timeout_seconds.unwrap_or(300),
            req.capabilities.as_deref().unwrap_or("{}"),
            req.env_vars.as_deref().unwrap_or("{}"),
            req.sandbox_config.as_deref().unwrap_or("{}"),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_agent(
    conn: &Connection,
    id: i64,
    req: &UpdateAgentRequest,
    encrypted_key: Option<&str>,
) -> Result<()> {
    let mut sets = Vec::new();
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref name) = req.name {
        sets.push("name = ?");
        values.push(Box::new(name.clone()));
    }
    if let Some(ref agent_type) = req.agent_type {
        sets.push("agent_type = ?");
        values.push(Box::new(agent_type.clone()));
    }
    if let Some(ref cli_path) = req.cli_path {
        sets.push("cli_path = ?");
        values.push(Box::new(cli_path.clone()));
    }
    if let Some(key) = encrypted_key {
        sets.push("api_key_encrypted = ?");
        values.push(Box::new(key.to_string()));
    }
    if let Some(ref default_model) = req.default_model {
        sets.push("default_model = ?");
        values.push(Box::new(default_model.clone()));
    }
    if let Some(max_concurrent) = req.max_concurrent {
        sets.push("max_concurrent = ?");
        values.push(Box::new(max_concurrent));
    }
    if let Some(timeout_seconds) = req.timeout_seconds {
        sets.push("timeout_seconds = ?");
        values.push(Box::new(timeout_seconds));
    }
    if let Some(ref capabilities) = req.capabilities {
        sets.push("capabilities = ?");
        values.push(Box::new(capabilities.clone()));
    }
    if let Some(ref env_vars) = req.env_vars {
        sets.push("env_vars = ?");
        values.push(Box::new(env_vars.clone()));
    }
    if let Some(ref sandbox_config) = req.sandbox_config {
        sets.push("sandbox_config = ?");
        values.push(Box::new(sandbox_config.clone()));
    }
    if let Some(enabled) = req.enabled {
        sets.push("enabled = ?");
        values.push(Box::new(enabled));
    }

    if sets.is_empty() {
        return Ok(());
    }

    sets.push("updated_at = datetime('now', 'localtime')");
    values.push(Box::new(id));

    let sql = format!(
        "UPDATE agent_configs SET {} WHERE id = ?",
        sets.join(", ")
    );
    let params: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
    conn.execute(&sql, params.as_slice())?;

    Ok(())
}

pub fn delete_agent(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM agent_configs WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn update_agent_cli_version(conn: &Connection, id: i64, version: &str) -> Result<()> {
    conn.execute(
        "UPDATE agent_configs SET cli_version = ?1, updated_at = datetime('now', 'localtime') WHERE id = ?2",
        params![version, id],
    )?;
    Ok(())
}
