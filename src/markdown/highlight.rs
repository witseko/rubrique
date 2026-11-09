//! Find Markdown syntax in raw text so the editor can color it.
//!
//! Line-based and forgiving: it never fails, it only labels byte ranges. It
//! does not have to match the preview parser exactly; it only guides the eye.

use std::ops::Range;

/// What a run of text is, for coloring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Plain,
    /// Syntax characters: `#`, `>`, `-`, `**`, `` ` ``, `[`, `](`, fences, rules.
    Marker,
    Heading,
    Strong,
    Emphasis,
    Strikethrough,
    /// Code spans and the inside of fenced code blocks.
    Code,
    LinkText,
    Url,
    /// Text inside a block quote.
    Quote,
    /// `<!-- ... -->` comments, including notes; `#` comments in YAML.
    Comment,
    /// A YAML mapping key.
    Key,
    /// The inside of a Helm (Go template) action, `{{ ... }}`.
    Template,
    /// A language keyword in code (`def`, `return`, `true`), or an XML tag name.
    Keyword,
    /// A string literal in code, or an XML attribute value.
    String,
    /// A number literal in code.
    Number,
    /// A name being defined (`def name`, `class Name`), a Python decorator,
    /// or an XML attribute name.
    Name,
}

/// A labeled byte range of the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: Kind,
}

/// Label every byte of `text`. Spans are in order, touch each other, never
/// empty, and neighbors never share a kind.
pub fn highlight(text: &str) -> Vec<Span> {
    let mut out = Spans::default();
    let mut fence = None;
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        highlight_line(&mut out, line_start, content, &mut fence);
        line_start += line.len();
        out.to(line_start, Kind::Plain);
    }
    out.spans
}

/// Spans built front to back: each call labels everything up to a position.
#[derive(Default)]
pub(crate) struct Spans {
    pub(crate) spans: Vec<Span>,
    end: usize,
}

impl Spans {
    /// Label the bytes from the current end up to `upto` with `kind`.
    pub(crate) fn to(&mut self, upto: usize, kind: Kind) {
        if upto <= self.end {
            return;
        }
        match self.spans.last_mut() {
            Some(last) if last.kind == kind => last.range.end = upto,
            _ => self.spans.push(Span { range: self.end..upto, kind }),
        }
        self.end = upto;
    }
}

/// An open fenced code block: its fence character and length.
type Fence = (u8, usize);

/// Most leading spaces a block marker may have (more is indented code).
const MAX_MARKER_INDENT: usize = 3;
const MAX_HEADING_LEVEL: usize = 6;
const MIN_FENCE: usize = 3;
/// Longest ordered-list number, as in CommonMark.
const MAX_LIST_DIGITS: usize = 9;

/// Label one line (without its line break) that starts at byte `start`.
fn highlight_line(out: &mut Spans, start: usize, line: &str, fence: &mut Option<Fence>) {
    let end = start + line.len();
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];

    if let Some((fence_char, fence_len)) = *fence {
        let closes =
            indent <= MAX_MARKER_INDENT && fence_run(rest, fence_char) >= fence_len && is_blank_after_run(rest);
        if closes {
            *fence = None;
            out.to(end, Kind::Marker);
        } else {
            out.to(end, Kind::Code);
        }
        return;
    }
    if indent <= MAX_MARKER_INDENT {
        if let Some(opened) = fence_open(rest) {
            *fence = Some(opened);
            out.to(end, Kind::Marker);
            return;
        }
        if let Some(level) = heading_level(rest) {
            out.to(start + indent, Kind::Plain);
            out.to(start + indent + level, Kind::Marker);
            out.to(end, Kind::Heading);
            return;
        }
        if is_rule(rest) {
            out.to(end, Kind::Marker);
            return;
        }
    }
    let (body, base) = block_markers(out, start, line);
    highlight_inlines(out, start, line, body, base);
}

/// Label quote markers (`>`), then a list marker and task box. Returns where
/// the text starts and its base kind.
fn block_markers(out: &mut Spans, start: usize, line: &str) -> (usize, Kind) {
    let bytes = line.as_bytes();
    let mut base = Kind::Plain;
    let mut pos = 0;
    loop {
        let spaces = count_while(&bytes[pos..], b' ');
        if spaces <= MAX_MARKER_INDENT && bytes.get(pos + spaces) == Some(&b'>') {
            out.to(start + pos + spaces, base);
            out.to(start + pos + spaces + 1, Kind::Marker);
            pos += spaces + 1;
            base = Kind::Quote;
        } else {
            break;
        }
    }
    let spaces = count_while(&bytes[pos..], b' ');
    if let Some(marker_len) = list_marker(&line[pos + spaces..]) {
        out.to(start + pos + spaces, base);
        pos += spaces + marker_len;
        out.to(start + pos, Kind::Marker);
        let after = &line[pos..];
        let task = [" [ ]", " [x]", " [X]"].iter().any(|task| after.starts_with(task));
        if task {
            out.to(start + pos + 1, base);
            pos += " [ ]".len();
            out.to(start + pos, Kind::Marker);
        }
    }
    (pos, base)
}

