//! 应用内更新日志：内容是随程序嵌入的 `crates/app/assets/changelog.md`（[`CHANGELOG_MD`]）；升级后第一次运行时弹出
//! 「更新日志」窗口（[`check_on_startup`]），设置 → 关于「更新日志」随时查看全部（窗口在 `windows::changelog`）。
//!
//! ## 文件格式（[`parse`]）
//!
//! - 每个版本一个二级标题 `## <版本号>`，后面可带日期：`## 3.1.0-pre（2026-11-10）`（全角 / 半角括号都行，也可以只用空格隔开）；
//!   版本号可带前缀 `v`，按 [`compare_versions`]（semver，含预发布）比较
//! - 段落内容是 Markdown（列表、`###` 小标题……），交给 `ui::markdown` 只读渲染；显示时小标题降一级、至少四级并加粗
//!   （[`Section::display_body`]：`###` 按三级标题渲染比版本号还大）
//! - `## 未发布`：已合入、还没发版的改动，应用里**不显示**；发版改版本号时把它改成 `## <版本号>（日期）`
//!   （单测 `current_version_has_a_changelog_section` 兜底：工作区版本在更新日志里没有段落时 CI 失败）
//! - 容错：第一个二级标题之前的内容、标题不是版本号的段落、内容为空的段落、重复的版本（保留先出现的）都忽略并记日志；
//!   围栏代码块里的 `## ` 不算标题；显示顺序按版本号从新到旧，与文件里的先后无关
//!
//! ## 启动时的判定（[`decide`]，纯函数）
//!
//! 本机记一个设备本地的 settings 键（core `settings::LAST_RUN_VERSION_KEY` = `last_run_version`，不同步、不导出）：
//!
//! | 记录 | 处理 |
//! |---|---|
//! | 等于当前版本 | 什么都不做 |
//! | 没有记录、全新的库（迁移前库版本 0，core `Database::schema_version_at_open`） | 只写记录（新装不弹） |
//! | 没有记录、迁移前库版本 `< 31`（3.0.x 及更早） | 弹出所有 `<= 当前版本` 的段落 |
//! | 没有记录、迁移前库版本 `>= 31` | 只有 3.1.0-pre 是 v31 且没有这个键：按上次是 3.1.0-pre，弹出 `(3.1.0-pre, 当前]` |
//! | 比当前旧 | 弹出 `(记录, 当前]` 的段落 |
//! | 比当前新（降级） | 只写记录 |
//!
//! 要弹的区间里没有段落时只写记录、不弹。弹出（窗口打开成功）或判定不弹时把记录更新为当前版本。
//! 弹窗以默认清单窗口为打开方（同托盘「打开设置」：模态、不设 owner）。
//!
//! ## 测试环境
//!
//! e2e / regress / 介绍视频都是 debug 构建 + `MINI_TODO_DATA_DIR`，常常先用 `examples/seed.rs` 灌数据（= 已有数据库、没有记录），
//! 自动判定会弹窗、打乱窗口断言。所以 **debug 构建且使用 `MINI_TODO_DATA_DIR` 时不自动判定、也不写记录**；
//! `MINI_TODO_DEBUG_CHANGELOG=auto` 恢复自动判定（手测用）。release 一律按上表。`MINI_TODO_DEBUG_OPEN=changelog` 直接打开窗口（全部段落）。

use std::cmp::Ordering;
use std::sync::LazyLock;

use gpui_kit::App;
use mini_todo_core::settings;

use crate::db::spawn_db;
use crate::state::APP_VERSION;
use crate::util::version::compare_versions;
use crate::windows::{ChildOf, WindowManager, changelog as changelog_window};

/// 更新日志原文（`crates/app/assets/changelog.md`）
pub const CHANGELOG_MD: &str = include_str!("../../assets/changelog.md");
/// 不显示的段落标题
pub const UNRELEASED_TITLE: &str = "未发布";
/// 有这个库结构版本、却没有记录的库，上次运行的是 [`LEGACY_VERSION`]（3.1.0-pre 引入 v31，还没有更新日志）
pub const LEGACY_SCHEMA_VERSION: i32 = 31;
pub const LEGACY_VERSION: &str = "3.1.0-pre";
/// debug 专用：`auto` 时数据隔离下也自动判定（见模块文档「测试环境」）
pub const DEBUG_CHANGELOG_ENV: &str = "MINI_TODO_DEBUG_CHANGELOG";

