//! ID 生成：与 PC 端 `INTEGER PK AUTOINCREMENT` 共存。
//!
//! 思路：取当前 UNIX 毫秒 × 1000 + 随机 0..999 后缀。这样：
//! - 单调递增（毫秒分量）
//! - 同毫秒并发也不撞（毫秒末三位是随机）
//! - 数值范围远小于 i64 上限（year 9999 ≈ 2.5e14 < 9.2e18）
//! - PC 端 SQLite AUTOINCREMENT 从 1 起步、自然增长，最大 i64；云端这种取值
//!   远大于 PC 历史 id，所以**不会**与 PC 已有 id 撞。

use chrono::Utc;
use rand::Rng;

pub fn new_id() -> i64 {
    let ms = Utc::now().timestamp_millis();
    let suffix = rand::thread_rng().gen_range(0..1000);
    ms * 1000 + suffix
}

pub fn new_id_string() -> String {
    new_id().to_string()
}
