//! Mini Todo 二进制入口，见 `lib.rs`。

// release 是 Windows 子系统程序（没有控制台窗口）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    mini_todo::run();
}
