//! The flags Rubrique knows in a tool's command template, behind the chips
//! above the command field in Settings: read a flag's value, set it or take
//! it out, and leave every other word of the template as it is. The text
//! stays the source of truth; a flag Rubrique does not know is simply not a
//! chip.

use std::path::Path;

use super::agents::with_model;
use super::{AiTool, join_command, split_command};

/// One flag a chip can set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flag {
    /// Shown on the chip: "Model".
    pub label: &'static str,
    pub kind: FlagKind,
}

/// How a flag sits in the command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagKind {
    /// `--model <name>` (or `-m`, or `--model=<name>`); for `ollama run`
    /// the model word itself. Free text.
    Model,
    /// An option with a fixed set of values: `--sandbox read-only`.
    Choice { option: &'static str, values: &'static [&'static str] },
}

const MODEL: Flag = Flag { label: "Model", kind: FlagKind::Model };

const CLAUDE_FLAGS: [Flag; 2] = [
    MODEL,
    Flag {
        label: "Permissions",
        kind: FlagKind::Choice {
            option: "--permission-mode",
            values: &["plan", "default", "acceptEdits", "bypassPermissions"],
        },
    },
];

const CODEX_FLAGS: [Flag; 2] = [
    MODEL,
    Flag {
        label: "Sandbox",
        kind: FlagKind::Choice { option: "--sandbox", values: &["read-only", "workspace-write", "danger-full-access"] },
    },
];

const GEMINI_FLAGS: [Flag; 2] = [
    MODEL,
    Flag {
        label: "Approval",
        kind: FlagKind::Choice { option: "--approval-mode", values: &["plan", "default", "auto_edit", "yolo"] },
    },
];

const LOCAL_FLAGS: [Flag; 1] = [MODEL];

/// The flags Rubrique knows for `tool`, in chip order. Grok has none yet.
pub fn known_flags(tool: AiTool) -> &'static [Flag] {
    match tool {
        AiTool::Claude => &CLAUDE_FLAGS,
        AiTool::Codex => &CODEX_FLAGS,
        AiTool::Gemini => &GEMINI_FLAGS,
        AiTool::Local => &LOCAL_FLAGS,
        AiTool::Grok => &[],
    }
}

/// The flag's value in `command`, when the flag is there.
pub fn value(command: &str, tool: AiTool, flag: &Flag) -> Option<String> {
    let words = split_command(command);
    match flag.kind {
        FlagKind::Model => model_value(&words, tool),
        FlagKind::Choice { option, .. } => option_value(&words, option),
    }
}

/// `command` with the flag set to `value`, or taken out for `None`. The
/// Ollama model word cannot be taken out (the program needs one), so the
/// command then stays as it is.
pub fn with_value(command: &str, tool: AiTool, flag: &Flag, value: Option<&str>) -> String {
    match (flag.kind, value) {
        (FlagKind::Model, Some(model)) => with_model(command, tool, model),
        (FlagKind::Model, None) => {
            let mut words = split_command(command);
            remove_option(&mut words, &["--model", "-m"]);
            join_command(&words)
        }
        (FlagKind::Choice { option, .. }, Some(value)) => {
            let mut words = split_command(command);
            set_option(&mut words, tool, option, value);
            join_command(&words)
        }
        (FlagKind::Choice { option, .. }, None) => {
            let mut words = split_command(command);
            remove_option(&mut words, &[option]);
            join_command(&words)
        }
    }
}

/// The model after `--model` or `-m`, in `--model=<name>`, or the word after
/// `ollama run` for the local tool. A placeholder word is not a model.
fn model_value(words: &[String], tool: AiTool) -> Option<String> {
    if let Some(found) = option_value(words, "--model").or_else(|| option_value(words, "-m")) {
        return Some(found);
    }
    if tool != AiTool::Local {
        return None;
    }
    match words {
        [program, run, model, ..] if is_ollama(program) && run == "run" && is_value(model) => Some(model.clone()),
        _ => None,
    }
}

