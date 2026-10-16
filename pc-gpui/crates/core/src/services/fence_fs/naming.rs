//! 纯函数：收纳文件夹命名、文件名校验、重名后缀、显示名。全平台可用，不访问文件系统。
//!
//! 名字比较一律不区分大小写（NTFS 默认不区分），"是否已占用"由调用方通过闭包告诉这里。

/// 标题清洗后为空时使用的默认文件夹名
pub const DEFAULT_FOLDER_NAME: &str = "收纳箱";

/// 清洗后文件夹名的最大字符数（按 char 计）。
///
/// 收纳路径形如 `C:\Users\<用户>\MiniTodo收纳箱\<文件夹名>\<文件名>`，给文件名留足空间，
/// 避免整体超过 MAX_PATH（260）后部分老程序 / Shell 操作失败。去重后缀 ` (N)` 不计入此上限。
pub const MAX_FOLDER_NAME_CHARS: usize = 48;

/// 单个路径段在 NTFS 上的上限是 255 个 UTF-16 单元
const MAX_SEGMENT_UTF16: usize = 255;

/// Windows 文件名中不允许出现的可见字符
const ILLEGAL_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Windows 保留设备名（不区分大小写；`CON.txt` 这类"保留名 + 扩展名"同样不可用）
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM0", "COM1", "COM2", "COM3", "COM4",
    "COM5", "COM6", "COM7", "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT0", "LPT1", "LPT2", "LPT3",
    "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
];

/// 系统自动生成的元数据文件（小写）：不收纳、不显示、删除收纳箱时不算"剩余文件"（文件夹只剩它们时连同它们一起删掉）。
///
/// * Windows 资源管理器：`desktop.ini`、`Thumbs.db`
/// * macOS 访达：`.DS_Store`（打开过的文件夹都会写）、`Icon\r`（文件夹自定义图标，名字末尾是回车）、
///   `.localized`（文件夹名本地化标记）
///
/// 所有平台共用同一张表：Mac 拷来的 U 盘 / 网络共享上 Windows 也常见 `.DS_Store`，在 macOS 上也会遇到 `desktop.ini`
const SHELL_METADATA_FILES: &[&str] = &[
    "desktop.ini",
    "thumbs.db",
    ".ds_store",
    "icon\r",
    ".localized",
];

/// macOS 的"包"（bundle）扩展名（小写）：这类**文件夹**在访达里显示、打开、移动都像一个文件（`Safari.app`）。
/// 去重加后缀时按文件处理，后缀插在扩展名前（`Safari (2).app`），否则 `Safari.app (2)` 在访达里不再是应用。
///
/// 所有平台都按这张表处理（纯函数，不看平台）：Windows 上这类文件夹几乎只来自 Mac（U 盘 / 网络共享），
/// 保持扩展名在最后对 Mac 才有意义，对 Windows 无影响；同时移回时恢复原名的判断（[`restore_file_name`]）在各平台一致
const PACKAGE_EXTENSIONS: &[&str] = &[
    "app",
    "appex",
    "band",
    "bundle",
    "component",
    "docset",
    "fcpbundle",
    "framework",
    "imovielibrary",
    "kext",
    "key",
    "logicx",
    "mdimporter",
    "mpkg",
    "numbers",
    "pages",
    "photoslibrary",
    "pkg",
    "playground",
    "plugin",
    "prefpane",
    "qlgenerator",
    "rtfd",
    "saver",
    "scptd",
    "wdgt",
    "xcodeproj",
    "xcworkspace",
    "xpc",
];

fn is_illegal_char(c: char) -> bool {
    c.is_control() || ILLEGAL_CHARS.contains(&c)
}

/// 名字的"主干"（第一个 `.` 之前、去掉尾部空格）是否是保留设备名
fn is_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").trim_end_matches(' ');
    RESERVED_NAMES
        .iter()
        .any(|r| r.to_lowercase() == stem.to_lowercase())
}

/// 去掉 Windows 会自动丢弃的尾部 `.` 与空格
fn trim_trailing_dots_spaces(s: &str) -> &str {
    s.trim_end_matches(['.', ' '])
}