/// Label emphasis, code spans, links and escapes from byte `from` to the end of the line.
fn highlight_inlines(out: &mut Spans, start: usize, line: &str, from: usize, base: Kind) {
    let bytes = line.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        let found = match bytes[i] {
            b'\\' if bytes.get(i + 1).is_some_and(u8::is_ascii_punctuation) => {
                out.to(start + i, base);
                out.to(start + i + 1, Kind::Marker);
                out.to(start + i + 2, base);
                Some(i + 2)
            }
            b'`' => Some(code_span(out, start, line, i, base)),
            b'*' | b'_' => emphasis(out, start, line, i, base),
            b'~' if bytes.get(i + 1) == Some(&b'~') => delimited(out, start, line, i, "~~", Kind::Strikethrough, base),
            b'<' if line[i..].starts_with("<!--") => comment(out, start, line, i, base),
            b'[' => link(out, start, line, i, base),
            b'<' => autolink(out, start, line, i, base),
            _ => None,
        };
        i = found.unwrap_or_else(|| i + line[i..].chars().next().map_or(1, char::len_utf8));
    }
    out.to(start + line.len(), base);
}

/// `` `code` `` with a closing run of the same length. Returns the end, or
/// skips the whole opening run when there is no closing one.
fn code_span(out: &mut Spans, start: usize, line: &str, i: usize, base: Kind) -> usize {
    let run = count_while(&line.as_bytes()[i..], b'`');
    let ticks = &line[i..i + run];
    let mut search = i + run;
    while let Some(found) = line[search..].find(ticks) {
        let close = search + found;
        let close_run = count_while(&line.as_bytes()[close..], b'`');
        if close_run == run {
            out.to(start + i, base);
            out.to(start + i + run, Kind::Marker);
            out.to(start + close, Kind::Code);
            out.to(start + close + run, Kind::Marker);
            return close + run;
        }
        search = close + close_run;
    }
    i + run
}

/// `**strong**`, `__strong__`, `*emphasis*` or `_emphasis_`.
fn emphasis(out: &mut Spans, start: usize, line: &str, i: usize, base: Kind) -> Option<usize> {
    let bytes = line.as_bytes();
    let delimiter = bytes[i];
    // `snake_case_names` are not emphasis.
    if delimiter == b'_' && i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
        return None;
    }
    let doubled = bytes.get(i + 1) == Some(&delimiter);
    let (marker, kind) = match (delimiter, doubled) {
        (b'*', true) => ("**", Kind::Strong),
        (b'_', true) => ("__", Kind::Strong),
        (b'*', false) => ("*", Kind::Emphasis),
        _ => ("_", Kind::Emphasis),
    };
    delimited(out, start, line, i, marker, kind, base)
}

/// Text between two copies of `marker`, not starting or ending with a space.
fn delimited(
    out: &mut Spans,
    start: usize,
    line: &str,
    i: usize,
    marker: &str,
    kind: Kind,
    base: Kind,
) -> Option<usize> {
    let inner_start = i + marker.len();
    let close = inner_start + line[inner_start..].find(marker)?;
    let inner = &line[inner_start..close];
    if inner.is_empty() || inner.starts_with(' ') || inner.ends_with(' ') {
        return None;
    }
    out.to(start + i, base);
    out.to(start + inner_start, Kind::Marker);
    out.to(start + close, kind);
    out.to(start + close + marker.len(), Kind::Marker);
    Some(close + marker.len())
}

/// `<!-- ... -->` on one line.
fn comment(out: &mut Spans, start: usize, line: &str, i: usize, base: Kind) -> Option<usize> {
    let end = i + line[i..].find("-->")? + "-->".len();
    out.to(start + i, base);
    out.to(start + end, Kind::Comment);
    Some(end)
}

