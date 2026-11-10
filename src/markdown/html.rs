//! Turn parsed Markdown into a standalone HTML page for the system browser.
//!
//! All text is escaped. Links keep only web and mail targets and relative
//! paths; anything else (such as `javascript:`) is shown as plain text.

use std::fmt::Write as _;
use std::path::Path;

use super::blocks::{Block, ColumnAlign, List, Table};
use super::inlines::{HtmlStyle, ImageSize, Inline, Script, is_openable_url};
use crate::theme::palette::{Palette, Rgb};

/// A full HTML page for `blocks`, colored like the theme `palette`.
/// `base` is the folder URL that relative links and images resolve against.
pub fn html_page(title: &str, blocks: &[Block], palette: &Palette, base: Option<&str>) -> String {
    let mut body = String::new();
    write_blocks(&mut body, blocks);
    let base_tag = base.map(|url| format!("<base href=\"{}\">\n", escape(url))).unwrap_or_default();
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"generator\" content=\"Rubrique\">\n{base_tag}<title>{}</title>\n<style>\n{}</style>\n\
         </head>\n<body>\n<main>\n{body}</main>\n</body>\n</html>\n",
        escape(title),
        style(palette),
    )
}

/// Put `<base href>` into an HTML document so its relative links work from a
/// temporary copy: right after `<head>` when there is one, else first.
pub fn with_base(html: &str, base: &str) -> String {
    let tag = format!("<base href=\"{}\">", escape(base));
    let lower = html.to_ascii_lowercase();
    match lower.find("<head").and_then(|start| lower[start..].find('>').map(|end| start + end + 1)) {
        Some(insert_at) => format!("{}{tag}{}", &html[..insert_at], &html[insert_at..]),
        None => format!("{tag}\n{html}"),
    }
}

/// The page's CSS with the theme's colors.
fn style(palette: &Palette) -> String {
    format!(
        ":root {{ --bg: {}; --text: {}; --strong: {}; --weak: {}; --accent: {}; --border: {}; --code: {}; }}\n\
         html {{ background: var(--bg); }}\n\
         body {{ margin: 0; color: var(--text); font: 16px/1.6 system-ui, -apple-system, \"Segoe UI\", Ubuntu, sans-serif; }}\n\
         main {{ max-width: 760px; margin: 0 auto; padding: 40px 24px 64px; }}\n\
         h1, h2, h3, h4, h5, h6 {{ color: var(--strong); font-weight: 500; line-height: 1.3; margin: 1.6em 0 0.5em; }}\n\
         h1:first-child {{ margin-top: 0; }}\n\
         h1, h2 {{ padding-bottom: 0.3em; border-bottom: 1px solid var(--border); }}\n\
         a {{ color: var(--accent); }}\n\
         code, pre {{ font-family: ui-monospace, \"JetBrains Mono\", Menlo, monospace; font-size: 0.9em; }}\n\
         code {{ background: var(--code); padding: 0.1em 0.35em; border-radius: 4px; }}\n\
         img, video {{ max-width: 100%; height: auto; }}\n\
         pre {{ background: var(--code); border: 1px solid var(--border); border-radius: 6px; padding: 12px 14px; overflow-x: auto; }}\n\
         pre code {{ background: none; padding: 0; }}\n\
         blockquote {{ margin: 1em 0; padding: 0 0 0 16px; border-left: 3px solid var(--accent); color: var(--weak); }}\n\
         table {{ border-collapse: collapse; margin: 1em 0; }}\n\
         th, td {{ border: 1px solid var(--border); padding: 4px 10px; text-align: left; }}\n\
         th {{ background: var(--code); }}\n\
         hr {{ border: 0; border-top: 1px solid var(--border); margin: 2em 0; }}\n\
         li > p {{ margin: 0; }}\n\
         li.task {{ list-style: none; margin-left: -1.4em; }}\n\
         li.task input {{ accent-color: var(--accent); margin-right: 0.5em; }}\n\
         figure.memo {{ margin: 1em 0; }}\n\
         figure.memo audio {{ width: 100%; }}\n\
         figure.memo figcaption {{ color: var(--weak); font-size: 0.9em; }}\n",
        css(palette.background),
        css(palette.text),
        css(palette.text_strong),
        css(palette.text_weak),
        css(palette.accent),
        css(palette.border),
        css(palette.code_background),
    )
}

fn css(Rgb(red, green, blue): Rgb) -> String {
    format!("#{red:02x}{green:02x}{blue:02x}")
}

