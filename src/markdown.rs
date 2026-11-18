//! Our own small Markdown parser and the egui preview renderer.
//! The parser only feeds the preview and never writes to the file; the
//! table editor rewrites a table's source text, which the app puts back.

pub mod blocks;
pub mod css;
pub mod front_matter;
mod front_matter_view;
pub mod highlight;
pub mod html;
pub mod html_blocks;
pub mod inlines;
pub mod local_links;
pub mod notes;
pub mod outline;
pub(crate) mod preview_find;
mod preview_highlights;
pub mod render;
pub mod table_edit;
