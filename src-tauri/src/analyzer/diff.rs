//! Side-by-side line diff (P4-T08): config backups, running ↔ startup.
//!
//! Common prefix and suffix are trimmed first (two configs are mostly the same); the middle
//! is aligned with a longest-common-subsequence table when it is small enough
//! ([`MAX_CELLS`]), otherwise the middle is shown as replaced. Removed and added runs are
//! paired into "changed" rows so the two sides line up.

use serde::Serialize;

/// LCS table limit (cells): ~16 MB of u32.
const MAX_CELLS: usize = 4_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RowKind {
    Same,
    Changed,
    Removed,
    Added,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub kind: RowKind,
    /// 1-based line numbers.
    pub left_no: Option<usize>,
    pub left: Option<String>,
    pub right_no: Option<usize>,
    pub right: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    pub rows: Vec<Row>,
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    /// The middle was too large to align line by line.
    pub coarse: bool,
}

enum Op {
    Same(usize, usize),
    Del(usize),
    Add(usize),
}

pub fn diff(left: &str, right: &str) -> Diff {
    let a: Vec<&str> = left.lines().map(str::trim_end).collect();
    let b: Vec<&str> = right.lines().map(str::trim_end).collect();
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf]
    {
        suf += 1;
    }
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    let mut ops: Vec<Op> = (0..pre).map(|i| Op::Same(i, i)).collect();
    let coarse = am.len().saturating_mul(bm.len()) > MAX_CELLS;
    if coarse {
        ops.extend((0..am.len()).map(|i| Op::Del(pre + i)));
        ops.extend((0..bm.len()).map(|j| Op::Add(pre + j)));
    } else {
        ops.extend(lcs(am, bm, pre));
    }
    ops.extend((0..suf).map(|k| Op::Same(a.len() - suf + k, b.len() - suf + k)));

    // Pair runs of removals with the following additions.
    let mut rows = Vec::with_capacity(ops.len());
    let (mut added, mut removed, mut changed) = (0, 0, 0);
    let mut i = 0;
    while i < ops.len() {
        if let Op::Same(x, y) = ops[i] {
            rows.push(Row {
                kind: RowKind::Same,
                left_no: Some(x + 1),
                left: Some(a[x].to_owned()),
                right_no: Some(y + 1),
                right: Some(b[y].to_owned()),
            });
            i += 1;
            continue;
        }
        let mut dels = Vec::new();
        let mut adds = Vec::new();
        while i < ops.len() {
            match ops[i] {
                Op::Del(x) => dels.push(x),
                Op::Add(y) => adds.push(y),
                Op::Same(..) => break,
            }
            i += 1;
        }
        for k in 0..dels.len().max(adds.len()) {
            let (l, r) = (dels.get(k).copied(), adds.get(k).copied());
            let kind = match (l, r) {
                (Some(_), Some(_)) => {
                    changed += 1;
                    RowKind::Changed
                }
                (Some(_), None) => {
                    removed += 1;
                    RowKind::Removed
                }
                _ => {
                    added += 1;
                    RowKind::Added
                }
            };
            rows.push(Row {
                kind,
                left_no: l.map(|x| x + 1),
                left: l.map(|x| a[x].to_owned()),
                right_no: r.map(|y| y + 1),
                right: r.map(|y| b[y].to_owned()),
            });
        }
    }
    Diff {
        rows,
        added,
        removed,
        changed,
        coarse,
    }
}

/// Edit script for the middle part (`off` = its offset in both inputs).
fn lcs(a: &[&str], b: &[&str], off: usize) -> Vec<Op> {
    let (n, m) = (a.len(), b.len());
    let w = m + 1;
    let mut t = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i * w + j] = if a[i] == b[j] {
                t[(i + 1) * w + j + 1] + 1
            } else {
                t[(i + 1) * w + j].max(t[i * w + j + 1])
            };
        }
    }
    let mut ops = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            ops.push(Op::Same(off + i, off + j));
            i += 1;
            j += 1;
        } else if t[(i + 1) * w + j] >= t[i * w + j + 1] {
            ops.push(Op::Del(off + i));
            i += 1;
        } else {
            ops.push(Op::Add(off + j));
            j += 1;
        }
    }
    ops.extend((i..n).map(|x| Op::Del(off + x)));
    ops.extend((j..m).map(|y| Op::Add(off + y)));
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(d: &Diff) -> Vec<RowKind> {
        d.rows.iter().map(|r| r.kind).collect()
    }

    #[test]
    fn changes_line_up_side_by_side() {
        let left =
            "hostname sw1\ninterface Gi1/0/1\n description old\n shutdown\nntp server 1.1.1.1\nend";
        let right = "hostname sw1\ninterface Gi1/0/1\n description new\nntp server 1.1.1.1\nlogging host 10.0.0.5\nend";
        let d = diff(left, right);
        use RowKind::*;
        assert_eq!(kinds(&d), [Same, Same, Changed, Removed, Same, Added, Same]);
        assert_eq!((d.added, d.removed, d.changed), (1, 1, 1));
        assert_eq!(d.rows[2].left.as_deref(), Some(" description old"));
        assert_eq!(d.rows[2].right.as_deref(), Some(" description new"));
        assert_eq!((d.rows[5].left_no, d.rows[5].right_no), (None, Some(5)));
        assert!(!d.coarse);
    }

    #[test]
    fn identical_empty_and_trailing_space() {
        assert!(
            kinds(&diff("a\nb", "a  \nb"))
                .iter()
                .all(|k| *k == RowKind::Same)
        );
        assert!(diff("", "").rows.is_empty());
        assert_eq!(kinds(&diff("", "x")), [RowKind::Added]);
        assert_eq!(kinds(&diff("x", "")), [RowKind::Removed]);
    }

    #[test]
    fn huge_different_middles_fall_back_to_replaced() {
        let left: String = (0..3000).map(|i| format!("a{i}\n")).collect();
        let right: String = (0..3000).map(|i| format!("b{i}\n")).collect();
        let d = diff(&left, &right);
        assert!(d.coarse);
        assert_eq!(d.changed, 3000);
    }
}