/// 收纳箱标题 → 文件夹名。
///
/// 规则（顺序执行）：
/// 1. 去掉首尾空白（含制表符、换行）
/// 2. 控制字符与 `<>:"/\|?*` 替换为 `_`
/// 3. 再去掉首尾空白与尾部的 `.` 和空格（Windows 会静默丢弃它们，导致名字对不上）
/// 4. 超过 [`MAX_FOLDER_NAME_CHARS`] 个字符时截断，截断后再做一次第 3 步
/// 5. 结果为空时用 [`DEFAULT_FOLDER_NAME`]
/// 6. 主干是保留设备名（`CON`、`com1.txt` 等）时在主干后插入 `_`（`CON` → `CON_`）
///
/// 不负责去重，去重见 [`dedupe_name`]。
pub fn sanitize_folder_name(title: &str) -> String {
    let replaced: String = title
        .trim()
        .chars()
        .map(|c| if is_illegal_char(c) { '_' } else { c })
        .collect();
    let mut name = trim_trailing_dots_spaces(replaced.trim()).to_string();
    if name.chars().count() > MAX_FOLDER_NAME_CHARS {
        let truncated: String = name.chars().take(MAX_FOLDER_NAME_CHARS).collect();
        name = trim_trailing_dots_spaces(truncated.trim()).to_string();
    }
    if name.is_empty() {
        return DEFAULT_FOLDER_NAME.to_string();
    }
    if is_reserved_name(&name) {
        let stem_len = name.find('.').unwrap_or(name.len());
        name.insert(stem_len, '_');
    }
    name
}

/// 是否是可以直接拼到收纳根目录下的文件夹名（来自数据库 / 前端的名字先过这一关）。
///
/// 要求：非空、不是 `.` / `..`、不含路径分隔符与非法字符、首尾无空白、尾部无 `.`、
/// 不是保留设备名、长度不超过 `MAX_FOLDER_NAME_CHARS + 16` 个字符（给去重后缀留余量）。
pub fn is_valid_folder_name(name: &str) -> bool {
    if !is_safe_segment(name) {
        return false;
    }
    if name.trim() != name || trim_trailing_dots_spaces(name) != name {
        return false;
    }
    if name.chars().count() > MAX_FOLDER_NAME_CHARS + 16 {
        return false;
    }
    !is_reserved_name(name)
}

/// 是否是收纳文件夹内的合法项目名（单个路径段，防止 `..`、绝对路径、ADS `a:b` 逃逸）。
///
/// 比文件夹名宽松：允许首尾空格 / 尾部点等 Windows 罕见但可能真实存在的名字，
/// 只拦截会改变路径含义的写法。
pub fn is_valid_item_name(name: &str) -> bool {
    is_safe_segment(name)
}

fn is_safe_segment(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.chars().any(is_illegal_char)
        && name.encode_utf16().count() <= MAX_SEGMENT_UTF16
}

/// 去重：`base` 未被占用就用 `base`，否则依次尝试 `base (2)`、`base (3)`……
///
/// 用于文件夹名（不拆扩展名）。`is_taken` 应按不区分大小写的语义实现。
pub fn dedupe_name(base: &str, is_taken: impl Fn(&str) -> bool) -> String {
    if !is_taken(base) {
        return base.to_string();
    }
    (2u32..)
        .map(|n| format!("{base} ({n})"))
        .find(|candidate| !is_taken(candidate))
        .expect("u32 范围内总能找到未占用的名字")
}

/// 名字是否带 macOS 包扩展名（`Safari.app`、`报告.rtfd`；不区分大小写），见 [`PACKAGE_EXTENSIONS`]
pub fn is_package_name(name: &str) -> bool {
    match split_file_extension(name) {
        (_, "") => false,
        (_, ext) => PACKAGE_EXTENSIONS.contains(&ext[1..].to_lowercase().as_str()),
    }
}

/// 按文件规则拆：最后一个 `.` 之后是扩展名；以点开头且没有第二个点的名字（`.env`）、以点结尾的名字视为无扩展名
fn split_file_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(idx) if idx > 0 && idx + 1 < name.len() => (&name[..idx], &name[idx..]),
        _ => (name, ""),
    }
}

/// 把文件名拆成（主干, 扩展名含点）。文件夹不拆（macOS 的包除外，见 [`is_package_name`]）；
/// 以点开头且没有第二个点的名字（`.env`）视为无扩展名。
pub fn split_extension(name: &str, is_dir: bool) -> (&str, &str) {
    if is_dir && !is_package_name(name) {
        return (name, "");
    }
    split_file_extension(name)
}