/// The word after `option`, or the part after `option=`.
fn option_value(words: &[String], option: &str) -> Option<String> {
    let prefix = format!("{option}=");
    words.iter().enumerate().find_map(|(index, word)| {
        if word == option {
            words.get(index + 1).filter(|next| is_value(next)).cloned()
        } else {
            word.strip_prefix(&prefix).map(str::to_string)
        }
    })
}

/// Replace the option's value where it is, else add the option after the
/// program (after `exec` for Codex), where tools read their options.
fn set_option(words: &mut Vec<String>, tool: AiTool, option: &str, value: &str) {
    let prefix = format!("{option}=");
    if let Some(index) = words.iter().position(|word| word == option)
        && index + 1 < words.len()
    {
        value.clone_into(&mut words[index + 1]);
    } else if let Some(index) = words.iter().position(|word| word.starts_with(&prefix)) {
        words[index] = format!("{option}={value}");
    } else {
        let at = insert_at(words, tool);
        words.splice(at..at, [option.to_string(), value.to_string()]);
    }
}

/// Take out every option in `options`, with its value.
fn remove_option(words: &mut Vec<String>, options: &[&str]) {
    let mut index = 0;
    while index < words.len() {
        let word = &words[index];
        if options.contains(&word.as_str()) {
            let end = if words.get(index + 1).is_some_and(|next| is_value(next)) { index + 2 } else { index + 1 };
            words.drain(index..end);
        } else if options.iter().any(|option| word.starts_with(&format!("{option}="))) {
            words.remove(index);
        } else {
            index += 1;
        }
    }
}

/// Where a new option goes: right after the program, or after Codex's
/// `exec` subcommand.
pub(super) fn insert_at(words: &[String], tool: AiTool) -> usize {
    if words.is_empty() {
        return 0;
    }
    if tool == AiTool::Codex && words.get(1).is_some_and(|word| word == "exec") { 2 } else { 1 }
}

/// A word that can be an option's value: not another option, not a
/// placeholder.
fn is_value(word: &str) -> bool {
    !word.starts_with('-') && !word.contains('{')
}