fn write_blocks(out: &mut String, blocks: &[Block]) {
    for block in blocks {
        write_block(out, block);
    }
}

fn write_block(out: &mut String, block: &Block) {
    match block {
        Block::Heading { level, content } => {
            let level = (*level).clamp(1, 6);
            let _ = write!(out, "<h{level}>");
            write_inlines(out, content);
            let _ = writeln!(out, "</h{level}>");
        }
        Block::Paragraph(content) => {
            if let Some(memo) = crate::voice::memo_link(content).filter(|memo| is_safe_link(&memo.url)) {
                write_memo(out, &memo);
            } else {
                out.push_str("<p>");
                write_inlines(out, content);
                out.push_str("</p>\n");
            }
        }
        Block::Code { info, code } => {
            let language = info.split_whitespace().next().unwrap_or("");
            if language.is_empty() {
                out.push_str("<pre><code>");
            } else {
                let _ = write!(out, "<pre><code class=\"language-{}\">", escape(language));
            }
            out.push_str(&escape(code));
            out.push_str("</code></pre>\n");
        }
        Block::Quote(blocks) => {
            out.push_str("<blockquote>\n");
            write_blocks(out, blocks);
            out.push_str("</blockquote>\n");
        }
        Block::List(list) => write_list(out, list),
        Block::Table(table) => write_table(out, table),
        Block::ThematicBreak => out.push_str("<hr>\n"),
        Block::Aligned { align, blocks } => {
            let _ = writeln!(out, "<div style=\"text-align: {}\">", align_name(*align));
            write_blocks(out, blocks);
            out.push_str("</div>\n");
        }
        // Written from what the preview read, never the source as it is,
        // so no script or style sheet in the file reaches the page.
        Block::Html(blocks) => write_blocks(out, blocks),
        Block::Details { summary, open, blocks } => {
            out.push_str(if *open { "<details open>" } else { "<details>" });
            out.push_str("<summary>");
            write_inlines(out, summary);
            out.push_str("</summary>\n");
            write_blocks(out, blocks);
            out.push_str("</details>\n");
        }
    }
}

/// The CSS name of an alignment.
fn align_name(align: ColumnAlign) -> &'static str {
    match align {
        ColumnAlign::Left => "left",
        ColumnAlign::Center => "center",
        ColumnAlign::Right => "right",
    }
}

/// A voice memo: the browser's audio player with the memo's name under it.
fn write_memo(out: &mut String, memo: &crate::voice::MemoLink) {
    let label = if memo.label.is_empty() { "Voice memo" } else { memo.label.as_str() };
    let _ = writeln!(
        out,
        "<figure class=\"memo\"><audio controls preload=\"metadata\" src=\"{}\"></audio><figcaption>{}</figcaption></figure>",
        escape(&memo.url),
        escape(label)
    );
}

fn write_list(out: &mut String, list: &List) {
    let (open, close) = match list.start {
        Some(1) => ("<ol>".to_string(), "</ol>"),
        Some(start) => (format!("<ol start=\"{start}\">"), "</ol>"),
        None => ("<ul>".to_string(), "</ul>"),
    };
    out.push_str(&open);
    out.push('\n');
    for item in &list.items {
        match item.task {
            Some(checked) => {
                let checked = if checked { " checked" } else { "" };
                let _ = write!(out, "<li class=\"task\"><input type=\"checkbox\" disabled{checked}>");
            }
            None => out.push_str("<li>"),
        }
        // A single paragraph renders inline, like a tight list.
        match item.blocks.as_slice() {
            [Block::Paragraph(content)] => write_inlines(out, content),
            blocks => write_blocks(out, blocks),
        }
        out.push_str("</li>\n");
    }
    out.push_str(close);
    out.push('\n');
}

fn write_table(out: &mut String, table: &Table) {
    let align_attr = |column: usize| match table.align.get(column) {
        Some(ColumnAlign::Center) => " align=\"center\"",
        Some(ColumnAlign::Right) => " align=\"right\"",
        _ => "",
    };
    out.push_str("<table>\n<thead>\n<tr>");
    for (column, cell) in table.header.iter().enumerate() {
        let _ = write!(out, "<th{}>", align_attr(column));
        write_inlines(out, cell);
        out.push_str("</th>");
    }
    out.push_str("</tr>\n</thead>\n");
    if !table.rows.is_empty() {
        out.push_str("<tbody>\n");
        for row in &table.rows {
            out.push_str("<tr>");
            for (column, cell) in row.iter().enumerate() {
                let _ = write!(out, "<td{}>", align_attr(column));
                write_inlines(out, cell);
                out.push_str("</td>");
            }
            out.push_str("</tr>\n");
        }
        out.push_str("</tbody>\n");
    }
    out.push_str("</table>\n");
}

