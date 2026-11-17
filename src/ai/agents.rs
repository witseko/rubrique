//! Agents: named helpers with their own personality, skills and AI tool.
//! You call one with `@ai-<handle>` in the Ask bar. Each agent is a folder
//! with an `AGENT.md` file, or a plain `<name>.md` file (the format of Claude
//! Code's agents in `~/.claude/agents`, so those work too):
//!
//! ```text
//! ---
//! name: John
//! handle: john
//! description: Fact checker. Checks dates, numbers and names.
//! tool: claude
//! model: opus
//! skills: fact-check, plain-english
//! ---
//! You are John, a careful fact checker ...
//! ```
//!
//! The body is the agent's personality and rules. Rubrique adds it to what it
//! sends to the tool, after its own rules about how to answer.

use std::fs;
use std::path::{Path, PathBuf};

use super::flags::{insert_at, is_ollama};
use super::skills::{Skill, split_front_matter, unquote};
use super::{AiTool, join_command, split_command};

/// The file that makes a folder an agent.
pub const AGENT_FILE: &str = "AGENT.md";
/// What every mention starts with, after `@`: `@ai-john`. The prefix keeps
/// agents apart from real people named in shared notes.
pub const MENTION_PREFIX: &str = "ai-";

/// One agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    /// Shown in the UI.
    pub name: String,
    /// The part after `@ai-`: lower case letters, digits and `-`.
    pub handle: String,
    pub description: String,
    /// Tool to run on; `None` means the default tool from Settings.
    pub tool: Option<AiTool>,
    /// Model for tools that take `--model` (or `ollama run <model>`).
    pub model: Option<String>,
    /// A full command template that replaces the tool's command.
    pub command: Option<String>,
    /// Names of skills to always add.
    pub skills: Vec<String>,
    /// The body: personality and rules.
    pub instructions: String,
    /// The AGENT.md (or `<name>.md`) file.
    pub path: PathBuf,
}

impl Agent {
    /// How you call the agent: `@ai-john`.
    pub fn mention(&self) -> String {
        format!("@{MENTION_PREFIX}{}", self.handle)
    }

    /// The agent's command for `tool`: its own command when it has one,
    /// otherwise `base` with its model added when it runs on its own tool.
    pub fn command_for(&self, tool: AiTool, base: &str) -> String {
        if let Some(command) = &self.command {
            return command.clone();
        }
        match &self.model {
            Some(model) if self.tool.is_none_or(|own| own == tool) => with_model(base, tool, model),
            _ => base.to_string(),
        }
    }

    /// A short line for lists: "Claude · opus" or "Default tool".
    pub fn engine(&self) -> String {
        let tool = if self.command.is_some() { "Own command" } else { self.tool.map_or("Default tool", AiTool::name) };
        match &self.model {
            Some(model) if self.command.is_none() => format!("{tool} · {model}"),
            _ => tool.to_string(),
        }
    }
}

/// A handle made from `text`: lower case, spaces and `_` as `-`, other
/// characters dropped, and no `ai-` at the start (the mention adds it).
pub fn make_handle(text: &str) -> String {
    let mut handle = String::new();
    for character in text.trim().chars() {
        if character.is_alphanumeric() {
            handle.extend(character.to_lowercase());
        } else if (character.is_whitespace() || character == '_' || character == '-') && !handle.ends_with('-') {
            handle.push('-');
        }
    }
    let handle = handle.trim_matches('-');
    handle.strip_prefix(MENTION_PREFIX).unwrap_or(handle).to_string()
}

/// Read an agent file. `fallback_name` is the folder or file name.
/// `default_tool` is used when the file names none (Claude Code agents run
/// on Claude).
pub fn parse_agent(text: &str, fallback_name: &str, path: PathBuf, default_tool: Option<AiTool>) -> Option<Agent> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let (front, body) = split_front_matter(text);
    let mut name = None;
    let mut handle = None;
    let mut description = None;
    let mut tool = default_tool;
    let mut model = None;
    let mut command = None;
    let mut skills = Vec::new();
    for line in front.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = unquote(value.trim());
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "name" => name = Some(value),
            "handle" => handle = Some(value),
            "description" => description = Some(value),
            "tool" => tool = AiTool::from_id(&value.to_lowercase()).or(tool),
            // Claude Code uses `inherit` for "the session's model".
            "model" if value != "inherit" => model = Some(value),
            "command" if super::check_command(&value).is_none() => command = Some(value),
            "skills" => {
                skills =
                    value.split(',').map(|skill| unquote(skill.trim())).filter(|skill| !skill.is_empty()).collect();
            }
            _ => {}
        }
    }
    let instructions = body.trim().to_string();
    if instructions.is_empty() {
        return None;
    }
    let name = name.unwrap_or_else(|| fallback_name.to_string());
    let handle = make_handle(handle.as_deref().unwrap_or(&name));
    if handle.is_empty() {
        return None;
    }
    let description = description.unwrap_or_else(|| format!("Agent {name}"));
    Some(Agent { name, handle, description, tool, model, command, skills, instructions, path })
}

