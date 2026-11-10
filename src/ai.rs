//! AI tools Rubrique can run: command-line programs such as `claude` or
//! `codex`, started with a command template from Settings. Rubrique embeds no
//! AI and holds no API keys; each tool handles its own login.
//!
//! This module is pure apart from [`find_program`], which looks at `PATH`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub mod agents;
pub mod flags;
pub mod highlight_rule;
pub mod mention_list;
pub mod naming;
pub mod preview;
pub mod run;
pub mod skills;
pub mod summary;
pub mod view;

use skills::SkillSource;

/// Placeholder in a command template for the request text.
pub const PROMPT: &str = "{prompt}";
/// Placeholder for Rubrique's instructions (how to answer). Tools without a
/// system prompt option get them at the start of `{prompt}` instead.
pub const SYSTEM: &str = "{system}";
/// Largest prompt Rubrique passes as one argument (Linux allows 128 KiB).
pub const MAX_PROMPT_BYTES: usize = 120_000;

/// The tools Rubrique knows about, in the order Settings lists them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AiTool {
    #[default]
    Claude,
    Codex,
    Grok,
    /// A small model on this machine (Ollama or llama.cpp).
    Local,
}

impl AiTool {
    pub const ALL: [AiTool; 4] = [AiTool::Claude, AiTool::Codex, AiTool::Grok, AiTool::Local];

    /// Name shown in Settings.
    pub fn name(self) -> &'static str {
        match self {
            AiTool::Claude => "Claude",
            AiTool::Codex => "Codex",
            AiTool::Grok => "Grok",
            AiTool::Local => "Local model",
        }
    }

    /// Key used in the settings file.
    pub fn id(self) -> &'static str {
        match self {
            AiTool::Claude => "claude",
            AiTool::Codex => "codex",
            AiTool::Grok => "grok",
            AiTool::Local => "local",
        }
    }

    pub fn from_id(id: &str) -> Option<AiTool> {
        AiTool::ALL.into_iter().find(|tool| tool.id() == id)
    }

    /// Command template used until the user changes it.
    pub fn default_command(self) -> &'static str {
        match self {
            // Plan mode is read-only: Claude can read nearby files but not change
            // them or run commands. No session is saved for these quick runs.
            AiTool::Claude => {
                "claude -p --output-format text --permission-mode plan --no-session-persistence \
                 --append-system-prompt {system} {prompt}"
            }
            // Read-only sandbox; notes folders are often not git repositories.
            AiTool::Codex => "codex exec --sandbox read-only --skip-git-repo-check --color never {prompt}",
            AiTool::Grok => "grok --prompt {prompt}",
            AiTool::Local => "ollama run qwen2.5:1.5b {prompt}",
        }
    }

    /// Full name for the Commands list.
    pub fn full_name(self) -> &'static str {
        match self {
            AiTool::Claude => "Claude Code",
            tool => tool.name(),
        }
    }
}

/// AI settings: which tool to use and each tool's command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiSettings {
    /// Tool for requests (Ask, Resolve notes, folder questions).
    pub default_tool: AiTool,
    /// Tool for summaries and file titles; `None` turns them off.
    pub summary_tool: Option<AiTool>,
    /// Command templates, one per tool in [`AiTool::ALL`] order.
    commands: [String; 4],
    /// Folders where the user allowed AI runs ("Don't ask again").
    pub allowed_folders: Vec<PathBuf>,
    /// Folder with Rubrique's skills; `None` means the default one.
    pub skills_folder: Option<PathBuf>,
    /// Skill sources the user switched off (all others are read).
    pub skill_sources_off: Vec<SkillSource>,
    /// Folder with Rubrique's agents; `None` means the default one.
    pub agents_folder: Option<PathBuf>,
    /// Also offer Claude Code's agents from `~/.claude/agents`.
    pub use_claude_agents: bool,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            default_tool: AiTool::Claude,
            summary_tool: Some(AiTool::Local),
            commands: AiTool::ALL.map(|tool| tool.default_command().to_string()),
            allowed_folders: Vec::new(),
            skills_folder: None,
            skill_sources_off: Vec::new(),
            agents_folder: None,
            use_claude_agents: true,
        }
    }
}

impl AiSettings {
    pub fn command(&self, tool: AiTool) -> &str {
        &self.commands[index(tool)]
    }

    pub fn command_mut(&mut self, tool: AiTool) -> &mut String {
        &mut self.commands[index(tool)]
    }

