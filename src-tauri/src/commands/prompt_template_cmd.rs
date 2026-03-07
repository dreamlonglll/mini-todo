use tauri::State;
use crate::db::Database;
use crate::db::prompt_template_db;
use crate::db::models::PromptTemplate;

#[tauri::command]
pub fn get_prompt_templates(
    db: State<Database>,
) -> Result<Vec<PromptTemplate>, String> {
    db.with_connection(|conn| {
        prompt_template_db::get_all_templates(conn)
    })
    .map_err(|e| format!("获取模板列表失败: {}", e))
}

#[tauri::command]
pub fn get_prompt_template(
    db: State<Database>,
    template_id: String,
) -> Result<PromptTemplate, String> {
    db.with_connection(|conn| {
        prompt_template_db::get_template_by_id(conn, &template_id)
    })
    .map_err(|e| format!("获取模板失败: {}", e))
}

#[tauri::command]
pub fn get_prompt_templates_by_category(
    db: State<Database>,
    category: String,
) -> Result<Vec<PromptTemplate>, String> {
    db.with_connection(|conn| {
        prompt_template_db::get_templates_by_category(conn, &category)
    })
    .map_err(|e| format!("获取模板失败: {}", e))
}

#[tauri::command]
pub fn create_prompt_template(
    db: State<Database>,
    id: String,
    name: String,
    category: Option<String>,
    description: Option<String>,
    template_content: String,
    variables: String,
    recommended_agent: Option<String>,
) -> Result<(), String> {
    db.with_connection(|conn| {
        prompt_template_db::create_template(
            conn,
            &id,
            &name,
            category.as_deref(),
            description.as_deref(),
            &template_content,
            &variables,
            recommended_agent.as_deref(),
        )
    })
    .map_err(|e| format!("创建模板失败: {}", e))
}

#[tauri::command]
pub fn update_prompt_template(
    db: State<Database>,
    id: String,
    name: String,
    category: Option<String>,
    description: Option<String>,
    template_content: String,
    variables: String,
    recommended_agent: Option<String>,
) -> Result<(), String> {
    db.with_connection(|conn| {
        prompt_template_db::update_template(
            conn,
            &id,
            &name,
            category.as_deref(),
            description.as_deref(),
            &template_content,
            &variables,
            recommended_agent.as_deref(),
        )
    })
    .map_err(|e| format!("更新模板失败: {}", e))
}

#[tauri::command]
pub fn delete_prompt_template(
    db: State<Database>,
    template_id: String,
) -> Result<(), String> {
    db.with_connection(|conn| {
        prompt_template_db::delete_template(conn, &template_id)
    })
    .map_err(|e| format!("删除模板失败: {}", e))
}

#[tauri::command]
pub fn render_prompt_template(
    db: State<Database>,
    template_id: String,
    variables: String,
) -> Result<String, String> {
    let template = db
        .with_connection(|conn| {
            prompt_template_db::get_template_by_id(conn, &template_id)
        })
        .map_err(|e| format!("获取模板失败: {}", e))?;

    let vars: serde_json::Value = serde_json::from_str(&variables)
        .map_err(|e| format!("解析变量失败: {}", e))?;

    Ok(prompt_template_db::render_template(
        &template.template_content,
        &vars,
    ))
}
