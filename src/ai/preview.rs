//! What a run sends, in parts, for showing before it starts ("What Claude
//! gets"). Built from the same code that builds the real prompt, so what is
//! shown is what is sent.

use super::agents::Agent;
use super::skills::Skill;
use super::{AiTool, AskKind, Prompt, SYSTEM, build_prompt, display_command, split_command};

/// Longest selection shown as it is; longer text is only counted.
const SHOWN_TEXT_CHARS: usize = 300;

/// The prompt for `request` about `text`, with the agent's personality and
/// each skill's instructions, in that order.
pub fn full_prompt(
    request: &str,
    text: &str,
    kind: AskKind,
    whole_file: bool,
    agent: Option<&Agent>,
    skills: &[&Skill],
) -> Prompt {
    let mut prompt = build_prompt(request, text, kind, whole_file);
    if let Some(agent) = agent {
        prompt = prompt.with_agent(agent);
    }
    for skill in skills {
        prompt = prompt.with_skill(skill);
    }
    prompt
}

/// The parts of a prompt and the commands that will carry it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptParts {
    /// The request as sent.
    pub request: String,
    pub text: TextSummary,
    /// Rubrique's instructions, with the agent's and any skill's.
    pub system: String,
    /// The agent, as "@ai-editor (Editor)".
    pub agent: Option<String>,
    /// The skills' names: the agent's, then the one picked.
    pub skills: Vec<String>,
    /// One per tool (two when comparing).
    pub commands: Vec<ShownCommand>,
    /// Too long to pass as an argument; the run will not start.
    pub too_long: bool,
}

/// The text a request is about, described instead of repeated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSummary {
    pub whole_file: bool,
    pub lines: usize,
    pub words: usize,
    /// The text itself when it is short enough to show.
    pub shown: Option<String>,
}

impl TextSummary {
    pub fn new(text: &str, whole_file: bool) -> Self {
        let short = !whole_file && text.chars().count() <= SHOWN_TEXT_CHARS;
        TextSummary {
            whole_file,
            lines: text.lines().count(),
            words: text.split_whitespace().count(),
            shown: short.then(|| text.trim().to_string()),
        }
    }

    /// "Whole file, 120 words" or "3 lines selected, 42 words".
    pub fn describe(&self) -> String {
        let words = plural(self.words, "word");
        if self.whole_file {
            format!("Whole file, {words}")
        } else {
            format!("{} selected, {words}", plural(self.lines.max(1), "line"))
        }
    }
}

/// One tool's command as it will run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShownCommand {
    pub tool: AiTool,
    /// The filled-in command, request shortened.
    pub command: String,
    /// The command has no `{system}`, so the instructions lead the request.
    pub instructions_in_prompt: bool,
    /// The command keeps the tool from changing files.
    pub read_only: bool,
}

/// The parts of what `commands` will get for `request` about `text`.
pub fn prompt_parts(
    request: &str,
    text: &str,
    kind: AskKind,
    whole_file: bool,
    agent: Option<&Agent>,
    skills: &[&Skill],
    commands: &[(AiTool, &str)],
) -> PromptParts {
    let prompt = full_prompt(request, text, kind, whole_file, agent, skills);
    let commands = commands
        .iter()
        .map(|(tool, command)| ShownCommand {
            tool: *tool,
            command: display_command(command, request),
            instructions_in_prompt: !has_system(command),
            read_only: is_read_only(command),
        })
        .collect();
    PromptParts {
        request: request.trim().to_string(),
        text: TextSummary::new(text, whole_file),
        too_long: prompt.is_too_long(),
        system: prompt.system,
        agent: agent.map(|agent| format!("{} ({})", agent.mention(), agent.name)),
        skills: skills.iter().map(|skill| skill.name.clone()).collect(),
        commands,
    }
}

impl PromptParts {
    /// "Claude" or "Claude and Codex".
    pub fn tools(&self) -> String {
        self.commands.iter().map(|shown| shown.tool.name()).collect::<Vec<_>>().join(" and ")
    }
}

/// The template has a `{system}` word for the instructions.
pub fn has_system(command: &str) -> bool {
    split_command(command).iter().skip(1).any(|word| word.contains(SYSTEM))
}

