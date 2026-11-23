//! Find shell syntax (sh, bash, zsh) in raw text so it can be colored.
//!
//! Forgiving, like the other highlighters: it never fails, it only labels
//! byte ranges. Comments are comments; `'...'`, `"..."`, `$'...'` and
//! heredoc bodies are strings; `$name`, `${...}` and the name of an
//! assignment (`NAME=value`) are keys; the first word of each command is a
//! name; the words that shape a script (`if`, `for`, `done`, `export`) are
//! keywords; a whole-number argument is a number.
//!
//! It does not parse. It follows just enough state to know where a command
//! starts: after a newline, `;`, `|`, `&&`, `(`, `{` and keywords like
//! `then` and `do`. Text inside double quotes and backticks is not scanned.

use std::path::Path;

use super::code::{closing_quote, line_end};
use crate::markdown::highlight::{Kind, Span, Spans};

/// Extensions of shell script files.
const SHELL_EXTENSIONS: [&str; 4] = ["sh", "bash", "zsh", "ksh"];

/// Shell startup files, which have no extension.
const SHELL_FILE_NAMES: [&str; 7] =
    [".bashrc", ".bash_profile", ".bash_aliases", ".profile", ".zshrc", ".zshenv", ".zprofile"];

/// Characters after `$` that name a special parameter (`$@`, `$?`, `$1`).
const SPECIAL_PARAMETERS: &[u8] = b"@*#?$!-";

/// True when the file is a shell script, by its extension or its name.
pub fn is_shell_path(path: &Path) -> bool {
    let has_shell_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| SHELL_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()));
    has_shell_extension
        || path.file_name().and_then(|name| name.to_str()).is_some_and(|name| SHELL_FILE_NAMES.contains(&name))
}

/// Label every byte of `text`. Spans are in order, touch each other, never
/// empty, and neighbors never share a kind.
pub fn highlight(text: &str) -> Vec<Span> {
    let scanner = Scanner {
        text,
        out: Spans::default(),
        position: 0,
        expect: Expect::Command,
        open_parentheses: 0,
        arithmetic: false,
        heredoc: None,
    };
    scanner.run()
}

/// What the next word is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// The name of a command.
    Command,
    /// An argument of the current command.
    Argument,
    /// The name after `function`.
    FunctionName,
    /// Words up to the `in` of `for x in` and `case x in`.
    In,
}

/// A heredoc (`<<EOF`) whose body starts on the next line.
struct Heredoc<'a> {
    /// The line that ends the body.
    delimiter: &'a str,
    /// `<<-`: tabs before the delimiter line are allowed.
    strip_tabs: bool,
}

/// Walks the text once, front to back. Every position it stops at is a char
/// boundary: it only steps over whole chars or ASCII bytes.
struct Scanner<'a> {
    text: &'a str,
    out: Spans,
    position: usize,
    expect: Expect,
    /// How many `(` of subshells, `$(...)` and arrays are open.
    open_parentheses: usize,
    /// Inside `(( ... ))`, where no word is a command.
    arithmetic: bool,
    heredoc: Option<Heredoc<'a>>,
}