/// 带序号后缀的文件名：`a.lnk` + 2 → `a (2).lnk`；文件夹 `docs` + 2 → `docs (2)`；包 `Safari.app` + 2 → `Safari (2).app`
pub fn suffixed_file_name(name: &str, n: u32, is_dir: bool) -> String {
    let (stem, ext) = split_extension(name, is_dir);
    format!("{stem} ({n}){ext}")
}

/// 在目标目录里给 `name` 找一个不冲突的文件名（重名时加 ` (N)` 后缀，绝不覆盖）。
/// `is_taken` 应按不区分大小写的语义实现。
pub fn unique_file_name(name: &str, is_dir: bool, is_taken: impl Fn(&str) -> bool) -> String {
    if !is_taken(name) {
        return name.to_string();
    }
    (2u32..)
        .map(|n| suffixed_file_name(name, n, is_dir))
        .find(|candidate| !is_taken(candidate))
        .expect("u32 范围内总能找到未占用的名字")
}

/// `current` 是否是 `original` 加序号后缀后的名字（如 `Chrome (2).lnk` 之于 `Chrome.lnk`）。
/// 包文件夹两种写法都认：`Safari (2).app`（现在的写法）与 `Safari.app (2)`（以前的版本把包当普通文件夹）
fn is_suffixed_variant(current: &str, original: &str, is_dir: bool) -> bool {
    let (stem, ext) = split_extension(original, is_dir);
    suffixed_with(current, stem, ext)
        || (is_dir && !ext.is_empty() && suffixed_with(current, original, ""))
}

/// `current` 是否形如 `{stem} (N){ext}`（N ≥ 2，不区分大小写）
fn suffixed_with(current: &str, stem: &str, ext: &str) -> bool {
    let current_l = current.to_lowercase();
    let prefix = format!("{} (", stem.to_lowercase());
    let suffix = format!("){}", ext.to_lowercase());
    if current_l.len() <= prefix.len() + suffix.len() {
        return false;
    }
    let Some(rest) = current_l.strip_prefix(&prefix) else {
        return false;
    };
    let Some(digits) = rest.strip_suffix(&suffix) else {
        return false;
    };
    !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
        && digits.parse::<u32>().is_ok_and(|n| n >= 2)
}

/// 移回时希望使用的文件名。
///
/// 收纳时如果因重名被改成 `Chrome (2).lnk`，移回原目录时应恢复成原名 `Chrome.lnk`；
/// 但如果用户在资源管理器里把它改成了别的名字，就保留用户的名字。
/// 最终是否还要加后缀由 [`unique_file_name`] 按目标目录决定。
pub fn restore_file_name(current: &str, original_name: Option<&str>, is_dir: bool) -> String {
    match original_name {
        Some(original)
            if is_valid_item_name(original)
                && (current.to_lowercase() == original.to_lowercase()
                    || is_suffixed_variant(current, original, is_dir)) =>
        {
            if current.to_lowercase() == original.to_lowercase() {
                current.to_string()
            } else {
                original.to_string()
            }
        }
        _ => current.to_string(),
    }
}

/// 是否是快捷方式（`.lnk` / `.url`）
pub fn is_shortcut_name(file_name: &str) -> bool {
    let lower = file_name.to_lowercase();
    lower.ends_with(".lnk") || lower.ends_with(".url")
}

/// 收纳箱里显示的名字：去掉 `.lnk` / `.url` 后缀；去掉后为空则保留原名。文件夹原样显示。
pub fn display_name(file_name: &str, is_dir: bool) -> String {
    if is_dir || !is_shortcut_name(file_name) {
        return file_name.to_string();
    }
    let stem = &file_name[..file_name.len() - 4];
    if stem.is_empty() {
        file_name.to_string()
    } else {
        stem.to_string()
    }
}

/// 是否是系统元数据文件（`desktop.ini` / `Thumbs.db` / `.DS_Store` / `Icon\r` / `.localized`，见 [`SHELL_METADATA_FILES`]）
pub fn is_shell_metadata_file(file_name: &str) -> bool {
    let lower = file_name.to_lowercase();
    SHELL_METADATA_FILES.contains(&lower.as_str())
}

