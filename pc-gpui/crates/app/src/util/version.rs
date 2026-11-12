//! 版本号比较（旧前端 `appStore.compareVersions`，加上 semver 预发布排序）。版本检查（`services::update_check`）与
//! 更新日志（`services::changelog`）共用。

use std::cmp::Ordering;

/// `compareVersions`：去掉前缀 `v`，按 `.` 逐段数值比较（非数字段按 0）。
///
/// 预发布号按 semver 排序：主版本段相同时 `3.0.0-pre < 3.0.0`（装了预发布版的用户在正式版发布后
/// 也能看到更新提示）；两边都是预发布时逐个标识符比较（数字按数值、数字 < 字母、前缀相同则短的小）。
/// `+` 之后的构建元数据不参与比较。
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    // (数值段, 预发布标识符)
    let split = |v: &str| -> (Vec<i64>, Option<String>) {
        let v = v.trim();
        let v = v.strip_prefix('v').unwrap_or(v);
        let v = v.split('+').next().unwrap_or(v);
        let (core, pre) = match v.split_once('-') {
            Some((core, pre)) => (core, Some(pre.to_string())),
            None => (v, None),
        };
        let nums = core
            .split('.')
            .map(|p| p.trim().parse::<i64>().unwrap_or(0))
            .collect();
        (nums, pre)
    };
    let ((na, pa), (nb, pb)) = (split(a), split(b));
    for i in 0..na.len().max(nb.len()) {
        let (x, y) = (
            na.get(i).copied().unwrap_or(0),
            nb.get(i).copied().unwrap_or(0),
        );
        match x.cmp(&y) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    match (pa, pb) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(pa), Some(pb)) => compare_prerelease(&pa, &pb),
    }
}

/// semver 预发布标识符比较（`compare_versions` 用）
fn compare_prerelease(a: &str, b: &str) -> Ordering {
    let (ia, ib): (Vec<&str>, Vec<&str>) = (a.split('.').collect(), b.split('.').collect());
    for (x, y) in ia.iter().zip(ib.iter()) {
        let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => x.cmp(y),
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    ia.len().cmp(&ib.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(compare_versions("v2.6.0", "2.5.0"), Ordering::Greater);
        assert_eq!(compare_versions("v2.5", "2.5.0"), Ordering::Equal);
        assert_eq!(compare_versions("2.10.0", "2.9.9"), Ordering::Greater);
        assert_eq!(compare_versions("v2.4.9", "2.5.0"), Ordering::Less);
        assert_eq!(compare_versions("vX", "0"), Ordering::Equal);
    }

    #[test]
    fn versions_compare_prerelease() {
        // 预发布版低于同号正式版，高于上一个正式版
        assert_eq!(compare_versions("v3.0.0", "3.0.0-pre"), Ordering::Greater);
        assert_eq!(compare_versions("v3.0.0-pre", "3.0.0"), Ordering::Less);
        assert_eq!(compare_versions("v2.5.0", "3.0.0-pre"), Ordering::Less);
        assert_eq!(compare_versions("v3.0.1-pre", "3.0.0"), Ordering::Greater);
        assert_eq!(compare_versions("v3.0.0-pre", "3.0.0-pre"), Ordering::Equal);
        // 预发布之间按 semver 标识符规则
        assert_eq!(
            compare_versions("v3.0.0-pre.2", "3.0.0-pre"),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("v3.0.0-pre.10", "3.0.0-pre.9"),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("v3.0.0-rc", "3.0.0-pre"),
            Ordering::Greater
        );
        assert_eq!(compare_versions("v3.0.0-1", "3.0.0-pre"), Ordering::Less);
        // 构建元数据不参与比较
        assert_eq!(compare_versions("v3.0.0+abc", "3.0.0"), Ordering::Equal);
    }
}
