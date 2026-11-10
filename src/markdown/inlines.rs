//! Inline Markdown: emphasis, strong, strikethrough, code spans, links, breaks.
//!
//! A simplified version of CommonMark's rules: a delimiter run is closed by
//! the next run of the same character and the same length. Unclosed
//! delimiters are shown literally. Common inline HTML tags (`<b>`, `<kbd>`,
//! `<br>`, `<img>`, `<span style>` and the like) are read through the HTML
//! page reader; other tags stay as written.

use crate::theme::palette::Rgb;

/// One piece of inline content.
#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String),
    /// Code span content, shown verbatim.
    Code(String),
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Link {
        content: Vec<Inline>,
        url: String,
    },
    /// `![alt](url)`: an image, or a video when the url names one.
    Image {
        alt: String,
        url: String,
        /// The size an HTML `<img>` asks for; Markdown images have none.
        size: ImageSize,
    },
    /// Hard line break (two trailing spaces or a trailing backslash).
    LineBreak,
    /// A note comment (`<!-- note on "quote": text -->`) about the text
    /// just before it. Other HTML comments are dropped.
    Note {
        quote: String,
        text: String,
    },
    /// Text with a look only HTML gives (a color, a highlight, underline,
    /// superscript), from an HTML page's `style` attributes and tags such
    /// as `<mark>` and `<sup>`. Markdown never makes one.
    Styled {
        style: HtmlStyle,
        content: Vec<Inline>,
    },
}

/// The width and height in pixels an `<img>` asks for. With one of them
/// the picture keeps its proportions; with both it is drawn at that size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageSize {
    pub width: Option<u16>,
    pub height: Option<u16>,
}

/// The look an HTML page sets on a run of text, beyond bold, italic and
/// strikethrough (which become the Markdown styles). The preview keeps
/// author colors readable on the theme's page, see `render.rs`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HtmlStyle {
    /// Text color (`color`, `<font color>`).
    pub color: Option<Rgb>,
    /// Color behind the text (`background-color`, `<mark>`).
    pub background: Option<Rgb>,
    /// `text-decoration: underline`, `<u>` and `<ins>`.
    pub underline: bool,
    /// `<sup>` and `<sub>`.
    pub script: Option<Script>,
}

impl HtmlStyle {
    /// True when the style changes nothing.
    pub fn is_plain(&self) -> bool {
        *self == HtmlStyle::default()
    }

    /// This style inside `outer`: what this one sets wins, the rest comes
    /// from `outer`, as in CSS.
    pub fn inside(self, outer: HtmlStyle) -> HtmlStyle {
        HtmlStyle {
            color: self.color.or(outer.color),
            background: self.background.or(outer.background),
            underline: self.underline || outer.underline,
            script: self.script.or(outer.script),
        }
    }
}

/// Text set smaller, above or below the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    Superscript,
    Subscript,
}

/// Deepest nesting of inlines read as markup (links, emphasis, inline
/// HTML); text nested deeper stays as written, so a hostile file cannot
/// overflow the stack.
const MAX_INLINE_DEPTH: usize = 32;

/// Inline HTML tags read in Markdown text, with their end tag. Other tags
/// stay as written: `<T>` or `<name>` in prose is more often text than
/// markup, and showing it loses nothing.
const INLINE_HTML_TAGS: &[&str] = &[
    "a", "b", "cite", "code", "del", "em", "font", "i", "ins", "kbd", "mark", "s", "samp", "span", "strike", "strong",
    "sub", "sup", "u", "var",
];

/// Inline HTML tags that have no end tag.
const INLINE_VOID_TAGS: &[&str] = &["br", "img"];

/// Longest start tag looked for, in characters, so a stray `<` does not
/// copy the rest of a long paragraph.
const MAX_INLINE_TAG_LENGTH: usize = 2048;

/// Parse the text of a paragraph or heading. Lines are joined with `\n`.
pub fn parse_inlines(text: &str) -> Vec<Inline> {
    let chars: Vec<char> = text.chars().collect();
    parse_chars(&chars, 0)
}