fn write_inlines(out: &mut String, inlines: &[Inline]) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => out.push_str(&escape(text)),
            Inline::Code(code) => {
                out.push_str("<code>");
                out.push_str(&escape(code));
                out.push_str("</code>");
            }
            Inline::Emphasis(children) => wrap(out, "em", children),
            Inline::Strong(children) => wrap(out, "strong", children),
            Inline::Strikethrough(children) => wrap(out, "del", children),
            Inline::Link { content, url } => {
                if is_safe_link(url) {
                    let _ = write!(out, "<a href=\"{}\">", escape(url));
                    write_inlines(out, content);
                    out.push_str("</a>");
                } else {
                    write_inlines(out, content);
                }
            }
            Inline::Image { alt, url, size } if is_safe_link(url) => {
                if crate::media::media_kind_of_url(url) == Some(crate::media::MediaKind::Video) {
                    let _ = write!(out, "<video controls src=\"{}\" title=\"{}\"></video>", escape(url), escape(alt));
                } else {
                    let size = size_attributes(*size);
                    let _ = write!(out, "<img src=\"{}\" alt=\"{}\"{size}>", escape(url), escape(alt));
                }
            }
            Inline::Image { alt, .. } => out.push_str(&escape(alt)),
            Inline::LineBreak => out.push_str("<br>\n"),
            // Notes stay private to Rubrique, like the comments they come from.
            Inline::Note { .. } => {}
            Inline::Styled { style, content } => {
                let _ = write!(out, "<span style=\"{}\">", style_css(style));
                write_inlines(out, content);
                out.push_str("</span>");
            }
        }
    }
}

/// ` width="..." height="..."` for the sizes an image asks for.
fn size_attributes(size: ImageSize) -> String {
    let mut attributes = String::new();
    if let Some(width) = size.width {
        let _ = write!(attributes, " width=\"{width}\"");
    }
    if let Some(height) = size.height {
        let _ = write!(attributes, " height=\"{height}\"");
    }
    attributes
}

/// The CSS declarations for an HTML style (the colors as written, not
/// adjusted to the theme: the exported page has the author's look).
fn style_css(style: &HtmlStyle) -> String {
    let mut declarations = Vec::new();
    if let Some(color) = style.color {
        declarations.push(format!("color: {}", css(color)));
    }
    if let Some(background) = style.background {
        declarations.push(format!("background-color: {}", css(background)));
    }
    if style.underline {
        declarations.push("text-decoration: underline".to_string());
    }
    match style.script {
        Some(Script::Superscript) => declarations.push("vertical-align: super; font-size: smaller".to_string()),
        Some(Script::Subscript) => declarations.push("vertical-align: sub; font-size: smaller".to_string()),
        None => {}
    }
    declarations.join("; ")
}

fn wrap(out: &mut String, tag: &str, children: &[Inline]) {
    let _ = write!(out, "<{tag}>");
    write_inlines(out, children);
    let _ = write!(out, "</{tag}>");
}

/// Web and mail links, and relative paths (no scheme before the first `/`).
fn is_safe_link(url: &str) -> bool {
    if is_openable_url(url) {
        return true;
    }
    let before_path = url.split(['/', '?', '#']).next().unwrap_or("");
    !before_path.contains(':')
}

/// True for `.html` and `.htm` files (any case).
pub fn is_html_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm"))
}

/// A `file://` URL for an absolute path, with unsafe bytes percent-encoded.
pub fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            url.push(char::from(byte));
        } else {
            let _ = write!(url, "%{byte:02X}");
        }
    }
    url
}

