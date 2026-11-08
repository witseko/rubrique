//! Which top-level blocks differ from the file as last saved, so the
//! preview can mark them and the Changes panel can list them until Save.
//!
//! The saved text and the buffer are both split into blocks, the blocks
//! that are the same in both are lined up (longest common subsequence),
//! and whatever sits between two matches is a change: paired up in order
//! as edits, with the extra blocks on either side counted as new or deleted.
//! Comparing with the saved text (not keeping a log of edits) means undoing
//! back to the saved version drops its mark by itself.

use std::iter;

use crate::markdown::blocks::{SourceBlock, parse_source_blocks};

/// Above this many cells (saved blocks times current blocks, after the
/// shared start and end are set aside) the lining up is skipped and the
/// blocks in between are paired in order. Keeps a huge rewrite fast.
const MAX_MATCH_CELLS: usize = 1_000_000;

/// How a block differs from the saved file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    /// The block is there in both, with different text.
    Edited,
    /// The block is not in the saved file.
    Added,
    /// The saved file has a block the buffer no longer has.
    Deleted,
}

/// One block that will change on Save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockChange {
    pub kind: ChangeKind,
    /// Index of the current block this change is on (edited, added), or the
    /// block a deleted one sat before (the block count when it was last).
    pub block: usize,
    /// The block's Markdown as saved; empty for an added block.
    pub saved: String,
    /// The block's Markdown now; empty for a deleted block.
    pub now: String,
}

/// Every block of `current_text` (split into `current`, as the preview
/// does) that differs from `saved_text`, in the order they appear.
pub fn compare_blocks(saved_text: &str, current_text: &str, current: &[SourceBlock]) -> Vec<BlockChange> {
    let saved_blocks = parse_source_blocks(saved_text);
    let saved: Vec<&str> = saved_blocks.iter().map(|block| &saved_text[block.range.clone()]).collect();
    let now: Vec<&str> = current.iter().map(|block| &current_text[block.range.clone()]).collect();
    let mut changes = Vec::new();
    let mut next_saved = 0;
    let mut next_now = 0;
    // The end counts as one more match, so the blocks after the last real
    // match are a gap like any other.
    for (saved_match, now_match) in
        matching_blocks(&saved, &now).into_iter().chain(iter::once((saved.len(), now.len())))
    {
        let gap = Gap { saved: next_saved..saved_match, now: next_now..now_match };
        gap_changes(&gap, &saved, &now, &mut changes);
        next_saved = saved_match + 1;
        next_now = now_match + 1;
    }
    changes
}

/// Blocks between two matches: `saved` in the saved file, `now` in the buffer.
struct Gap {
    saved: std::ops::Range<usize>,
    now: std::ops::Range<usize>,
}

/// The changes in one gap: blocks paired in order are edits, extra current
/// blocks are new, extra saved blocks are deleted (placed before the
/// block that ends the gap).
fn gap_changes(gap: &Gap, saved: &[&str], now: &[&str], changes: &mut Vec<BlockChange>) {
    let paired = gap.saved.len().min(gap.now.len());
    for offset in 0..paired {
        let (saved_index, block) = (gap.saved.start + offset, gap.now.start + offset);
        changes.push(BlockChange {
            kind: ChangeKind::Edited,
            block,
            saved: saved[saved_index].to_string(),
            now: now[block].to_string(),
        });
    }
    for (block, now_block) in now.iter().enumerate().take(gap.now.end).skip(gap.now.start + paired) {
        changes.push(BlockChange {
            kind: ChangeKind::Added,
            block,
            saved: String::new(),
            now: (*now_block).to_string(),
        });
    }
    for saved_block in &saved[gap.saved.start + paired..gap.saved.end] {
        changes.push(BlockChange {
            kind: ChangeKind::Deleted,
            block: gap.now.end,
            saved: (*saved_block).to_string(),
            now: String::new(),
        });
    }
}

/// Pairs of (saved index, current index) of blocks with the same text,
/// increasing in both. The shared start and end are matched directly; the
/// middle by longest common subsequence when it is small enough.
fn matching_blocks(saved: &[&str], now: &[&str]) -> Vec<(usize, usize)> {
    let prefix = saved.iter().zip(now).take_while(|(a, b)| a == b).count();
    let suffix = saved[prefix..].iter().rev().zip(now[prefix..].iter().rev()).take_while(|(a, b)| a == b).count();
    let saved_middle = &saved[prefix..saved.len() - suffix];
    let now_middle = &now[prefix..now.len() - suffix];
    let mut pairs: Vec<(usize, usize)> = (0..prefix).map(|index| (index, index)).collect();
    if saved_middle.len().saturating_mul(now_middle.len()) <= MAX_MATCH_CELLS {
        pairs.extend(
            longest_common_subsequence(saved_middle, now_middle)
                .into_iter()
                .map(|(saved_index, now_index)| (saved_index + prefix, now_index + prefix)),
        );
    }
    let saved_tail = saved.len() - suffix;
    let now_tail = now.len() - suffix;
    pairs.extend((0..suffix).map(|offset| (saved_tail + offset, now_tail + offset)));
    pairs
}