impl Scanner<'_> {
    fn run(mut self) -> Vec<Span> {
        while self.position < self.text.len() {
            self.step();
        }
        self.out.to(self.text.len(), Kind::Plain);
        self.out.spans
    }

    /// Read one token at the current position.
    fn step(&mut self) {
        let start = self.position;
        let rest = &self.text[start..];
        match rest.as_bytes()[0] {
            b'\n' => self.newline(),
            b'#' if self.at_word_start() => self.mark(line_end(self.text, start), Kind::Comment),
            b'\'' => {
                let end = rest[1..].find('\'').map_or(self.text.len(), |offset| start + offset + 2);
                self.string(end);
            }
            b'"' => self.string(closing_quote(self.text, start + 1, b"\"", true)),
            b'$' => self.dollar(),
            // An escaped character opens nothing; `\` before a newline joins the lines.
            b'\\' => self.position += 1 + rest[1..].chars().next().map_or(0, char::len_utf8),
            b'<' if rest.starts_with("<<") && !self.arithmetic => self.heredoc_opener(),
            b'|' | b'&' | b';' | b'(' | b')' | b'<' | b'>' => self.operator(),
            b'`' => self.position += 1,
            byte if byte.is_ascii_whitespace() => self.position += 1,
            _ => self.word(),
        }
    }

    /// The end of a line: a new command starts, and the body of a heredoc
    /// opened on the line.
    fn newline(&mut self) {
        self.position += 1;
        // An unclosed `((` stops here, so it cannot switch off command names
        // in the rest of the file.
        self.arithmetic = false;
        self.expect = Expect::Command;
        if let Some(heredoc) = self.heredoc.take() {
            self.mark(heredoc_end(self.text, self.position, &heredoc), Kind::String);
        }
    }

    /// `$name`, `${...}`, `$(command)`, `$((arithmetic))` or `$'string'`.
    fn dollar(&mut self) {
        let start = self.position;
        match &self.text.as_bytes()[start + 1..] {
            [b'\'', ..] => self.string(closing_quote(self.text, start + 2, b"'", true)),
            [b'(', b'(', ..] => {
                self.arithmetic = true;
                self.expect = Expect::Argument;
                self.position += 3;
            }
            [b'(', ..] => {
                self.open_parentheses += 1;
                self.position += 2;
                self.start_command();
            }
            _ => {
                let end = variable_end(self.text, start);
                if end == start {
                    self.position += 1;
                } else {
                    self.take_command_name();
                    self.mark(end, Kind::Key);
                }
            }
        }
    }

    /// `<<EOF`, `<<-EOF`, `<<'EOF'`: remember the delimiter; the body starts
    /// after this line. `<<<` (a here-string) and `<<` without a delimiter
    /// are plain operators.
    fn heredoc_opener(&mut self) {
        let after = &self.text[self.position + 2..];
        let (strip_tabs, after) = match after.strip_prefix('-') {
            Some(after_dash) => (true, after_dash),
            None => (false, after),
        };
        let token = after.trim_start_matches([' ', '\t']);
        if let Some((delimiter, token_length)) = heredoc_delimiter(token) {
            self.heredoc = Some(Heredoc { delimiter, strip_tabs });
            self.position = self.text.len() - token.len() + token_length;
        } else {
            self.expect = Expect::Argument;
            self.position += 2;
        }
    }

    /// `|`, `&`, `;`, `(`, `)`, `<` or `>`: most of them start a new command.
    fn operator(&mut self) {
        let bytes = self.text.as_bytes();
        let after_redirect = self.position > 0 && matches!(bytes[self.position - 1], b'<' | b'>');
        let after_equals = self.position > 0 && bytes[self.position - 1] == b'=';
        let mut length = 1;
        match &bytes[self.position..] {
            [b'(', b'(', ..] => {
                self.arithmetic = true;
                self.expect = Expect::Argument;
                length = 2;
            }
            [b')', b')', ..] if self.arithmetic => {
                self.arithmetic = false;
                length = 2;
            }
            [b'(', ..] => {
                self.open_parentheses += 1;
                // `names=(a b)` is an array, not a subshell.
                if after_equals {
                    self.expect = Expect::Argument;
                } else {
                    self.start_command();
                }
            }
            [b')', ..] => {
                // With nothing open, `)` ends a `case` pattern and a command follows.
                if self.open_parentheses == 0 {
                    self.start_command();
                } else {
                    self.expect = Expect::Argument;
                }
                self.open_parentheses = self.open_parentheses.saturating_sub(1);
            }
            // The word after a redirect is a file. `2>&1` and `&>file` are
            // redirects too, so their `&` starts no command.
            [b'<' | b'>', ..] | [b'&', b'>', ..] => self.expect = Expect::Argument,
            [b'&', ..] if after_redirect => {}
            _ => self.start_command(),
        }
        self.position += length;
    }

    /// A run of characters with no space, quote or operator in it.
    fn word(&mut self) {
        let end = word_end(self.text, self.position);
        let word = &self.text[self.position..end];
        if !self.at_word_start() {
            // The rest of a word that a quote or a variable began.
            self.position = end;
        } else if word == "{" {
            self.start_command();
            self.position = end;
        } else {
            match self.expect {
                Expect::Command => self.command_word(word, end),
                Expect::Argument => self.argument_word(word, end),
                Expect::FunctionName => {
                    self.expect = Expect::Argument;
                    self.mark(end, Kind::Name);
                }
                Expect::In if word == "in" => {
                    self.expect = Expect::Argument;
                    self.mark(end, Kind::Keyword);
                }
                Expect::In => self.position = end,
            }
        }
    }

    /// A word where a command starts: a keyword, an assignment, or the
    /// command's name.
    fn command_word(&mut self, word: &str, end: usize) {
        let next = self.text.as_bytes().get(end).copied();
        if let Some(expect) = keyword_expects(word) {
            self.expect = expect;
            self.mark(end, Kind::Keyword);
        } else if let Some(name_length) = assignment_name_length(word) {
            // `NAME=value command`: the command's name is still to come.
            self.mark(self.position + name_length, Kind::Key);
            self.position = end;
        } else if word.starts_with('-') || matches!(word, "}" | "!") {
            self.position = end;
        } else if self.open_parentheses == 0 && next == Some(b')') {
            // A `case` pattern: `start) ...`.
            self.position = end;
        } else {
            self.expect = Expect::Argument;
            let kind = if matches!(word, "[" | "[[") { Kind::Plain } else { Kind::Name };
            self.mark(end, kind);
        }
    }

    /// A word after the command's name: `NAME=value` (as in `make NAME=1`),
    /// a whole number, or plain.
    fn argument_word(&mut self, word: &str, end: usize) {
        if let Some(name_length) = assignment_name_length(word) {
            self.mark(self.position + name_length, Kind::Key);
        } else if word.bytes().all(|byte| byte.is_ascii_digit()) {
            self.mark(end, Kind::Number);
        }
        self.position = end;
    }

    /// A string from the current position to `end`.
    fn string(&mut self, end: usize) {
        self.take_command_name();
        self.mark(end, Kind::String);
    }

    /// A quote or a variable that starts a word where a command's name is
    /// expected takes that place (`"$tool" --version`).
    fn take_command_name(&mut self) {
        if self.expect == Expect::Command && self.at_word_start() {
            self.expect = Expect::Argument;
        }
    }

    /// The next word is a command's name, except inside `(( ... ))`.
    fn start_command(&mut self) {
        if !self.arithmetic {
            self.expect = Expect::Command;
        }
    }

    /// True when the current position is not glued to a word before it.
    fn at_word_start(&self) -> bool {
        self.position
            .checked_sub(1)
            .and_then(|before| self.text.as_bytes().get(before))
            .is_none_or(|&byte| is_word_break(byte))
    }

    /// Label plain text up to the current position, then up to `end` as
    /// `kind`, and move on to `end`.
    fn mark(&mut self, end: usize, kind: Kind) {
        self.out.to(self.position, Kind::Plain);
        self.out.to(end, kind);
        self.position = end;
    }
}