/// Parse a slice of characters, `depth` levels inside other inlines; also
/// used for the content of nested inlines.
fn parse_chars(chars: &[char], depth: usize) -> Vec<Inline> {
    let mut output = Output { depth, ..Output::default() };
    if depth > MAX_INLINE_DEPTH {
        output.text.extend(chars);
        return output.finish();
    }
    let mut index = 0;
    while index < chars.len() {
        let next = match chars[index] {
            '\\' => parse_escape(chars, index, &mut output),
            '\n' => Some(parse_newline(index, &mut output)),
            '`' => Some(parse_code_span(chars, index, &mut output)),
            '<' => parse_comment(chars, index, &mut output)
                .or_else(|| parse_autolink(chars, index, &mut output))
                .or_else(|| parse_inline_html(chars, index, &mut output)),
            '[' => parse_link(chars, index, &mut output),
            '!' if chars.get(index + 1) == Some(&'[') => parse_image(chars, index, &mut output),
            '*' | '_' | '~' => Some(parse_delimiter_run(chars, index, &mut output)),
            _ => None,
        };
        if let Some(next) = next {
            index = next;
        } else {
            // Not markup: plain text.
            output.text.push(chars[index]);
            index += 1;
        }
    }
    output.finish()
}

/// Collects inlines, merging adjacent text.
#[derive(Default)]
struct Output {
    text: String,
    inlines: Vec<Inline>,
    /// How many inlines this text is nested in.
    depth: usize,
}

impl Output {
    /// Add a non-text inline after any pending text.
    fn push(&mut self, inline: Inline) {
        self.flush_text();
        self.inlines.push(inline);
    }

    /// Add several non-text inlines after any pending text.
    fn extend(&mut self, inlines: Vec<Inline>) {
        self.flush_text();
        self.inlines.extend(inlines);
    }

    /// Turn the pending text into an `Inline::Text`.
    fn flush_text(&mut self) {
        if !self.text.is_empty() {
            self.inlines.push(Inline::Text(std::mem::take(&mut self.text)));
        }
    }

    /// The collected inlines, including trailing text.
    fn finish(mut self) -> Vec<Inline> {
        self.flush_text();
        self.inlines
    }
}

/// `\*` gives a literal `*`; a backslash at the end of a line is a hard break.
fn parse_escape(chars: &[char], index: usize, output: &mut Output) -> Option<usize> {
    let next = *chars.get(index + 1)?;
    if next == '\n' {
        output.push(Inline::LineBreak);
        return Some(index + 2);
    }
    if !next.is_ascii_punctuation() {
        return None;
    }
    output.text.push(next);
    Some(index + 2)
}

/// Two or more trailing spaces make a hard break; otherwise a soft break,
/// which renders as a space.
fn parse_newline(index: usize, output: &mut Output) -> usize {
    let trimmed_length = output.text.trim_end_matches(' ').len();
    let trailing_spaces = output.text.len() - trimmed_length;
    output.text.truncate(trimmed_length);
    if trailing_spaces >= 2 {
        output.push(Inline::LineBreak);
    } else {
        output.text.push(' ');
    }
    index + 1
}

/// `` `code` ``: closed by a backtick run of the same length.
fn parse_code_span(chars: &[char], index: usize, output: &mut Output) -> usize {
    let run = run_length(chars, index);
    let content_start = index + run;
    let Some(close) = find_backtick_run(chars, content_start, run) else {
        output.text.extend(&chars[index..content_start]);
        return content_start;
    };
    let mut code: String = chars[content_start..close].iter().map(|&c| if c == '\n' { ' ' } else { c }).collect();
    // One space on each side lets code start or end with a backtick.
    if code.len() >= 2 && code.starts_with(' ') && code.ends_with(' ') && !code.trim().is_empty() {
        code = code[1..code.len() - 1].to_string();
    }
    output.push(Inline::Code(code));
    close + run
}

/// `<!-- ... -->`: hidden, except notes, which become [`Inline::Note`].
fn parse_comment(chars: &[char], index: usize, output: &mut Output) -> Option<usize> {
    let opener = ['<', '!', '-', '-'];
    if !chars[index..].starts_with(&opener) {
        return None;
    }
    let content_start = index + opener.len();
    let close = (content_start..chars.len().saturating_sub(2))
        .find(|&at| chars[at] == '-' && chars[at + 1] == '-' && chars[at + 2] == '>')?;
    let inner: String = chars[content_start..close].iter().collect();
    if let Some((quote, text)) = super::notes::parse_note(&inner) {
        output.push(Inline::Note { quote, text });
    }
    Some(close + 3)
}

