//! Notes on text, stored in the Markdown itself as hidden comments placed
//! right after the text they are about:
//!
//! `the first Monday<!-- note on "the first Monday": check with legal -->`
//!
//! GitHub and other viewers hide the comment; Rubrique highlights the quoted text.

use std::ops::Range;

/// Start of every note comment.
const NOTE_START: &str = "<!-- note on \"";
/// Start of a voice note comment.
const VOICE_NOTE_START: &str = "<!-- voice note on \"";
/// Separates the quoted text from the note.
const QUOTE_END: &str = "\": ";
const COMMENT_END: &str = "-->";
/// Characters of Markdown syntax skipped when matching a selection to the source.
const MARKUP: [char; 4] = ['*', '_', '~', '`'];

/// A note found in the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// The text the note is about.
    pub quote: String,
    /// What the note says or where its recording lives.
    pub content: NoteContent,
    /// Byte range of the whole `<!-- ... -->` comment in the source.
    pub range: Range<usize>,
}

/// The contents of a text or voice note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteContent {
    Text(String),
    Voice(String),
}

/// Read a text or voice note from the inside of an HTML comment.
pub fn parse_note(inner: &str) -> Option<(String, NoteContent)> {
    let inner = inner.trim();
    let (rest, voice) = if let Some(rest) = inner.strip_prefix("voice note on \"") {
        (rest, true)
    } else {
        (inner.strip_prefix("note on \"")?, false)
    };
    let (quote, text) = rest.split_once(QUOTE_END)?;
    let text = text.trim().to_string();
    let content = if voice { NoteContent::Voice(text) } else { NoteContent::Text(text) };
    Some((quote.to_string(), content))
}

/// A quick check for the opening of a note comment.
pub fn has_notes(source: &str) -> bool {
    source.contains(NOTE_START) || source.contains(VOICE_NOTE_START)
}

/// Every note in `source`, in order.
pub fn find_notes(source: &str) -> Vec<Note> {
    let mut notes = Vec::new();
    let mut from = 0;
    while let Some(start) = source[from..].find("<!--").map(|offset| from + offset) {
        let Some(end) = source[start..].find(COMMENT_END).map(|offset| start + offset + COMMENT_END.len()) else {
            break;
        };
        let inner = &source[start + "<!--".len()..end - COMMENT_END.len()];
        if (source[start..].starts_with(NOTE_START) || source[start..].starts_with(VOICE_NOTE_START))
            && let Some((quote, content)) = parse_note(inner)
        {
            notes.push(Note { quote, content, range: start..end });
        }
        from = end;
    }
    notes
}

/// The comment for a note. Line breaks become spaces, and `-->` or `": ` in
/// the text is broken up so the comment stays one valid, readable note.
pub fn note_comment(quote: &str, text: &str) -> String {
    comment(NOTE_START, quote, text)
}

/// A hidden note comment holding a recording's relative URL.
pub fn voice_note_comment(quote: &str, url: &str) -> String {
    comment(VOICE_NOTE_START, &quote.replace("--!>", "- -!>"), url)
}

/// Format a note without letting either field close the comment.
fn comment(start: &str, quote: &str, text: &str) -> String {
    let clean = |value: &str| value.split_whitespace().collect::<Vec<_>>().join(" ").replace(COMMENT_END, "- ->");
    let quote = clean(quote).replace(QUOTE_END, "\" : ");
    format!("{start}{quote}{QUOTE_END}{} {COMMENT_END}", clean(text))
}

/// Find where the selected preview text sits in the source. The preview
/// hides syntax such as `**` and `_`, so those are skipped in the source,
/// and any run of whitespace matches any other. Returns the first match.
pub fn locate(source: &str, selected: &str) -> Option<Range<usize>> {
    locate_matches(source, selected).next()
}

/// All source ranges that match a selection, including overlaps.
pub fn locate_matches(source: &str, selected: &str) -> impl Iterator<Item = Range<usize>> {
    let wanted: Vec<char> = selected.split_whitespace().collect::<Vec<_>>().join(" ").chars().collect();
    let chars: Vec<(usize, char)> = source.char_indices().collect();
    (0..chars.len()).filter_map(move |start| {
        if Some(&chars[start].1) != wanted.first() {
            return None;
        }
        let end = match_from(&chars, start, &wanted)?;
        let end_byte = chars.get(end).map_or(source.len(), |(offset, _)| *offset);
        Some(chars[start].0..end_byte)
    })
}

/// Match `wanted` against the source starting at `chars[start]`; returns the
/// index just after the last matched character.
fn match_from(chars: &[(usize, char)], start: usize, wanted: &[char]) -> Option<usize> {
    let mut index = start;
    for &want in wanted {
        if want == ' ' {
            if !chars.get(index)?.1.is_whitespace() {
                // Markup can sit between words: `**a** b`.
                while chars.get(index).is_some_and(|(_, c)| MARKUP.contains(c)) {
                    index += 1;
                }
                if !chars.get(index)?.1.is_whitespace() {
                    return None;
                }
            }
            let mut new_line = false;
            while let Some((_, c)) = chars.get(index).filter(|(_, c)| c.is_whitespace()) {
                new_line |= *c == '\n';
                index += 1;
            }
            // A selection across list items or quote lines: the preview does
            // not show the `- `, `1. `, `- [ ] ` or `> ` that start each line.
            if new_line {
                index = skip_line_marker(chars, index);
            }
            continue;
        }
        while chars.get(index).is_some_and(|(_, c)| MARKUP.contains(c) && *c != want) {
            index += 1;
        }
        if chars.get(index)?.1 != want {
            return None;
        }
        index += 1;
    }
    Some(index)
}