/// 一个版本的段落
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// 版本号（去掉前缀 `v`）
    pub version: String,
    /// 标题里版本号后面的日期（原样，只用于显示）
    pub date: Option<String>,
    /// Markdown 内容（去掉首尾空行）
    pub body: String,
}

/// 二级标题的种类
#[derive(Debug, PartialEq, Eq)]
enum Heading {
    Release {
        version: String,
        date: Option<String>,
    },
    Unreleased,
    Unknown,
}

/// 解析更新日志（规则见模块文档「文件格式」），按版本号从新到旧
pub fn parse(source: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    // 当前段落：(标题, 内容行)；None = 还没遇到二级标题
    let mut current: Option<(Heading, Vec<&str>)> = None;
    // 围栏代码块（开始标记的字符与长度）
    let mut fence: Option<(char, usize)> = None;
    for line in source.lines() {
        if let Some(open) = fence {
            if closes_fence(line, open) {
                fence = None;
            }
        } else if let Some(open) = opens_fence(line) {
            fence = Some(open);
        } else if let Some(text) = level2_heading(line) {
            finish_section(current.take(), &mut sections);
            let heading = classify(text);
            if heading == Heading::Unknown {
                log::warn!("[changelog] 无法识别的二级标题「{text}」，忽略这一段");
            }
            current = Some((heading, Vec::new()));
            continue;
        }
        if let Some((_, lines)) = current.as_mut() {
            lines.push(line);
        }
    }
    finish_section(current.take(), &mut sections);
    // 稳定排序：从新到旧
    sections.sort_by(|a, b| compare_versions(&b.version, &a.version));
    sections
}

/// 一段结束：版本段落去掉首尾空行后收下；空段落、重复的版本、`未发布` / 无法识别的标题丢掉
fn finish_section(current: Option<(Heading, Vec<&str>)>, sections: &mut Vec<Section>) {
    let Some((Heading::Release { version, date }, lines)) = current else {
        return;
    };
    let first = lines.iter().position(|l| !l.trim().is_empty());
    let last = lines.iter().rposition(|l| !l.trim().is_empty());
    let body = match (first, last) {
        (Some(first), Some(last)) => lines[first..=last].join("\n"),
        _ => String::new(),
    };
    if body.is_empty() {
        log::warn!("[changelog] 版本 {version} 的段落是空的，忽略");
        return;
    }
    if sections
        .iter()
        .any(|s| compare_versions(&s.version, &version) == Ordering::Equal)
    {
        log::warn!("[changelog] 版本 {version} 的段落重复，只保留先出现的");
        return;
    }
    sections.push(Section {
        version,
        date,
        body,
    });
}

/// 嵌入的更新日志解析结果（只解析一次，格式问题的日志也只记一次）
pub fn sections() -> &'static [Section] {
    static SECTIONS: LazyLock<Vec<Section>> = LazyLock::new(|| parse(CHANGELOG_MD));
    &SECTIONS
}

/// `## 标题`（行首最多 3 个空格、`##` 后要有空白或行尾；`###` 不算）→ 去掉结尾 `#` 的标题文字
fn level2_heading(line: &str) -> Option<&str> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = line[indent..].strip_prefix("##")?;
    if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    Some(strip_closing_hashes(rest.trim()))
}

/// 围栏代码块的开始行：行首最多 3 个空格 + 至少 3 个 `` ` `` 或 `~`
fn opens_fence(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let count = trimmed.chars().take_while(|c| *c == marker).count();
    (count >= 3).then_some((marker, count))
}

/// 围栏代码块的结束行：同一种字符、不少于开始标记的长度、后面只有空白
fn closes_fence(line: &str, (marker, count): (char, usize)) -> bool {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return false;
    }
    let run = trimmed.chars().take_while(|c| *c == marker).count();
    run >= count && trimmed[run * marker.len_utf8()..].trim().is_empty()
}

/// 段落内容中的小标题至少显示成几级：版本号是这一段的标题（`theme::changelog_window::VERSION_SIZE`），内容里的 `###`
/// 照常按三级标题渲染会比它还大，所以显示时一律降到四级及以下
const MIN_BODY_HEADING_LEVEL: usize = 4;

