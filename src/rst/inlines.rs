//! Inline reStructuredText: emphasis, strong, literals, interpreted text
//! and roles, hyperlink references, substitutions, footnote references and
//! standalone URLs, read into the Markdown preview's inlines.
//!
//! RST inline markup does not nest, so the content of `*...*` or
//! `` ``...`` `` is plain text. Markup starts only after whitespace or
//! opening punctuation and ends only before whitespace or closing
//! punctuation, which keeps `snake_case` and `2*3*4` as text.

use super::refs::{Refs, Substitution};
use crate::markdown::inlines::{HtmlStyle, ImageSize, Inline, Script};

/// Deepest substitution inside a substitution that is expanded, so a
/// definition that uses itself ends.
const MAX_SUBSTITUTION_DEPTH: usize = 4;

/// Characters that may come right before inline markup.
const START_BEFORE: &str = "'\"([{<-/:\u{2018}\u{201c}\u{2019}\u{ab}\u{a1}\u{bf}";

/// Characters that may come right after inline markup.
const END_AFTER: &str = "'\")]}>-/:.,;!?\\\u{2019}\u{201d}\u{bb}";

/// Schemes recognized as standalone links in running text.
const URL_SCHEMES: [&str; 4] = ["https://", "http://", "ftp://", "mailto:"];

/// Trailing characters that end a sentence rather than a standalone URL.
const URL_TRAILING: &str = ".,;:!?)'\"";

/// Roles shown as code.
const CODE_ROLES: &[&str] = &[
    "code", "literal", "command", "file", "samp", "kbd", "program", "envvar", "option", "makevar", "mimetype",
    "regexp", "math",
];

/// Roles shown as their text (cross references within a Sphinx project,
/// which link nowhere in a single file).
const TEXT_ROLES: &[&str] = &["ref", "doc", "any", "term", "numref", "guilabel", "menuselection", "abbr", "dfn"];

/// Parse the text of a paragraph, heading or cell.
pub(super) fn parse_inlines(text: &str, refs: &mut Refs) -> Vec<Inline> {
    parse_at_depth(text, refs, 0)
}

/// Inlines being built: finished inlines plus the text not yet pushed.
#[derive(Default)]
struct Output {
    inlines: Vec<Inline>,
    text: String,
}

impl Output {
    fn flush(&mut self) {
        if !self.text.is_empty() {
            self.inlines.push(Inline::Text(std::mem::take(&mut self.text)));
        }
    }

    fn push(&mut self, inlines: Vec<Inline>) {
        self.flush();
        self.inlines.extend(inlines);
    }

    fn finish(mut self) -> Vec<Inline> {
        self.flush();
        self.inlines
    }
}

fn parse_at_depth(text: &str, refs: &mut Refs, depth: usize) -> Vec<Inline> {
    let chars: Vec<char> = text.chars().collect();
    let mut output = Output::default();
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if character == '\\' {
            // `\x` is a literal x; a backslash before whitespace vanishes with it.
            match chars.get(index + 1) {
                Some(next) if !next.is_whitespace() => output.text.push(*next),
                _ => {}
            }
            index += 2;
            continue;
        }
        let before = index.checked_sub(1).map(|at| chars[at]);
        if can_start(before)
            && let Some((inlines, next)) = markup_at(&chars, index, refs, depth)
        {
            output.push(inlines);
            index = next;
            continue;
        }
        if character == '_'
            && let Some(next) = word_reference(&chars, index, &mut output, refs)
        {
            index = next;
            continue;
        }
        output.text.push(character);
        index += 1;
    }
    output.finish()
}

/// True when inline markup may start after `before`.
fn can_start(before: Option<char>) -> bool {
    before.is_none_or(|character| character.is_whitespace() || START_BEFORE.contains(character))
}

/// True when inline markup may end before `after`.
fn can_end(after: Option<char>) -> bool {
    after.is_none_or(|character| character.is_whitespace() || END_AFTER.contains(character))
}