/// Index pairs of one longest common subsequence of `a` and `b`.
fn longest_common_subsequence(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let width = b.len() + 1;
    // lengths[i * width + j]: length of the LCS of a[i..] and b[j..].
    let mut lengths = vec![0_u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lengths[i * width + j] = if a[i] == b[j] {
                lengths[(i + 1) * width + j + 1] + 1
            } else {
                lengths[(i + 1) * width + j].max(lengths[i * width + j + 1])
            };
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if lengths[(i + 1) * width + j] >= lengths[i * width + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// `old` and `new` split around the part that differs: the shared start,
/// the old and new middles, and the shared end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InlineDiff<'a> {
    pub before: &'a str,
    pub removed: &'a str,
    pub added: &'a str,
    pub after: &'a str,
}

/// Split `old` and `new` around the one stretch that differs, for showing
/// "saved" and "now" with that stretch highlighted.
pub fn inline_diff<'a>(old: &'a str, new: &'a str) -> InlineDiff<'a> {
    let prefix: usize =
        old.chars().zip(new.chars()).take_while(|(a, b)| a == b).map(|(character, _)| character.len_utf8()).sum();
    // The shared end may not reach into the shared start.
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(character, _)| character.len_utf8())
        .sum();
    InlineDiff {
        before: &old[..prefix],
        removed: &old[prefix..old.len() - suffix],
        added: &new[prefix..new.len() - suffix],
        after: &old[old.len() - suffix..],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes(saved: &str, current: &str) -> Vec<(ChangeKind, usize, String)> {
        let blocks = parse_source_blocks(current);
        compare_blocks(saved, current, &blocks)
            .into_iter()
            .map(|change| (change.kind, change.block, change.saved))
            .collect()
    }

    #[test]
    fn the_same_text_has_no_changes() {
        assert_eq!(changes("# Title\n\nOne\n\nTwo\n", "# Title\n\nOne\n\nTwo\n"), []);
    }

    #[test]
    fn only_spacing_between_blocks_is_no_block_change() {
        assert_eq!(changes("One\n\nTwo\n", "One\n\n\n\nTwo\n"), []);
    }

    #[test]
    fn an_edited_block_keeps_its_saved_text() {
        assert_eq!(
            changes("One\n\nTwo\n\nThree\n", "One\n\nTwo!\n\nThree\n"),
            [(ChangeKind::Edited, 1, "Two".to_string())]
        );
    }

    #[test]
    fn a_new_block_is_added() {
        assert_eq!(changes("One\n\nThree\n", "One\n\nTwo\n\nThree\n"), [(ChangeKind::Added, 1, String::new())]);
    }

    #[test]
    fn a_deleted_block_sits_before_the_block_that_followed_it() {
        assert_eq!(changes("One\n\nTwo\n\nThree\n", "One\n\nThree\n"), [(ChangeKind::Deleted, 1, "Two".to_string())]);
        // The last block deleted: it sits at the end.
        assert_eq!(changes("One\n\nTwo\n", "One\n"), [(ChangeKind::Deleted, 1, "Two".to_string())]);
    }

    #[test]
    fn changes_far_apart_are_told_apart() {
        let saved = "# Notes\n\nKeep\n\n- a\n- b\n\nGone\n\nEnd\n";
        let current = "# Notes\n\nKeep\n\n- a\n- b\n- c\n\nEnd\n\nNew\n";
        assert_eq!(
            changes(saved, current),
            [
                (ChangeKind::Edited, 2, "- a\n- b".to_string()),
                (ChangeKind::Deleted, 3, "Gone".to_string()),
                (ChangeKind::Added, 4, String::new()),
            ]
        );
    }

    #[test]
    fn moving_a_block_shows_as_deleted_and_added() {
        assert_eq!(
            changes("A\n\nB\n\nC\n", "B\n\nC\n\nA\n"),
            [(ChangeKind::Deleted, 0, "A".to_string()), (ChangeKind::Added, 2, String::new()),]
        );
    }

    #[test]
    fn inline_diff_finds_the_changed_stretch() {
        let diff = inline_diff("open ideas. Old plan", "open ideas before the release. Old plan");
        assert_eq!(diff.before, "open ideas");
        assert_eq!(diff.removed, "");
        assert_eq!(diff.added, " before the release");
        assert_eq!(diff.after, ". Old plan");
    }

    #[test]
    fn inline_diff_keeps_whole_characters_and_does_not_overlap() {
        let diff = inline_diff("- [ ] ✔ done", "- [x] ✔ done");
        assert_eq!((diff.before, diff.removed, diff.added, diff.after), ("- [", " ", "x", "] ✔ done"));
        // A repeated letter: the shared end stops where the shared start ends.
        let diff = inline_diff("aa", "aaa");
        assert_eq!((diff.before, diff.removed, diff.added, diff.after), ("aa", "", "a", ""));
    }
}