/// Skip what starts a Markdown line but is not shown as text: quote marks,
/// a list marker (`-`, `*`, `+`, `1.`, `1)`) and a task box (`[ ]`, `[x]`),
/// each with the spaces after it. Returns `index` when there is none.
fn skip_line_marker(chars: &[(usize, char)], index: usize) -> usize {
    let at = |i: usize| chars.get(i).map(|(_, c)| *c);
    let skip_spaces = |mut i: usize| {
        while at(i).is_some_and(|c| c == ' ' || c == '\t') {
            i += 1;
        }
        i
    };
    let mut i = index;
    while at(i) == Some('>') {
        i = skip_spaces(i + 1);
    }
    let marker_end = match at(i) {
        Some('-' | '*' | '+') => Some(i + 1),
        Some(c) if c.is_ascii_digit() => {
            let mut j = i;
            while at(j).is_some_and(|c| c.is_ascii_digit()) {
                j += 1;
            }
            matches!(at(j), Some('.' | ')')).then_some(j + 1)
        }
        _ => None,
    };
    // A marker counts only when a space follows it (`-x` is text).
    if let Some(end) = marker_end.filter(|&end| at(end).is_some_and(|c| c == ' ' || c == '\t')) {
        i = skip_spaces(end);
        if at(i) == Some('[') && matches!(at(i + 1), Some(' ' | 'x' | 'X')) && at(i + 2) == Some(']') {
            i = skip_spaces(i + 3);
        }
    }
    i
}

/// Add a note about `quote`, right after the text at `range`.
pub fn insert_note(source: &mut String, range: &Range<usize>, quote: &str, text: &str) {
    source.insert_str(range.end, &note_comment(quote, text));
}

/// Remove a note's comment from the source.
pub fn remove_note(source: &mut String, note: &Note) {
    source.replace_range(note.range.clone(), "");
}

/// Change what a note says.
pub fn rewrite_note(source: &mut String, note: &Note, text: &str) {
    source.replace_range(note.range.clone(), &note_comment(&note.quote, text));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_round_trip() {
        let comment = note_comment("the first\nMonday", "check -->\n with legal");
        assert_eq!(comment, "<!-- note on \"the first Monday\": check - -> with legal -->");
        let notes = find_notes(&format!("We ship on the first Monday{comment} after."));
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].quote, "the first Monday");
        assert_eq!(notes[0].content, NoteContent::Text("check - -> with legal".to_string()));
    }

    #[test]
    fn plain_comments_are_not_notes() {
        assert_eq!(find_notes("a <!-- todo --> b <!-- note on \"b\": yes -->").len(), 1);
        assert_eq!(parse_note(" note on \"x\": y "), Some(("x".to_string(), NoteContent::Text("y".to_string()))));
        assert_eq!(parse_note("just a comment"), None);
    }

    #[test]
    fn locates_selected_text_through_markup() {
        let source = "We ship on **the first** Monday of\nMarch, `soon`.";
        let range = locate(source, "the first Monday of March").unwrap();
        assert_eq!(&source[range], "the first** Monday of\nMarch");
        let range = locate(source, "soon").unwrap();
        assert_eq!(&source[range], "soon");
        assert_eq!(locate(source, "not there"), None);
        assert_eq!(locate(source, "   "), None);
    }

    #[test]
    fn locates_selections_across_list_items() {
        let bullets = "Intro\n\n- first item\n- second *item*\n- third\n";
        let range = locate(bullets, "first item second item third").unwrap();
        assert_eq!(&bullets[range], "first item\n- second *item*\n- third");

        let numbered = "1. alpha one\n2) beta two\n10. gamma\n";
        let range = locate(numbered, "one beta two gamma").unwrap();
        assert_eq!(&numbered[range], "one\n2) beta two\n10. gamma");

        let tasks = "- [x] Add themes\n  - [ ] Ship the Windows build\n";
        let range = locate(tasks, "Add themes Ship the Windows").unwrap();
        assert_eq!(&tasks[range], "Add themes\n  - [ ] Ship the Windows");

        let quote = "> said once\n> said twice\n";
        assert!(locate(quote, "once said twice").is_some());
    }

    #[test]
    fn a_dash_inside_a_line_is_text() {
        // Only the start of a line is a marker: "a - b" still needs the dash.
        assert_eq!(locate("a - b", "a b"), None);
        assert!(locate("plain\n-x item", "plain x item").is_none());
    }

    #[test]
    fn insert_rewrite_and_remove() {
        let mut source = "Ship on Monday.".to_string();
        let range = locate(&source, "Monday").unwrap();
        insert_note(&mut source, &range, "Monday", "or Tuesday?");
        assert_eq!(source, "Ship on Monday<!-- note on \"Monday\": or Tuesday? -->.");
        let note = find_notes(&source).remove(0);
        rewrite_note(&mut source, &note, "Tuesday.");
        assert_eq!(source, "Ship on Monday<!-- note on \"Monday\": Tuesday. -->.");
        let note = find_notes(&source).remove(0);
        remove_note(&mut source, &note);
        assert_eq!(source, "Ship on Monday.");
    }
}
