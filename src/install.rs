//! Install a missing tool with one click: the recipes for the programs
//! Rubrique runs (whisper.cpp, Ollama, Claude Code, Codex, Grok, tmux) and the
//! files they need (the whisper model, an Ollama model).
//!
//! A recipe has a one-click command only when it is a single, well-known
//! package-manager command (`brew`, `npm`, `ollama`, or `curl` for a file)
//! and that program was found. Install scripts piped to a shell are only
//! offered to copy. Nothing uses `sudo` or a shell.
//!
//! Pure apart from [`find_installer`] and [`ollama_model_present`], which
//! look at the disk.

use std::path::{Path, PathBuf};

use crate::ai::{find_on_path, split_command};

pub mod view;

/// Where Homebrew puts `brew`: Apple silicon, Intel Macs, Linux. Apps opened
/// from the Finder often do not have these folders on `PATH`.
const BREW_FOLDERS: [&str; 3] = ["/opt/homebrew/bin", "/usr/local/bin", "/home/linuxbrew/.linuxbrew/bin"];

const WHISPER_PAGE: &str = "https://github.com/ggml-org/whisper.cpp#quick-start";
const OLLAMA_PAGE: &str = "https://ollama.com/download";
/// Ollama's Linux installer, a script piped to a shell: copied, never run.
const OLLAMA_SCRIPT: &str = "curl -fsSL https://ollama.com/install.sh | sh";
const CLAUDE_PAGE: &str = "https://docs.anthropic.com/en/docs/claude-code/setup";
const CODEX_PAGE: &str = "https://github.com/openai/codex#quickstart";
const GROK_PAGE: &str = "https://github.com/superagent-ai/grok-cli";
const TMUX_PAGE: &str = "https://github.com/tmux/tmux/wiki/Installing";
/// Linux package managers and their tmux install command, in the order
/// they are looked for. They need `sudo`, so the command is only copied.
const LINUX_TMUX_COMMANDS: [(&str, &str); 4] = [
    ("apt-get", "sudo apt install tmux"),
    ("dnf", "sudo dnf install tmux"),
    ("pacman", "sudo pacman -S tmux"),
    ("zypper", "sudo zypper install tmux"),
];
/// Where Ollama keeps models unless `OLLAMA_MODELS` says otherwise.
const OLLAMA_REGISTRY: &str = "registry.ollama.ai";

/// The system Rubrique runs on, for picking a recipe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Linux,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") { Platform::Mac } else { Platform::Linux }
    }
}

/// A program and its arguments, started without a shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl InstallCommand {
    fn new(program: &Path, args: &[&str]) -> InstallCommand {
        InstallCommand { program: program.display().to_string(), args: args.iter().map(ToString::to_string).collect() }
    }

    /// The command as shown to the user, words with spaces quoted.
    pub fn display(&self) -> String {
        std::iter::once(&self.program).chain(&self.args).map(|word| quote(word)).collect::<Vec<_>>().join(" ")
    }
}

fn quote(word: &str) -> String {
    if word.is_empty() || word.contains(char::is_whitespace) { format!("'{word}'") } else { word.to_string() }
}

/// How to get one missing thing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipe {
    /// What gets installed, as shown: "whisper.cpp".
    pub name: String,
    /// Run by the Install button.
    pub run: Option<InstallCommand>,
    /// A command to copy for a terminal, when it cannot run with one click.
    pub copy: Option<String>,
    /// A web page with install steps.
    pub page: Option<&'static str>,
    /// Said after a good install, when something is left to do.
    pub after: Option<&'static str>,
    /// The program the install should put on `PATH`, to check afterwards.
    pub provides: Option<String>,
    /// A download goes to the first file and is renamed to the second when
    /// it finishes, so a broken download never looks like the real file.
    pub download: Option<(PathBuf, PathBuf)>,
}

impl Recipe {
    fn named(name: &str) -> Recipe {
        Recipe {
            name: name.to_string(),
            run: None,
            copy: None,
            page: None,
            after: None,
            provides: None,
            download: None,
        }
    }

    /// The command to copy for a terminal: the short one when there is
    /// one, else the one Install runs.
    pub fn shown_command(&self) -> Option<String> {
        self.copy.clone().or_else(|| self.run.as_ref().map(InstallCommand::display))
    }
}

