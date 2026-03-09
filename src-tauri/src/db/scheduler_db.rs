use rusqlite::{Connection, Result, params};

pub fn update_schedule_status(
    conn: &Connection,
    subtask_id: i64,
    status: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE subtasks SET schedule_status = ?1, updated_at = datetime('now', 'localtime')
         WHERE id = ?2",
        params![status, subtask_id],
    )?;
    Ok(())
}

pub fn update_schedule_error(
    conn: &Connection,
    subtask_id: i64,
    error: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE subtasks SET schedule_error = ?1, updated_at = datetime('now', 'localtime')
         WHERE id = ?2",
        params![error, subtask_id],
    )?;
    Ok(())
}

pub fn increment_retry_count(
    conn: &Connection,
    subtask_id: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE subtasks SET retry_count = retry_count + 1, updated_at = datetime('now', 'localtime')
         WHERE id = ?1",
        params![subtask_id],
    )?;
    Ok(())
}

pub fn reset_retry_count(
    conn: &Connection,
    subtask_id: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE subtasks SET retry_count = 0, updated_at = datetime('now', 'localtime')
         WHERE id = ?1",
        params![subtask_id],
    )?;
    Ok(())
}

pub fn update_priority_score(
    conn: &Connection,
    subtask_id: i64,
    score: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE subtasks SET priority_score = ?1, updated_at = datetime('now', 'localtime')
         WHERE id = ?2",
        params![score, subtask_id],
    )?;
    Ok(())
}

pub fn update_timeout_secs(
    conn: &Connection,
    subtask_id: i64,
    timeout_secs: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE subtasks SET timeout_secs = ?1, updated_at = datetime('now', 'localtime')
         WHERE id = ?2",
        params![timeout_secs, subtask_id],
    )?;
    Ok(())
}

pub fn update_max_retries(
    conn: &Connection,
    subtask_id: i64,
    max_retries: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE subtasks SET max_retries = ?1, updated_at = datetime('now', 'localtime')
         WHERE id = ?2",
        params![max_retries, subtask_id],
    )?;
    Ok(())
}

pub fn get_pending_subtasks(
    conn: &Connection,
) -> Result<Vec<(i64, i64, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.parent_id, s.priority_score
         FROM subtasks s
         JOIN todos t ON s.parent_id = t.id
         WHERE s.schedule_status = 'pending' AND s.completed = 0
         AND (t.schedule_strategy != 'cron' OR t.schedule_enabled = 0)
         ORDER BY s.priority_score DESC",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })?;

    rows.collect()
}

pub fn update_todo_schedule_strategy(
    conn: &Connection,
    todo_id: i64,
    strategy: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE todos SET schedule_strategy = ?1, updated_at = datetime('now', 'localtime')
         WHERE id = ?2",
        params![strategy, todo_id],
    )?;
    Ok(())
}

pub fn update_todo_cron(
    conn: &Connection,
    todo_id: i64,
    cron_expression: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE todos SET cron_expression = ?1, updated_at = datetime('now', 'localtime')
         WHERE id = ?2",
        params![cron_expression, todo_id],
    )?;
    Ok(())
}

pub fn toggle_todo_schedule(
    conn: &Connection,
    todo_id: i64,
    enabled: bool,
) -> Result<()> {
    conn.execute(
        "UPDATE todos SET schedule_enabled = ?1, updated_at = datetime('now', 'localtime')
         WHERE id = ?2",
        params![if enabled { 1 } else { 0 }, todo_id],
    )?;
    Ok(())
}