/// Schemes the preview may hand to the system browser. Anything else
/// (`file://`, relative paths, custom app schemes) is shown but not opened.
const OPENABLE_SCHEMES: [&str; 3] = ["http://", "https://", "mailto:"];

/// True when a link target is a web or mail address that is safe to open.
pub fn is_openable_url(url: &str) -> bool {
    OPENABLE_SCHEMES.iter().any(|scheme| url.starts_with(scheme))
}

/// `<https://example.com>`
fn parse_autolink(chars: &[char], index: usize, output: &mut Output) -> Option<usize> {
    let close = index + 1 + chars[index + 1..].iter().position(|&c| c == '>')?;
    let url: String = chars[index + 1..close].iter().collect();
    if !is_openable_url(&url) || url.contains(char::is_whitespace) || url.contains('<') {
        return None;
    }
    output.push(Inline::Link { content: vec![Inline::Text(url.clone())], url });
    Some(close + 1)
}

/// An HTML tag in Markdown text: `<br>` and `<img ...>`, or one of
/// `INLINE_HTML_TAGS` with its matching end tag and the Markdown between
/// them. `None` (the `<` stays text) for other tags and unclosed ones.
fn parse_inline_html(chars: &[char], index: usize, output: &mut Output) -> Option<usize> {
    let window = &chars[index + 1..chars.len().min(index + 1 + MAX_INLINE_TAG_LENGTH)];
    let text: String = window.iter().collect();
    let tag = super::html_blocks::parse_start_tag(&text)?;
    let content_start = index + 1 + text[..tag.length].chars().count();
    if INLINE_VOID_TAGS.contains(&tag.name.as_str()) {
        output.extend(super::html_blocks::inline_tag(tag.name, tag.attributes, Vec::new()));
        return Some(content_start);
    }
    if !INLINE_HTML_TAGS.contains(&tag.name.as_str()) {
        return None;
    }
    if tag.self_closing {
        // `<span/>` holds nothing.
        return Some(content_start);
    }
    let (close, next) = find_end_tag(chars, content_start, &tag.name)?;
    let content = parse_chars(&chars[content_start..close], output.depth + 1);
    output.extend(super::html_blocks::inline_tag(tag.name, tag.attributes, content));
    Some(next)
}

/// Where the end tag `</name>` that matches a start tag ending at `from`
/// begins and ends; tags of the same name in between nest.
fn find_end_tag(chars: &[char], from: usize, name: &str) -> Option<(usize, usize)> {
    let name: Vec<char> = name.chars().collect();
    // `<name` followed by the end of the name, after `at`.
    let tag_named = |at: usize| {
        let name_end = at + name.len();
        name_end <= chars.len()
            && chars[at..name_end].iter().zip(&name).all(|(a, b)| a.eq_ignore_ascii_case(b))
            && chars.get(name_end).is_some_and(|c| c.is_whitespace() || *c == '>' || *c == '/')
    };
    let mut open = 0usize;
    let mut at = from;
    while at < chars.len() {
        if chars[at] == '<' {
            if chars.get(at + 1) == Some(&'/') && tag_named(at + 2) {
                let close = at + 2 + chars[at + 2..].iter().position(|&c| c == '>')?;
                if open == 0 {
                    return Some((at, close + 1));
                }
                open -= 1;
            } else if tag_named(at + 1) {
                open += 1;
                // Deeper than the parser reads anyway: stop here, so a run
                // of unclosed tags is not scanned to its end once per tag.
                if open > MAX_INLINE_DEPTH {
                    return None;
                }
            }
        }
        at += 1;
    }
    None
}

/// `[text](url "optional title")`
fn parse_link(chars: &[char], index: usize, output: &mut Output) -> Option<usize> {
    let (text_close, url, next) = link_parts(chars, index)?;
    let content = parse_chars(&chars[index + 1..text_close], output.depth + 1);
    output.push(Inline::Link { content, url });
    Some(next)
}

/// `![alt](url "optional title")`; the alt text is kept as plain text.
fn parse_image(chars: &[char], index: usize, output: &mut Output) -> Option<usize> {
    let (text_close, url, next) = link_parts(chars, index + 1)?;
    let alt = plain_text(&parse_chars(&chars[index + 2..text_close], output.depth + 1));
    output.push(Inline::Image { alt, url, size: ImageSize::default() });
    Some(next)
}

