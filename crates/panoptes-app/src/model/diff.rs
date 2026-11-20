//! Line diffs of a tool call's file edits. Port of `src/diff.ts`.

use panoptes_harness::FileEdit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Hunk,
    Context,
    Add,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
}

/// Above this many lines on either side, the diff is not aligned line by line.
const MAX_ALIGNED: usize = 400;

fn lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    text.strip_suffix('\n')
        .unwrap_or(text)
        .split('\n')
        .collect()
}

fn line(kind: LineKind, text: &str) -> DiffLine {
    DiffLine {
        kind,
        text: text.to_string(),
    }
}

/// Line diff of one edit. Unchanged lines at the edges are kept as context,
/// and the middle is aligned on its longest common subsequence. A whole-file
/// write is all additions.
pub fn edit_diff(edit: &FileEdit) -> Vec<DiffLine> {
    let after = lines(&edit.new_text);
    let Some(old) = &edit.old_text else {
        return after.iter().map(|t| line(LineKind::Add, t)).collect();
    };
    let before = lines(old);
    let mut start = 0;
    while start < before.len() && start < after.len() && before[start] == after[start] {
        start += 1;
    }
    let mut end = 0;
    while end < before.len() - start
        && end < after.len() - start
        && before[before.len() - 1 - end] == after[after.len() - 1 - end]
    {
        end += 1;
    }
    let mut out: Vec<DiffLine> = before[..start]
        .iter()
        .map(|t| line(LineKind::Context, t))
        .collect();
    out.extend(align(
        &before[start..before.len() - end],
        &after[start..after.len() - end],
    ));
    out.extend(
        before[before.len() - end..]
            .iter()
            .map(|t| line(LineKind::Context, t)),
    );
    out
}

fn align(a: &[&str], b: &[&str]) -> Vec<DiffLine> {
    let removed = |i: usize| line(LineKind::Remove, a[i]);
    let added = |j: usize| line(LineKind::Add, b[j]);
    if a.is_empty() || b.is_empty() || a.len() > MAX_ALIGNED || b.len() > MAX_ALIGNED {
        return (0..a.len())
            .map(removed)
            .chain((0..b.len()).map(added))
            .collect();
    }
    // lcs[i][j]: length of the longest common subsequence of a[i..] and b[j..].
    let mut lcs = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            out.push(line(LineKind::Context, a[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(removed(i));
            i += 1;
        } else {
            out.push(added(j));
            j += 1;
        }
    }
    out.extend((i..a.len()).map(removed));
    out.extend((j..b.len()).map(added));
    out
}

/// Lines added and removed across a call's edits, for its row.
pub fn diff_stat(diffs: &[Vec<DiffLine>]) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for l in diffs.iter().flatten() {
        match l.kind {
            LineKind::Add => added += 1,
            LineKind::Remove => removed += 1,
            _ => {}
        }
    }
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(old: Option<&str>, new: &str) -> FileEdit {
        FileEdit {
            path: "a.ts".into(),
            old_text: old.map(Into::into),
            new_text: new.into(),
        }
    }

    fn sign(lines: Vec<DiffLine>) -> Vec<String> {
        lines
            .into_iter()
            .map(|l| {
                let s = match l.kind {
                    LineKind::Add => "+",
                    LineKind::Remove => "-",
                    _ => " ",
                };
                format!("{s}{}", l.text)
            })
            .collect()
    }

    #[test]
    fn keeps_unchanged_lines_as_context_and_aligns_the_rest() {
        assert_eq!(
            sign(edit_diff(&edit(Some("a\nb\nc\nd"), "a\nB\nc\nd\ne"))),
            [" a", "-b", "+B", " c", " d", "+e"]
        );
    }

    #[test]
    fn shows_a_whole_file_write_as_additions() {
        assert_eq!(sign(edit_diff(&edit(None, "x\ny\n"))), ["+x", "+y"]);
    }

    #[test]
    fn counts_additions_and_removals_across_edits() {
        let diffs = [
            edit_diff(&edit(Some("a"), "b")),
            edit_diff(&edit(Some(""), "c\nd")),
        ];
        assert_eq!(diff_stat(&diffs), (3, 1));
    }
}
