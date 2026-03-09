use rusqlite::{Connection, Result, params};
use super::models::{WorkflowStep, WorkflowStepInput};

pub fn get_workflow_steps(conn: &Connection, todo_id: i64) -> Result<Vec<WorkflowStep>> {
    let mut stmt = conn.prepare(
        "SELECT id, todo_id, step_order, step_type, subtask_id, prompt_text, status, carry_context, created_at
         FROM workflow_steps WHERE todo_id = ?1 ORDER BY step_order"
    )?;
    let rows = stmt.query_map([todo_id], |row| {
        Ok(WorkflowStep {
            id: row.get(0)?,
            todo_id: row.get(1)?,
            step_order: row.get(2)?,
            step_type: row.get(3)?,
            subtask_id: row.get(4)?,
            prompt_text: row.get(5)?,
            status: row.get(6)?,
            carry_context: row.get(7)?,
            created_at: row.get(8)?,
        })
    })?;
    rows.collect()
}

pub fn set_workflow_steps(
    conn: &Connection,
    todo_id: i64,
    steps: &[WorkflowStepInput],
) -> Result<()> {
    conn.execute("DELETE FROM workflow_steps WHERE todo_id = ?1", [todo_id])?;
    let mut stmt = conn.prepare(
        "INSERT INTO workflow_steps (todo_id, step_order, step_type, subtask_id, prompt_text, carry_context)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
    )?;
    for (i, step) in steps.iter().enumerate() {
        stmt.execute(params![
            todo_id,
            i as i32,
            step.step_type,
            step.subtask_id,
            step.prompt_text,
            step.carry_context,
        ])?;
    }
    Ok(())
}

pub fn update_step_status(conn: &Connection, step_id: i64, status: &str) -> Result<()> {
    conn.execute(
        "UPDATE workflow_steps SET status = ?1 WHERE id = ?2",
        params![status, step_id],
    )?;
    Ok(())
}

pub fn find_step_by_subtask(conn: &Connection, subtask_id: i64) -> Result<Option<WorkflowStep>> {
    let result = conn.query_row(
        "SELECT id, todo_id, step_order, step_type, subtask_id, prompt_text, status, carry_context, created_at
         FROM workflow_steps WHERE subtask_id = ?1 AND step_type = 'subtask'",
        [subtask_id],
        |row| {
            Ok(WorkflowStep {
                id: row.get(0)?,
                todo_id: row.get(1)?,
                step_order: row.get(2)?,
                step_type: row.get(3)?,
                subtask_id: row.get(4)?,
                prompt_text: row.get(5)?,
                status: row.get(6)?,
                carry_context: row.get(7)?,
                created_at: row.get(8)?,
            })
        },
    );
    match result {
        Ok(step) => Ok(Some(step)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn reset_all_steps(conn: &Connection, todo_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE workflow_steps SET status = 'pending' WHERE todo_id = ?1",
        [todo_id],
    )?;
    Ok(())
}

pub fn get_step_count(conn: &Connection, todo_id: i64) -> Result<i32> {
    conn.query_row(
        "SELECT COUNT(*) FROM workflow_steps WHERE todo_id = ?1",
        [todo_id],
        |row| row.get(0),
    )
}

pub fn get_step_at_order(conn: &Connection, todo_id: i64, step_order: i32) -> Result<Option<WorkflowStep>> {
    let result = conn.query_row(
        "SELECT id, todo_id, step_order, step_type, subtask_id, prompt_text, status, carry_context, created_at
         FROM workflow_steps WHERE todo_id = ?1 AND step_order = ?2",
        params![todo_id, step_order],
        |row| {
            Ok(WorkflowStep {
                id: row.get(0)?,
                todo_id: row.get(1)?,
                step_order: row.get(2)?,
                step_type: row.get(3)?,
                subtask_id: row.get(4)?,
                prompt_text: row.get(5)?,
                status: row.get(6)?,
                carry_context: row.get(7)?,
                created_at: row.get(8)?,
            })
        },
    );
    match result {
        Ok(step) => Ok(Some(step)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}