impl Section {
    /// 窗口里显示的 Markdown：内容里的 ATX 标题降一级、至少 [`MIN_BODY_HEADING_LEVEL`] 级并加粗（`### 新功能` →
    /// `#### **新功能**`；已含 `**` 的不再包），围栏代码块里的不动
    pub fn display_body(&self) -> String {
        let mut fence: Option<(char, usize)> = None;
        let mut out: Vec<String> = Vec::new();
        for line in self.body.lines() {
            if let Some(open) = fence {
                if closes_fence(line, open) {
                    fence = None;
                }
                out.push(line.to_string());
            } else if let Some(open) = opens_fence(line) {
                fence = Some(open);
                out.push(line.to_string());
            } else {
                out.push(demote_heading(line));
            }
        }
        out.join("\n")
    }
}

/// ATX 标题行（行首最多 3 个空格、1~6 个 `#`、后面是空白或行尾）降级，见 [`Section::display_body`]；其它行原样
fn demote_heading(line: &str) -> String {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    let level = rest.len() - rest.trim_start_matches('#').len();
    let after = &rest[level..];
    if indent > 3
        || !(1..=6).contains(&level)
        || !(after.is_empty() || after.starts_with([' ', '\t']))
    {
        return line.to_string();
    }
    let demoted = (level + 1).clamp(MIN_BODY_HEADING_LEVEL, 6);
    // 三级及以下的标题在 `ui::markdown` 里是中等字重（Windows 上实际是常规），降级后与正文列表分不开：整行加粗
    let text = strip_closing_hashes(after.trim());
    let text = if text.is_empty() || text.contains("**") {
        text.to_string()
    } else {
        format!("**{text}**")
    };
    let hashes = "#".repeat(demoted);
    if text.is_empty() {
        format!("{}{hashes}", &line[..indent])
    } else {
        format!("{}{hashes} {text}", &line[..indent])
    }
}

/// ATX 标题文字去掉结尾的 `#`（前面要有空白，否则是文字的一部分）
fn strip_closing_hashes(text: &str) -> &str {
    let without_closing = text.trim_end_matches('#');
    if without_closing.len() < text.len()
        && (without_closing.is_empty() || without_closing.ends_with([' ', '\t']))
    {
        without_closing.trim_end()
    } else {
        text
    }
}

/// 标题文字 → 种类：`未发布` / `<版本号>[（日期）]` / 其它
fn classify(text: &str) -> Heading {
    if text.starts_with(UNRELEASED_TITLE) {
        return Heading::Unreleased;
    }
    let end = text
        .find(|c: char| c.is_whitespace() || c == '（' || c == '(')
        .unwrap_or(text.len());
    let (raw_version, rest) = text.split_at(end);
    let version = raw_version.strip_prefix(['v', 'V']).unwrap_or(raw_version);
    let looks_like_version = version.starts_with(|c: char| c.is_ascii_digit())
        && version.contains('.')
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    if !looks_like_version {
        return Heading::Unknown;
    }
    let rest = rest.trim();
    let date = rest
        .strip_prefix('（')
        .and_then(|r| r.strip_suffix('）'))
        .or_else(|| rest.strip_prefix('(').and_then(|r| r.strip_suffix(')')))
        .unwrap_or(rest)
        .trim();
    Heading::Release {
        version: version.to_string(),
        date: (!date.is_empty()).then(|| date.to_string()),
    }
}

/// `(after, upto]` 区间内的段落（`after = None`：`<= upto` 的全部），从新到旧
pub fn sections_between(sections: &[Section], after: Option<&str>, upto: &str) -> Vec<Section> {
    sections
        .iter()
        .filter(|s| compare_versions(&s.version, upto) != Ordering::Greater)
        .filter(|s| {
            after.is_none_or(|after| compare_versions(&s.version, after) == Ordering::Greater)
        })
        .cloned()
        .collect()
}

/// 启动时的判定结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupDecision {
    /// 记录就是当前版本：什么都不做
    UpToDate,
    /// 只把记录更新为当前版本（新装、降级、区间里没有段落）
    RecordOnly,
    /// 弹出这些段落（从新到旧），再把记录更新为当前版本
    Show(Vec<Section>),
}