/// The markup starting at `chars[index]` and the index after it.
fn markup_at(chars: &[char], index: usize, refs: &mut Refs, depth: usize) -> Option<(Vec<Inline>, usize)> {
    let rest = &chars[index..];
    if rest.starts_with(&['`', '`']) {
        let end = find_end(chars, index + 2, &['`', '`'])?;
        return Some((vec![Inline::Code(collect(&chars[index + 2..end]))], end + 2));
    }
    if rest.starts_with(&['*', '*']) {
        let end = find_end(chars, index + 2, &['*', '*'])?;
        return Some((vec![Inline::Strong(vec![Inline::Text(collect(&chars[index + 2..end]))])], end + 2));
    }
    match rest[0] {
        '*' => {
            let end = find_end(chars, index + 1, &['*'])?;
            Some((vec![Inline::Emphasis(vec![Inline::Text(collect(&chars[index + 1..end]))])], end + 1))
        }
        '`' => interpreted_text(chars, index + 1, None, refs),
        ':' => role(chars, index, refs),
        '|' => substitution_reference(chars, index, refs, depth),
        '[' => footnote_reference(chars, index, refs),
        _ => standalone_url(chars, index),
    }
}

/// Start of the end string `end` for markup whose content starts at
/// `from`: content is not empty, does not end in whitespace, and the end
/// string is followed by whitespace, closing punctuation or nothing.
fn find_end(chars: &[char], from: usize, end: &[char]) -> Option<usize> {
    if chars.get(from).is_none_or(|first| first.is_whitespace()) {
        return None;
    }
    (from + 1..chars.len()).find(|&at| {
        chars[at..].starts_with(end)
            && !chars[at - 1].is_whitespace()
            && (end == ['`', '`'].as_slice() || chars[at - 1] != '\\')
            && can_end(chars.get(at + end.len()).copied())
    })
}

fn collect(chars: &[char]) -> String {
    chars.iter().collect()
}

/// What follows the closing backquote of interpreted text.
enum Suffix {
    None,
    Reference,
    AnonymousReference,
}

/// `` `text` `` (title reference), `` `text`_ `` and `` `text <url>`_ ``
/// (links), with the content starting at `from`. A `role` given by a
/// `:role:` prefix decides how plain interpreted text shows.
fn interpreted_text(chars: &[char], from: usize, role: Option<&str>, refs: &mut Refs) -> Option<(Vec<Inline>, usize)> {
    if chars.get(from).is_none_or(|first| first.is_whitespace()) {
        return None;
    }
    let (close, suffix, next) = (from + 1..chars.len()).find_map(|at| backquote_end(chars, at))?;
    let content = collect(&chars[from..close]);
    let inlines = match (suffix, role) {
        (Suffix::None, Some(role)) => role_inlines(role, &content),
        (Suffix::None, None) => vec![Inline::Emphasis(vec![Inline::Text(content)])],
        (Suffix::Reference, _) => reference(&content, false, refs),
        (Suffix::AnonymousReference, _) => reference(&content, true, refs),
    };
    Some((inlines, next))
}

/// When `chars[at]` closes interpreted text: its index, the suffix after
/// it and the index after the suffix.
fn backquote_end(chars: &[char], at: usize) -> Option<(usize, Suffix, usize)> {
    if chars[at] != '`' || chars[at - 1].is_whitespace() {
        return None;
    }
    let after = &chars[at + 1..];
    let (suffix, length) = if after.starts_with(&['_', '_']) {
        (Suffix::AnonymousReference, 2)
    } else if after.starts_with(&['_']) {
        (Suffix::Reference, 1)
    } else {
        (Suffix::None, 0)
    };
    let next = at + 1 + length;
    can_end(chars.get(next).copied()).then_some((at, suffix, next))
}

/// A hyperlink reference's inlines: a link when its target is known, its
/// text otherwise. `content` is `name` or `text <url>`.
fn reference(content: &str, anonymous: bool, refs: &mut Refs) -> Vec<Inline> {
    let (text, url) = match embedded_url(content) {
        Some((text, url)) => {
            let url = match url.strip_suffix('_') {
                Some(alias) => refs.target(alias).map(str::to_string),
                None => Some(url.split_whitespace().collect()),
            };
            (text, url)
        }
        None if anonymous => (content.to_string(), refs.next_anonymous_target()),
        None => (content.to_string(), refs.target(content).map(str::to_string)),
    };
    link_or_text(text, url)
}

