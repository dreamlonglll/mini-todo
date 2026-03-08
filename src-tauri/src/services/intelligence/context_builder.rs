use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectContext {
    pub name: String,
    pub path: String,
    pub tech_stack: Vec<TechInfo>,
    pub directory_tree: String,
    pub documentation: Vec<DocEntry>,
    pub recent_commits: Vec<CommitInfo>,
    pub active_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TechInfo {
    pub category: String,
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocEntry {
    pub filename: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    pub hash: String,
    pub message: String,
    pub date: String,
}

const EXCLUDE_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "__pycache__",
    ".next",
    ".nuxt",
    "vendor",
    ".idea",
    ".vscode",
];

const DOC_FILES: &[&str] = &[
    "CLAUDE.md",
    "AGENTS.md",
    "README.md",
    "CONTRIBUTING.md",
    "docs/README.md",
];

const MAX_DOC_LINES: usize = 200;
const MAX_TREE_DEPTH: usize = 3;
const MAX_TREE_ENTRIES: usize = 200;
const MAX_COMMITS: usize = 10;
const MAX_CONTEXT_CHARS: usize = 4000;

pub async fn build_context(project_path: &str) -> Result<ProjectContext, String> {
    let path = Path::new(project_path);
    if !path.exists() {
        return Err(format!("项目路径不存在: {}", project_path));
    }

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let pp = project_path.to_string();
    let (tech_stack, directory_tree, documentation, git_info) = tokio::join!(
        tokio::task::spawn_blocking({
            let pp = pp.clone();
            move || detect_tech_stack(Path::new(&pp))
        }),
        tokio::task::spawn_blocking({
            let pp = pp.clone();
            move || generate_directory_tree(Path::new(&pp), MAX_TREE_DEPTH, MAX_TREE_ENTRIES)
        }),
        tokio::task::spawn_blocking({
            let pp = pp.clone();
            move || extract_documentation(Path::new(&pp))
        }),
        tokio::task::spawn_blocking({
            let pp = pp.clone();
            move || get_git_info(Path::new(&pp))
        }),
    );

    let tech_stack = tech_stack.unwrap_or_default();
    let directory_tree = directory_tree.unwrap_or_default();
    let documentation = documentation.unwrap_or_default();
    let (active_branch, recent_commits) = git_info
        .unwrap_or_else(|_| (String::new(), Vec::new()));

    Ok(ProjectContext {
        name,
        path: project_path.to_string(),
        tech_stack,
        directory_tree,
        documentation,
        recent_commits,
        active_branch,
    })
}

pub fn render_context_text(ctx: &ProjectContext) -> String {
    let mut parts = Vec::new();

    // 1. 项目信息
    parts.push(format!(
        "【项目信息】\n名称: {}\n路径: {}\n分支: {}",
        ctx.name, ctx.path, ctx.active_branch
    ));

    // 2. 技术栈
    if !ctx.tech_stack.is_empty() {
        let mut tech_lines = vec!["【技术栈】".to_string()];
        for t in &ctx.tech_stack {
            if let Some(v) = &t.version {
                tech_lines.push(format!("- {}: {} ({})", t.category, t.name, v));
            } else {
                tech_lines.push(format!("- {}: {}", t.category, t.name));
            }
        }
        parts.push(tech_lines.join("\n"));
    }

    // 3. 目录结构
    if !ctx.directory_tree.is_empty() {
        parts.push(format!("【目录结构】\n{}", ctx.directory_tree));
    }

    // 4. 项目文档
    for doc in &ctx.documentation {
        parts.push(format!("【项目文档: {}】\n{}", doc.filename, doc.content));
    }

    // 5. 最近提交
    if !ctx.recent_commits.is_empty() {
        let mut commit_lines = vec!["【最近提交】".to_string()];
        for c in &ctx.recent_commits {
            commit_lines.push(format!("{} {}", c.hash, c.message));
        }
        parts.push(commit_lines.join("\n"));
    }

    let mut result = parts.join("\n\n");

    // 大小控制：超过限制则逐步缩减
    if result.len() > MAX_CONTEXT_CHARS {
        // 移除 Git 提交
        parts.retain(|p| !p.starts_with("【最近提交】"));
        result = parts.join("\n\n");
    }
    if result.len() > MAX_CONTEXT_CHARS {
        // 截断文档
        parts = parts
            .into_iter()
            .map(|p| {
                if p.starts_with("【项目文档") && p.len() > 500 {
                    let truncated: String = p.chars().take(500).collect();
                    format!("{}... (truncated)", truncated)
                } else {
                    p
                }
            })
            .collect();
        result = parts.join("\n\n");
    }
    if result.len() > MAX_CONTEXT_CHARS {
        result = result.chars().take(MAX_CONTEXT_CHARS).collect();
        result.push_str("\n... (context truncated)");
    }

    result
}

// ========== 内部实现 ==========

fn detect_tech_stack(project_path: &Path) -> Vec<TechInfo> {
    let mut stack = Vec::new();

    // package.json
    let pkg_path = project_path.join("package.json");
    if let Ok(content) = std::fs::read_to_string(&pkg_path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            let deps = json["dependencies"].as_object();
            let dev_deps = json["devDependencies"].as_object();

            let all_deps: Vec<&str> = deps
                .into_iter()
                .chain(dev_deps.into_iter())
                .flat_map(|m| m.keys().map(|k| k.as_str()))
                .collect();

            if all_deps.iter().any(|d| *d == "vue") {
                let ver = deps
                    .and_then(|m| m.get("vue"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim_start_matches('^').trim_start_matches('~').to_string());
                stack.push(TechInfo {
                    category: "frontend".into(),
                    name: "Vue".into(),
                    version: ver,
                });
            }
            if all_deps.iter().any(|d| *d == "react") {
                let ver = deps
                    .and_then(|m| m.get("react"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim_start_matches('^').trim_start_matches('~').to_string());
                stack.push(TechInfo {
                    category: "frontend".into(),
                    name: "React".into(),
                    version: ver,
                });
            }
            if all_deps.iter().any(|d| *d == "next") {
                stack.push(TechInfo {
                    category: "frontend".into(),
                    name: "Next.js".into(),
                    version: None,
                });
            }
            if all_deps.iter().any(|d| *d == "express") {
                stack.push(TechInfo {
                    category: "backend".into(),
                    name: "Express".into(),
                    version: None,
                });
            }
            if all_deps.iter().any(|d| *d == "typescript" || *d == "@types/node") {
                stack.push(TechInfo {
                    category: "tool".into(),
                    name: "TypeScript".into(),
                    version: None,
                });
            }
            if all_deps.iter().any(|d| *d == "element-plus") {
                stack.push(TechInfo {
                    category: "frontend".into(),
                    name: "Element Plus".into(),
                    version: None,
                });
            }
            if all_deps.iter().any(|d| *d == "@tauri-apps/api" || *d == "@tauri-apps/cli") {
                stack.push(TechInfo {
                    category: "framework".into(),
                    name: "Tauri".into(),
                    version: None,
                });
            }
        }
    }

    // Cargo.toml
    let cargo_path = project_path.join("Cargo.toml");
    if !cargo_path.exists() {
        let sub_cargo = project_path.join("src-tauri").join("Cargo.toml");
        if sub_cargo.exists() {
            parse_cargo_toml(&sub_cargo, &mut stack);
        }
    } else {
        parse_cargo_toml(&cargo_path, &mut stack);
    }

    // tsconfig.json
    if project_path.join("tsconfig.json").exists()
        && !stack.iter().any(|t| t.name == "TypeScript")
    {
        stack.push(TechInfo {
            category: "tool".into(),
            name: "TypeScript".into(),
            version: None,
        });
    }

    // go.mod
    if project_path.join("go.mod").exists() {
        stack.push(TechInfo {
            category: "backend".into(),
            name: "Go".into(),
            version: None,
        });
    }

    // requirements.txt / pyproject.toml
    if project_path.join("requirements.txt").exists()
        || project_path.join("pyproject.toml").exists()
    {
        stack.push(TechInfo {
            category: "backend".into(),
            name: "Python".into(),
            version: None,
        });
    }

    // *.csproj
    if let Ok(entries) = std::fs::read_dir(project_path) {
        for entry in entries.flatten() {
            if entry
                .path()
                .extension()
                .map_or(false, |ext| ext == "csproj" || ext == "sln")
            {
                stack.push(TechInfo {
                    category: "backend".into(),
                    name: ".NET".into(),
                    version: None,
                });
                break;
            }
        }
    }

    stack
}

fn parse_cargo_toml(path: &Path, stack: &mut Vec<TechInfo>) {
    if let Ok(content) = std::fs::read_to_string(path) {
        if !stack.iter().any(|t| t.name == "Rust") {
            stack.push(TechInfo {
                category: "backend".into(),
                name: "Rust".into(),
                version: None,
            });
        }
        if content.contains("rusqlite") || content.contains("sqlite") {
            stack.push(TechInfo {
                category: "database".into(),
                name: "SQLite".into(),
                version: None,
            });
        }
        if content.contains("tauri") && !stack.iter().any(|t| t.name == "Tauri") {
            stack.push(TechInfo {
                category: "framework".into(),
                name: "Tauri".into(),
                version: None,
            });
        }
    }
}

fn generate_directory_tree(project_path: &Path, max_depth: usize, max_entries: usize) -> String {
    let mut lines = Vec::new();
    let mut count = 0;
    build_tree(project_path, "", max_depth, 0, &mut lines, &mut count, max_entries);
    if count >= max_entries {
        lines.push(format!("... ({} more items)", count - max_entries));
    }
    lines.join("\n")
}

fn build_tree(
    dir: &Path,
    prefix: &str,
    max_depth: usize,
    depth: usize,
    lines: &mut Vec<String>,
    count: &mut usize,
    max_entries: usize,
) {
    if depth >= max_depth || *count >= max_entries {
        return;
    }

    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };
    entries.sort_by(|a, b| {
        let a_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let b_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
        b_dir.cmp(&a_dir).then_with(|| a.file_name().cmp(&b.file_name()))
    });

    let total = entries.len();
    for (i, entry) in entries.iter().enumerate() {
        if *count >= max_entries {
            break;
        }
        let name = entry.file_name().to_string_lossy().to_string();

        if EXCLUDE_DIRS.contains(&name.as_str()) {
            continue;
        }
        if name.starts_with('.') && name != ".env.example" {
            continue;
        }

        let is_last = i == total - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

        let display = if is_dir {
            format!("{}/", name)
        } else {
            name.clone()
        };

        lines.push(format!("{}{}{}", prefix, connector, display));
        *count += 1;

        if is_dir {
            let child_prefix = if is_last {
                format!("{}    ", prefix)
            } else {
                format!("{}│   ", prefix)
            };
            build_tree(
                &entry.path(),
                &child_prefix,
                max_depth,
                depth + 1,
                lines,
                count,
                max_entries,
            );
        }
    }
}

fn extract_documentation(project_path: &Path) -> Vec<DocEntry> {
    let mut docs = Vec::new();

    for filename in DOC_FILES {
        let file_path = project_path.join(filename);
        if let Ok(content) = std::fs::read_to_string(&file_path) {
            let lines: Vec<&str> = content.lines().collect();
            let truncated = if lines.len() > MAX_DOC_LINES {
                format!(
                    "{}\n... (truncated, total {} lines)",
                    lines[..MAX_DOC_LINES].join("\n"),
                    lines.len()
                )
            } else {
                content
            };

            docs.push(DocEntry {
                filename: filename.to_string(),
                content: truncated,
            });
        }
    }

    docs
}

fn get_git_info(project_path: &Path) -> (String, Vec<CommitInfo>) {
    let branch = std::process::Command::new("git")
        .current_dir(project_path)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();

    let commits = std::process::Command::new("git")
        .current_dir(project_path)
        .args([
            "log",
            &format!("-{}", MAX_COMMITS),
            "--format=%h %s",
        ])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.is_empty())
                .map(|line| {
                    let (hash, message) = line.split_once(' ').unwrap_or((line, ""));
                    CommitInfo {
                        hash: hash.to_string(),
                        message: message.to_string(),
                        date: String::new(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    (branch, commits)
}