/// For a keyword, what the word after it is; `None` for other words.
fn keyword_expects(word: &str) -> Option<Expect> {
    match word {
        "if" | "then" | "else" | "elif" | "do" | "while" | "until" | "time" => Some(Expect::Command),
        "for" | "case" | "select" => Some(Expect::In),
        "function" => Some(Expect::FunctionName),
        "fi" | "done" | "esac" | "return" | "exit" | "break" | "continue" | "export" | "local" | "readonly"
        | "declare" | "typeset" => Some(Expect::Argument),
        _ => None,
    }
}

/// The length of `NAME` when `word` is an assignment: `NAME=value`,
/// `NAME+=value` or `NAME[index]=value`.
fn assignment_name_length(word: &str) -> Option<usize> {
    let (target, _value) = word.split_once('=')?;
    let target = target.strip_suffix('+').unwrap_or(target);
    let name = match target.split_once('[') {
        Some((name, index)) if index.ends_with(']') => name,
        _ => target,
    };
    let starts_as_name = name.starts_with(|character: char| character.is_ascii_alphabetic() || character == '_');
    (starts_as_name && name.bytes().all(is_name_byte)).then_some(name.len())
}

/// A byte of a variable name after its first one.
fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// A byte that ends the word before it.
fn is_word_break(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'|' | b'&' | b';' | b'(' | b')' | b'<' | b'>')
}

/// The end of the word that starts at `start`: before the next space,
/// operator, quote, `$`, backslash or backtick.
fn word_end(text: &str, start: usize) -> usize {
    text.as_bytes()[start..]
        .iter()
        .position(|&byte| is_word_break(byte) || matches!(byte, b'\'' | b'"' | b'$' | b'\\' | b'`'))
        .map_or(text.len(), |offset| start + offset)
}

