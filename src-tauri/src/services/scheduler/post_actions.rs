use crate::db::{Database, scheduler_db};
use tauri::Manager;

pub enum PostAction {
    None,
    GitCommit,
    Review,
    GitCommitAndReview,
}

impl PostAction {
    pub fn from_str(s: &str) -> Self {
        match s {
            "git_commit" => PostAction::GitCommit,
            "review" => PostAction::Review,
            "git_commit_and_review" => PostAction::GitCommitAndReview,
            _ => PostAction::None,
        }
    }
}

/// 执行后续动作。
/// 返回 true 表示应立即触发下游任务，false 表示需等待人工审核。
pub async fn execute_post_action(
    app: &tauri::AppHandle,
    subtask_id: i64,
    project_path: &str,
    subtask_title: &str,
    action: &PostAction,
) -> Result<bool, String> {
    match action {
        PostAction::None => {
            mark_completed(app, subtask_id);
            Ok(true)
        }
        PostAction::GitCommit => {
            if let Err(e) = do_git_commit(project_path, subtask_title).await {
                eprintln!("[post_action] Git commit failed for subtask {}: {}", subtask_id, e);
            }
            mark_completed(app, subtask_id);
            Ok(true)
        }
        PostAction::Review => {
            mark_reviewing(app, subtask_id);
            Ok(false)
        }
        PostAction::GitCommitAndReview => {
            if let Err(e) = do_git_commit(project_path, subtask_title).await {
                eprintln!("[post_action] Git commit failed for subtask {}: {}", subtask_id, e);
            }
            mark_reviewing(app, subtask_id);
            Ok(false)
        }
    }
}

fn mark_completed(app: &tauri::AppHandle, subtask_id: i64) {
    let db = app.state::<Database>();
    let _ = db.with_connection(|conn| {
        scheduler_db::update_schedule_status(conn, subtask_id, "completed")
    });
}

fn mark_reviewing(app: &tauri::AppHandle, subtask_id: i64) {
    let db = app.state::<Database>();
    let _ = db.with_connection(|conn| {
        scheduler_db::update_schedule_status(conn, subtask_id, "reviewing")
    });
}

async fn do_git_commit(project_path: &str, subtask_title: &str) -> Result<(), String> {
    let path = project_path.to_string();
    let title = subtask_title.to_string();

    tokio::task::spawn_blocking(move || {
        let status_output = std::process::Command::new("git")
            .args(["-C", &path, "status", "--porcelain"])
            .output()
            .map_err(|e| format!("git status failed: {}", e))?;

        let status_text = String::from_utf8_lossy(&status_output.stdout);
        if status_text.trim().is_empty() {
            return Ok(());
        }

        let add_output = std::process::Command::new("git")
            .args(["-C", &path, "add", "-A"])
            .output()
            .map_err(|e| format!("git add failed: {}", e))?;

        if !add_output.status.success() {
            let stderr = String::from_utf8_lossy(&add_output.stderr);
            return Err(format!("git add failed: {}", stderr));
        }

        let commit_msg = format!("auto: {}", title);
        let commit_output = std::process::Command::new("git")
            .args(["-C", &path, "commit", "-m", &commit_msg])
            .output()
            .map_err(|e| format!("git commit failed: {}", e))?;

        if !commit_output.status.success() {
            let stderr = String::from_utf8_lossy(&commit_output.stderr);
            return Err(format!("git commit failed: {}", stderr));
        }

        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking error: {}", e))?
}