/// Split `text <url>` into its text (the URL when there is none) and URL.
fn embedded_url(content: &str) -> Option<(String, String)> {
    let inner = content.strip_suffix('>')?;
    let open = inner.rfind('<')?;
    let text = inner[..open].trim();
    if open > 0 && text.len() == inner[..open].len() {
        // `a<b>` without a space before `<` is a name, not an embedded URL.
        return None;
    }
    let url = inner[open + 1..].trim().to_string();
    let text = if text.is_empty() { url.clone() } else { text.to_string() };
    Some((text, url))
}

fn link_or_text(text: String, url: Option<String>) -> Vec<Inline> {
    match url {
        Some(url) => vec![Inline::Link { content: vec![Inline::Text(text)], url }],
        None => vec![Inline::Text(text)],
    }
}

/// ``:role:`text` `` at `chars[index]`.
fn role(chars: &[char], index: usize, refs: &mut Refs) -> Option<(Vec<Inline>, usize)> {
    let name_length = chars[index + 1..]
        .iter()
        .take_while(|character| character.is_ascii_alphanumeric() || "-_.:+".contains(**character))
        .count();
    let open = index + 1 + name_length;
    if name_length < 2 || chars[open - 1] != ':' || chars.get(open) != Some(&'`') {
        return None;
    }
    let name = collect(&chars[index + 1..open - 1]).to_ascii_lowercase();
    interpreted_text(chars, open + 1, Some(&name), refs)
}

/// How a role's text shows.
fn role_inlines(role: &str, content: &str) -> Vec<Inline> {
    // Sphinx domains prefix roles (`py:func`); the last part names it.
    let short = role.rsplit(':').next().unwrap_or(role);
    let text = || vec![Inline::Text(content.to_string())];
    match short {
        "emphasis" | "title-reference" | "title" | "t" => vec![Inline::Emphasis(text())],
        "strong" => vec![Inline::Strong(text())],
        "sub" | "subscript" => vec![scripted(Script::Subscript, content)],
        "sup" | "superscript" => vec![scripted(Script::Superscript, content)],
        "pep" => link_or_text(format!("PEP {content}"), pep_url(content)),
        "rfc" => link_or_text(format!("RFC {content}"), rfc_url(content)),
        _ if CODE_ROLES.contains(&short) => vec![Inline::Code(content.to_string())],
        _ if TEXT_ROLES.contains(&short) => vec![Inline::Text(cross_reference_text(content))],
        // Other roles name code objects (`:func:`, `:class:`, `:meth:`).
        _ => vec![Inline::Code(object_name(content))],
    }
}

fn scripted(script: Script, content: &str) -> Inline {
    let style = HtmlStyle { script: Some(script), ..HtmlStyle::default() };
    Inline::Styled { style, content: vec![Inline::Text(content.to_string())] }
}

fn pep_url(number: &str) -> Option<String> {
    let number: u32 = number.trim().parse().ok()?;
    Some(format!("https://peps.python.org/pep-{number:04}/"))
}

fn rfc_url(number: &str) -> Option<String> {
    let number: u32 = number.trim().parse().ok()?;
    Some(format!("https://datatracker.ietf.org/doc/html/rfc{number}"))
}

/// The text of a cross reference: `title <target>` shows the title, an
/// abbreviation `LIFO (last in, first out)` shows `LIFO`.
fn cross_reference_text(content: &str) -> String {
    if let Some((text, _)) = embedded_url(content) {
        return text;
    }
    match content.split_once(" (") {
        Some((short, _)) if content.ends_with(')') => short.to_string(),
        _ => content.to_string(),
    }
}

/// The name a code role shows: `~a.b.c` shows `c`, a leading `!` is dropped,
/// and `title <target>` shows the title.
fn object_name(content: &str) -> String {
    if let Some((text, _)) = embedded_url(content) {
        return text;
    }
    let content = content.trim_start_matches('!');
    match content.strip_prefix('~') {
        Some(path) => path.rsplit('.').next().unwrap_or(path).to_string(),
        None => content.to_string(),
    }
}