/// The end of the variable whose `$` is at `dollar`, or `dollar` itself
/// when no variable starts there.
fn variable_end(text: &str, dollar: usize) -> usize {
    let bytes = text.as_bytes();
    let name = dollar + 1;
    match bytes.get(name) {
        Some(b'{') => brace_end(text, name),
        Some(&byte) if byte.is_ascii_alphabetic() || byte == b'_' => {
            bytes[name..].iter().position(|&byte| !is_name_byte(byte)).map_or(text.len(), |offset| name + offset)
        }
        Some(byte) if byte.is_ascii_digit() || SPECIAL_PARAMETERS.contains(byte) => name + 1,
        _ => dollar,
    }
}

/// Just past the `}` that closes the `{` at `open` (counting nested
/// braces), or the end of the line when it never closes.
fn brace_end(text: &str, open: usize) -> usize {
    let line_end = line_end(text, open);
    let mut depth = 0_usize;
    for (offset, byte) in text.as_bytes()[open..line_end].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return open + offset + 1;
                }
            }
            _ => {}
        }
    }
    line_end
}

/// The delimiter that the text after `<<` names, and the length of its
/// token (with quotes or a backslash). A plain delimiter starts with a
/// letter or `_`, so `1 << 2` opens no heredoc.
fn heredoc_delimiter(token: &str) -> Option<(&str, usize)> {
    let first = token.chars().next()?;
    if first == '\'' || first == '"' {
        let length = token[1..].find(first)?;
        let delimiter = &token[1..=length];
        return (!delimiter.contains('\n')).then_some((delimiter, length + 2));
    }
    let name = token.strip_prefix('\\').unwrap_or(token);
    if !name.starts_with(|character: char| character.is_ascii_alphabetic() || character == '_') {
        return None;
    }
    let length = name.bytes().position(|byte| !is_name_byte(byte)).unwrap_or(name.len());
    Some((&name[..length], token.len() - name.len() + length))
}