    pub fn set_command(&mut self, tool: AiTool, command: &str) {
        command.trim().clone_into(&mut self.commands[index(tool)]);
    }

    /// The command differs from the built-in one.
    pub fn is_custom(&self, tool: AiTool) -> bool {
        self.command(tool).trim() != tool.default_command()
    }

    pub fn reset_command(&mut self, tool: AiTool) {
        self.set_command(tool, tool.default_command());
    }

    /// Read one settings line; returns false when the key is not an AI key.
    pub fn parse_line(&mut self, key: &str, value: &str) -> bool {
        match key {
            "ai_tool" => self.default_tool = AiTool::from_id(value).unwrap_or(self.default_tool),
            "ai_summary_tool" if value == "off" => self.summary_tool = None,
            "ai_summary_tool" => self.summary_tool = AiTool::from_id(value).or(self.summary_tool),
            "skills_folder" => self.skills_folder = (!value.is_empty()).then(|| PathBuf::from(value)),
            // `ai_use_<source>_skills=false`, for example `ai_use_claude_skills`.
            _ if let Some(source) = skill_source_key(key) => self.set_skill_source(source, value != "false"),
            "agents_folder" => self.agents_folder = (!value.is_empty()).then(|| PathBuf::from(value)),
            "ai_use_claude_agents" => self.use_claude_agents = value != "false",
            "ai_allowed_folder" => {
                if !value.is_empty() {
                    self.allowed_folders.push(PathBuf::from(value));
                }
            }
            _ => {
                let Some(tool) = key.strip_prefix("ai_command_").and_then(AiTool::from_id) else {
                    return false;
                };
                if !value.trim().is_empty() {
                    self.set_command(tool, value);
                }
            }
        }
        true
    }

    /// Settings lines. Commands are written only when changed, so a better
    /// built-in command in a later version reaches everyone else.
    pub fn to_lines(&self) -> String {
        let summary = self.summary_tool.map_or("off", AiTool::id);
        let mut lines = format!("ai_tool={}\nai_summary_tool={summary}\n", self.default_tool.id());
        for tool in AiTool::ALL.into_iter().filter(|tool| self.is_custom(*tool)) {
            let _ = writeln!(lines, "ai_command_{}={}", tool.id(), self.command(tool).trim());
        }
        if let Some(folder) = &self.skills_folder {
            let _ = writeln!(lines, "skills_folder={}", folder.display());
        }
        for source in &self.skill_sources_off {
            let _ = writeln!(lines, "ai_use_{}_skills=false", source.id());
        }
        if let Some(folder) = &self.agents_folder {
            let _ = writeln!(lines, "agents_folder={}", folder.display());
        }
        if !self.use_claude_agents {
            lines.push_str("ai_use_claude_agents=false\n");
        }
        for folder in &self.allowed_folders {
            let _ = writeln!(lines, "ai_allowed_folder={}", folder.display());
        }
        lines
    }
}

/// The source in a settings key like `ai_use_claude_skills`.
fn skill_source_key(key: &str) -> Option<SkillSource> {
    let id = key.strip_prefix("ai_use_")?.strip_suffix("_skills")?;
    SkillSource::ALL.into_iter().find(|source| source.id() == id)
}

fn index(tool: AiTool) -> usize {
    AiTool::ALL.iter().position(|each| *each == tool).unwrap_or_default()
}

impl AiSettings {
    /// AI runs in `folder` were allowed before.
    pub fn is_allowed(&self, folder: &Path) -> bool {
        self.allowed_folders.iter().any(|allowed| allowed == folder)
    }

    /// Every skill source with its folder, in reading order, whether it is
    /// switched on or not (Settings lists them all).
    pub fn all_skill_folders(&self, default_folder: Option<&Path>) -> Vec<(SkillSource, PathBuf)> {
        SkillSource::ALL
            .into_iter()
            .filter_map(|source| {
                let folder = match source {
                    SkillSource::Rubrique => {
                        self.skills_folder.clone().or_else(|| default_folder.map(Path::to_path_buf))
                    }
                    _ => source.home_folder(),
                };
                folder.map(|folder| (source, folder))
            })
            .collect()
    }

    /// The skill sources that are switched on, with their folders, in reading order.
    pub fn skill_folders(&self, default_folder: Option<&Path>) -> Vec<(SkillSource, PathBuf)> {
        let mut folders = self.all_skill_folders(default_folder);
        folders.retain(|(source, _)| self.uses_skill_source(*source));
        folders
    }

