//! 极简行级 diff（LCS）。零依赖，配置文件体量足够。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffTag {
    Keep,
    Add,
    Del,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub tag: DiffTag,
    pub text: String,
    pub old_no: Option<usize>,
    pub new_no: Option<usize>,
}

/// 行级差异（完整，不裁剪上下文）。
pub fn line_diff(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    lcs_diff(&a, &b)
}

fn lcs_diff(a: &[&str], b: &[&str]) -> Vec<DiffLine> {
    let n = a.len();
    let m = b.len();
    // dp[i][j] = LCS 长度（后缀）
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut out = Vec::with_capacity(n.max(m));
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(DiffLine {
                tag: DiffTag::Keep,
                text: a[i].to_string(),
                old_no: Some(i + 1),
                new_no: Some(j + 1),
            });
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push(DiffLine {
                tag: DiffTag::Del,
                text: a[i].to_string(),
                old_no: Some(i + 1),
                new_no: None,
            });
            i += 1;
        } else {
            out.push(DiffLine {
                tag: DiffTag::Add,
                text: b[j].to_string(),
                old_no: None,
                new_no: Some(j + 1),
            });
            j += 1;
        }
    }
    while i < n {
        out.push(DiffLine {
            tag: DiffTag::Del,
            text: a[i].to_string(),
            old_no: Some(i + 1),
            new_no: None,
        });
        i += 1;
    }
    while j < m {
        out.push(DiffLine {
            tag: DiffTag::Add,
            text: b[j].to_string(),
            old_no: None,
            new_no: Some(j + 1),
        });
        j += 1;
    }
    out
}

/// 统计增删行数。
pub fn stats(lines: &[DiffLine]) -> (usize, usize) {
    let add = lines.iter().filter(|l| l.tag == DiffTag::Add).count();
    let del = lines.iter().filter(|l| l.tag == DiffTag::Del).count();
    (add, del)
}

/// 渲染为带上下文的 unified 文本（用于 CLI / GUI 预览）。
pub fn unified(old: &str, new: &str, context: usize) -> String {
    let all = line_diff(old, new);
    if all.iter().all(|l| l.tag == DiffTag::Keep) {
        return String::new();
    }

    // 标记需要保留的行：变更行 ± context
    let keep_idx: Vec<bool> = all
        .iter()
        .enumerate()
        .map(|(i, l)| {
            l.tag != DiffTag::Keep
                || all
                    .iter()
                    .enumerate()
                    .any(|(j, o)| o.tag != DiffTag::Keep && j.abs_diff(i) <= context)
        })
        .collect();

    let mut out = String::new();
    let mut skipping = false;
    for (i, l) in all.iter().enumerate() {
        if !keep_idx[i] {
            if !skipping {
                out.push_str("  ...\n");
                skipping = true;
            }
            continue;
        }
        skipping = false;
        let sign = match l.tag {
            DiffTag::Keep => ' ',
            DiffTag::Add => '+',
            DiffTag::Del => '-',
        };
        out.push(sign);
        out.push_str(&l.text);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_has_no_changes() {
        let d = line_diff("a\nb\n", "a\nb\n");
        assert_eq!(stats(&d), (0, 0));
        assert!(unified("a\nb\n", "a\nb\n", 3).is_empty());
    }

    #[test]
    fn detects_added_line() {
        let d = line_diff("a\nb\n", "a\nx\nb\n");
        assert_eq!(stats(&d), (1, 0));
        assert!(unified("a\nb\n", "a\nx\nb\n", 3).contains("+x"));
    }

    #[test]
    fn detects_removed_line() {
        let d = line_diff("a\nx\nb\n", "a\nb\n");
        assert_eq!(stats(&d), (0, 1));
        assert!(unified("a\nx\nb\n", "a\nb\n", 3).contains("-x"));
    }

    #[test]
    fn context_truncates_far_lines() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let new = "1\n2\n3\n4\n5\n6\n7\n8\nCHANGED\n";
        let u = unified(old, new, 1);
        assert!(u.contains("..."), "远离变更的行应折叠");
        assert!(u.contains("-9"));
        assert!(u.contains("+CHANGED"));
    }
}