pub(super) fn is_ollama(program: &str) -> bool {
    Path::new(program).file_name().is_some_and(|name| name == "ollama")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flag(tool: AiTool, label: &str) -> Flag {
        *known_flags(tool).iter().find(|flag| flag.label == label).expect("a known flag")
    }

    #[test]
    fn reads_the_model_in_its_spellings() {
        let model = flag(AiTool::Claude, "Model");
        assert_eq!(value("claude --model opus {prompt}", AiTool::Claude, &model).as_deref(), Some("opus"));
        assert_eq!(value("claude -m opus {prompt}", AiTool::Claude, &model).as_deref(), Some("opus"));
        assert_eq!(value("claude --model=opus {prompt}", AiTool::Claude, &model).as_deref(), Some("opus"));
        assert_eq!(value("claude -p {prompt}", AiTool::Claude, &model), None);
        // The next word must be a value, not a placeholder.
        assert_eq!(value("claude --model {prompt}", AiTool::Claude, &model), None);
    }

    #[test]
    fn reads_the_ollama_model_word_for_the_local_tool_only() {
        let model = flag(AiTool::Local, "Model");
        assert_eq!(value("ollama run qwen2.5:1.5b {prompt}", AiTool::Local, &model).as_deref(), Some("qwen2.5:1.5b"));
        assert_eq!(value("/opt/bin/ollama run llama3 {prompt}", AiTool::Local, &model).as_deref(), Some("llama3"));
        assert_eq!(value("llama-cli -m model.gguf -p {prompt}", AiTool::Local, &model).as_deref(), Some("model.gguf"));
        assert_eq!(value("ollama run {prompt}", AiTool::Local, &model), None);
        assert_eq!(value("ollama run qwen {prompt}", AiTool::Claude, &flag(AiTool::Claude, "Model")), None);
    }

    #[test]
    fn sets_and_removes_the_model() {
        let model = flag(AiTool::Claude, "Model");
        let set = with_value("claude -p {prompt}", AiTool::Claude, &model, Some("opus"));
        assert_eq!(set, "claude --model opus -p {prompt}");
        assert_eq!(with_value(&set, AiTool::Claude, &model, None), "claude -p {prompt}");
        assert_eq!(with_value("claude --model=opus -p {prompt}", AiTool::Claude, &model, None), "claude -p {prompt}");
        assert_eq!(with_value("claude -m opus -p {prompt}", AiTool::Claude, &model, None), "claude -p {prompt}");
        // Ollama needs a model word, so it stays.
        let local = flag(AiTool::Local, "Model");
        assert_eq!(with_value("ollama run qwen {prompt}", AiTool::Local, &local, None), "ollama run qwen {prompt}");
    }

    #[test]
    fn reads_and_sets_a_choice() {
        let mode = flag(AiTool::Claude, "Permissions");
        let command = "claude -p --permission-mode plan {prompt}";
        assert_eq!(value(command, AiTool::Claude, &mode).as_deref(), Some("plan"));
        assert_eq!(
            with_value(command, AiTool::Claude, &mode, Some("acceptEdits")),
            "claude -p --permission-mode acceptEdits {prompt}"
        );
        assert_eq!(
            with_value("claude --permission-mode=plan {prompt}", AiTool::Claude, &mode, Some("default")),
            "claude --permission-mode=default {prompt}"
        );
        assert_eq!(with_value(command, AiTool::Claude, &mode, None), "claude -p {prompt}");
    }

    #[test]
    fn adds_a_choice_where_the_tool_reads_options() {
        let mode = flag(AiTool::Claude, "Permissions");
        assert_eq!(
            with_value("claude -p {prompt}", AiTool::Claude, &mode, Some("plan")),
            "claude --permission-mode plan -p {prompt}"
        );
        let approval = flag(AiTool::Gemini, "Approval");
        assert_eq!(
            with_value("gemini --prompt {prompt}", AiTool::Gemini, &approval, Some("plan")),
            "gemini --approval-mode plan --prompt {prompt}"
        );
        assert_eq!(value(AiTool::Gemini.default_command(), AiTool::Gemini, &approval).as_deref(), Some("plan"));
        let sandbox = flag(AiTool::Codex, "Sandbox");
        assert_eq!(
            with_value("codex exec --color never {prompt}", AiTool::Codex, &sandbox, Some("read-only")),
            "codex exec --sandbox read-only --color never {prompt}"
        );
    }

    #[test]
    fn leaves_other_words_and_quotes_alone() {
        let model = flag(AiTool::Claude, "Model");
        let command = "claude -p --append-system-prompt {system} --model opus \"two words\" {prompt}";
        let changed = with_value(command, AiTool::Claude, &model, Some("sonnet"));
        assert_eq!(changed, "claude -p --append-system-prompt {system} --model sonnet \"two words\" {prompt}");
        assert_eq!(value(&changed, AiTool::Claude, &model).as_deref(), Some("sonnet"));
    }

    #[test]
    fn an_empty_command_gets_the_option_without_panicking() {
        let mode = flag(AiTool::Claude, "Permissions");
        assert_eq!(with_value("", AiTool::Claude, &mode, Some("plan")), "--permission-mode plan");
        assert_eq!(value("", AiTool::Claude, &mode), None);
    }

    #[test]
    fn every_tool_lists_its_chips() {
        assert_eq!(known_flags(AiTool::Claude).len(), 2);
        assert_eq!(known_flags(AiTool::Codex).len(), 2);
        assert_eq!(known_flags(AiTool::Gemini).len(), 2);
        assert_eq!(known_flags(AiTool::Local).len(), 1);
        assert_eq!(known_flags(AiTool::Grok), &[] as &[Flag]);
    }
}
