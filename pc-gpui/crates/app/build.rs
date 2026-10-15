//! Windows 目标：把 exe 图标（ICON 资源 ID 1，GPUI 的窗口类图标取它，托盘也可以
//! `Icon::from_resource(1, ..)`）与版本信息编进 exe。
//!
//! 清单（manifest）由 gpui-pre 的默认 feature `windows-manifest` 嵌入，这里**不要**再嵌第二份。
//! 找不到资源编译器（例如在 Linux 上做交叉 `cargo check` 而没装 mingw windres）时只给 warning、
//! 不让构建失败。

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut parts: Vec<u16> = version
        .split(['.', '-', '+'])
        .take(3)
        .map(|p| p.parse().unwrap_or(0))
        .collect();
    parts.resize(4, 0);
    let comma = parts
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    // rc 字符串里的反斜杠要转义；正斜杠 rc.exe / windres 都接受
    let icon = Path::new(&manifest_dir)
        .join("assets")
        .join("icon.ico")
        .display()
        .to_string()
        .replace('\\', "/");

    let rc = format!(
        r#"#pragma code_page(65001)
1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION     {comma}
PRODUCTVERSION  {comma}
FILEOS          0x40004
FILETYPE        0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "080404b0"
    BEGIN
      VALUE "CompanyName",      "mini-todo"
      VALUE "FileDescription",  "Mini Todo"
      VALUE "FileVersion",      "{version}"
      VALUE "InternalName",     "mini-todo"
      VALUE "OriginalFilename", "mini-todo.exe"
      VALUE "ProductName",      "Mini Todo"
      VALUE "ProductVersion",   "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0804, 1200
  END
END
"#
    );
    let rc_path = Path::new(&out_dir).join("mini-todo.rc");
    if let Err(e) = std::fs::write(&rc_path, rc) {
        println!("cargo:warning=写入资源脚本失败，exe 将没有图标与版本信息: {e}");
        return;
    }
    match embed_resource::compile(&rc_path, embed_resource::NONE) {
        embed_resource::CompilationResult::Ok | embed_resource::CompilationResult::NotWindows => {}
        other => println!("cargo:warning=未嵌入 exe 图标与版本信息（{other}）"),
    }
}
