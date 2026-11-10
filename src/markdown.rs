//! Our own small Markdown parser and the egui preview renderer.
//! The parser only feeds the preview and never writes to the file; the
//! table editor rewrites a table's source text, which the app puts back.

pub mod blocks;
pub mod css;
pub mod highlight;
pub mod html;
pub mod html_blocks;
pub mod inlines;
pub mod notes;
pub mod outline;
pub mod render;
pub mod table_edit;