/// `[text](url)`.
fn link(out: &mut Spans, start: usize, line: &str, i: usize, base: Kind) -> Option<usize> {
    let text_end = i + 1 + line[i + 1..].find(']')?;
    if !line[text_end..].starts_with("](") {
        return None;
    }
    let url_start = text_end + 2;
    let url_end = url_start + line[url_start..].find(')')?;
    out.to(start + i, base);
    out.to(start + i + 1, Kind::Marker);
    out.to(start + text_end, Kind::LinkText);
    out.to(start + url_start, Kind::Marker);
    out.to(start + url_end, Kind::Url);
    out.to(start + url_end + 1, Kind::Marker);
    Some(url_end + 1)
}

/// `<https://...>` and `<mailto:...>`.
fn autolink(out: &mut Spans, start: usize, line: &str, i: usize, base: Kind) -> Option<usize> {
    let rest = &line[i + 1..];
    if !["http://", "https://", "mailto:"].iter().any(|scheme| rest.starts_with(scheme)) {
        return None;
    }
    let close = i + 1 + rest.find('>')?;
    if line[i + 1..close].contains(' ') {
        return None;
    }
    out.to(start + i, base);
    out.to(start + i + 1, Kind::Marker);
    out.to(start + close, Kind::Url);
    out.to(start + close + 1, Kind::Marker);
    Some(close + 1)
}

/// Opening fence: three or more backticks or tildes.
fn fence_open(rest: &str) -> Option<Fence> {
    let fence_char = *rest.as_bytes().first().filter(|byte| matches!(byte, b'`' | b'~'))?;
    let len = fence_run(rest, fence_char);
    // A backtick fence's info string cannot hold backticks.
    let info_ok = fence_char == b'~' || !rest[len..].contains('`');
    (len >= MIN_FENCE && info_ok).then_some((fence_char, len))
}

/// Length of the run of `fence_char` at the start of `rest`.
fn fence_run(rest: &str, fence_char: u8) -> usize {
    count_while(rest.as_bytes(), fence_char)
}

/// True when only spaces follow the leading fence run.
fn is_blank_after_run(rest: &str) -> bool {
    rest.trim_start_matches(['`', '~']).trim().is_empty()
}

/// `#` to `######` followed by a space or the end of the line.
fn heading_level(rest: &str) -> Option<usize> {
    let level = count_while(rest.as_bytes(), b'#');
    let followed_ok = matches!(rest.as_bytes().get(level), None | Some(b' ' | b'\t'));
    ((1..=MAX_HEADING_LEVEL).contains(&level) && followed_ok).then_some(level)
}

/// `---`, `***` or `___`, three or more, spaces allowed between.
fn is_rule(rest: &str) -> bool {
    let compact: Vec<u8> = rest.bytes().filter(|byte| *byte != b' ' && *byte != b'\t').collect();
    compact.len() >= MIN_FENCE
        && matches!(compact[0], b'-' | b'*' | b'_')
        && compact.iter().all(|byte| *byte == compact[0])
}

/// Length of a list marker (`-`, `*`, `+`, `1.`, `1)`) that is followed by a
/// space or the end of the line.
fn list_marker(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let len = match bytes.first()? {
        b'-' | b'*' | b'+' => 1,
        _ => {
            let digits = count_while_digit(bytes);
            let ok = (1..=MAX_LIST_DIGITS).contains(&digits) && matches!(bytes.get(digits), Some(b'.' | b')'));
            if !ok {
                return None;
            }
            digits + 1
        }
    };
    matches!(bytes.get(len), None | Some(b' ' | b'\t')).then_some(len)
}

fn count_while(bytes: &[u8], wanted: u8) -> usize {
    bytes.iter().take_while(|byte| **byte == wanted).count()
}