/// 是否按名字自动隐藏：以 `.` 开头的文件 / 文件夹（`.gitkeep`、`.env`、`.DS_Store`……），所有平台一样。
///
/// 只影响收纳箱的显示（"显示隐藏的图标"打开时半透明显示），不改文件属性；不能单独取消隐藏。
/// 与 Windows 隐藏 / 系统属性无关：带这两个属性的文件列举时直接跳过，永不显示
pub fn is_auto_hidden_name(file_name: &str) -> bool {
    file_name.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn taken_set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|n| n.to_lowercase()).collect()
    }

    #[test]
    fn sanitize_replaces_illegal_chars() {
        assert_eq!(
            sanitize_folder_name("a<b>c:d\"e/f\\g|h?i*j"),
            "a_b_c_d_e_f_g_h_i_j"
        );
        assert_eq!(sanitize_folder_name("tab\there\nnl"), "tab_here_nl");
        assert_eq!(sanitize_folder_name("工作 / 学习"), "工作 _ 学习");
    }

    #[test]
    fn sanitize_trims_spaces_and_trailing_dots() {
        assert_eq!(sanitize_folder_name("  常用软件  "), "常用软件");
        assert_eq!(sanitize_folder_name("项目..."), "项目");
        assert_eq!(sanitize_folder_name("项目. . ."), "项目");
        assert_eq!(sanitize_folder_name(".hidden"), ".hidden");
        assert_eq!(sanitize_folder_name("a.b"), "a.b");
    }

    #[test]
    fn sanitize_empty_falls_back_to_default() {
        for empty in ["", "   ", "...", " . . ", "\t"] {
            assert_eq!(
                sanitize_folder_name(empty),
                DEFAULT_FOLDER_NAME,
                "{empty:?}"
            );
        }
        assert_eq!(sanitize_folder_name(".."), DEFAULT_FOLDER_NAME);
        assert_eq!(sanitize_folder_name("???"), "___");
    }

    #[test]
    fn sanitize_avoids_reserved_names() {
        assert_eq!(sanitize_folder_name("CON"), "CON_");
        assert_eq!(sanitize_folder_name("con"), "con_");
        assert_eq!(sanitize_folder_name("Nul.txt"), "Nul_.txt");
        assert_eq!(sanitize_folder_name("com1"), "com1_");
        assert_eq!(sanitize_folder_name("LPT9"), "LPT9_");
        assert_eq!(sanitize_folder_name("COM¹"), "COM¹_");
        assert_eq!(sanitize_folder_name("CONOUT$"), "CONOUT$_");
        // 只是包含保留名的普通名字不受影响
        assert_eq!(sanitize_folder_name("CONsole"), "CONsole");
        assert_eq!(sanitize_folder_name("COM10"), "COM10");
        assert_eq!(sanitize_folder_name("my CON"), "my CON");
        // 尾部空格去掉后才是保留名
        assert_eq!(sanitize_folder_name("aux   "), "aux_");
    }

    #[test]
    fn sanitize_truncates_long_titles() {
        let long = "长".repeat(100);
        let name = sanitize_folder_name(&long);
        assert_eq!(name.chars().count(), MAX_FOLDER_NAME_CHARS);
        // 截断点落在空格 / 点上时要再修剪一次
        let tricky = format!("{}{}", "a".repeat(MAX_FOLDER_NAME_CHARS - 2), " .xyz");
        let name = sanitize_folder_name(&tricky);
        assert_eq!(name, "a".repeat(MAX_FOLDER_NAME_CHARS - 2));
        assert!(is_valid_folder_name(&name));
    }

    #[test]
    fn sanitized_names_are_valid_folder_names() {
        for title in [
            "收纳箱",
            "CON",
            "a:b",
            "  x  ",
            "...",
            "工作.",
            "COM1.txt",
            "😀 emoji",
            "a\u{0}b",
        ] {
            let name = sanitize_folder_name(title);
            assert!(is_valid_folder_name(&name), "{title:?} → {name:?}");
        }
    }

    #[test]
    fn folder_name_validation_rejects_traversal() {
        for bad in [
            "", ".", "..", "a\\b", "a/b", "..\\x", "C:", "a:b", " lead", "trail ", "dot.", "CON",
            "nul.txt", "a*b",
        ] {
            assert!(!is_valid_folder_name(bad), "应拒绝 {bad:?}");
        }
        assert!(is_valid_folder_name("收纳箱 (2)"));
        assert!(is_valid_folder_name(".hidden"));
        assert!(!is_valid_folder_name(
            &"a".repeat(MAX_FOLDER_NAME_CHARS + 17)
        ));
    }

    #[test]
    fn item_name_validation() {
        for ok in [
            "Chrome.lnk",
            "a b.txt",
            "文件夹",
            ".env",
            "trail.",
            "x (2).url",
        ] {
            assert!(is_valid_item_name(ok), "应接受 {ok:?}");
        }
        for bad in [
            "", ".", "..", "a\\b", "a/b", "C:\\x", "a:stream", "a?b", "a\u{1}b",
        ] {
            assert!(!is_valid_item_name(bad), "应拒绝 {bad:?}");
        }
        assert!(!is_valid_item_name(&"a".repeat(256)));
        assert!(is_valid_item_name(&"a".repeat(255)));
    }

    #[test]
    fn dedupe_appends_counter() {
        let taken = taken_set(&["收纳箱", "收纳箱 (2)", "其他"]);
        let is_taken = |c: &str| taken.contains(&c.to_lowercase());
        assert_eq!(dedupe_name("收纳箱", is_taken), "收纳箱 (3)");
        assert_eq!(dedupe_name("新的", is_taken), "新的");
        assert_eq!(dedupe_name("其他", is_taken), "其他 (2)");
        // 不区分大小写
        let taken = taken_set(&["Tools"]);
        assert_eq!(
            dedupe_name("tools", |c| taken.contains(&c.to_lowercase())),
            "tools (2)"
        );
    }

    #[test]
    fn split_extension_rules() {
        assert_eq!(split_extension("a.lnk", false), ("a", ".lnk"));
        assert_eq!(split_extension("a.tar.gz", false), ("a.tar", ".gz"));
        assert_eq!(split_extension(".env", false), (".env", ""));
        assert_eq!(split_extension("noext", false), ("noext", ""));
        assert_eq!(split_extension("trail.", false), ("trail.", ""));
        assert_eq!(split_extension("v1.2", true), ("v1.2", ""));
    }

    #[test]
    fn unique_file_name_adds_suffix_before_extension() {
        let taken = taken_set(&["chrome.lnk", "Chrome (2).lnk", "docs"]);
        let is_taken = |c: &str| taken.contains(&c.to_lowercase());
        assert_eq!(
            unique_file_name("Chrome.lnk", false, is_taken),
            "Chrome (3).lnk"
        );
        assert_eq!(
            unique_file_name("CHROME.LNK", false, is_taken),
            "CHROME (3).LNK"
        );
        assert_eq!(unique_file_name("Edge.lnk", false, is_taken), "Edge.lnk");
        assert_eq!(unique_file_name("docs", true, is_taken), "docs (2)");
        assert_eq!(suffixed_file_name("v1.2", 2, true), "v1.2 (2)");
        assert_eq!(suffixed_file_name("v1.2", 2, false), "v1 (2).2");
        assert_eq!(suffixed_file_name(".env", 2, false), ".env (2)");
    }

    #[test]
    fn restore_name_undoes_collision_suffix() {
        // 收纳时被改名的，移回时恢复原名
        assert_eq!(
            restore_file_name("Chrome (2).lnk", Some("Chrome.lnk"), false),
            "Chrome.lnk"
        );
        assert_eq!(
            restore_file_name("chrome (12).LNK", Some("Chrome.lnk"), false),
            "Chrome.lnk"
        );
        assert_eq!(restore_file_name("docs (3)", Some("docs"), true), "docs");
        // 名字没变
        assert_eq!(
            restore_file_name("Chrome.lnk", Some("Chrome.lnk"), false),
            "Chrome.lnk"
        );
        // 用户改过名字：保留用户的名字
        assert_eq!(
            restore_file_name("我的浏览器.lnk", Some("Chrome.lnk"), false),
            "我的浏览器.lnk"
        );
        // (1) 不是收纳产生的后缀；非数字也不算
        assert_eq!(
            restore_file_name("Chrome (1).lnk", Some("Chrome.lnk"), false),
            "Chrome (1).lnk"
        );
        assert_eq!(
            restore_file_name("Chrome (x).lnk", Some("Chrome.lnk"), false),
            "Chrome (x).lnk"
        );
        assert_eq!(
            restore_file_name("Chrome ().lnk", Some("Chrome.lnk"), false),
            "Chrome ().lnk"
        );
        // 没有原始路径 / 原始名非法
        assert_eq!(restore_file_name("a.txt", None, false), "a.txt");
        assert_eq!(
            restore_file_name("a (2).txt", Some(".."), false),
            "a (2).txt"
        );
    }

    #[test]
    fn display_name_strips_shortcut_extensions() {
        assert_eq!(display_name("Chrome.lnk", false), "Chrome");
        assert_eq!(display_name("官网.URL", false), "官网");
        assert_eq!(display_name("报告.docx", false), "报告.docx");
        assert_eq!(display_name(".lnk", false), ".lnk");
        assert_eq!(display_name("folder.lnk", true), "folder.lnk");
        assert!(is_shortcut_name("A.LNK"));
        assert!(!is_shortcut_name("a.lnk.txt"));
    }

    #[test]
    fn shell_metadata_files() {
        assert!(is_shell_metadata_file("desktop.ini"));
        assert!(is_shell_metadata_file("Desktop.INI"));
        assert!(is_shell_metadata_file("Thumbs.db"));
        assert!(!is_shell_metadata_file("desktop.ini.bak"));
        // macOS 访达的元数据（所有平台一样）
        assert!(is_shell_metadata_file(".DS_Store"));
        assert!(is_shell_metadata_file(".ds_store"));
        assert!(is_shell_metadata_file("Icon\r"));
        assert!(is_shell_metadata_file(".localized"));
        assert!(!is_shell_metadata_file("Icon"));
        assert!(!is_shell_metadata_file("Icon\r.png"));
        assert!(!is_shell_metadata_file(".DS_Store (2)"));
        assert!(!is_shell_metadata_file("DS_Store"));
    }

    #[test]
    fn package_folders_keep_their_extension_last() {
        for package in [
            "Safari.app",
            "工具.APP",
            "x.bundle",
            "报告.rtfd",
            "a.b.framework",
            "Proj.xcodeproj",
        ] {
            assert!(is_package_name(package), "{package:?}");
        }
        for plain in [
            "docs",
            "v1.2",
            ".app",
            "app",
            "x.app.bak",
            "x.",
            "Safari.App (2)",
        ] {
            assert!(!is_package_name(plain), "{plain:?}");
        }
        // 包文件夹按文件拆扩展名：后缀插在扩展名前
        assert_eq!(split_extension("Safari.app", true), ("Safari", ".app"));
        assert_eq!(suffixed_file_name("Safari.app", 2, true), "Safari (2).app");
        assert_eq!(
            suffixed_file_name("a.b.framework", 3, true),
            "a.b (3).framework"
        );
        let taken = taken_set(&["safari.app", "Safari (2).app"]);
        assert_eq!(
            unique_file_name("Safari.app", true, |c| taken.contains(&c.to_lowercase())),
            "Safari (3).app"
        );
        // 普通文件夹不变
        assert_eq!(suffixed_file_name("v1.2", 2, true), "v1.2 (2)");
        assert_eq!(suffixed_file_name("docs", 2, true), "docs (2)");
        // 移回时恢复原名：新写法与以前的写法都认
        assert_eq!(
            restore_file_name("Safari (2).app", Some("Safari.app"), true),
            "Safari.app"
        );
        assert_eq!(
            restore_file_name("Safari.app (2)", Some("Safari.app"), true),
            "Safari.app"
        );
        assert_eq!(
            restore_file_name("我的浏览器.app", Some("Safari.app"), true),
            "我的浏览器.app"
        );
    }

    #[test]
    fn dot_names_are_auto_hidden() {
        for hidden in [".gitkeep", ".env", ".DS_Store", ".config", ".env (2)", "."] {
            assert!(is_auto_hidden_name(hidden), "{hidden:?}");
        }
        for visible in ["a.env", "Chrome.lnk", "文件夹", " .env", "", "x."] {
            assert!(!is_auto_hidden_name(visible), "{visible:?}");
        }
    }
}
