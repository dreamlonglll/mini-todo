//! 把 app 里不依赖 GPUI 的 mac 专属源文件原样编进来，在 Linux / Windows 上 `--target aarch64-apple-darwin` 做类型检查。
//! 用法见 `Cargo.toml` 顶部注释。每加一个这样的源文件，就在下面加一行 `#[path]` 模块。
#![cfg(target_os = "macos")]
#![allow(dead_code)]