/// The recipe for a command's program, when Rubrique knows it. Only bare
/// names match: a full path in the command is the user's own setup.
/// `find` says where an installer such as `brew` is.
pub fn recipe_for_command(command: &str, platform: Platform, find: &dyn Fn(&str) -> Option<PathBuf>) -> Option<Recipe> {
    let program = split_command(command).into_iter().next()?;
    let mut recipe = match program.as_str() {
        "whisper-cli" => brew_recipe("whisper.cpp", "whisper-cpp", WHISPER_PAGE, platform, find),
        "ollama" => ollama_recipe(platform, find),
        "claude" => npm_recipe("Claude Code", "@anthropic-ai/claude-code", CLAUDE_PAGE, find),
        "codex" => npm_recipe("Codex", "@openai/codex", CODEX_PAGE, find),
        "grok" => Recipe { page: Some(GROK_PAGE), ..Recipe::named("Grok CLI") },
        _ => return None,
    };
    recipe.provides = Some(program);
    Some(recipe)
}

/// `brew install <formula>` when Homebrew is there; otherwise the command to
/// copy (macOS, where Homebrew is the usual way) and the project page.
fn brew_recipe(
    name: &str,
    formula: &str,
    page: &'static str,
    platform: Platform,
    find: &dyn Fn(&str) -> Option<PathBuf>,
) -> Recipe {
    let mut recipe = Recipe { page: Some(page), ..Recipe::named(name) };
    match find("brew") {
        Some(brew) => recipe.run = Some(InstallCommand::new(&brew, &["install", formula])),
        None if platform == Platform::Mac => recipe.copy = Some(format!("brew install {formula}")),
        None => {}
    }
    recipe
}

/// Homebrew when it is there; on Linux otherwise, Ollama's install script to
/// copy (it is piped to a shell, so it never runs from here).
fn ollama_recipe(platform: Platform, find: &dyn Fn(&str) -> Option<PathBuf>) -> Recipe {
    let mut recipe = Recipe { page: Some(OLLAMA_PAGE), ..Recipe::named("Ollama") };
    if let Some(brew) = find("brew") {
        recipe.run = Some(InstallCommand::new(&brew, &["install", "ollama"]));
        recipe.after = Some("Start it with: brew services start ollama");
    } else if platform == Platform::Linux {
        recipe.copy = Some(OLLAMA_SCRIPT.to_string());
    }
    recipe
}

/// `npm install -g <package>` when npm is there; the page either way.
fn npm_recipe(name: &str, package: &str, page: &'static str, find: &dyn Fn(&str) -> Option<PathBuf>) -> Recipe {
    let command = |npm: &Path| InstallCommand::new(npm, &["install", "-g", package]);
    let run = find("npm").map(|npm| command(&npm));
    let copy = run.is_none().then(|| command(Path::new("npm")).display());
    Recipe { run, copy, page: Some(page), ..Recipe::named(name) }
}

/// tmux, for Terminals: `brew install tmux` with one click when Homebrew
/// is there; otherwise the command to copy (the system package manager's
/// on Linux, which needs `sudo`; Homebrew's on macOS) and the tmux page.
pub fn tmux_recipe(platform: Platform, find: &dyn Fn(&str) -> Option<PathBuf>) -> Recipe {
    let mut recipe = Recipe { page: Some(TMUX_PAGE), provides: Some("tmux".to_string()), ..Recipe::named("tmux") };
    if let Some(brew) = find("brew") {
        recipe.run = Some(InstallCommand::new(&brew, &["install", "tmux"]));
    } else if platform == Platform::Mac {
        recipe.copy = Some("brew install tmux".to_string());
    } else {
        recipe.copy = LINUX_TMUX_COMMANDS
            .iter()
            .find(|(manager, _)| find(manager).is_some())
            .map(|(_, command)| (*command).to_string());
    }
    recipe
}