/// `|name|`, `|name|_` and `|name|__` at `chars[index]`.
fn substitution_reference(chars: &[char], index: usize, refs: &mut Refs, depth: usize) -> Option<(Vec<Inline>, usize)> {
    let close = (index + 2..chars.len()).find(|&at| chars[at] == '|' && !chars[at - 1].is_whitespace())?;
    if chars[index + 1].is_whitespace() {
        return None;
    }
    let after = &chars[close + 1..];
    let link_length = after.iter().take(2).take_while(|character| **character == '_').count();
    let next = close + 1 + link_length;
    if !can_end(chars.get(next).copied()) {
        return None;
    }
    let name = collect(&chars[index + 1..close]);
    let content = match refs.substitution(&name).cloned() {
        Some(Substitution::Text(text)) if depth < MAX_SUBSTITUTION_DEPTH => parse_at_depth(&text, refs, depth + 1),
        Some(Substitution::Text(text)) => vec![Inline::Text(text)],
        Some(Substitution::Image { url, alt }) => vec![Inline::Image { alt, url, size: ImageSize::default() }],
        None => vec![Inline::Text(collect(&chars[index..=close]))],
    };
    let url = match link_length {
        0 => None,
        1 => refs.target(&name).map(str::to_string),
        _ => refs.next_anonymous_target(),
    };
    let inlines = match url {
        Some(url) => vec![Inline::Link { content, url }],
        None => content,
    };
    Some((inlines, next))
}

/// `[1]_`, `[#]_`, `[#name]_`, `[*]_` or `[Citation]_` at `chars[index]`,
/// shown as a superscript label.
fn footnote_reference(chars: &[char], index: usize, refs: &mut Refs) -> Option<(Vec<Inline>, usize)> {
    let close = (index + 1..chars.len()).find(|&at| chars[at] == ']')?;
    let label = collect(&chars[index + 1..close]);
    let valid = !label.is_empty() && !label.contains(char::is_whitespace);
    if !valid || chars.get(close + 1) != Some(&'_') || !can_end(chars.get(close + 2).copied()) {
        return None;
    }
    let shown = refs.footnote_reference_label(&label);
    Some((vec![scripted(Script::Superscript, &shown)], close + 2))
}

/// A bare `https://...` link at `chars[index]`.
fn standalone_url(chars: &[char], index: usize) -> Option<(Vec<Inline>, usize)> {
    let rest = &chars[index..];
    let scheme = URL_SCHEMES.iter().find(|scheme| rest.starts_with(&scheme.chars().collect::<Vec<_>>()))?;
    let mut end =
        index + rest.iter().take_while(|character| !character.is_whitespace() && !"<>`".contains(**character)).count();
    while end > index && URL_TRAILING.contains(chars[end - 1]) {
        end -= 1;
    }
    if end <= index + scheme.len() {
        return None;
    }
    let url = collect(&chars[index..end]);
    Some((vec![Inline::Link { content: vec![Inline::Text(url.clone())], url }], end))
}

/// A `word_` or `word__` reference ending at the `_` at `chars[index]`:
/// takes the word back out of `output`'s text and pushes a link (or the
/// word, when its target is unknown). `None` when this `_` is just text.
fn word_reference(chars: &[char], index: usize, output: &mut Output, refs: &mut Refs) -> Option<usize> {
    let anonymous = chars.get(index + 1) == Some(&'_');
    let next = index + if anonymous { 2 } else { 1 };
    if !can_end(chars.get(next).copied()) {
        return None;
    }
    let word_start = word_start(&output.text)?;
    let word = output.text.split_off(word_start);
    let url = if anonymous { refs.next_anonymous_target() } else { refs.target(&word).map(str::to_string) };
    output.push(link_or_text(word, url));
    Some(next)
}