/// Escape text for HTML content and double-quoted attributes.
pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::blocks::parse_blocks;

    fn body(markdown: &str) -> String {
        let mut out = String::new();
        write_blocks(&mut out, &parse_blocks(markdown));
        out
    }

    #[test]
    fn tables() {
        assert_eq!(
            body("| a | b |\n| - | -: |\n| 1 | *2* |"),
            "<table>\n<thead>\n<tr><th>a</th><th align=\"right\">b</th></tr>\n</thead>\n\
             <tbody>\n<tr><td>1</td><td align=\"right\"><em>2</em></td></tr>\n</tbody>\n</table>\n"
        );
    }

    #[test]
    fn headings_paragraphs_and_inlines() {
        assert_eq!(body("# Title"), "<h1>Title</h1>\n");
        assert_eq!(
            body("a *b* **c** ~~d~~ `e`"),
            "<p>a <em>b</em> <strong>c</strong> <del>d</del> <code>e</code></p>\n"
        );
    }

    #[test]
    fn text_is_escaped() {
        assert_eq!(
            body("a <script>alert(1)</script> & \"x\""),
            "<p>a &lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;x&quot;</p>\n"
        );
        // An HTML block is written from what was read: the script is gone.
        assert_eq!(body("<script>alert(1)</script> & \"x\""), "<p>&amp; &quot;x&quot;</p>\n");
        assert_eq!(body("<p onclick=\"x()\">hi <b>there</b></p>"), "<p>hi <strong>there</strong></p>\n");
        assert_eq!(body("```html\n<b>\n```"), "<pre><code class=\"language-html\">&lt;b&gt;</code></pre>\n");
    }

    #[test]
    fn only_safe_links_become_links() {
        assert_eq!(body("[a](https://x.org)"), "<p><a href=\"https://x.org\">a</a></p>\n");
        assert_eq!(body("[a](notes/b.md)"), "<p><a href=\"notes/b.md\">a</a></p>\n");
        assert_eq!(body("[a](javascript:alert(1))"), "<p>a</p>\n");
    }

    #[test]
    fn voice_memos_get_an_audio_player() {
        assert_eq!(
            body("[▶ Voice memo](memos/a%20b.wav)"),
            "<figure class=\"memo\"><audio controls preload=\"metadata\" src=\"memos/a%20b.wav\"></audio>\
             <figcaption>Voice memo</figcaption></figure>\n"
        );
        assert_eq!(body("[x](javascript:a.wav)"), "<p>x</p>\n");
        assert_eq!(body("Listen: [x](a.wav)"), "<p>Listen: <a href=\"a.wav\">x</a></p>\n");
    }

    #[test]
    fn images_and_videos() {
        assert_eq!(body("![a \"cat\"](img/cat.png)"), "<p><img src=\"img/cat.png\" alt=\"a &quot;cat&quot;\"></p>\n");
        assert_eq!(body("![demo](clip.mp4)"), "<p><video controls src=\"clip.mp4\" title=\"demo\"></video></p>\n");
        assert_eq!(body("![x](javascript:alert(1))"), "<p>x</p>\n");
    }

    #[test]
    fn lists_and_tasks() {
        assert_eq!(body("- a\n- b"), "<ul>\n<li>a</li>\n<li>b</li>\n</ul>\n");
        assert_eq!(body("3. c"), "<ol start=\"3\">\n<li>c</li>\n</ol>\n");
        assert_eq!(
            body("- [x] done"),
            "<ul>\n<li class=\"task\"><input type=\"checkbox\" disabled checked>done</li>\n</ul>\n"
        );
    }

    #[test]
    fn page_has_title_base_and_theme_colors() {
        let palette = crate::theme::palette::Theme::builtin(crate::theme::palette::Base::Dark).palette;
        let page = html_page("a <b>", &parse_blocks("hi"), &palette, Some("file:///home/a/"));
        assert!(page.contains("<title>a &lt;b&gt;</title>"));
        assert!(page.contains("<base href=\"file:///home/a/\">"));
        assert!(page.contains("--bg: #1d1f23"));
        assert!(page.contains("<p>hi</p>"));
    }

    #[test]
    fn html_paths_and_file_urls() {
        assert!(is_html_path(Path::new("a/Page.HTM")));
        assert!(!is_html_path(Path::new("a/page.md")));
        assert_eq!(file_url(Path::new("/home/a/my notes/é.md")), "file:///home/a/my%20notes/%C3%A9.md");
    }

    #[test]
    fn base_goes_into_head() {
        assert_eq!(
            with_base("<html><HEAD lang=x><title>t</title>", "file:///d/"),
            "<html><HEAD lang=x><base href=\"file:///d/\"><title>t</title>"
        );
        assert_eq!(with_base("<p>hi</p>", "file:///d/"), "<base href=\"file:///d/\">\n<p>hi</p>");
    }
}