/// All agents in `folders`, each with the tool to use when its file names
/// none. When two agents have the same handle, the first one found wins.
pub fn load_agents(folders: &[(PathBuf, Option<AiTool>)]) -> Vec<Agent> {
    let mut agents: Vec<Agent> = Vec::new();
    for (folder, default_tool) in folders {
        let Ok(entries) = fs::read_dir(folder) else {
            continue;
        };
        let mut found: Vec<Agent> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                let (file, fallback) = if path.is_dir() {
                    (path.join(AGENT_FILE), entry.file_name().to_string_lossy().into_owned())
                } else if path.extension().is_some_and(|extension| extension == "md") {
                    let stem = path.file_stem()?.to_string_lossy().into_owned();
                    (path, stem)
                } else {
                    return None;
                };
                let text = fs::read_to_string(&file).ok()?;
                parse_agent(&text, &fallback, file, *default_tool)
            })
            .collect();
        found.sort_by(|a, b| a.path.cmp(&b.path));
        for agent in found {
            if !agents.iter().any(|known| known.handle == agent.handle) {
                agents.push(agent);
            }
        }
    }
    agents.sort_by_key(|agent| agent.handle.clone());
    agents
}

/// Claude Code's own agents folder, `~/.claude/agents`.
pub fn claude_agents_folder() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| Path::new(&home).join(".claude/agents"))
}

/// Gemini CLI's own agents folder, `~/.gemini/agents`. Its `<name>.md`
/// files have the same front matter (`name`, `description`, `model`).
pub fn gemini_agents_folder() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| Path::new(&home).join(".gemini/agents"))
}

/// Agents whose mention starts with `typed` (with or without `@` and `ai-`),
/// or whose name does. An empty `typed` matches every agent.
pub fn matching<'a>(agents: &'a [Agent], typed: &str) -> Vec<&'a Agent> {
    let typed = typed.trim().trim_start_matches('@').to_lowercase();
    let bare = typed.strip_prefix(MENTION_PREFIX).unwrap_or(&typed);
    agents
        .iter()
        .filter(|agent| {
            agent.handle.starts_with(bare)
                || format!("{MENTION_PREFIX}{}", agent.handle).starts_with(&typed)
                || agent.name.to_lowercase().starts_with(bare)
        })
        .collect()
}

/// A leading `@word` in `request`: the word (without `@`) and the rest of
/// the request. `None` when the request does not start with `@`.
pub fn leading_mention(request: &str) -> Option<(&str, &str)> {
    let rest = request.trim_start().strip_prefix('@')?;
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some((&rest[..end], rest[end..].trim_start()))
}

/// The agent a mention word (`ai-john`, without `@`) names exactly.
pub fn find_by_mention<'a>(agents: &'a [Agent], word: &str) -> Option<&'a Agent> {
    let word = word.to_lowercase();
    let handle = word.strip_prefix(MENTION_PREFIX)?;
    agents.iter().find(|agent| agent.handle == handle)
}

/// The agent's skills found in `skills` by name (any case), in the agent's
/// order, and the names that were not found.
pub fn find_skills<'a>(agent: &Agent, skills: &'a [Skill]) -> (Vec<&'a Skill>, Vec<String>) {
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for name in &agent.skills {
        match skills.iter().find(|skill| skill.name.eq_ignore_ascii_case(name)) {
            Some(skill) => found.push(skill),
            None => missing.push(name.clone()),
        }
    }
    (found, missing)
}

/// `command` with `--model <model>` set: an existing `--model` value is
/// replaced; otherwise the option goes after the program (after `exec` for
/// Codex). For `ollama run <model>` the model word is replaced.
pub fn with_model(command: &str, tool: AiTool, model: &str) -> String {
    let mut words = split_command(command);
    if words.is_empty() {
        return command.to_string();
    }
    if let Some(index) = words.iter().position(|word| word == "--model" || word == "-m")
        && index + 1 < words.len()
    {
        model.clone_into(&mut words[index + 1]);
    } else if let Some(index) = words.iter().position(|word| word.starts_with("--model=")) {
        words[index] = format!("--model={model}");
    } else if tool == AiTool::Local {
        if is_ollama(&words[0]) && words.get(1).is_some_and(|word| word == "run") && words.len() > 2 {
            model.clone_into(&mut words[2]);
        }
    } else {
        let at = insert_at(&words, tool);
        words.splice(at..at, ["--model".to_string(), model.to_string()]);
    }
    join_command(&words)
}

