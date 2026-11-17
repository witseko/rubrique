//! Our own small reStructuredText reader for the preview.
//!
//! It turns an `.rst` file into the same blocks the Markdown preview draws
//! (headings, paragraphs, lists, tables, code, quotes), so RST gets the
//! read-only preview without a second renderer. It covers what READMEs and
//! Sphinx pages use most: section titles, inline markup and roles, links
//! and targets, substitutions, footnotes, literal blocks, simple and grid
//! tables, and the common directives. Directives it does not know show
//! their content; comments and targets show nothing.

mod blocks;
mod directives;
mod inlines;
mod lines;
mod refs;
mod tables;

use std::path::Path;

use crate::markdown::blocks::{Block, SourceBlock};

/// Extensions treated as reStructuredText (compared case-insensitively).
const RST_EXTENSIONS: [&str; 2] = ["rst", "rest"];

/// True when a file should get the reStructuredText preview.
pub fn is_rst_path(path: &Path) -> bool {
    path.extension()
        .map(|extension| extension.to_string_lossy())
        .is_some_and(|extension| RST_EXTENSIONS.iter().any(|known| extension.eq_ignore_ascii_case(known)))
}

/// Parse a whole reStructuredText document into preview blocks.
pub fn parse_rst(text: &str) -> Vec<Block> {
    let expanded: Vec<String> = text.lines().map(lines::expand_tabs).collect();
    let line_refs: Vec<&str> = expanded.iter().map(String::as_str).collect();
    let mut parser = blocks::Parser::new(refs::Refs::collect(&line_refs));
    parser.parse_lines(&line_refs, 0)
}

/// The document's blocks for the preview cache. The RST preview never edits
/// a block in place, so each block's range is simply the whole file.
pub fn rst_source_blocks(text: &str) -> Vec<SourceBlock> {
    parse_rst(text).into_iter().map(|block| SourceBlock { block, range: 0..text.len() }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rst_paths_by_extension() {
        assert!(is_rst_path(Path::new("docs/index.rst")));
        assert!(is_rst_path(Path::new("README.RST")));
        assert!(is_rst_path(Path::new("notes.rest")));
        assert!(!is_rst_path(Path::new("README.md")));
        assert!(!is_rst_path(Path::new("rst")));
    }

    #[test]
    fn source_blocks_span_the_whole_file() {
        let text = "Title\n=====\n\nBody.\n";
        let blocks = rst_source_blocks(text);
        assert_eq!(blocks.len(), 2);
        assert!(blocks.iter().all(|source| source.range == (0..text.len())));
    }
}