    /// The source is switched on. Rubrique's own folder always is.
    pub fn uses_skill_source(&self, source: SkillSource) -> bool {
        source == SkillSource::Rubrique || !self.skill_sources_off.contains(&source)
    }

    /// Switch a source on or off. Rubrique's own folder stays on.
    pub fn set_skill_source(&mut self, source: SkillSource, on: bool) {
        self.skill_sources_off.retain(|each| *each != source);
        if !on && source != SkillSource::Rubrique {
            self.skill_sources_off.push(source);
        }
    }

    /// Folders to read agents from, Rubrique's own first, each with the tool
    /// its agents use when they name none (Claude Code's agents run on Claude).
    pub fn agent_folders(&self, default_folder: Option<PathBuf>) -> Vec<(PathBuf, Option<AiTool>)> {
        let mut folders: Vec<_> =
            self.agents_folder.clone().or(default_folder).into_iter().map(|folder| (folder, None)).collect();
        if self.use_claude_agents {
            folders.extend(agents::claude_agents_folder().map(|folder| (folder, Some(AiTool::Claude))));
        }
        folders
    }

    pub fn allow(&mut self, folder: &Path) {
        if !self.is_allowed(folder) {
            self.allowed_folders.push(folder.to_path_buf());
        }
    }
}

/// What the answer is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskKind {
    /// New text for the selection, reviewed in the merge screen.
    Rewrite,
    /// An answer to read (Explain); nothing in the file changes.
    Answer,
}

/// The small buttons under the Ask field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickAction {
    Shorter,
    FixGrammar,
    Table,
    Translate,
    Explain,
}

impl QuickAction {
    pub const ALL: [QuickAction; 5] = [
        QuickAction::Shorter,
        QuickAction::FixGrammar,
        QuickAction::Table,
        QuickAction::Translate,
        QuickAction::Explain,
    ];

    pub fn label(self) -> &'static str {
        match self {
            QuickAction::Shorter => "Shorter",
            QuickAction::FixGrammar => "Fix grammar",
            QuickAction::Table => "Table",
            QuickAction::Translate => "Translate…",
            QuickAction::Explain => "Explain",
        }
    }

    /// The request this button sends. Translate has none: it fills the
    /// field with "Translate to " for the user to finish.
    pub fn request(self) -> Option<&'static str> {
        match self {
            QuickAction::Shorter => Some("Make it shorter. Keep the meaning and the tone."),
            QuickAction::FixGrammar => Some("Fix spelling and grammar. Change nothing else."),
            QuickAction::Table => Some("Turn it into a Markdown table."),
            QuickAction::Translate => None,
            QuickAction::Explain => Some("Explain this in simple words."),
        }
    }

    pub fn kind(self) -> AskKind {
        if self == QuickAction::Explain { AskKind::Answer } else { AskKind::Rewrite }
    }
}

/// What is sent to a tool: Rubrique's instructions and the request with its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    /// How to answer; goes to `{system}` when the command has it.
    pub system: String,
    /// The request and the text it is about; goes to `{prompt}`.
    pub user: String,
}

impl Prompt {
    /// Add a skill's instructions to Rubrique's own.
    pub fn with_skill(mut self, skill: &skills::Skill) -> Prompt {
        let _ = write!(self.system, "\n\nFollow the \"{}\" skill:\n{}", skill.name, skill.instructions);
        self
    }

    /// Add an agent's personality and rules to Rubrique's own. Rubrique's
    /// rules about the answer format still come first and win.
    pub fn with_agent(mut self, agent: &agents::Agent) -> Prompt {
        let _ = write!(
            self.system,
            "\n\nYou are the agent \"{}\" ({}). Work in its way, but keep the answer format above:\n{}",
            agent.name, agent.description, agent.instructions
        );
        self
    }

    /// Too long to pass as a command-line argument.
    pub fn is_too_long(&self) -> bool {
        self.system.len() + self.user.len() > MAX_PROMPT_BYTES
    }
}

/// Build the prompt for `request` about `text`.
pub fn build_prompt(request: &str, text: &str, kind: AskKind, whole_file: bool) -> Prompt {
    let what = if whole_file { "a Markdown file" } else { "part of a Markdown file" };
    let how = match kind {
        AskKind::Rewrite => {
            "Apply the request to the text and reply with only the new text: \
             no explanations, no quotes, no code fences around it. \
             Keep the Markdown formatting and the language of the text unless the request is about them. \
             Do not use tools or change any files."
        }
        AskKind::Answer => {
            "Answer the request about the text. Reply in Markdown, short and clear. \
             Do not use tools or change any files."
        }
    };
    Prompt {
        system: format!("You are helping inside Rubrique, a Markdown editor. {how}"),
        user: format!("Request: {}\n\nThe text is {what}:\n<text>\n{text}\n</text>\n", request.trim()),
    }
}