/// The end of the heredoc body that starts at `from`: the end of its
/// delimiter line, or of the text when that line never comes.
fn heredoc_end(text: &str, from: usize, heredoc: &Heredoc) -> usize {
    let mut line_start = from;
    while line_start < text.len() {
        let end = line_end(text, line_start);
        let line = text[line_start..end].trim_end_matches('\r');
        let line = if heredoc.strip_tabs { line.trim_start_matches('\t') } else { line };
        if line == heredoc.delimiter {
            return end;
        }
        line_start = end + 1;
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The labeled pieces of `text`, skipping plain ones.
    fn colored(text: &str) -> Vec<(&str, Kind)> {
        highlight(text)
            .into_iter()
            .filter(|span| span.kind != Kind::Plain)
            .map(|span| (&text[span.range], span.kind))
            .collect()
    }

    #[test]
    fn commands_arguments_and_comments() {
        let text = "make demo-up      # cold boot\nbash scripts/tunnel.sh status 8190 # check (8190/8988)\n";
        assert_eq!(
            colored(text),
            vec![
                ("make", Kind::Name),
                ("# cold boot", Kind::Comment),
                ("bash", Kind::Name),
                ("8190", Kind::Number),
                ("# check (8190/8988)", Kind::Comment),
            ]
        );
    }

    #[test]
    fn hash_inside_a_word_opens_no_comment() {
        assert_eq!(
            colored("curl http://host/#top $# ${#items[@]}"),
            vec![("curl", Kind::Name), ("$#", Kind::Key), ("${#items[@]}", Kind::Key)]
        );
    }

    #[test]
    fn commands_after_pipes_and_lists() {
        assert_eq!(
            colored("cat a.txt | grep -c x && echo ok; ls 2>&1 &> log"),
            vec![
                ("cat", Kind::Name),
                ("grep", Kind::Name),
                ("echo", Kind::Name),
                ("ls", Kind::Name),
                ("2", Kind::Number),
                ("1", Kind::Number),
            ]
        );
    }

    #[test]
    fn strings_span_lines_and_keep_their_quotes() {
        let text = "echo 'it is\n# not a comment' \"say \\\"hi\\\" $USER\" $'tab\\'s'\nls";
        assert_eq!(
            colored(text),
            vec![
                ("echo", Kind::Name),
                ("'it is\n# not a comment'", Kind::String),
                ("\"say \\\"hi\\\" $USER\"", Kind::String),
                ("$'tab\\'s'", Kind::String),
                ("ls", Kind::Name),
            ]
        );
    }

    #[test]
    fn variables_and_assignments() {
        let text = "NAME=demo PORT=\"80\" serve $HOME/site ${PORT:-${DEFAULT}} $1\nmake JOBS=4";
        assert_eq!(
            colored(text),
            vec![
                ("NAME", Kind::Key),
                ("PORT", Kind::Key),
                ("\"80\"", Kind::String),
                ("serve", Kind::Name),
                ("$HOME", Kind::Key),
                ("${PORT:-${DEFAULT}}", Kind::Key),
                ("$1", Kind::Key),
                ("make", Kind::Name),
                ("JOBS", Kind::Key),
            ]
        );
    }

    #[test]
    fn control_flow_keywords() {
        let text = "if [ -f \"$f\" ]; then\n  echo done\nelse exit 1\nfi\nfor x in a b; do run $x; done";
        assert_eq!(
            colored(text),
            vec![
                ("if", Kind::Keyword),
                ("\"$f\"", Kind::String),
                ("then", Kind::Keyword),
                ("echo", Kind::Name),
                ("else", Kind::Keyword),
                ("exit", Kind::Keyword),
                ("1", Kind::Number),
                ("fi", Kind::Keyword),
                ("for", Kind::Keyword),
                ("in", Kind::Keyword),
                ("do", Kind::Keyword),
                ("run", Kind::Name),
                ("$x", Kind::Key),
                ("done", Kind::Keyword),
            ]
        );
    }

    #[test]
    fn case_patterns_are_not_commands() {
        let text = "case \"$1\" in\n  start) run ;;\n  *) usage ;;\nesac";
        assert_eq!(
            colored(text),
            vec![
                ("case", Kind::Keyword),
                ("\"$1\"", Kind::String),
                ("in", Kind::Keyword),
                ("run", Kind::Name),
                ("usage", Kind::Name),
                ("esac", Kind::Keyword),
            ]
        );
    }

    #[test]
    fn functions_declarations_and_arrays() {
        let text = "function greet { local name=$1; }\nbuild() { export -f build; }\nitems=(one two)";
        assert_eq!(
            colored(text),
            vec![
                ("function", Kind::Keyword),
                ("greet", Kind::Name),
                ("local", Kind::Keyword),
                ("name", Kind::Key),
                ("$1", Kind::Key),
                ("build", Kind::Name),
                ("export", Kind::Keyword),
                ("items", Kind::Key),
            ]
        );
    }

    #[test]
    fn command_substitution_and_arithmetic() {
        let text = "now=$(date +%s)\ncp $(find . -name x) dest\nfor ((i = 0; i < 3; i++)); do echo $((i << 1)); done";
        assert_eq!(
            colored(text),
            vec![
                ("now", Kind::Key),
                ("date", Kind::Name),
                ("cp", Kind::Name),
                ("find", Kind::Name),
                ("for", Kind::Keyword),
                ("0", Kind::Number),
                ("3", Kind::Number),
                ("do", Kind::Keyword),
                ("echo", Kind::Name),
                ("1", Kind::Number),
                ("done", Kind::Keyword),
            ]
        );
    }

    #[test]
    fn heredoc_body_is_a_string() {
        let text = "cat <<EOF | wc -l\nit's $HOME\n# text\nEOF\necho after <<-'END'\n\tbody\n\tEND\nls <<< word";
        assert_eq!(
            colored(text),
            vec![
                ("cat", Kind::Name),
                ("wc", Kind::Name),
                ("it's $HOME\n# text\nEOF", Kind::String),
                ("echo", Kind::Name),
                ("\tbody\n\tEND", Kind::String),
                ("ls", Kind::Name),
            ]
        );
    }

    #[test]
    fn shell_paths() {
        for name in ["deploy.sh", "run.BASH", "env.zsh", ".bashrc", "home/.zshrc"] {
            assert!(is_shell_path(Path::new(name)), "{name}");
        }
        for name in ["notes.md", "Makefile", "sh", "bashrc.txt"] {
            assert!(!is_shell_path(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn spans_cover_odd_input() {
        let odd = ["", "'", "\"\\", "$", "${", "$(", "((", "<<", "<<'", "<<EOF", "\\", "é='ü' # ö\n", "a\\é", "`$é`"];
        for text in odd {
            let spans = highlight(text);
            let mut end = 0;
            for pair in spans.windows(2) {
                assert_ne!(pair[0].kind, pair[1].kind, "{text:?}");
            }
            for span in &spans {
                assert_eq!(span.range.start, end, "{text:?}");
                assert!(span.range.end > span.range.start, "{text:?}");
                assert!(text.is_char_boundary(span.range.end), "{text:?}");
                end = span.range.end;
            }
            assert_eq!(end, text.len(), "{text:?}");
        }
    }
}
