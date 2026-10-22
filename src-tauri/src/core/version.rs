//! 版本号比较。
//!
//! 桌面端与 MCP 服务都要判断「哪个版本更新」，实现不一致会出现界面提示可升级、
//! 服务却认为已是最新的情况。这里只保留一套规则：去掉可选的 `v` 前缀，按 `.`、
//! `-`、`+` 分段，逐段按数字比较，缺失段视为 0，非数字段视为 0。
//!
//! 规则刻意与 MCP 侧 `compareVersions` 完全一致，`fixtures/versions.json` 用同一批
//! 用例约束两端。已知限制：预发布标识按普通数字段参与比较，因此 `1.0.0-beta.2`
//! 会被判为高于 `1.0.0`。现有制品版本形如 `0.0.12-<提交号>`，两种规则结论相同，
//! 暂不改变比较规则；若要支持语义化版本优先级，需要同时调整升级提示与最低版本校验，
//! 属于行为变更，需单独评估。

use std::cmp::Ordering;

fn segments(value: &str) -> Vec<u64> {
    let trimmed = value.trim();
    let body = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);
    if body.is_empty() {
        return vec![0];
    }
    body.split(['.', '-', '+'])
        .map(|part| {
            let segment = part.trim();
            if segment.is_empty() || !segment.bytes().all(|byte| byte.is_ascii_digit()) {
                return 0;
            }
            segment.parse::<u64>().unwrap_or(0)
        })
        .collect()
}

/// 逐段比较两个版本号，返回界面的排序方向。
pub fn compare(left: &str, right: &str) -> Ordering {
    let left = segments(left);
    let right = segments(right);
    for index in 0..left.len().max(right.len()) {
        match left
            .get(index)
            .copied()
            .unwrap_or(0)
            .cmp(&right.get(index).copied().unwrap_or(0))
        {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

/// `candidate` 是否高于 `current`，用于判断能否升级。
pub fn is_newer(candidate: &str, current: &str) -> bool {
    compare(candidate, current) == Ordering::Greater
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Case {
        left: String,
        right: String,
        expected: i32,
    }

    #[derive(Deserialize)]
    struct Fixture {
        cases: Vec<Case>,
    }

    fn sign(ordering: Ordering) -> i32 {
        match ordering {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        }
    }

    #[test]
    fn matches_shared_fixture() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../../fixtures/versions.json")).unwrap();
        assert!(!fixture.cases.is_empty());
        for case in fixture.cases {
            assert_eq!(
                sign(compare(&case.left, &case.right)),
                case.expected,
                "{} ? {}",
                case.left,
                case.right
            );
        }
    }

    #[test]
    fn empty_input_is_not_newer() {
        assert_eq!(compare("", ""), Ordering::Equal);
        assert_eq!(compare("", "0.0.1"), Ordering::Less);
        assert!(!is_newer("v1.0.0", "1.0.0"));
        assert!(is_newer("1.0.1", "1.0.0"));
    }
}