/// Download the whisper model `url` to `model` with curl.
pub fn model_recipe(model: &Path, url: &str, find: &dyn Fn(&str) -> Option<PathBuf>) -> Recipe {
    let partial = PathBuf::from(format!("{}.part", model.display()));
    let partial_text = partial.display().to_string();
    let run = find("curl").map(|curl| {
        InstallCommand::new(
            &curl,
            &["-L", "--fail", "--silent", "--show-error", "--create-dirs", "-o", &partial_text, url],
        )
    });
    let copy = Some(format!("curl -L --create-dirs -o {} {url}", model.display()));
    Recipe { run, copy, download: Some((partial, model.to_path_buf())), ..Recipe::named("the whisper model (466 MB)") }
}

/// The model in an `ollama run <model> …` command.
pub fn ollama_model(command: &str) -> Option<String> {
    let words = split_command(command);
    match words.as_slice() {
        [program, run, model, ..] if program == "ollama" && run == "run" && !model.starts_with('-') => {
            (!model.contains('{')).then(|| model.clone())
        }
        _ => None,
    }
}

/// `ollama pull <model>`, with Ollama found at `ollama`.
pub fn ollama_pull_recipe(ollama: &Path, model: &str) -> Recipe {
    let run = Some(InstallCommand::new(ollama, &["pull", model]));
    Recipe { run, ..Recipe::named(model) }
}

/// Where Ollama writes a model's manifest under `models`: `qwen2.5:1.5b` is
/// `manifests/registry.ollama.ai/library/qwen2.5/1.5b`.
pub fn ollama_manifest(models: &Path, model: &str) -> PathBuf {
    let (name, tag) = match model.rsplit_once(':') {
        Some((name, tag)) if !tag.contains('/') => (name, tag),
        _ => (model, "latest"),
    };
    let parts: Vec<&str> = name.split('/').collect();
    let (host, namespace, name) = match parts.as_slice() {
        [name] => (OLLAMA_REGISTRY, "library", *name),
        [namespace, name] => (OLLAMA_REGISTRY, *namespace, *name),
        [host, namespace, name, ..] => (*host, *namespace, *name),
        [] => (OLLAMA_REGISTRY, "library", model),
    };
    models.join("manifests").join(host).join(namespace).join(name).join(tag)
}

/// Folders Ollama may keep models in: `OLLAMA_MODELS`, the user's, and the
/// Linux service's.
fn ollama_model_folders() -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::env::var_os("OLLAMA_MODELS").map(PathBuf::from).into_iter().collect();
    folders.extend(std::env::var_os("HOME").map(|home| Path::new(&home).join(".ollama/models")));
    folders.push(PathBuf::from("/usr/share/ollama/.ollama/models"));
    folders
}

/// The model was downloaded (its manifest is in one of Ollama's folders).
pub fn ollama_model_present(model: &str) -> bool {
    ollama_model_folders().iter().any(|folder| ollama_manifest(folder, model).is_file())
}

/// Where an installer program is: on `PATH`, or for `brew` in Homebrew's
/// own folders.
pub fn find_installer(program: &str) -> Option<PathBuf> {
    find_on_path(program).or_else(|| {
        (program == "brew")
            .then(|| BREW_FOLDERS.iter().map(|folder| Path::new(folder).join("brew")).find(|path| path.is_file()))
            .flatten()
    })
}

/// The last line worth showing from an install's output.
pub fn last_line(errors: &str, output: &str) -> String {
    let last = |text: &str| text.lines().rev().find(|line| !line.trim().is_empty()).map(|line| line.trim().to_string());
    last(errors).or_else(|| last(output)).unwrap_or_else(|| "no details".to_string())
}

/// "Installed whisper.cpp." ("Downloaded" for a model), with what is left
/// to do. `found`: the program it provides is on `PATH` now.
pub fn installed_message(recipe: &Recipe, found: bool) -> String {
    match (&recipe.provides, recipe.after) {
        (Some(program), _) if !found => format!(
            "Installed {}, but Rubrique cannot find {program} on its PATH: put its full path in the command.",
            recipe.name
        ),
        (_, Some(after)) => format!("Installed {}. {after}", recipe.name),
        (None, None) => format!("Downloaded {}.", recipe.name),
        _ => format!("Installed {}.", recipe.name),
    }
}

/// Where an install stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    Running { seconds: u64 },
    Done(String),
    Failed(String),
}

/// The install that runs or ran last, for the prompts.
#[derive(Clone, Debug, Default)]
pub struct InstallStatus {
    /// Recipe name and where it stands.
    pub progress: Option<(String, Progress)>,
    /// An install runs, so no other may start.
    pub busy: bool,
}

