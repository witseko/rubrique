//! Skills: reusable instructions for AI requests, in the same format as
//! Claude Code skills, so one skill works in both. Each skill is a folder
//! with a `SKILL.md` file:
//!
//! ```text
//! ---
//! name: action-items
//! description: Turn meeting notes into a list of action items.
//! ---
//! Make a Markdown task list ...
//! ```
//!
//! Rubrique adds the instructions to what it sends to any tool. An optional
//! `output: answer` line makes the result an answer to read instead of new
//! text for the selection.
//!
//! Skills come from several [`SkillSource`]s: Rubrique's own folder and the
//! folders other agents read (`~/.agents/skills`, Claude Code, Grok, Gemini).

use std::fs;
use std::path::{Path, PathBuf};

use super::AskKind;

/// The file that makes a folder a skill.
pub const SKILL_FILE: &str = "SKILL.md";

/// A folder of skills Rubrique reads, in the order they are read: when two
/// sources have a skill with the same name, the earlier source wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SkillSource {
    /// Rubrique's own folder (the chosen one or the default). Always read.
    Rubrique,
    /// `~/.agents/skills`, the shared folder Codex and other agents read.
    Agents,
    /// Claude Code's `~/.claude/skills`.
    Claude,
    /// Grok's `~/.grok/skills`.
    Grok,
    /// Gemini CLI's `~/.gemini/skills`.
    Gemini,
}

impl SkillSource {
    pub const ALL: [SkillSource; 5] = [Self::Rubrique, Self::Agents, Self::Claude, Self::Grok, Self::Gemini];

    /// Key used in the settings file.
    pub fn id(self) -> &'static str {
        match self {
            Self::Rubrique => "rubrique",
            Self::Agents => "agents",
            Self::Claude => "claude",
            Self::Grok => "grok",
            Self::Gemini => "gemini",
        }
    }

    /// Name shown in Settings.
    pub fn name(self) -> &'static str {
        match self {
            Self::Rubrique => "Rubrique",
            Self::Agents => "Agents",
            Self::Claude => "Claude Code",
            Self::Grok => "Grok",
            Self::Gemini => "Gemini CLI",
        }
    }

    /// The folder in the home folder, for every source but Rubrique's own
    /// (which lives in the config folder or wherever the user put it).
    pub fn home_folder(self) -> Option<PathBuf> {
        let relative = match self {
            Self::Rubrique => return None,
            Self::Agents => ".agents/skills",
            Self::Claude => ".claude/skills",
            Self::Grok => ".grok/skills",
            Self::Gemini => ".gemini/skills",
        };
        std::env::var_os("HOME").map(|home| Path::new(&home).join(relative))
    }
}

/// One skill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// The body of SKILL.md: what the tool should do.
    pub instructions: String,
    /// New text for the selection, or an answer to read.
    pub kind: AskKind,
    /// The SKILL.md file.
    pub path: PathBuf,
    /// Where [`Skill::path`] was found.
    pub source: SkillSource,
    /// Other sources that hold the same file (often a symlink, for example
    /// `~/.claude/skills/x` pointing at `~/.agents/skills/x`).
    pub linked: Vec<(SkillSource, PathBuf)>,
    /// Different skills with the same name in later sources, not used.
    pub skipped: Vec<(SkillSource, PathBuf)>,
}

impl Skill {
    /// Every source that offers this skill: its own first, then the linked ones.
    pub fn sources(&self) -> impl Iterator<Item = SkillSource> + '_ {
        std::iter::once(self.source).chain(self.linked.iter().map(|(source, _)| *source))
    }

    /// The name or description contains `query`, ignoring case. An empty
    /// query matches every skill.
    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        self.name.to_lowercase().contains(&query) || self.description.to_lowercase().contains(&query)
    }
}

/// A skill source's folder and whether it exists, for Settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFolder {
    pub source: SkillSource,
    pub path: PathBuf,
    pub exists: bool,
}

/// Look up which of `folders` exist. Done once per load, not every frame.
pub fn source_folders(folders: Vec<(SkillSource, PathBuf)>) -> Vec<SourceFolder> {
    folders.into_iter().map(|(source, path)| SourceFolder { source, exists: path.is_dir(), path }).collect()
}