fn count_while_digit(bytes: &[u8]) -> usize {
    bytes.iter().take_while(|byte| byte.is_ascii_digit()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text of every span with its kind, skipping plain text.
    fn marked(text: &str) -> Vec<(&str, Kind)> {
        highlight(text)
            .into_iter()
            .filter(|span| span.kind != Kind::Plain)
            .map(|span| (&text[span.range], span.kind))
            .collect()
    }

    #[test]
    fn spans_cover_the_whole_text_in_order() {
        let text = "# Title\n\nSome *em* and **strong**, `code` and [a](b).\r\n> quote\n- [x] done\n```\nx\n```\n";
        let spans = highlight(text);
        let mut end = 0;
        for pair in spans.windows(2) {
            assert_ne!(pair[0].kind, pair[1].kind, "{spans:?}");
        }
        for span in &spans {
            assert_eq!(span.range.start, end);
            assert!(span.range.end > span.range.start);
            end = span.range.end;
        }
        assert_eq!(end, text.len());
    }

    #[test]
    fn empty_text_has_no_spans() {
        assert_eq!(highlight(""), Vec::<Span>::new());
    }

    #[test]
    fn headings() {
        assert_eq!(marked("## Tasks"), [("##", Kind::Marker), (" Tasks", Kind::Heading)]);
        assert_eq!(marked("#hashtag"), []);
        assert_eq!(marked("####### seven"), []);
    }

    #[test]
    fn emphasis_strong_and_strikethrough() {
        assert_eq!(
            marked("a *b* **c** ~~d~~"),
            [
                ("*", Kind::Marker),
                ("b", Kind::Emphasis),
                ("*", Kind::Marker),
                ("**", Kind::Marker),
                ("c", Kind::Strong),
                ("**", Kind::Marker),
                ("~~", Kind::Marker),
                ("d", Kind::Strikethrough),
                ("~~", Kind::Marker),
            ]
        );
    }

    #[test]
    fn unclosed_or_spaced_delimiters_stay_plain() {
        assert_eq!(marked("a * b * c"), []);
        assert_eq!(marked("2 * 3"), []);
        assert_eq!(marked("**open"), []);
        assert_eq!(marked("snake_case_name"), []);
    }

    #[test]
    fn code_spans_need_a_matching_run() {
        assert_eq!(marked("`a`"), [("`", Kind::Marker), ("a", Kind::Code), ("`", Kind::Marker)]);
        assert_eq!(marked("``a ` b``"), [("``", Kind::Marker), ("a ` b", Kind::Code), ("``", Kind::Marker)]);
        assert_eq!(marked("`open"), []);
    }

    #[test]
    fn code_hides_other_syntax() {
        assert_eq!(marked("`*not em*`"), [("`", Kind::Marker), ("*not em*", Kind::Code), ("`", Kind::Marker)]);
    }

    #[test]
    fn links_and_autolinks() {
        assert_eq!(
            marked("[docs](https://x.org)"),
            [
                ("[", Kind::Marker),
                ("docs", Kind::LinkText),
                ("](", Kind::Marker),
                ("https://x.org", Kind::Url),
                (")", Kind::Marker),
            ]
        );
        assert_eq!(marked("<https://x.org>"), [("<", Kind::Marker), ("https://x.org", Kind::Url), (">", Kind::Marker)]);
        assert_eq!(marked("[just brackets]"), []);
    }

    #[test]
    fn fenced_code_blocks() {
        let text = "```rust\nlet a = *b*;\n```\nafter *x*";
        let marks = marked(text);
        assert_eq!(marks[0], ("```rust", Kind::Marker));
        assert_eq!(marks[1], ("let a = *b*;", Kind::Code));
        assert_eq!(marks[2], ("```", Kind::Marker));
        assert_eq!(marks[4], ("x", Kind::Emphasis));
    }

    #[test]
    fn unclosed_fence_runs_to_the_end() {
        assert_eq!(marked("~~~\n# not a heading"), [("~~~", Kind::Marker), ("# not a heading", Kind::Code)]);
    }

    #[test]
    fn quotes_lists_and_tasks() {
        assert_eq!(marked("> said"), [(">", Kind::Marker), (" said", Kind::Quote)]);
        assert_eq!(marked("- item"), [("-", Kind::Marker)]);
        assert_eq!(marked("12. item"), [("12.", Kind::Marker)]);
        assert_eq!(marked("  - [x] done"), [("-", Kind::Marker), ("[x]", Kind::Marker)]);
        assert_eq!(marked("-not a list"), []);
    }

    #[test]
    fn thematic_breaks() {
        assert_eq!(marked("---"), [("---", Kind::Marker)]);
        assert_eq!(marked("* * *"), [("* * *", Kind::Marker)]);
    }

    #[test]
    fn escapes_hide_the_next_character() {
        assert_eq!(marked(r"\*not em\*"), [("\\", Kind::Marker), ("\\", Kind::Marker)]);
    }

    #[test]
    fn non_ascii_text_is_safe() {
        let text = "Привіт *світ* — `код` ✓";
        let marks = marked(text);
        assert!(marks.contains(&("світ", Kind::Emphasis)));
        assert!(marks.contains(&("код", Kind::Code)));
    }

    #[test]
    fn comments_and_notes_are_dimmed() {
        assert_eq!(marked("a b<!-- note on \"b\": hi --> c"), [("<!-- note on \"b\": hi -->", Kind::Comment)]);
        assert_eq!(marked("<!-- open"), []);
    }
}