/// For `[text](url …)` starting at `open`: where the `]` is, the url, and
/// the index after the `)`.
fn link_parts(chars: &[char], open: usize) -> Option<(usize, String, usize)> {
    let text_close = find_closing(chars, open, '[', ']')?;
    if chars.get(text_close + 1) != Some(&'(') {
        return None;
    }
    let destination_close = find_closing(chars, text_close + 1, '(', ')')?;
    let destination: String = chars[text_close + 2..destination_close].iter().collect();
    // The title after the first whitespace is not shown.
    let url = destination.split_whitespace().next().unwrap_or("");
    let url = url.trim_start_matches('<').trim_end_matches('>').to_string();
    Some((text_close, url, destination_close + 1))
}

/// The text of `inlines` without any formatting.
pub fn plain_text(inlines: &[Inline]) -> String {
    let mut text = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(part) | Inline::Code(part) => text.push_str(part),
            Inline::Emphasis(children) | Inline::Strong(children) | Inline::Strikethrough(children) => {
                text.push_str(&plain_text(children));
            }
            Inline::Link { content, .. } | Inline::Styled { content, .. } => text.push_str(&plain_text(content)),
            Inline::Image { alt, .. } => text.push_str(alt),
            Inline::LineBreak => text.push(' '),
            Inline::Note { .. } => {}
        }
    }
    text
}

/// `*em*`, `_em_`, `**strong**`, `__strong__`, `***both***`, `~~strike~~`.
/// Always consumes the run; an unclosed run is kept as literal text.
fn parse_delimiter_run(chars: &[char], index: usize, output: &mut Output) -> usize {
    let delimiter = chars[index];
    let run = run_length(chars, index);
    let content_start = index + run;
    let literal = |output: &mut Output| {
        output.text.extend(&chars[index..content_start]);
        content_start
    };

    let valid_length = if delimiter == '~' { run == 2 } else { run <= 3 };
    let next_is_text = chars.get(content_start).is_some_and(|c| !c.is_whitespace());
    // Underscores inside words (snake_case) are not emphasis.
    let intraword_underscore = delimiter == '_' && index > 0 && chars[index - 1].is_alphanumeric();
    if !valid_length || !next_is_text || intraword_underscore {
        return literal(output);
    }
    let Some(close) = find_closing_run(chars, content_start, delimiter, run) else {
        return literal(output);
    };

    let content = parse_chars(&chars[content_start..close], output.depth + 1);
    let inline = match (delimiter, run) {
        ('~', _) => Inline::Strikethrough(content),
        (_, 1) => Inline::Emphasis(content),
        (_, 2) => Inline::Strong(content),
        _ => Inline::Strong(vec![Inline::Emphasis(content)]),
    };
    output.push(inline);
    close + run
}

/// Find a closing run of exactly `length` `delimiter`s that follows text.
fn find_closing_run(chars: &[char], start: usize, delimiter: char, length: usize) -> Option<usize> {
    let mut index = start;
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 2,
            '`' => index = skip_code_span(chars, index),
            c if c == delimiter => {
                let run = run_length(chars, index);
                if run == length && is_closing_run(chars, index, run, delimiter) {
                    return Some(index);
                }
                index += run;
            }
            _ => index += 1,
        }
    }
    None
}

/// A run closes emphasis when it follows text, and (for `_`) is not
/// followed by a word character.
fn is_closing_run(chars: &[char], index: usize, run: usize, delimiter: char) -> bool {
    let after_text = !chars[index - 1].is_whitespace();
    let next_is_word = chars.get(index + run).is_some_and(|c| c.is_alphanumeric());
    after_text && !(delimiter == '_' && next_is_word)
}