/// Read a SKILL.md. The name falls back to the folder name. The source is
/// Rubrique's own; [`load_skills`] sets the real one.
pub fn parse_skill(text: &str, folder_name: &str, path: PathBuf) -> Option<Skill> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let (front, body) = split_front_matter(text);
    let mut name = None;
    let mut description = None;
    let mut kind = AskKind::Rewrite;
    for line in front.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = unquote(value.trim());
        match key.trim() {
            "name" if !value.is_empty() => name = Some(value),
            "description" if !value.is_empty() => description = Some(value),
            "output" if value == "answer" => kind = AskKind::Answer,
            _ => {}
        }
    }
    let instructions = body.trim().to_string();
    if instructions.is_empty() {
        return None;
    }
    let name = name.unwrap_or_else(|| folder_name.to_string());
    let description = description.unwrap_or_else(|| format!("Use the {name} skill."));
    Some(Skill {
        name,
        description,
        instructions,
        kind,
        path,
        source: SkillSource::Rubrique,
        linked: Vec::new(),
        skipped: Vec::new(),
    })
}

/// Front matter between `---` lines at the top, and the rest.
pub(super) fn split_front_matter(text: &str) -> (&str, &str) {
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else {
        return ("", text);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return (&rest[..offset], &rest[offset + line.len()..]);
        }
        offset += line.len();
    }
    ("", text)
}

pub(super) fn unquote(value: &str) -> String {
    let quoted = value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"')) || (value.starts_with('\'') && value.ends_with('\'')));
    if quoted { value[1..value.len() - 1].to_string() } else { value.to_string() }
}

/// All skills in `folders` (each skill is a sub-folder with SKILL.md), by
/// name. The same file seen again in a later folder (through a symlink) is
/// recorded in [`Skill::linked`]; another skill with a name already taken is
/// recorded in [`Skill::skipped`], so the first folder wins.
pub fn load_skills(folders: &[(SkillSource, PathBuf)]) -> Vec<Skill> {
    let mut skills: Vec<Skill> = Vec::new();
    // The resolved SKILL.md path of each entry in `skills`, to spot links.
    let mut real_paths: Vec<PathBuf> = Vec::new();
    for (source, folder) in folders {
        for mut skill in read_folder(folder) {
            skill.source = *source;
            let real_path = fs::canonicalize(&skill.path).unwrap_or_else(|_| skill.path.clone());
            if let Some(index) = real_paths.iter().position(|known| *known == real_path) {
                skills[index].linked.push((*source, skill.path));
            } else if let Some(known) = skills.iter_mut().find(|known| known.name == skill.name) {
                known.skipped.push((*source, skill.path));
            } else {
                skills.push(skill);
                real_paths.push(real_path);
            }
        }
    }
    skills.sort_by_key(|skill| skill.name.to_lowercase());
    skills
}

/// The skills in one folder, in file name order so results do not depend
/// on the order the file system lists them.
fn read_folder(folder: &Path) -> Vec<Skill> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    entries
        .iter()
        .filter_map(|entry| {
            let path = entry.path().join(SKILL_FILE);
            let text = fs::read_to_string(&path).ok()?;
            parse_skill(&text, &entry.file_name().to_string_lossy(), path)
        })
        .collect()
}

/// Name of the example skill.
pub const EXAMPLE_NAME: &str = "action-items";

/// Write an example skill into `folder`. Returns its SKILL.md path.
///
/// # Errors
///
/// Fails when the folder cannot be created or the file written. An
/// existing example is left as it is.
pub fn create_example(folder: &Path) -> std::io::Result<PathBuf> {
    let skill_folder = folder.join(EXAMPLE_NAME);
    fs::create_dir_all(&skill_folder)?;
    let path = skill_folder.join(SKILL_FILE);
    if !path.exists() {
        fs::write(&path, EXAMPLE_SKILL)?;
    }
    Ok(path)
}

const EXAMPLE_SKILL: &str = "---
name: action-items
description: Turn meeting notes into a list of action items with owners and dates.
---
Turn the text into a Markdown task list of action items.

