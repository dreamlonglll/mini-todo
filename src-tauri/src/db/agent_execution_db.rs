use rusqlite::{Connection, Result, params};

use super::models::AgentExecution;

pub fn save_execution(
    conn: &Connection,
    task_id: &str,
    subtask_id: Option<i64>,
    agent_id: Option<i64>,
    agent_type: &str,
    status: &str,
    logs_json: &str,
    result_text: &str,
    error: Option<&str>,
    input_tokens: i64,
    output_tokens: i64,
    start_time_ms: i64,
    duration_ms: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO agent_executions
            (task_id, subtask_id, agent_id, agent_type, status, logs, result_text, error,
             input_tokens, output_tokens, start_time_ms, duration_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            task_id,
            subtask_id,
            agent_id,
            agent_type,
            status,
            logs_json,
            result_text,
            error,
            input_tokens,
            output_tokens,
            start_time_ms,
            duration_ms,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_by_task_id_prefix(
    conn: &Connection,
    prefix: &str,
) -> Result<Vec<AgentExecution>> {
    let pattern = format!("{}%", prefix);
    let mut stmt = conn.prepare(
        "SELECT id, task_id, subtask_id, agent_id, agent_type, status, logs, result_text, error,
                input_tokens, output_tokens, start_time_ms, duration_ms, created_at
         FROM agent_executions
         WHERE task_id LIKE ?1
         ORDER BY start_time_ms DESC",
    )?;

    let rows = stmt.query_map(params![pattern], |row| {
        Ok(AgentExecution {
            id: row.get(0)?,
            task_id: row.get(1)?,
            subtask_id: row.get(2)?,
            agent_id: row.get(3)?,
            agent_type: row.get(4)?,
            status: row.get(5)?,
            logs: row.get(6)?,
            result_text: row.get(7)?,
            error: row.get(8)?,
            input_tokens: row.get(9)?,
            output_tokens: row.get(10)?,
            start_time_ms: row.get(11)?,
            duration_ms: row.get(12)?,
            created_at: row.get(13)?,
        })
    })?;
    rows.collect()
}

pub fn get_latest_by_subtask(
    conn: &Connection,
    subtask_id: i64,
) -> Result<Option<AgentExecution>> {
    let mut stmt = conn.prepare(
        "SELECT id, task_id, subtask_id, agent_id, agent_type, status, logs, result_text, error,
                input_tokens, output_tokens, start_time_ms, duration_ms, created_at
         FROM agent_executions
         WHERE subtask_id = ?1
         ORDER BY id DESC
         LIMIT 1",
    )?;

    let mut rows = stmt.query_map(params![subtask_id], |row| {
        Ok(AgentExecution {
            id: row.get(0)?,
            task_id: row.get(1)?,
            subtask_id: row.get(2)?,
            agent_id: row.get(3)?,
            agent_type: row.get(4)?,
            status: row.get(5)?,
            logs: row.get(6)?,
            result_text: row.get(7)?,
            error: row.get(8)?,
            input_tokens: row.get(9)?,
            output_tokens: row.get(10)?,
            start_time_ms: row.get(11)?,
            duration_ms: row.get(12)?,
            created_at: row.get(13)?,
        })
    })?;

    match rows.next() {
        Some(Ok(record)) => Ok(Some(record)),
        _ => Ok(None),
    }
}
