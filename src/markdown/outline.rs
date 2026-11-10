//! The outline of a Markdown document: its headings in order, for the
//! Structure panel. Built from the parsed top-level blocks, so it agrees
//! with what the preview shows.

use super::blocks::{Block, SourceBlock};
use super::inlines::plain_text;

/// One heading of the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    /// 1 to 6 (`#` to `######`).
    pub level: u8,
    /// The heading's text without Markdown formatting.
    pub text: String,
    /// Index of the heading's top-level block, as drawn by the preview.
    pub block_index: usize,
    /// Byte offset in the source where the heading's line starts.
    pub start: usize,
}

/// Every top-level heading in `blocks`, in document order. Headings inside
/// quotes or lists are left out: the preview can only scroll to top-level
/// blocks, and they are rarely meant as section titles. A heading in HTML
/// (an HTML block of a Markdown file, or one HTML centers) still counts:
/// it is all or most of its top-level block.
pub fn headings(blocks: &[SourceBlock]) -> Vec<Heading> {
    let mut found = Vec::new();
    for (block_index, source) in blocks.iter().enumerate() {
        push_headings(&mut found, &source.block, block_index, source.range.start);
    }
    found
}

/// Add the heading `block` is, or the headings an aligned or HTML block
/// holds.
fn push_headings(found: &mut Vec<Heading>, block: &Block, block_index: usize, start: usize) {
    match block {
        Block::Heading { level, content } => {
            found.push(Heading { level: *level, text: plain_text(content).trim().to_string(), block_index, start });
        }
        Block::Aligned { blocks, .. } | Block::Html(blocks) => {
            for block in blocks {
                push_headings(found, block, block_index, start);
            }
        }
        _ => {}
    }
}

/// Index into `headings` of the section that contains byte `offset` of the
/// source: the last heading that starts at or before it. `None` when the
/// offset is above the first heading.
pub fn current_heading(headings: &[Heading], offset: usize) -> Option<usize> {
    // Headings are in document order, so they are sorted by `start`.
    headings.partition_point(|heading| heading.start <= offset).checked_sub(1)
}

/// The smallest heading level used, so the outline can start its indent
/// there (a file without a `#` title should not be indented by one step).
pub fn top_level(headings: &[Heading]) -> u8 {
    headings.iter().map(|heading| heading.level).min().unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::blocks::parse_source_blocks;

    fn outline(text: &str) -> Vec<Heading> {
        headings(&parse_source_blocks(text))
    }

    #[test]
    fn lists_headings_in_order_with_levels_and_plain_text() {
        let text = "# Title\n\nIntro.\n\n## First *part*\n\n### Deep `code`\n\n###### Six\n";
        let found = outline(text);
        let summary: Vec<(u8, &str)> = found.iter().map(|heading| (heading.level, heading.text.as_str())).collect();
        assert_eq!(summary, vec![(1, "Title"), (2, "First part"), (3, "Deep code"), (6, "Six")]);
    }

    #[test]
    fn each_heading_maps_to_its_block_and_line() {
        let text = "Intro.\n\n# One\n\nBody.\n\n## Two\n";
        let found = outline(text);
        assert_eq!(found[0].block_index, 1);
        assert_eq!(found[0].start, text.find("# One").unwrap());
        assert_eq!(found[1].block_index, 3);
        assert_eq!(found[1].start, text.find("## Two").unwrap());
    }

    #[test]
    fn setext_headings_count() {
        let found = outline("Title\n=====\n\nSub\n---\n");
        let levels: Vec<u8> = found.iter().map(|heading| heading.level).collect();
        assert_eq!(levels, vec![1, 2]);
        assert_eq!(found[0].text, "Title");
        assert_eq!(found[0].start, 0);
    }

    #[test]
    fn ignores_hashes_in_code_and_headings_in_quotes() {
        let found = outline("```\n# not a heading\n```\n\n> # quoted\n\n#nospace\n");
        assert_eq!(found, Vec::new());
    }

    #[test]
    fn empty_document_has_no_headings() {
        assert_eq!(outline(""), Vec::new());
        assert_eq!(top_level(&[]), 1);
    }

    #[test]
    fn current_heading_is_the_last_one_at_or_before_the_offset() {
        let text = "Intro.\n\n# One\n\nBody.\n\n## Two\n\nMore.\n";
        let found = outline(text);
        assert_eq!(current_heading(&found, 0), None);
        assert_eq!(current_heading(&found, found[0].start), Some(0));
        assert_eq!(current_heading(&found, text.find("Body").unwrap()), Some(0));
        assert_eq!(current_heading(&found, found[1].start), Some(1));
        assert_eq!(current_heading(&found, text.len()), Some(1));
    }

    #[test]
    fn top_level_is_the_smallest_level_used() {
        assert_eq!(top_level(&outline("## A\n\n### B\n")), 2);
        assert_eq!(top_level(&outline("### A\n\n# B\n")), 1);
    }

    #[test]
    fn centered_html_headings_are_in_the_outline() {
        let html = "<h1 align=center>Title</h1><p>text</p><div style=\"text-align: right\"><h2>Part</h2></div>";
        let found = headings(&crate::markdown::html_blocks::html_source_blocks(html));
        let titles: Vec<_> =
            found.iter().map(|heading| (heading.level, heading.text.as_str(), heading.block_index)).collect();
        assert_eq!(titles, [(1, "Title", 0), (2, "Part", 2)]);
    }
}