/// The template asks the tool not to change files: Claude's plan mode or
/// Codex's read-only sandbox. Gemini's plan mode does not count: in a
/// headless run Gemini may leave plan mode on its own and then approve
/// every tool (its `exit_plan_mode` switches to YOLO when not interactive).
pub fn is_read_only(command: &str) -> bool {
    let words = split_command(command);
    let flag = |name: &str, value: &str| {
        words.windows(2).any(|pair| pair[0] == name && pair[1] == value)
            || words.iter().any(|word| *word == format!("{name}={value}"))
    };
    flag("--permission-mode", "plan") || flag("--sandbox", "read-only")
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 { format!("1 {noun}") } else { format!("{count} {noun}s") }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::{MAX_PROMPT_BYTES, agents, command_args, skills};
    use super::*;

    const CLAUDE: &str = "claude -p --permission-mode plan --append-system-prompt {system} {prompt}";
    const CODEX: &str = "codex exec --sandbox read-only {prompt}";

    #[test]
    fn parts_match_what_is_sent() {
        let request = " Make it shorter. Keep the meaning and the tone. ";
        let parts =
            prompt_parts(request, "Hello there.", AskKind::Rewrite, false, None, &[], &[(AiTool::Claude, CLAUDE)]);
        let prompt = full_prompt(request, "Hello there.", AskKind::Rewrite, false, None, &[]);
        assert_eq!(parts.request, "Make it shorter. Keep the meaning and the tone.");
        assert_eq!(parts.system, prompt.system);
        let (_, args) = command_args(CLAUDE, &prompt).unwrap_or_default();
        assert!(args.contains(&parts.system));
        assert_eq!(parts.commands[0].command, display_command(CLAUDE, request));
        assert_eq!(parts.agent, None);
        assert_eq!(parts.skills, Vec::<String>::new());
        assert!(!parts.too_long);
        assert_eq!(parts.tools(), "Claude");
    }

    #[test]
    fn parts_include_the_skill() {
        let skill = skills::parse_skill("---\nname: tidy\n---\nUse lists.", "tidy", PathBuf::new()).unwrap();
        let parts = prompt_parts("x", "y", AskKind::Rewrite, false, None, &[&skill], &[(AiTool::Claude, CLAUDE)]);
        assert_eq!(parts.skills, ["tidy"]);
        assert!(parts.system.ends_with("Follow the \"tidy\" skill:\nUse lists."));
    }

    #[test]
    fn parts_include_the_agent_before_skills() {
        let agent =
            agents::parse_agent("---\nname: Ed\ndescription: Editor\n---\nBe brief.", "ed", PathBuf::new(), None)
                .unwrap();
        let skill = skills::parse_skill("---\nname: tidy\n---\nUse lists.", "tidy", PathBuf::new()).unwrap();
        let parts =
            prompt_parts("x", "y", AskKind::Rewrite, false, Some(&agent), &[&skill], &[(AiTool::Claude, CLAUDE)]);
        assert_eq!(parts.agent.as_deref(), Some("@ai-ed (Ed)"));
        assert_eq!(parts.skills, ["tidy"]);
        let agent_at = parts.system.find("Be brief.").unwrap();
        let skill_at = parts.system.find("Use lists.").unwrap();
        assert!(agent_at < skill_at);
    }

    #[test]
    fn compare_shows_both_commands() {
        let commands = [(AiTool::Claude, CLAUDE), (AiTool::Codex, CODEX)];
        let parts = prompt_parts("why?", "x", AskKind::Answer, true, None, &[], &commands);
        assert_eq!(parts.tools(), "Claude and Codex");
        assert!(!parts.commands[0].instructions_in_prompt);
        assert!(parts.commands[1].instructions_in_prompt);
        assert!(parts.commands[1].command.starts_with("codex exec"));
        assert!(parts.commands.iter().all(|shown| shown.read_only));
    }

    #[test]
    fn describes_the_text() {
        let whole = TextSummary::new("# Notes\n\nOne two three.\n", true);
        assert_eq!(whole.describe(), "Whole file, 5 words");
        assert_eq!(whole.shown, None);
        let one = TextSummary::new("Hello", false);
        assert_eq!(one.describe(), "1 line selected, 1 word");
        assert_eq!(one.shown.as_deref(), Some("Hello"));
        let long = TextSummary::new(&"word ".repeat(100), false);
        assert_eq!(long.shown, None);
        assert_eq!(TextSummary::new("a\nb\n", false).describe(), "2 lines selected, 2 words");
    }

    #[test]
    fn flags_long_text() {
        let text = "a".repeat(MAX_PROMPT_BYTES);
        assert!(prompt_parts("x", &text, AskKind::Rewrite, true, None, &[], &[]).too_long);
    }

    #[test]
    fn finds_read_only_and_system() {
        assert!(is_read_only(CLAUDE));
        assert!(is_read_only("codex exec --sandbox=read-only {prompt}"));
        assert!(!is_read_only("grok --prompt {prompt}"));
        assert!(!is_read_only(AiTool::Gemini.default_command()));
        assert!(has_system(CLAUDE));
        assert!(!has_system("ollama run x {prompt}"));
        assert!(!has_system("{system} {prompt}"));
    }
}