/// Folder name of the example agent.
pub const EXAMPLE_NAME: &str = "editor";

/// Write an example agent into `folder`. Returns its AGENT.md path.
///
/// # Errors
///
/// Fails when the folder cannot be created or the file written. An
/// existing example is left as it is.
pub fn create_example(folder: &Path) -> std::io::Result<PathBuf> {
    let agent_folder = folder.join(EXAMPLE_NAME);
    fs::create_dir_all(&agent_folder)?;
    let path = agent_folder.join(AGENT_FILE);
    if !path.exists() {
        fs::write(&path, EXAMPLE_AGENT)?;
    }
    Ok(path)
}

const EXAMPLE_AGENT: &str = "---
name: Editor
handle: editor
description: Tightens prose. Keeps the meaning, the tone and the language.
# tool: claude        (claude, codex, gemini, grok or local; default: the tool picked in Ask)
# model: opus         (passed as --model)
# skills: action-items
---
You are a calm, experienced editor.

- Cut filler words and repeated ideas. Prefer short sentences and plain words.
- Keep the author's meaning, tone, facts and language.
- Keep Markdown structure: headings, lists, links and code stay as they are.
- Never add new facts or opinions.
";

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(text: &str) -> Agent {
        parse_agent(text, "folder", PathBuf::from("/a/AGENT.md"), None).unwrap_or_else(|| panic!("parsed"))
    }

    #[test]
    fn reads_front_matter() {
        let john = agent(
            "---\nname: John Smith\nhandle: John\ndescription: \"Facts: dates\"\ntool: Claude\nmodel: opus\n\
             skills: fact-check, 'plain'\n---\n\nCheck facts.\n",
        );
        assert_eq!(john.name, "John Smith");
        assert_eq!(john.handle, "john");
        assert_eq!(john.mention(), "@ai-john");
        assert_eq!(john.description, "Facts: dates");
        assert_eq!(john.tool, Some(AiTool::Claude));
        assert_eq!(john.model.as_deref(), Some("opus"));
        assert_eq!(john.skills, ["fact-check", "plain"]);
        assert_eq!(john.instructions, "Check facts.");
        assert_eq!(john.engine(), "Claude · opus");
    }

    #[test]
    fn fills_in_defaults() {
        let plain = agent("Be nice.");
        assert_eq!((plain.name.as_str(), plain.handle.as_str()), ("folder", "folder"));
        assert_eq!(plain.description, "Agent folder");
        assert_eq!(plain.tool, None);
        assert_eq!(plain.engine(), "Default tool");
        let claude = parse_agent("---\nname: Rev\nmodel: inherit\n---\nx", "f", PathBuf::new(), Some(AiTool::Claude));
        let claude = claude.unwrap_or_else(|| panic!("parsed"));
        assert_eq!((claude.tool, claude.model), (Some(AiTool::Claude), None));
        assert_eq!(parse_agent("---\nname: e\n---\n ", "f", PathBuf::new(), None), None);
        assert_eq!(parse_agent("---\nname: '!!'\n---\nx", "f", PathBuf::new(), None), None);
        // A command without {prompt} is ignored.
        assert_eq!(agent("---\ncommand: claude -p\n---\nx").command, None);
    }

    #[test]
    fn makes_handles() {
        assert_eq!(make_handle("Mary Ann_Lee"), "mary-ann-lee");
        assert_eq!(make_handle("  ai-John!! "), "john");
        assert_eq!(make_handle("--x--"), "x");
        assert_eq!(make_handle("Ölaf"), "ölaf");
    }

    #[test]
    fn finds_mentions() {
        let agents = [agent("---\nname: John\n---\nx"), agent("---\nname: Mary\nhandle: m\n---\nx")];
        assert_eq!(leading_mention("  @ai-john  make it short"), Some(("ai-john", "make it short")));
        assert_eq!(leading_mention("@"), Some(("", "")));
        assert_eq!(leading_mention("hi @ai-john"), None);
        assert_eq!(find_by_mention(&agents, "AI-John").map(|a| a.name.as_str()), Some("John"));
        assert_eq!(find_by_mention(&agents, "john"), None);
        assert_eq!(matching(&agents, "@").len(), 2);
        assert_eq!(matching(&agents, "@ai-j").len(), 1);
        assert_eq!(matching(&agents, "mar").len(), 1);
        assert_eq!(matching(&agents, "@ai-x").len(), 0);
    }

    #[test]
    fn finds_the_agents_skills() {
        let fact = super::super::skills::parse_skill("---\nname: Fact-Check\n---\nCheck.", "f", PathBuf::new());
        let skills = [fact.unwrap_or_else(|| panic!("skill"))];
        let editor = agent("---\nname: Ed\nskills: fact-check, missing\n---\nBe brief.");
        let (found, missing) = find_skills(&editor, &skills);
        assert_eq!(found.iter().map(|skill| skill.name.as_str()).collect::<Vec<_>>(), ["Fact-Check"]);
        assert_eq!(missing, ["missing"]);
    }

    #[test]
    fn adds_the_model() {
        let claude = "claude -p --append-system-prompt {system} {prompt}";
        assert_eq!(
            with_model(claude, AiTool::Claude, "opus"),
            "claude --model opus -p --append-system-prompt {system} {prompt}"
        );
        assert_eq!(
            with_model("codex exec --model x --sandbox read-only {prompt}", AiTool::Codex, "o3"),
            "codex exec --model o3 --sandbox read-only {prompt}"
        );
        assert_eq!(with_model("codex exec {prompt}", AiTool::Codex, "o3"), "codex exec --model o3 {prompt}");
        assert_eq!(with_model("grok --model=a {prompt}", AiTool::Grok, "b"), "grok --model=b {prompt}");
        assert_eq!(
            with_model("ollama run qwen2.5:1.5b {prompt}", AiTool::Local, "gemma3:1b"),
            "ollama run gemma3:1b {prompt}"
        );
        assert_eq!(with_model("llama-cli -p {prompt}", AiTool::Local, "q"), "llama-cli -p {prompt}");
        assert_eq!(
            with_model(r#"tool -x "two words" '' {prompt}"#, AiTool::Grok, "m"),
            r#"tool --model m -x "two words" "" {prompt}"#
        );
    }

    #[test]
    fn picks_the_command() {
        let mut john = agent("---\ntool: claude\nmodel: opus\n---\nx");
        assert_eq!(john.command_for(AiTool::Claude, "claude {prompt}"), "claude --model opus {prompt}");
        // The model belongs to Claude, so Codex runs as set up.
        assert_eq!(john.command_for(AiTool::Codex, "codex exec {prompt}"), "codex exec {prompt}");
        john.command = Some("my-tool {prompt}".to_string());
        assert_eq!(john.command_for(AiTool::Codex, "codex exec {prompt}"), "my-tool {prompt}");
        assert_eq!(john.engine(), "Own command");
    }

    #[test]
    fn loads_folders_and_files() {
        let root = std::env::temp_dir().join(format!("rubrique-agents-{}", std::process::id()));
        let mine = root.join("mine");
        let claude = root.join("claude");
        fs::create_dir_all(mine.join("john")).unwrap_or_default();
        fs::create_dir_all(&claude).unwrap_or_default();
        fs::write(mine.join("john").join(AGENT_FILE), "---\nname: John\n---\nmine").unwrap_or_default();
        fs::write(claude.join("john.md"), "---\nname: john\n---\nclaude").unwrap_or_default();
        fs::write(claude.join("reviewer.md"), "---\nname: reviewer\n---\nreview").unwrap_or_default();
        fs::write(claude.join("notes.txt"), "not an agent").unwrap_or_default();
        let agents = load_agents(&[(mine.clone(), None), (claude, Some(AiTool::Claude)), (root.join("gone"), None)]);
        let handles: Vec<&str> = agents.iter().map(|agent| agent.handle.as_str()).collect();
        assert_eq!(handles, ["john", "reviewer"]);
        assert_eq!(agents[0].instructions, "mine");
        assert_eq!(agents[1].tool, Some(AiTool::Claude));

        let path = create_example(&mine).unwrap_or_default();
        assert!(path.ends_with("editor/AGENT.md"));
        let example = load_agents(&[(mine, None)]).into_iter().find(|agent| agent.handle == "editor");
        let example = example.unwrap_or_else(|| panic!("example"));
        // The commented lines are not read as settings.
        assert_eq!((example.tool, example.model, example.skills.len()), (None, None, 0));
        fs::remove_dir_all(&root).unwrap_or_default();
    }
}