/// Request for resolving every note in a file.
pub const RESOLVE_ALL_REQUEST: &str = "Resolve every note. A note is an HTML comment like \
<!-- note on \"quoted text\": what to do -->, placed right after the text it is about. \
For each note, change the quoted text as the note asks, then remove that comment. \
Leave a note in place when you cannot resolve it. Change nothing else.";

/// Request for resolving the note on `quote` that says `text`.
pub fn resolve_note_request(quote: &str, text: &str) -> String {
    format!(
        "Resolve one note. The note on \"{quote}\" says: \"{text}\". \
         Change that text as the note asks, then remove this note's comment \
         (<!-- note on \"{quote}\": ... -->). Leave all other text and notes as they are."
    )
}

/// The tool's reply as new text: surrounding blank lines and a wrapping
/// code fence removed, and the original's final line break kept.
pub fn clean_reply(reply: &str, original: &str) -> String {
    let mut text = reply.trim();
    if let Some(inner) = text.strip_prefix("```") {
        // Drop the fence line (with any language name) and the closing fence.
        if let Some((_, body)) = inner.split_once('\n')
            && let Some(body) = body.trim_end().strip_suffix("```")
        {
            text = body.trim();
        }
    }
    let mut cleaned = text.to_string();
    if original.ends_with('\n') {
        cleaned.push('\n');
    }
    cleaned
}

/// The program and its arguments for `command` with the prompt filled in.
/// Placeholders inside a word are replaced too (`--prompt={prompt}`).
pub fn command_args(command: &str, prompt: &Prompt) -> Option<(String, Vec<String>)> {
    let mut words = split_command(command).into_iter();
    let program = words.next().filter(|program| !program.is_empty())?;
    let has_system = preview::has_system(command);
    let user = if has_system { prompt.user.clone() } else { format!("{}\n\n{}", prompt.system, prompt.user) };
    let args = words.map(|word| word.replace(SYSTEM, &prompt.system).replace(PROMPT, &user)).collect();
    Some((program, args))
}

/// The command as shown to the user, with a long request cut short.
pub fn display_command(command: &str, request: &str) -> String {
    const SHOWN: usize = 48;
    let short: String = request.chars().take(SHOWN).collect();
    let quoted = if request.chars().count() > SHOWN { format!("\"{short}…\"") } else { format!("\"{short}\"") };
    split_command(command)
        .iter()
        .map(|word| word.replace(SYSTEM, "\"(Rubrique's instructions)\"").replace(PROMPT, &quoted))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Seconds as `m:ss`.
pub fn format_elapsed(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// What is wrong with a command template, if anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandProblem {
    Empty,
    /// No `{prompt}` word, so the request would not reach the tool.
    NoPrompt,
}

impl CommandProblem {
    pub fn message(self) -> &'static str {
        match self {
            CommandProblem::Empty => "Enter a command.",
            CommandProblem::NoPrompt => "Add {prompt} where the request goes.",
        }
    }
}

/// Check a command template.
pub fn check_command(command: &str) -> Option<CommandProblem> {
    let words = split_command(command);
    if words.is_empty() {
        Some(CommandProblem::Empty)
    } else if !words.iter().any(|word| word == PROMPT) {
        Some(CommandProblem::NoPrompt)
    } else {
        None
    }
}

