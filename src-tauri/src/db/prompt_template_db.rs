use rusqlite::{Connection, Result, params};

use super::models::PromptTemplate;

fn row_to_template(row: &rusqlite::Row) -> rusqlite::Result<PromptTemplate> {
    Ok(PromptTemplate {
        id: row.get(0)?,
        name: row.get(1)?,
        category: row.get(2)?,
        description: row.get(3)?,
        template_content: row.get(4)?,
        variables: row.get(5)?,
        recommended_agent: row.get(6)?,
        is_builtin: row.get::<_, i64>(7)? != 0,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

const TEMPLATE_COLUMNS: &str =
    "id, name, category, description, template_content, variables, recommended_agent, is_builtin, created_at, updated_at";

pub fn get_all_templates(conn: &Connection) -> Result<Vec<PromptTemplate>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM prompt_templates ORDER BY is_builtin DESC, name ASC",
        TEMPLATE_COLUMNS
    ))?;

    let rows = stmt.query_map([], |row| row_to_template(row))?;
    rows.collect()
}

pub fn get_template_by_id(conn: &Connection, id: &str) -> Result<PromptTemplate> {
    conn.query_row(
        &format!(
            "SELECT {} FROM prompt_templates WHERE id = ?1",
            TEMPLATE_COLUMNS
        ),
        params![id],
        |row| row_to_template(row),
    )
}

pub fn get_templates_by_category(
    conn: &Connection,
    category: &str,
) -> Result<Vec<PromptTemplate>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM prompt_templates WHERE category = ?1 ORDER BY is_builtin DESC, name ASC",
        TEMPLATE_COLUMNS
    ))?;

    let rows = stmt.query_map(params![category], |row| row_to_template(row))?;
    rows.collect()
}

pub fn create_template(
    conn: &Connection,
    id: &str,
    name: &str,
    category: Option<&str>,
    description: Option<&str>,
    template_content: &str,
    variables: &str,
    recommended_agent: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO prompt_templates (id, name, category, description, template_content, variables, recommended_agent)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, name, category, description, template_content, variables, recommended_agent],
    )?;
    Ok(())
}

pub fn update_template(
    conn: &Connection,
    id: &str,
    name: &str,
    category: Option<&str>,
    description: Option<&str>,
    template_content: &str,
    variables: &str,
    recommended_agent: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE prompt_templates SET name = ?2, category = ?3, description = ?4,
         template_content = ?5, variables = ?6, recommended_agent = ?7,
         updated_at = datetime('now', 'localtime')
         WHERE id = ?1 AND is_builtin = 0",
        params![id, name, category, description, template_content, variables, recommended_agent],
    )?;
    Ok(())
}

pub fn delete_template(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM prompt_templates WHERE id = ?1 AND is_builtin = 0",
        params![id],
    )?;
    Ok(())
}

/// 用变量值替换模板中的 `{{variable}}` 占位符
pub fn render_template(template: &str, variables: &serde_json::Value) -> String {
    let mut result = template.to_string();

    if let Some(obj) = variables.as_object() {
        for (key, value) in obj {
            let placeholder = format!("{{{{{}}}}}", key);
            let replacement = value.as_str().unwrap_or("");
            result = result.replace(&placeholder, replacement);
        }
    }

    result
}