- One item per line: `- [ ] Owner: what to do (by when)`.
- Use the names and dates from the text; write `?` when they are missing.
- Keep the language of the text.
- Put open questions last, under a `**Questions**` line.
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_front_matter() {
        let text = "---\nname: tidy\ndescription: \"Clean up: spacing and lists\"\noutput: answer\n---\n\nDo it.\n";
        let skill = parse_skill(text, "folder", PathBuf::from("/s/tidy/SKILL.md")).unwrap_or_else(|| panic!("parsed"));
        assert_eq!(skill.name, "tidy");
        assert_eq!(skill.description, "Clean up: spacing and lists");
        assert_eq!(skill.instructions, "Do it.");
        assert_eq!(skill.kind, AskKind::Answer);
    }

    #[test]
    fn matches_name_or_description() {
        let skill = parse_skill("---\ndescription: Make a TABLE\n---\nDo it.", "tidy", PathBuf::new())
            .unwrap_or_else(|| panic!("parsed"));
        assert!(skill.matches(""));
        assert!(skill.matches(" Tid "));
        assert!(skill.matches("table"));
        assert!(!skill.matches("grammar"));
    }

    #[test]
    fn falls_back_to_the_folder_name() {
        let skill = parse_skill("Just instructions.", "short", PathBuf::new()).unwrap_or_else(|| panic!("parsed"));
        assert_eq!(skill.name, "short");
        assert_eq!(skill.description, "Use the short skill.");
        assert_eq!(skill.kind, AskKind::Rewrite);
        assert_eq!(parse_skill("---\nname: empty\n---\n  \n", "x", PathBuf::new()), None);
        // An unclosed front matter block is just text.
        let open = parse_skill("---\nname: x\nbody", "f", PathBuf::new()).unwrap_or_else(|| panic!("parsed"));
        assert_eq!(open.name, "f");
    }

    #[test]
    fn gemini_skills_come_from_its_home_folder() {
        assert_eq!((SkillSource::Gemini.id(), SkillSource::Gemini.name()), ("gemini", "Gemini CLI"));
        if let Some(folder) = SkillSource::Gemini.home_folder() {
            assert!(folder.ends_with(".gemini/skills"));
        }
        let root = std::env::temp_dir().join(format!("rubrique-gemini-skills-{}", std::process::id()));
        fs::create_dir_all(root.join("tidy")).unwrap_or_default();
        fs::write(root.join("tidy").join(SKILL_FILE), "---\nname: tidy\n---\nUse lists.").unwrap_or_default();
        let skills = load_skills(&[(SkillSource::Gemini, root.clone())]);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].source, SkillSource::Gemini);
        assert_eq!(skills[0].instructions, "Use lists.");
        fs::remove_dir_all(&root).unwrap_or_default();
    }

    #[test]
    fn loads_folders_in_order() {
        let root = std::env::temp_dir().join(format!("rubrique-skills-{}", std::process::id()));
        let first = root.join("first");
        let second = root.join("second");
        let write = |folder: &Path, name: &str, body: &str| {
            fs::create_dir_all(folder.join(name)).unwrap_or_default();
            fs::write(folder.join(name).join(SKILL_FILE), body).unwrap_or_default();
        };
        write(&first, "b", "---\nname: Beta\n---\nfirst beta");
        write(&second, "b", "---\nname: Beta\n---\nsecond beta");
        write(&second, "a", "alpha");
        fs::create_dir_all(second.join("not-a-skill")).unwrap_or_default();
        let folders = [
            (SkillSource::Rubrique, first),
            (SkillSource::Agents, second.clone()),
            (SkillSource::Grok, root.join("missing")),
        ];
        let skills = load_skills(&folders);
        let names: Vec<&str> = skills.iter().map(|skill| skill.name.as_str()).collect();
        assert_eq!(names, ["a", "Beta"]);
        assert_eq!(skills[0].source, SkillSource::Agents);
        // The second Beta is a different file, so it is skipped, not linked.
        assert_eq!(skills[1].instructions, "first beta");
        assert_eq!(skills[1].source, SkillSource::Rubrique);
        assert_eq!(skills[1].skipped, [(SkillSource::Agents, second.join("b").join(SKILL_FILE))]);
        assert_eq!(skills[1].linked, []);

        let path = create_example(&second).unwrap_or_default();
        assert!(path.ends_with("action-items/SKILL.md"));
        let only_second = [(SkillSource::Agents, second)];
        assert!(load_skills(&only_second).iter().any(|skill| skill.name == EXAMPLE_NAME));
        fs::remove_dir_all(&root).unwrap_or_default();
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_skill_is_one_skill_from_two_sources() {
        let root = std::env::temp_dir().join(format!("rubrique-linked-skills-{}", std::process::id()));
        let agents = root.join("agents");
        let claude = root.join("claude");
        fs::create_dir_all(agents.join("tidy")).unwrap_or_default();
        fs::create_dir_all(&claude).unwrap_or_default();
        fs::write(agents.join("tidy").join(SKILL_FILE), "Use lists.").unwrap_or_default();
        std::os::unix::fs::symlink(agents.join("tidy"), claude.join("tidy")).unwrap_or_default();

        let skills = load_skills(&[(SkillSource::Agents, agents), (SkillSource::Claude, claude.clone())]);
        assert_eq!(skills.len(), 1);
        let sources: Vec<SkillSource> = skills[0].sources().collect();
        assert_eq!(sources, [SkillSource::Agents, SkillSource::Claude]);
        assert_eq!(skills[0].linked, [(SkillSource::Claude, claude.join("tidy").join(SKILL_FILE))]);
        assert_eq!(skills[0].skipped, []);
        fs::remove_dir_all(&root).unwrap_or_default();
    }
}
