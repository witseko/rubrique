//! Text highlight rules from a description: the AI writes a regular
//! expression for "ticket ids like PROJ-123", and Settings shows it as a new
//! rule to check before it is added.

use super::Prompt;

/// Longest description sent to the AI.
const MAX_DESCRIPTION_CHARS: usize = 500;
/// Most words of the description kept as the rule's name.
const MAX_NAME_WORDS: usize = 4;

/// The prompt asking an AI tool for a pattern that matches `description`.
pub fn pattern_prompt(description: &str) -> Prompt {
    let shown: String = description.trim().chars().take(MAX_DESCRIPTION_CHARS).collect();
    Prompt {
        system: "You write regular expressions for the Rust regex crate: no look-around and no \
                 backreferences. Reply with only the regular expression on one line: no explanation, \
                 no code fence, no slashes, no quotes. Do not use tools or change any files."
            .to_string(),
        user: format!("Write a regular expression that matches this in plain text:\n{shown}\n"),
    }
}

/// The pattern in an AI reply: its first line that is not a code fence,
/// without wrapping backticks, quotes or `/.../` slashes (a trailing `i`
/// flag becomes `(?i)`). `None` when nothing is left.
pub fn pattern_from_reply(reply: &str) -> Option<String> {
    let line = reply.lines().map(str::trim).find(|line| !line.is_empty() && !line.starts_with("```"))?;
    let line = strip_pair(strip_pair(line, "`"), "\"");
    let pattern = match line.strip_prefix('/').and_then(|rest| rest.strip_suffix("/i")) {
        Some(inner) => format!("(?i){inner}"),
        None => strip_pair(line, "/").to_string(),
    };
    (!pattern.is_empty()).then_some(pattern)
}

/// `text` without `mark` at both ends, when it has it at both (and is more
/// than the one mark).
fn strip_pair<'a>(text: &'a str, mark: &str) -> &'a str {
    if text.len() <= mark.len() {
        return text;
    }
    text.strip_prefix(mark).and_then(|rest| rest.strip_suffix(mark)).unwrap_or(text)
}

/// A short rule name from a description: its first few words.
pub fn name_from_description(description: &str) -> String {
    let words: Vec<&str> = description.split_whitespace().take(MAX_NAME_WORDS).collect();
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_come_out_of_replies() {
        assert_eq!(pattern_from_reply(r"\bPROJ-[0-9]+\b").as_deref(), Some(r"\bPROJ-[0-9]+\b"));
        assert_eq!(pattern_from_reply("```regex\n`[0-9]+`\n```").as_deref(), Some("[0-9]+"));
        assert_eq!(pattern_from_reply("\n\"a+b\"\n").as_deref(), Some("a+b"));
        assert_eq!(pattern_from_reply("/todo/i").as_deref(), Some("(?i)todo"));
        assert_eq!(pattern_from_reply("/x/").as_deref(), Some("x"));
        assert_eq!(pattern_from_reply("/").as_deref(), Some("/"));
        assert_eq!(pattern_from_reply("  \n```\n```"), None);
    }

    #[test]
    fn prompt_carries_the_description() {
        let prompt = pattern_prompt("  ticket ids like PROJ-123 ");
        assert!(prompt.user.contains("ticket ids like PROJ-123\n"));
        assert!(prompt.system.contains("Rust regex crate"));
        assert!(pattern_prompt(&"x".repeat(5_000)).user.len() < MAX_DESCRIPTION_CHARS + 100);
    }

    #[test]
    fn names_come_from_the_first_words() {
        assert_eq!(name_from_description("  ticket ids like PROJ-123 in notes"), "ticket ids like PROJ-123");
    }
}
