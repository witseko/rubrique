//! Line helpers for the reStructuredText reader: tabs, indentation,
//! indented bodies and adornment lines (the punctuation under a title).

/// RST tab stops are every 8 columns.
const TAB_WIDTH: usize = 8;

/// Characters RST accepts in title adornments and transitions.
const ADORNMENT_CHARS: &str = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";

/// Shortest adornment that counts as a title underline even when the
/// title is longer (docutils only warns about a short underline).
const MIN_UNDERLINE: usize = 3;

/// Shortest line of punctuation that is a transition (docutils' rule).
const MIN_TRANSITION: usize = 4;

/// The line with every tab expanded to the next 8-column stop, as
/// docutils reads it.
pub(super) fn expand_tabs(line: &str) -> String {
    if !line.contains('\t') {
        return line.to_string();
    }
    let mut expanded = String::with_capacity(line.len() + TAB_WIDTH);
    let mut column = 0;
    for character in line.chars() {
        if character == '\t' {
            let spaces = TAB_WIDTH - column % TAB_WIDTH;
            expanded.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            expanded.push(character);
            column += 1;
        }
    }
    expanded
}

/// True for an empty or all-whitespace line.
pub(super) fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// Number of leading spaces.
pub(super) fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// End (exclusive) of the indented body that starts at `start`: the blank
/// and indented lines before the first line at column 0, without the
/// blank lines at its end.
pub(super) fn body_end(lines: &[&str], start: usize) -> usize {
    let mut end = start;
    let mut last_text = start;
    while end < lines.len() && (is_blank(lines[end]) || indent(lines[end]) > 0) {
        end += 1;
        if !is_blank(lines[end - 1]) {
            last_text = end;
        }
    }
    last_text
}

/// The lines without their common indentation; blank lines become empty.
pub(super) fn dedent<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let common = lines.iter().filter(|line| !is_blank(line)).map(|line| indent(line)).min().unwrap_or(0);
    lines.iter().map(|line| if is_blank(line) { "" } else { &line[common..] }).collect()
}

/// The repeated character of an adornment line (`=====`, `-----`), which
/// starts at column 0 and has no spaces, or `None`.
pub(super) fn adornment_char(line: &str) -> Option<char> {
    let line = line.trim_end();
    let first = line.chars().next()?;
    let repeated = ADORNMENT_CHARS.contains(first) && line.chars().all(|character| character == first);
    (repeated && line.chars().count() >= 2).then_some(first)
}

/// True when `underline` is an adornment long enough to underline `title`.
pub(super) fn underlines(title: &str, underline: &str) -> bool {
    let length = underline.trim_end().chars().count();
    adornment_char(underline).is_some() && (length >= MIN_UNDERLINE || length >= title.trim().chars().count())
}

/// True for a line that can be a transition (a horizontal rule).
pub(super) fn is_transition_line(line: &str) -> bool {
    adornment_char(line).is_some() && line.trim_end().chars().count() >= MIN_TRANSITION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_expand_to_eight_column_stops() {
        assert_eq!(expand_tabs("\tx"), "        x");
        assert_eq!(expand_tabs("ab\tx"), "ab      x");
        assert_eq!(expand_tabs("no tabs"), "no tabs");
    }

    #[test]
    fn body_ends_before_column_zero_and_drops_trailing_blanks() {
        let lines = ["head", "  one", "", "  two", "", "next"];
        assert_eq!(body_end(&lines, 1), 4);
        assert_eq!(body_end(&lines, 5), 5);
    }

    #[test]
    fn dedent_keeps_relative_indentation() {
        assert_eq!(dedent(&["   a", "", "     b"]), ["a", "", "  b"]);
    }

    #[test]
    fn adornments_are_one_repeated_punctuation_character() {
        assert_eq!(adornment_char("====="), Some('='));
        assert_eq!(adornment_char("~~~  "), Some('~'));
        assert_eq!(adornment_char("=== ==="), None);
        assert_eq!(adornment_char("=-=-"), None);
        assert_eq!(adornment_char("aaaa"), None);
        assert!(underlines("Hi", "--"));
        assert!(underlines("A long title", "---"));
        assert!(!underlines("A long title", "--"));
    }
}