/// 启动时的判定（规则见模块文档「启动时的判定」）。`last_run`：本机记录（`None` = 没有记录）；
/// `schema_at_open`：本次启动迁移前的库结构版本（0 = 全新的库）；`current`：当前版本
pub fn decide(
    last_run: Option<&str>,
    schema_at_open: i32,
    current: &str,
    sections: &[Section],
) -> StartupDecision {
    let last_run = last_run.map(str::trim).filter(|v| !v.is_empty());
    let after = match last_run {
        Some(last) => match compare_versions(last, current) {
            Ordering::Equal => return StartupDecision::UpToDate,
            Ordering::Greater => return StartupDecision::RecordOnly,
            Ordering::Less => Some(last),
        },
        None if schema_at_open <= 0 => return StartupDecision::RecordOnly,
        None if schema_at_open < LEGACY_SCHEMA_VERSION => None,
        None => Some(LEGACY_VERSION),
    };
    let shown = sections_between(sections, after, current);
    if shown.is_empty() {
        StartupDecision::RecordOnly
    } else {
        StartupDecision::Show(shown)
    }
}

/// 是否在启动时自动判定：debug 构建、数据隔离（`MINI_TODO_DATA_DIR`）中默认不判定，`MINI_TODO_DEBUG_CHANGELOG=auto` 恢复
/// （release 不读环境变量、总是判定，见 [`auto_check_enabled`]）
#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn auto_check_allowed(debug_mode: Option<&str>, isolated: bool) -> bool {
    debug_mode.is_some_and(|mode| mode.trim().eq_ignore_ascii_case("auto")) || !isolated
}

fn auto_check_enabled() -> bool {
    #[cfg(debug_assertions)]
    {
        let mode = std::env::var(DEBUG_CHANGELOG_ENV).ok();
        let isolated = mini_todo_core::db::paths::dev_data_dir_override().is_some();
        let allowed = auto_check_allowed(mode.as_deref(), isolated);
        if !allowed {
            log::info!(
                "[changelog] 开发专用数据隔离中：不自动弹出更新日志、不写记录（{DEBUG_CHANGELOG_ENV}=auto 恢复）"
            );
        }
        allowed
    }
    #[cfg(not(debug_assertions))]
    {
        true
    }
}

