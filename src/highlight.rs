//! Find the syntax of code files (Python, JavaScript, TypeScript,
//! Rust, JSON, SQL, shell, XML, HTML with Angular templates) in raw text
//! so the editor and the preview can color it, and the user's text
//! highlight rules for plain text.

pub mod code;
pub mod shell;
pub mod text_rules;
pub mod xml;