impl InstallStatus {
    /// The progress of `recipe`'s install, if it is the one that ran.
    pub fn of(&self, recipe: &Recipe) -> Option<&Progress> {
        self.progress.as_ref().filter(|(name, _)| *name == recipe.name).map(|(_, progress)| progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(found: &'static [&'static str]) -> impl Fn(&str) -> Option<PathBuf> {
        move |program| found.contains(&program).then(|| PathBuf::from(format!("/bin/{program}")))
    }

    fn recipe(command: &str, platform: Platform, found: &'static [&'static str]) -> Recipe {
        recipe_for_command(command, platform, &with(found)).unwrap()
    }

    #[test]
    fn brew_installs_whisper_with_one_click() {
        let whisper = recipe(crate::voice::DEFAULT_COMMAND, Platform::Mac, &["brew"]);
        assert_eq!(whisper.name, "whisper.cpp");
        assert_eq!(whisper.shown_command().as_deref(), Some("/bin/brew install whisper-cpp"));
        assert_eq!(whisper.provides.as_deref(), Some("whisper-cli"));
        let linux = recipe(crate::voice::DEFAULT_COMMAND, Platform::Linux, &["brew"]);
        assert!(linux.run.is_some());
    }

    #[test]
    fn whisper_without_brew_links_the_project() {
        let whisper = recipe(crate::voice::DEFAULT_COMMAND, Platform::Linux, &["npm", "curl"]);
        assert_eq!(whisper.run, None);
        assert_eq!((whisper.copy, whisper.page), (None, Some(WHISPER_PAGE)));
        let mac = recipe(crate::voice::DEFAULT_COMMAND, Platform::Mac, &[]);
        assert_eq!((mac.run, mac.copy.as_deref()), (None, Some("brew install whisper-cpp")));
    }

    #[test]
    fn ollama_script_is_only_copied() {
        let linux = recipe("ollama run qwen2.5:1.5b {prompt}", Platform::Linux, &["curl"]);
        assert_eq!(linux.run, None);
        assert_eq!(linux.copy.as_deref(), Some(OLLAMA_SCRIPT));
        let mac = recipe("ollama run qwen2.5:1.5b {prompt}", Platform::Mac, &[]);
        assert_eq!((mac.run, mac.copy, mac.page), (None, None, Some(OLLAMA_PAGE)));
        let brew = recipe("ollama run x {prompt}", Platform::Mac, &["brew"]);
        assert_eq!(brew.shown_command().as_deref(), Some("/bin/brew install ollama"));
        assert!(brew.after.is_some());
    }

    #[test]
    fn npm_tools_need_npm_for_one_click() {
        let claude = recipe(crate::ai::AiTool::Claude.default_command(), Platform::Mac, &["npm"]);
        let run = claude.run.unwrap();
        assert_eq!(
            (run.program.as_str(), run.args.as_slice()),
            ("/bin/npm", &["install", "-g", "@anthropic-ai/claude-code"].map(String::from)[..])
        );
        let codex = recipe("codex exec {prompt}", Platform::Linux, &[]);
        assert_eq!(codex.run, None);
        assert_eq!(codex.copy.as_deref(), Some("npm install -g @openai/codex"));
        assert_eq!(codex.page, Some(CODEX_PAGE));
    }

    #[test]
    fn grok_and_unknown_programs() {
        let grok = recipe("grok --prompt {prompt}", Platform::Mac, &["brew", "npm"]);
        assert_eq!((grok.run, grok.copy, grok.page), (None, None, Some(GROK_PAGE)));
        assert_eq!(recipe_for_command("llama-cli -p {prompt}", Platform::Mac, &with(&["brew"])), None);
        assert_eq!(recipe_for_command("/opt/x/whisper-cli -f {audio}", Platform::Mac, &with(&["brew"])), None);
        assert_eq!(recipe_for_command("", Platform::Mac, &with(&[])), None);
    }