/// 启动时（清单窗口都起来、数据加载完之后，`lib.rs::run`）：读本机记录与迁移前的库版本 → [`decide`] → 需要时以默认清单窗口为
/// 打开方弹出更新日志 → 把记录更新为当前版本。数据库读写都在后台
pub fn check_on_startup(cx: &mut App) {
    if !auto_check_enabled() {
        return;
    }
    let facts = spawn_db(cx, |db| {
        Ok((settings::last_run_version(db)?, db.schema_version_at_open()))
    });
    cx.spawn(async move |cx| {
        let (last_run, schema_at_open) = match facts.await {
            Ok(facts) => facts,
            Err(e) => {
                log::warn!("[changelog] 读取上次运行的版本失败，这次不弹更新日志: {e}");
                return;
            }
        };
        let decision = decide(
            last_run.as_deref(),
            schema_at_open,
            APP_VERSION,
            sections(),
        );
        let record = match decision {
            StartupDecision::UpToDate => return,
            StartupDecision::RecordOnly => {
                log::info!(
                    "[changelog] 上次运行 {}（迁移前库版本 v{schema_at_open}），当前 {APP_VERSION}：不弹更新日志，只更新记录",
                    last_run.as_deref().unwrap_or("未记录")
                );
                true
            }
            StartupDecision::Show(shown) => {
                let versions: Vec<&str> = shown.iter().map(|s| s.version.as_str()).collect();
                log::info!(
                    "[changelog] 上次运行 {}（迁移前库版本 v{schema_at_open}），当前 {APP_VERSION}：弹出更新日志 {versions:?}",
                    last_run.as_deref().unwrap_or("未记录")
                );
                cx.update(|cx| {
                    let parent = WindowManager::default_list(cx)
                        .map(|main| ChildOf::handle(main, cx).unowned());
                    let opened = changelog_window::open(cx, parent, shown, true).is_some();
                    if !opened {
                        log::warn!("[changelog] 更新日志窗口没有打开（下次启动再试）");
                    }
                    opened
                })
            }
        };
        if !record {
            return;
        }
        let write = cx.update(|cx| {
            spawn_db(cx, |db| settings::set_last_run_version(db, APP_VERSION))
        });
        if let Err(e) = write.await {
            log::warn!("[changelog] 记录本次运行的版本失败: {e}");
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versions(sections: &[Section]) -> Vec<&str> {
        sections.iter().map(|s| s.version.as_str()).collect()
    }

    fn section(version: &str) -> Section {
        Section {
            version: version.into(),
            date: None,
            body: format!("- {version} 的要点"),
        }
    }

    #[test]
    fn current_version_has_a_changelog_section() {
        // 发版改了工作区版本号却忘了写更新日志（或忘了把「未发布」改成版本号）时这里失败
        let current = sections()
            .iter()
            .find(|s| compare_versions(&s.version, APP_VERSION) == Ordering::Equal);
        let current = current.unwrap_or_else(|| {
            panic!(
                "crates/app/assets/changelog.md 里没有当前版本 {APP_VERSION} 的段落（发版时把「## 未发布」改成「## {APP_VERSION}（日期）」，见 .trellis/spec/pc-gpui/release.md）"
            )
        });
        assert!(!current.body.trim().is_empty());
        // 嵌入的文件不含 emoji（项目规范）：只检查常见的 emoji 区段
        assert!(
            !CHANGELOG_MD
                .chars()
                .any(|c| matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF)),
            "changelog.md 里不要用 emoji"
        );
    }

    #[test]
    fn parses_versions_dates_and_skips_unreleased() {
        let md = "# 更新日志\n\n说明文字\n\n## 未发布\n\n- 还没发版\n\n## 3.1.0（2026-12-01）\n\n### 新功能\n\n- A\n- B\n\n## v3.1.0-pre (2026-11-10)\n- C\n\n## 3.0.6 2026-11-08\n- D\n## 3.0.5\n- E\n";
        let sections = parse(md);
        assert_eq!(
            versions(&sections),
            vec!["3.1.0", "3.1.0-pre", "3.0.6", "3.0.5"]
        );
        assert_eq!(sections[0].date.as_deref(), Some("2026-12-01"));
        assert_eq!(sections[0].body, "### 新功能\n\n- A\n- B");
        assert_eq!(sections[1].date.as_deref(), Some("2026-11-10"));
        assert_eq!(sections[1].body, "- C");
        assert_eq!(sections[2].date.as_deref(), Some("2026-11-08"));
        assert_eq!(sections[3].date, None);
        assert!(sections.iter().all(|s| !s.body.contains("还没发版")));
    }

    #[test]
    fn body_headings_render_below_the_version_title() {
        let mut s = section("3.2.0");
        s.body = "# 一级\n### 新功能\n#### 细节 ##\n###### 最小\n### 已经**加粗**\n####### 不是标题\n#没有空格\n    ### 缩进 4 格\n- ### 列表里\n```\n### 代码块里\n```\n###".into();
        assert_eq!(
            s.display_body(),
            "#### **一级**\n#### **新功能**\n##### **细节**\n###### **最小**\n#### 已经**加粗**\n####### 不是标题\n#没有空格\n    ### 缩进 4 格\n- ### 列表里\n```\n### 代码块里\n```\n####"
        );
    }

    #[test]
    fn parser_tolerates_malformed_input() {
        // 没有二级标题 / 空文件
        assert!(parse("").is_empty());
        assert!(parse("# 标题\n\n- 没有版本段落\n").is_empty());
        let md = "\
## 不是版本号
- 被忽略
## 3.0
## 3.2.0（）

## 3.1.0
- 正文
```
## 3.9.9 代码块里的不算标题
```
###3.5.0 不是二级标题
##3.3.0
- 没有空格的不是标题，仍是 3.1.0 的内容
## 3.1.0
- 重复的版本，忽略
   ## 3.0.1 ##
- 缩进 3 格内、带结尾 # 也认
    ## 3.0.0
- 缩进 4 格是代码，不是标题
## 未发布的东西
- 以「未发布」开头的都不显示
## 2.0.0
- 乱序也按版本排
";
        let sections = parse(md);
        assert_eq!(versions(&sections), vec!["3.1.0", "3.0.1", "2.0.0"]);
        assert!(
            sections[0]
                .body
                .contains("```\n## 3.9.9 代码块里的不算标题\n```")
        );
        assert!(sections[0].body.contains("##3.3.0"));
        assert!(!sections[0].body.contains("重复的版本"));
        assert!(sections[1].body.contains("## 3.0.0"));
        assert!(sections.iter().all(|s| !s.body.contains("被忽略")));
    }

    #[test]
    fn between_is_half_open_and_newest_first() {
        let all = vec![
            section("3.2.0"),
            section("3.1.1"),
            section("3.1.0"),
            section("3.1.0-pre"),
            section("3.0.6"),
        ];
        assert_eq!(
            versions(&sections_between(&all, Some("3.1.0-pre"), "3.1.1")),
            vec!["3.1.1", "3.1.0"]
        );
        assert_eq!(
            versions(&sections_between(&all, None, "3.1.0")),
            vec!["3.1.0", "3.1.0-pre", "3.0.6"]
        );
        assert!(sections_between(&all, Some("3.1.1"), "3.1.1").is_empty());
        // 预发布：3.1.0-pre < 3.1.0
        assert_eq!(
            versions(&sections_between(&all, Some("3.0.6"), "3.1.0-pre")),
            vec!["3.1.0-pre"]
        );
    }

    #[test]
    fn startup_decision_follows_the_rules() {
        use StartupDecision::*;
        let all = vec![
            section("3.2.0"),
            section("3.1.1"),
            section("3.1.0"),
            section("3.1.0-pre"),
        ];
        let shown = |d: StartupDecision| match d {
            Show(s) => versions(&s)
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>(),
            other => panic!("应该弹出，实际 {other:?}"),
        };

        // 记录 == 当前
        assert_eq!(decide(Some("3.1.1"), 31, "3.1.1", &all), UpToDate);
        assert_eq!(decide(Some("v3.1.1"), 31, "3.1.1", &all), UpToDate);
        // 没有记录：新装只写记录
        assert_eq!(decide(None, 0, "3.1.1", &all), RecordOnly);
        assert_eq!(
            decide(Some("  "), 0, "3.1.1", &all),
            RecordOnly,
            "空记录按没有记录"
        );
        // 没有记录、旧库 < v31：<= 当前的全部
        assert_eq!(
            shown(decide(None, 30, "3.1.1", &all)),
            vec!["3.1.1", "3.1.0", "3.1.0-pre"]
        );
        assert_eq!(
            shown(decide(None, 27, "3.1.0-pre", &all)),
            vec!["3.1.0-pre"]
        );
        // 没有记录、v31 库：按上次是 3.1.0-pre
        assert_eq!(
            shown(decide(None, 31, "3.1.1", &all)),
            vec!["3.1.1", "3.1.0"]
        );
        assert_eq!(
            shown(decide(None, 32, "3.1.1", &all)),
            vec!["3.1.1", "3.1.0"]
        );
        assert_eq!(
            decide(None, 31, "3.1.0-pre", &all),
            RecordOnly,
            "当前就是 3.1.0-pre：区间为空"
        );
        // 记录 < 当前：(记录, 当前]
        assert_eq!(
            shown(decide(Some("3.1.0"), 31, "3.2.0", &all)),
            vec!["3.2.0", "3.1.1"]
        );
        assert_eq!(
            shown(decide(Some("3.1.0-pre"), 31, "3.1.0", &all)),
            vec!["3.1.0"]
        );
        // 区间里没有段落：只写记录
        assert_eq!(decide(Some("3.2.0"), 31, "3.2.1", &all), RecordOnly);
        // 降级
        assert_eq!(decide(Some("3.2.0"), 31, "3.1.1", &all), RecordOnly);
    }

    #[test]
    fn auto_check_is_off_only_for_isolated_debug_runs_without_override() {
        assert!(auto_check_allowed(None, false));
        assert!(!auto_check_allowed(None, true));
        assert!(!auto_check_allowed(Some(""), true));
        assert!(auto_check_allowed(Some("auto"), true));
        assert!(auto_check_allowed(Some(" AUTO "), true));
    }

    #[test]
    fn embedded_changelog_is_well_formed() {
        let sections = sections();
        assert!(!sections.is_empty());
        // 嵌入的文件里每个版本都是合法版本号、从新到旧、没有「未发布」
        for pair in sections.windows(2) {
            assert_eq!(
                compare_versions(&pair[0].version, &pair[1].version),
                Ordering::Greater,
                "{} / {}",
                pair[0].version,
                pair[1].version
            );
        }
        assert!(
            sections
                .iter()
                .all(|s| !s.version.contains(UNRELEASED_TITLE))
        );
        // 文件里的版本顺序与解析出的一致（从新到旧排列，见文件头的写法说明）
        let in_file: Vec<String> = CHANGELOG_MD
            .lines()
            .filter_map(level2_heading)
            .filter_map(|h| match classify(h) {
                Heading::Release { version, .. } => Some(version),
                _ => None,
            })
            .collect();
        assert_eq!(in_file, versions(sections));
    }
}