/// Where the reference name at the end of `text` starts: a run of letters,
/// digits and inner `-_.+`, starting with a letter or digit, after
/// whitespace, opening punctuation or nothing.
fn word_start(text: &str) -> Option<usize> {
    let mut start = text.len();
    for (at, character) in text.char_indices().rev() {
        if character.is_alphanumeric() || "-_.+".contains(character) {
            start = at;
        } else {
            break;
        }
    }
    // The name starts with a letter or digit.
    let offset = text[start..].find(char::is_alphanumeric)?;
    if offset > 0 {
        // Punctuation glued to the front (`__init_`) means this is not a name.
        return None;
    }
    let before = text[..start].chars().next_back();
    can_start(before).then_some(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Vec<Inline> {
        let lines: Vec<&str> = text.lines().collect();
        let mut refs = Refs::collect(&lines);
        // Only the first line is the paragraph under test; the rest define targets.
        parse_inlines(lines[0], &mut refs)
    }

    fn text(value: &str) -> Inline {
        Inline::Text(value.to_string())
    }

    fn link(value: &str, url: &str) -> Inline {
        Inline::Link { content: vec![text(value)], url: url.to_string() }
    }

    fn sup(value: &str) -> Inline {
        scripted(Script::Superscript, value)
    }

    #[test]
    fn emphasis_strong_and_literals() {
        assert_eq!(
            parse("*a* **b** ``c *d*``"),
            [
                Inline::Emphasis(vec![text("a")]),
                text(" "),
                Inline::Strong(vec![text("b")]),
                text(" "),
                Inline::Code("c *d*".to_string())
            ]
        );
    }

    #[test]
    fn markup_needs_word_boundaries() {
        assert_eq!(parse("2*3*4 and snake_case_name"), [text("2*3*4 and snake_case_name")]);
        assert_eq!(parse("* not emphasis *"), [text("* not emphasis *")]);
        assert_eq!(parse("(*yes*)."), [text("("), Inline::Emphasis(vec![text("yes")]), text(").")]);
    }

    #[test]
    fn backslash_escapes() {
        assert_eq!(parse(r"\*not\* a\ b"), [text("*not* ab")]);
    }

    #[test]
    fn hyperlink_references() {
        assert_eq!(
            parse("Read `the docs <https://docs.rs>`_ or Rust_.\n.. _rust: https://rust-lang.org"),
            [
                text("Read "),
                link("the docs", "https://docs.rs"),
                text(" or "),
                link("Rust", "https://rust-lang.org"),
                text(".")
            ]
        );
        assert_eq!(
            parse("`Two words`_ and unknown_\n.. _two words: https://x.org"),
            [link("Two words", "https://x.org"), text(" and "), text("unknown")]
        );
        assert_eq!(parse("`anon`__\n__ https://a.org"), [link("anon", "https://a.org")]);
        assert_eq!(parse("`<https://b.org>`_"), [link("https://b.org", "https://b.org")]);
    }

    #[test]
    fn dunder_names_are_not_references() {
        assert_eq!(parse("call __init__ now"), [text("call __init__ now")]);
    }

    #[test]
    fn standalone_urls_drop_trailing_punctuation() {
        assert_eq!(
            parse("See https://example.com/a_b."),
            [text("See "), link("https://example.com/a_b", "https://example.com/a_b"), text(".")]
        );
    }

    #[test]
    fn roles() {
        assert_eq!(parse(":code:`x = 1`"), [Inline::Code("x = 1".to_string())]);
        assert_eq!(parse(":py:func:`~pkg.mod.run`"), [Inline::Code("run".to_string())]);
        assert_eq!(parse(":ref:`Intro <intro>`"), [text("Intro")]);
        assert_eq!(parse("H\\ :sub:`2`\\ O"), [text("H"), scripted(Script::Subscript, "2"), text("O")]);
        assert_eq!(parse(":pep:`8`"), [link("PEP 8", "https://peps.python.org/pep-0008/")]);
        assert_eq!(parse("`title`"), [Inline::Emphasis(vec![text("title")])]);
    }

    #[test]
    fn substitutions() {
        assert_eq!(
            parse("Hi |name|!\n.. |name| replace:: **Rubrique**"),
            [text("Hi "), Inline::Strong(vec![text("Rubrique")]), text("!")]
        );
        assert_eq!(parse("|loop|\n.. |loop| replace:: |loop|"), [text("|loop|")]);
        assert_eq!(parse("|unknown| stays"), [text("|unknown|"), text(" stays")]);
    }

    #[test]
    fn footnote_references() {
        assert_eq!(
            parse("Fact [1]_ and [#]_.\n.. [#] auto"),
            [text("Fact "), sup("1"), text(" and "), sup("1"), text(".")]
        );
    }
}