    #[test]
    fn tmux_installs_with_brew_or_copies_the_package_command() {
        let brew = tmux_recipe(Platform::Linux, &with(&["brew", "apt-get"]));
        assert_eq!(brew.shown_command().as_deref(), Some("/bin/brew install tmux"));
        assert_eq!(brew.provides.as_deref(), Some("tmux"));
        let ubuntu = tmux_recipe(Platform::Linux, &with(&["apt-get"]));
        assert_eq!((ubuntu.run, ubuntu.copy.as_deref()), (None, Some("sudo apt install tmux")));
        let fedora = tmux_recipe(Platform::Linux, &with(&["dnf"]));
        assert_eq!(fedora.copy.as_deref(), Some("sudo dnf install tmux"));
        let unknown = tmux_recipe(Platform::Linux, &with(&[]));
        assert_eq!((unknown.copy, unknown.page), (None, Some(TMUX_PAGE)));
        let mac = tmux_recipe(Platform::Mac, &with(&[]));
        assert_eq!(mac.copy.as_deref(), Some("brew install tmux"));
    }

    #[test]
    fn model_download_goes_through_a_partial_file() {
        let model = Path::new("/home/a/.local/share/whisper/ggml-small.bin");
        let recipe = model_recipe(model, "https://x/m.bin", &with(&["curl"]));
        let run = recipe.run.unwrap();
        assert_eq!(run.program, "/bin/curl");
        assert_eq!(
            &run.args[run.args.len() - 3..],
            ["-o", "/home/a/.local/share/whisper/ggml-small.bin.part", "https://x/m.bin"]
        );
        assert!(run.args.contains(&"--fail".to_string()));
        assert_eq!(recipe.download.unwrap().1, model);
        let no_curl = model_recipe(model, "https://x/m.bin", &with(&[]));
        assert_eq!(no_curl.run, None);
        assert!(no_curl.copy.unwrap().starts_with("curl -L --create-dirs -o /home/a/"));
    }

    #[test]
    fn reads_the_ollama_model() {
        assert_eq!(ollama_model("ollama run qwen2.5:1.5b {prompt}").as_deref(), Some("qwen2.5:1.5b"));
        assert_eq!(ollama_model("ollama run --verbose m {prompt}"), None);
        assert_eq!(ollama_model("ollama run {prompt}"), None);
        assert_eq!(ollama_model("llama-cli run m {prompt}"), None);
        let pull = ollama_pull_recipe(Path::new("/bin/ollama"), "gemma3:1b");
        assert_eq!(pull.shown_command().as_deref(), Some("/bin/ollama pull gemma3:1b"));
    }

    #[test]
    fn finds_ollama_manifests() {
        let models = Path::new("/m");
        assert_eq!(
            ollama_manifest(models, "qwen2.5:1.5b"),
            Path::new("/m/manifests/registry.ollama.ai/library/qwen2.5/1.5b")
        );
        assert_eq!(
            ollama_manifest(models, "gemma3"),
            Path::new("/m/manifests/registry.ollama.ai/library/gemma3/latest")
        );
        assert_eq!(ollama_manifest(models, "me/tiny:q4"), Path::new("/m/manifests/registry.ollama.ai/me/tiny/q4"));
        assert_eq!(ollama_manifest(models, "host:5000/me/tiny"), Path::new("/m/manifests/host:5000/me/tiny/latest"));
    }

    #[test]
    fn shows_commands_and_last_lines() {
        let command = InstallCommand::new(Path::new("curl"), &["-o", "/my files/a", ""]);
        assert_eq!(command.display(), "curl -o '/my files/a' ''");
        assert_eq!(last_line("warn\nError: no such formula\n\n", "ok"), "Error: no such formula");
        assert_eq!(last_line("  ", "done\n"), "done");
        assert_eq!(last_line("", ""), "no details");
    }

    #[test]
    fn says_what_is_left_after_installing() {
        let brew = recipe("ollama run x {prompt}", Platform::Mac, &["brew"]);
        assert_eq!(installed_message(&brew, true), "Installed Ollama. Start it with: brew services start ollama");
        assert!(installed_message(&brew, false).contains("cannot find ollama on its PATH"));
        let pull = ollama_pull_recipe(Path::new("/bin/ollama"), "m");
        assert_eq!(installed_message(&pull, false), "Downloaded m.");
    }

    #[test]
    fn finds_installers() {
        assert_eq!(find_installer("surely-not-a-real-program-xyz"), None);
        assert!(find_installer("sh").is_some());
    }
}