/// Index of the bracket closing the one at `open_index`, honoring nesting,
/// escapes, and code spans.
fn find_closing(chars: &[char], open_index: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    let mut index = open_index;
    while index < chars.len() {
        match chars[index] {
            '\\' => {
                index += 2;
                continue;
            }
            '`' => {
                index = skip_code_span(chars, index);
                continue;
            }
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Index after the code span starting at `index`, or after the backtick run
/// when it is not closed.
fn skip_code_span(chars: &[char], index: usize) -> usize {
    let run = run_length(chars, index);
    match find_backtick_run(chars, index + run, run) {
        Some(close) => close + run,
        None => index + run,
    }
}

/// Index of the next backtick run of exactly `length`, from `start`.
fn find_backtick_run(chars: &[char], start: usize, length: usize) -> Option<usize> {
    let mut index = start;
    while index < chars.len() {
        if chars[index] != '`' {
            index += 1;
            continue;
        }
        let run = run_length(chars, index);
        if run == length {
            return Some(index);
        }
        index += run;
    }
    None
}

/// How many times `chars[index]` repeats starting at `index`.
fn run_length(chars: &[char], index: usize) -> usize {
    chars[index..].iter().take_while(|&&c| c == chars[index]).count()
}

#[cfg(test)]
mod tests {
    use super::Inline::*;
    use super::*;

    fn text(value: &str) -> Inline {
        Text(value.to_string())
    }

    #[test]
    fn plain_text() {
        assert_eq!(parse_inlines("hello world"), vec![text("hello world")]);
    }

    #[test]
    fn images_keep_alt_text_and_url() {
        let image =
            |alt: &str, url: &str| Image { alt: alt.to_string(), url: url.to_string(), size: ImageSize::default() };
        assert_eq!(parse_inlines("![a *cat*](img/cat.png)"), vec![image("a cat", "img/cat.png")]);
        assert_eq!(
            parse_inlines("see ![](clip.mp4 \"Demo\") here"),
            vec![text("see "), image("", "clip.mp4"), text(" here")]
        );
        // A linked image: the image is the link's content.
        assert_eq!(
            parse_inlines("[![logo](logo.png)](https://example.com)"),
            vec![Link { content: vec![image("logo", "logo.png")], url: "https://example.com".to_string() }]
        );
        // A lone `!` or one before a plain bracket stays text.
        assert_eq!(parse_inlines("wow! [x]"), vec![text("wow! [x]")]);
    }

    #[test]
    fn emphasis_strong_strike() {
        assert_eq!(
            parse_inlines("*a* _b_ **c** __d__ ~~e~~ ***f***"),
            vec![
                Emphasis(vec![text("a")]),
                text(" "),
                Emphasis(vec![text("b")]),
                text(" "),
                Strong(vec![text("c")]),
                text(" "),
                Strong(vec![text("d")]),
                text(" "),
                Strikethrough(vec![text("e")]),
                text(" "),
                Strong(vec![Emphasis(vec![text("f")])]),
            ]
        );
    }

    #[test]
    fn nested_emphasis() {
        assert_eq!(
            parse_inlines("**bold *it* bold**"),
            vec![Strong(vec![text("bold "), Emphasis(vec![text("it")]), text(" bold")])]
        );
    }

    #[test]
    fn unclosed_and_spaced_delimiters_stay_literal() {
        assert_eq!(parse_inlines("*a"), vec![text("*a")]);
        assert_eq!(parse_inlines("2 * 3 * 4"), vec![text("2 * 3 * 4")]);
        assert_eq!(parse_inlines("**a*"), vec![text("**a*")]);
        assert_eq!(parse_inlines("~one~"), vec![text("~one~")]);
    }

    #[test]
    fn snake_case_is_not_emphasis() {
        assert_eq!(parse_inlines("my_var_name"), vec![text("my_var_name")]);
    }

    #[test]
    fn code_spans() {
        assert_eq!(parse_inlines("use `a*b*c` here"), vec![text("use "), Code("a*b*c".into()), text(" here")]);
        assert_eq!(parse_inlines("`` a`b ``"), vec![Code("a`b".into())]);
        assert_eq!(parse_inlines("`open"), vec![text("`open")]);
    }

    #[test]
    fn links() {
        assert_eq!(
            parse_inlines("see [the *docs*](https://x.io/a_(b) \"Title\")!"),
            vec![
                text("see "),
                Link { content: vec![text("the "), Emphasis(vec![text("docs")])], url: "https://x.io/a_(b)".into() },
                text("!"),
            ]
        );
        assert_eq!(parse_inlines("[not a link]"), vec![text("[not a link]")]);
    }

    #[test]
    fn only_web_and_mail_links_are_openable() {
        assert!(is_openable_url("https://a.io"));
        assert!(is_openable_url("http://a.io"));
        assert!(is_openable_url("mailto:me@a.io"));
        assert!(!is_openable_url("file:///etc/passwd"));
        assert!(!is_openable_url("other.md"));
        assert!(!is_openable_url("../docs/x.md"));
        assert!(!is_openable_url("javascript:alert(1)"));
        assert!(!is_openable_url("vscode://open"));
    }

    #[test]
    fn autolinks() {
        assert_eq!(
            parse_inlines("<https://a.io>"),
            vec![Link { content: vec![text("https://a.io")], url: "https://a.io".into() }]
        );
        assert_eq!(parse_inlines("<b>"), vec![text("<b>")]);
    }

    #[test]
    fn escapes() {
        assert_eq!(parse_inlines(r"\*not\* a\b"), vec![text(r"*not* a\b")]);
    }

    #[test]
    fn soft_and_hard_breaks() {
        assert_eq!(parse_inlines("a\nb"), vec![text("a b")]);
        assert_eq!(parse_inlines("a  \nb"), vec![text("a"), LineBreak, text("b")]);
        assert_eq!(parse_inlines("a\\\nb"), vec![text("a"), LineBreak, text("b")]);
    }

    #[test]
    fn inline_html_tags_take_markdown_inside() {
        assert_eq!(
            parse_inlines("<b>bold *and* it</b>, <I>i</I>, <s>s</s>, <del>d</del>"),
            vec![
                Strong(vec![text("bold "), Emphasis(vec![text("and")]), text(" it")]),
                text(", "),
                Emphasis(vec![text("i")]),
                text(", "),
                Strikethrough(vec![text("s")]),
                text(", "),
                Strikethrough(vec![text("d")]),
            ]
        );
        assert_eq!(
            parse_inlines("Press <kbd>Ctrl</kbd>+<code>C</code>"),
            vec![text("Press "), Code("Ctrl".into()), text("+"), Code("C".into())]
        );
        // Nested tags of the same name match their own end tags.
        assert_eq!(
            parse_inlines("<b>a<b>b</b>c</b>"),
            vec![Strong(vec![text("a"), Strong(vec![text("b")]), text("c")])]
        );
    }

    #[test]
    fn inline_html_styles_links_images_and_breaks() {
        let look = |style: HtmlStyle, value: &str| Styled { style, content: vec![text(value)] };
        let underline = HtmlStyle { underline: true, ..HtmlStyle::default() };
        let sup = HtmlStyle { script: Some(Script::Superscript), ..HtmlStyle::default() };
        let red = HtmlStyle { color: Some(Rgb(0xff, 0, 0)), ..HtmlStyle::default() };
        assert_eq!(
            parse_inlines("<u>u</u> x<sup>2</sup> <span style=\"color: red\">r</span><br/>next"),
            vec![look(underline, "u"), text(" x"), look(sup, "2"), text(" "), look(red, "r"), LineBreak, text("next")]
        );
        let mark = parse_inlines("<mark>m</mark>");
        assert!(matches!(&mark[..], [Styled { style, .. }] if style.background.is_some()));
        assert_eq!(
            parse_inlines("<a href=\"https://x.org\"><img src=\"logo.png\" alt=\"Logo\" height=\"20\"></a>"),
            vec![Link {
                content: vec![Image {
                    alt: "Logo".into(),
                    url: "logo.png".into(),
                    size: ImageSize { width: None, height: Some(20) }
                }],
                url: "https://x.org".into()
            }]
        );
    }

    #[test]
    fn other_tags_and_unclosed_ones_stay_as_written() {
        for source in
            ["Vec<String> and <T>", "<details>x</details>", "<b>never closed", "a </b> b", "<script>x()</script>"]
        {
            assert_eq!(parse_inlines(source), vec![text(source)], "{source}");
        }
    }

    #[test]
    fn comments_are_hidden() {
        assert_eq!(parse_inlines("a <!-- hidden --> b"), vec![text("a  b")]);
    }

    #[test]
    fn deep_nesting_stays_text_past_the_limit() {
        let source = format!("{}x{}", "<b>".repeat(10_000), "</b>".repeat(10_000));
        let mut inlines = parse_inlines(&source);
        let mut depth = 0;
        while let [Strong(inner)] = inlines.as_slice() {
            depth += 1;
            inlines = inner.clone();
        }
        assert!(depth <= MAX_INLINE_DEPTH + 1, "{depth}");
    }
}