/// Split a template into words. Double or single quotes keep spaces inside
/// one word. The program later gets these words as separate arguments, never
/// through a shell, so text in a note cannot run commands.
pub fn split_command(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote = None;
    for character in command.chars() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => word.push(character),
            (None, '"' | '\'') => {
                quote = Some(character);
                in_word = true;
            }
            (None, _) if character.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, _) => {
                word.push(character);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

/// Words back into a template that [`split_command`] reads the same.
pub fn join_command(words: &[String]) -> String {
    words
        .iter()
        .map(|word| {
            if !word.is_empty() && !word.contains(|c: char| c.is_whitespace() || c == '"' || c == '\'') {
                word.clone()
            } else if word.contains('"') {
                format!("'{word}'")
            } else {
                format!("\"{word}\"")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The program a template starts (its first word).
pub fn program(command: &str) -> Option<String> {
    split_command(command).into_iter().next()
}

/// Where the template's program is: the file itself for a path, otherwise
/// the first file of that name in a `PATH` folder.
pub fn find_program(command: &str) -> Option<PathBuf> {
    find_on_path(&program(command)?)
}

/// Where `program` is: the file itself for a path, otherwise the first file
/// of that name in a `PATH` folder.
pub fn find_on_path(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let path = PathBuf::from(program);
        return path.is_file().then_some(path);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|folder| folder.join(program)).find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_have_a_prompt() {
        for tool in AiTool::ALL {
            assert_eq!(check_command(tool.default_command()), None, "{}", tool.name());
            assert_eq!(AiTool::from_id(tool.id()), Some(tool));
        }
    }

    #[test]
    fn splits_words_and_quotes() {
        assert_eq!(split_command("claude -p {prompt}"), ["claude", "-p", "{prompt}"]);
        assert_eq!(
            split_command(r#"llama-cli -m "/my models/q.gguf" -p {prompt}"#),
            ["llama-cli", "-m", "/my models/q.gguf", "-p", "{prompt}"]
        );
        assert_eq!(split_command("a '' b"), ["a", "", "b"]);
        assert_eq!(split_command("   "), Vec::<String>::new());
    }

    #[test]
    fn finds_problems() {
        assert_eq!(check_command(" "), Some(CommandProblem::Empty));
        assert_eq!(check_command("claude -p"), Some(CommandProblem::NoPrompt));
        assert_eq!(check_command("claude -p '{prompt}'"), None);
    }

    #[test]
    fn writes_only_changed_commands() {
        let mut settings = AiSettings::default();
        assert_eq!(settings.to_lines(), "ai_tool=claude\nai_summary_tool=local\n");
        settings.set_command(AiTool::Local, "  ollama run gemma3:1b {prompt} ");
        settings.default_tool = AiTool::Codex;
        let text = settings.to_lines();
        assert_eq!(text, "ai_tool=codex\nai_summary_tool=local\nai_command_local=ollama run gemma3:1b {prompt}\n");

        let mut read = AiSettings::default();
        for line in text.lines() {
            let (key, value) = line.split_once('=').unwrap_or_default();
            assert!(read.parse_line(key, value));
        }
        assert_eq!(read, settings);
        read.reset_command(AiTool::Local);
        assert!(!read.is_custom(AiTool::Local));
    }

    #[test]
    fn summaries_can_be_off() {
        let mut settings = AiSettings::default();
        assert!(settings.parse_line("ai_summary_tool", "off"));
        assert_eq!(settings.summary_tool, None);
        assert!(settings.to_lines().contains("ai_summary_tool=off\n"));
        settings.parse_line("ai_summary_tool", "codex");
        assert_eq!(settings.summary_tool, Some(AiTool::Codex));
    }

    #[test]
    fn ignores_unknown_values() {
        let mut settings = AiSettings::default();
        assert!(settings.parse_line("ai_tool", "gpt"));
        assert!(settings.parse_line("ai_command_local", ""));
        assert!(!settings.parse_line("ai_command_other", "x {prompt}"));
        assert!(!settings.parse_line("theme", "dark"));
        assert_eq!(settings, AiSettings::default());
    }

    #[test]
    fn builds_prompts() {
        let prompt = build_prompt(" shorter ", "Hello there.", AskKind::Rewrite, false);
        assert!(prompt.system.contains("only the new text"));
        assert!(prompt.user.starts_with("Request: shorter\n"));
        assert!(prompt.user.contains("part of a Markdown file"));
        assert!(prompt.user.ends_with("<text>\nHello there.\n</text>\n"));
        let answer = build_prompt("why?", "x", AskKind::Answer, true);
        assert!(answer.system.contains("Answer the request"));
        assert!(answer.user.contains("The text is a Markdown file"));
        assert!(!answer.is_too_long());
        assert!(build_prompt("x", &"a".repeat(MAX_PROMPT_BYTES), AskKind::Rewrite, true).is_too_long());
        let skill = skills::parse_skill("---\nname: tidy\n---\nUse lists.", "tidy", PathBuf::new());
        let with =
            build_prompt("x", "y", AskKind::Rewrite, false).with_skill(&skill.unwrap_or_else(|| panic!("skill")));
        assert!(with.system.ends_with("Follow the \"tidy\" skill:\nUse lists."));
        let agent =
            agents::parse_agent("---\nname: John\ndescription: Editor\n---\nBe brief.", "j", PathBuf::new(), None);
        let with =
            build_prompt("x", "y", AskKind::Rewrite, false).with_agent(&agent.unwrap_or_else(|| panic!("agent")));
        assert!(with.system.starts_with("You are helping inside Rubrique"));
        assert!(
            with.system
                .ends_with("agent \"John\" (Editor). Work in its way, but keep the answer format above:\nBe brief.")
        );
    }

    #[test]
    fn note_requests_name_the_note() {
        let request = resolve_note_request("the first Monday", "check with legal");
        assert!(request.contains("The note on \"the first Monday\" says: \"check with legal\""));
        assert!(request.contains("remove this note's comment"));
        assert!(RESOLVE_ALL_REQUEST.contains("<!-- note on"));
    }

    #[test]
    fn cleans_replies() {
        assert_eq!(clean_reply("\n  New text.\n\n", "Old text."), "New text.");
        assert_eq!(clean_reply("New text.", "Old text.\n"), "New text.\n");
        assert_eq!(clean_reply("```markdown\n| a | b |\n```\n", "a b"), "| a | b |");
        assert_eq!(clean_reply("```rust\nfn main() {}\nno close", "x"), "```rust\nfn main() {}\nno close");
    }

    #[test]
    fn fills_in_the_prompt() {
        let prompt = Prompt { system: "Rules.".to_string(), user: "say \"hi\"; rm -rf /".to_string() };
        let (program, args) =
            command_args("claude -p --append-system-prompt {system} {prompt}", &prompt).unwrap_or_default();
        assert_eq!(program, "claude");
        assert_eq!(args, ["-p", "--append-system-prompt", "Rules.", "say \"hi\"; rm -rf /"]);
        // Without {system}, the rules lead the prompt.
        let (_, args) = command_args("tool --prompt={prompt}", &prompt).unwrap_or_default();
        assert_eq!(args, ["--prompt=Rules.\n\nsay \"hi\"; rm -rf /"]);
        assert_eq!(command_args("  ", &prompt), None);
    }

    #[test]
    fn shows_commands_short() {
        assert_eq!(display_command("claude -p {prompt}", "shorter"), "claude -p \"shorter\"");
        assert_eq!(
            display_command("claude --append-system-prompt {system} {prompt}", "x"),
            "claude --append-system-prompt \"(Rubrique's instructions)\" \"x\""
        );
        let long = "a".repeat(60);
        assert_eq!(display_command("codex exec {prompt}", &long), format!("codex exec \"{}…\"", "a".repeat(48)));
        assert_eq!(format_elapsed(7), "0:07");
        assert_eq!(format_elapsed(125), "2:05");
    }

    #[test]
    fn remembers_allowed_folders() {
        let mut settings = AiSettings {
            skills_folder: Some(PathBuf::from("/s")),
            skill_sources_off: vec![SkillSource::Agents, SkillSource::Claude, SkillSource::Grok],
            agents_folder: Some(PathBuf::from("/agents")),
            use_claude_agents: false,
            ..AiSettings::default()
        };
        assert_eq!(settings.agent_folders(None), [(PathBuf::from("/agents"), None)]);
        assert_eq!(settings.skill_folders(Some(Path::new("/default"))), [(SkillSource::Rubrique, PathBuf::from("/s"))]);
        let all: Vec<SkillSource> = settings.all_skill_folders(None).into_iter().map(|(source, _)| source).collect();
        assert_eq!(all.first(), Some(&SkillSource::Rubrique));
        settings.allow(Path::new("/home/a/notes"));
        settings.allow(Path::new("/home/a/notes"));
        assert!(settings.is_allowed(Path::new("/home/a/notes")));
        assert!(!settings.is_allowed(Path::new("/home/a")));
        let mut read = AiSettings::default();
        for line in settings.to_lines().lines() {
            let (key, value) = line.split_once('=').unwrap_or_default();
            read.parse_line(key, value);
        }
        assert_eq!(read, settings);
    }

    #[test]
    fn finds_programs_by_path() {
        assert_eq!(find_program("surely-not-a-real-program-xyz {prompt}"), None);
        assert_eq!(find_program(""), None);
        assert_eq!(find_program("/bin/sh {prompt}"), Some(PathBuf::from("/bin/sh")));
        assert!(find_program("sh {prompt}").is_some_and(|path| path.ends_with("sh")));
    }
}
