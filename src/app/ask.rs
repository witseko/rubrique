//! Ask (Cmd+K): pick the text, run the AI tool, show its output, and review
//! the answer in the merge screen.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{self, Context, Ui, pos2};

use super::{OpenFile, RubriqueApp, char_to_byte};
use crate::ai::agents::{Agent, find_skills};
use crate::ai::preview::{PromptParts, full_prompt, prompt_parts};
use crate::ai::run::{AiRun, RunEnd};
use crate::ai::skills::Skill;
use crate::ai::view::{
    AskBar, AskBarInfo, AskEvent, AskRequest, Consent, ConsentChoice, OutputAction, OutputInfo, OutputState, RunView,
    show_ask_bar, show_consent, show_output,
};
use crate::ai::{self, AiSettings, AiTool, AskKind};
use crate::document::merge::MergeSession;
use crate::document::preview_selection;
use crate::document::view::editor_selection;
use crate::explorer::tree::is_markdown_path;
use crate::markdown::notes::{find_notes, locate};

/// Space between the top of the page and the Ask bar.
const ASK_TOP: f32 = 56.0;
/// Width the Ask bar takes, to center it on the page.
const ASK_OUTER_WIDTH: f32 = 600.0;
/// Longest request shown in the merge column title.
const TITLE_REQUEST_CHARS: usize = 28;

/// The text a request is about.
#[derive(Clone, Debug)]
pub(super) struct AskTarget {
    path: PathBuf,
    /// Byte range in the document when the request was made.
    range: Range<usize>,
    text: String,
    whole_file: bool,
}

/// A request ready to run.
#[derive(Debug)]
pub(super) struct PendingRun {
    target: AskTarget,
    request: String,
    kind: AskKind,
    tool: AiTool,
    skill: Option<Box<Skill>>,
    /// The agent picked with `@ai-name`: its personality, skills, tool and model.
    agent: Option<Box<Agent>>,
    /// Set when resolving notes: which ones.
    resolving: Option<Resolving>,
    /// A second tool to run at the same time, to compare.
    compare: Option<AiTool>,
}

/// Which notes a run resolves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Resolving {
    All,
    /// The note about this quote.
    One(String),
}

/// One tool's run.
#[derive(Debug)]
struct ToolRun {
    tool: AiTool,
    run: AiRun,
    /// The command as shown in the panel.
    command: String,
}

/// The runs (one, or two when comparing) and what they are for, while the
/// output panel is open.
#[derive(Debug)]
pub(super) struct AiJob {
    runs: Vec<ToolRun>,
    pending: PendingRun,
}

impl AiJob {
    fn is_running(&self) -> bool {
        self.runs.iter().any(|run| run.run.is_running())
    }

    fn elapsed_seconds(&self) -> u64 {
        self.runs.iter().map(|run| run.run.elapsed_seconds()).max().unwrap_or_default()
    }

    /// Runs that finished well with some output.
    fn answered(&self) -> Vec<&ToolRun> {
        self.runs
            .iter()
            .filter(|run| {
                matches!(run.run.end(), Some(RunEnd::Exited { success: true, .. }))
                    && !run.run.output_text().trim().is_empty()
            })
            .collect()
    }
}

/// The Ask bar, the text it is about, and a run waiting for permission.
#[derive(Debug, Default)]
pub(super) struct AskState {
    bar: Option<(AskBar, AskTarget)>,
    /// "What Claude gets" is open in the Ask bar (kept for the session).
    prompt_open: bool,
    consent: Option<(Consent, PendingRun)>,
    pub(super) job: Option<AiJob>,
    /// Where the page (editor or preview) was drawn, to place the bar.
    pub(super) page_rect: Option<egui::Rect>,
}

impl AskState {
    pub(super) fn is_open(&self) -> bool {
        self.bar.is_some()
    }

    /// Close the Ask bar (its text belongs to a tab that is being left).
    pub(super) fn close_bar(&mut self) {
        self.bar = None;
    }
}

impl RubriqueApp {
    /// Open the Ask bar for the selection, or the whole file when nothing is selected.
    pub(super) fn open_ask(&mut self, ctx: &Context) {
        self.finish_block_edit();
        if self.merging.is_some() || self.ai_blocked() {
            return;
        }
        if self.ask.job.as_ref().is_some_and(AiJob::is_running) {
            self.show_status("Wait for the AI tool to finish, or press Stop".to_string());
            return;
        }
        let OpenFile::Text { document, .. } = &self.open_file else {
            self.show_status("Open a file first, then ask".to_string());
            return;
        };
        let text = &document.text;
        let range = if self.showing_preview() {
            match preview_selection::current(ctx) {
                Some(selection) => {
                    let Some(range) = locate(text, &selection.text) else {
                        self.show_status("Cannot find the selection in the file; try in Edit mode".to_string());
                        return;
                    };
                    Some(range)
                }
                None => None,
            }
        } else {
            editor_selection(ctx).map(|(first, last)| char_to_byte(text, first)..char_to_byte(text, last))
        };
        let (range, whole_file) = match range {
            Some(range) if !range.is_empty() => (range, false),
            _ => (0..text.len(), true),
        };
        let scope = if whole_file {
            "Whole file".to_string()
        } else {
            match text[range.clone()].lines().count() {
                0 | 1 => "1 line selected".to_string(),
                lines => format!("{lines} lines selected"),
            }
        };
        let target =
            AskTarget { path: document.path.clone(), text: text[range.clone()].to_string(), range, whole_file };
        self.ask.bar = Some((AskBar::new(self.settings.ai.default_tool, scope), target));
    }

    /// Draw the Ask bar and the consent dialog when they are open.
    pub(super) fn ask_dialogs(&mut self, ctx: &Context) {
        if let Some((bar, target)) = &mut self.ask.bar {
            let page = self.ask.page_rect.unwrap_or_else(|| ctx.content_rect());
            let left = page.left() + ((page.width() - ASK_OUTER_WIDTH) / 2.0).max(8.0);
            let folder = run_folder(&target.path);
            let shown = bar.shown();
            let agent = bar.agent.as_deref();
            let parts = target_parts(&self.settings.ai, &self.skills, target, &shown, (bar.tool, bar.compare), agent);
            let info = AskBarInfo {
                installed: self.ai_found.each_ref().map(Option::is_some),
                skills: &self.skills,
                agents: &self.agents,
                parts: &parts,
                folder: &folder,
                allowed: self.settings.ai.is_allowed(&folder),
            };
            let event = show_ask_bar(ctx, pos2(left, page.top() + ASK_TOP), bar, &info, &mut self.ask.prompt_open);
            match event {
                Some(AskEvent::Run { ask, confirmed }) => self.ask_run(ctx, ask, confirmed),
                Some(AskEvent::Close) => self.ask.bar = None,
                None => {}
            }
        }
        if let Some((consent, _)) = &mut self.ask.consent
            && let Some(choice) = show_consent(ctx, consent)
            && let Some((consent, pending)) = self.ask.consent.take()
            && choice == ConsentChoice::Run
        {
            if consent.remember {
                self.settings.ai.allow(&consent.folder);
                self.write_settings();
            }
            self.start_run(ctx, pending);
        }
    }

    /// A run from the Ask bar. In a folder not allowed yet it waits in the
    /// bar, with what the tool gets shown, until the user presses Run.
    fn ask_run(&mut self, ctx: &Context, ask: AskRequest, confirmed: bool) {
        let Some((bar, target)) = &mut self.ask.bar else {
            return;
        };
        let folder = run_folder(&target.path);
        if !confirmed && !self.settings.ai.is_allowed(&folder) {
            bar.wait(ask);
            return;
        }
        let remember = bar.remember;
        let pending = PendingRun {
            target: target.clone(),
            request: ask.request,
            kind: ask.kind,
            tool: bar.tool,
            skill: ask.skill,
            agent: bar.agent.clone(),
            resolving: None,
            compare: bar.compare.filter(|compare| *compare != bar.tool),
        };
        self.ask.bar = None;
        if self.offer_missing_ai_tool(pending.tool) {
            return;
        }
        if remember && !self.settings.ai.is_allowed(&folder) {
            self.settings.ai.allow(&folder);
            self.write_settings();
        }
        self.start_run(ctx, pending);
    }

    /// Ask for permission the first time in a folder, then run. For runs
    /// that do not come from the Ask bar.
    fn begin_run(&mut self, ctx: &Context, pending: PendingRun) {
        if self.offer_missing_ai_tool(pending.tool) {
            return;
        }
        let folder = run_folder(&pending.target.path);
        if self.settings.ai.is_allowed(&folder) {
            self.start_run(ctx, pending);
        } else {
            let parts = pending.parts(&self.settings.ai, &self.skills);
            self.ask.consent = Some((Consent { parts, folder, remember: true }, pending));
        }
    }

    fn start_run(&mut self, ctx: &Context, pending: PendingRun) {
        if self.ai_blocked() {
            return;
        }
        let target = &pending.target;
        let agent = pending.agent.as_deref();
        let (skills, missing) = run_skills(&self.skills, agent, pending.skill.as_deref());
        let prompt = full_prompt(&pending.request, &target.text, pending.kind, target.whole_file, agent, &skills);
        if let Some(agent) = agent {
            for name in missing {
                self.show_status(format!("{} uses the skill \"{name}\", which was not found", agent.name));
            }
        }
        if prompt.is_too_long() {
            self.show_status("The text is too long to send; select a smaller part".to_string());
            return;
        }
        let mut runs = Vec::new();
        for tool in std::iter::once(pending.tool).chain(pending.compare) {
            let command = run_command(&self.settings.ai, tool, pending.agent.as_deref());
            if let Some(problem) = ai::check_command(&command) {
                self.show_status(format!("{} command: {} (Settings → AI)", tool.name(), problem.message()));
                continue;
            }
            if self.offer_missing_ai_tool(tool) {
                continue;
            }
            let Some((program, args)) = ai::command_args(&command, &prompt) else {
                continue;
            };
            let repaint = ctx.clone();
            match AiRun::start(&program, &args, &run_folder(&target.path), move || repaint.request_repaint()) {
                Ok(run) => runs.push(ToolRun { tool, run, command: ai::display_command(&command, &pending.request) }),
                Err(error) => self.show_status(format!("Cannot start {program}: {error}. Check Settings → AI.")),
            }
        }
        if !runs.is_empty() {
            self.ask.job = Some(AiJob { runs, pending });
        }
    }

    /// The output panel of the current run(s).
    pub(super) fn ai_output_panel(&mut self, ui: &mut Ui) {
        let Some(job) = &mut self.ask.job else {
            return;
        };
        let mut views = Vec::new();
        let formatted = shows_as_markdown(job.pending.kind, &job.pending.target.path);
        for tool_run in &mut job.runs {
            let seconds = tool_run.run.elapsed_seconds();
            let state = match tool_run.run.poll() {
                None => OutputState::Running { seconds },
                Some(RunEnd::Stopped) => OutputState::Stopped { seconds },
                Some(RunEnd::Exited { success: true, .. }) => OutputState::Done { seconds },
                Some(RunEnd::Exited { code, .. }) => OutputState::Failed { seconds, code: *code },
            };
            views.push(RunView {
                tool: tool_run.tool,
                command: tool_run.command.clone(),
                output: tool_run.run.output_text(),
                errors: tool_run.run.error_text(),
                state,
                formatted,
            });
        }
        if job.is_running() {
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
        let reviewable =
            if job.is_running() || job.pending.kind != AskKind::Rewrite { 0 } else { job.answered().len() };
        let copy_text = job.answered().first().map(|run| run.run.output_text().trim().to_string());
        let request = job.pending.label();
        let info = OutputInfo { request: &request, runs: views, reviewable };
        match show_output(ui, &info) {
            Some(OutputAction::Stop) => job.runs.iter_mut().for_each(|run| run.run.stop()),
            Some(OutputAction::Copy) => {
                if let Some(text) = copy_text {
                    self.copy_to_clipboard(ui.ctx(), text);
                }
            }
            Some(OutputAction::CopyCode(code)) => self.copy_to_clipboard(ui.ctx(), code),
            Some(OutputAction::Close) => self.ask.job = None,
            Some(OutputAction::Review) => self.review_answer(),
            None => {}
        }
    }

    /// Open the merge screen with the answer in place of the target text.
    fn review_answer(&mut self) {
        let Some(job) = &self.ask.job else {
            return;
        };
        let target = &job.pending.target;
        let OpenFile::Text { document, .. } = &self.open_file else {
            self.show_status("Open the file again to review the answer".to_string());
            return;
        };
        if document.path != target.path {
            let name = target.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            self.show_status(format!("Open {name} to review the answer"));
            return;
        }
        if document.text.get(target.range.clone()) != Some(target.text.as_str()) {
            self.show_status("The text changed while the tool ran; ask again".to_string());
            return;
        }
        let suggestion = |run: &ToolRun| {
            let answer = ai::clean_reply(&run.run.output_text(), &target.text);
            let mut text = document.text.clone();
            text.replace_range(target.range.clone(), &answer);
            text
        };
        let title = |run: &ToolRun| format!("{} · {}", run.tool.name(), ai::format_elapsed(run.run.elapsed_seconds()));
        let session = match job.answered().as_slice() {
            [left, right] => {
                let (left_text, right_text) = (suggestion(left), suggestion(right));
                if left_text == document.text && right_text == document.text {
                    None
                } else {
                    Some(MergeSession::compare(&document.text, &left_text, &right_text, title(left), title(right)))
                }
            }
            [only] => {
                let text = suggestion(only);
                (text != document.text).then(|| {
                    let title = format!("{} · {}", only.tool.name(), job.pending.label());
                    MergeSession::review(&document.text, &text, title)
                })
            }
            _ => return,
        };
        match session {
            Some(session) => self.merging = Some(session),
            None => self.show_status("The tool suggested no changes".to_string()),
        }
    }
}

impl PendingRun {
    /// What the run will send, for showing before it starts.
    fn parts(&self, settings: &AiSettings, skills: &[Skill]) -> PromptParts {
        let ask = AskRequest { request: self.request.clone(), kind: self.kind, skill: self.skill.clone() };
        target_parts(settings, skills, &self.target, &ask, (self.tool, self.compare), self.agent.as_deref())
    }

    /// What the run is for, as shown in the output panel and merge screen.
    fn label(&self) -> String {
        match (&self.resolving, &self.skill) {
            (Some(Resolving::All), _) => "Resolve all notes".to_string(),
            (Some(Resolving::One(quote)), _) => format!("Resolve note: {}", short(quote)),
            (None, Some(skill)) => format!("{} skill", skill.name),
            (None, None) => format!("\u{201c}{}\u{201d}", short(&self.request)),
        }
    }
}

impl RubriqueApp {
    /// Ask the default tool to resolve one note (or all), on the whole file.
    pub(super) fn resolve_notes(&mut self, ctx: &Context, which: Resolving) {
        if self.ai_blocked() {
            return;
        }
        if self.ask.job.as_ref().is_some_and(AiJob::is_running) {
            self.show_status("Wait for the AI tool to finish, or press Stop".to_string());
            return;
        }
        let OpenFile::Text { document, .. } = &self.open_file else {
            return;
        };
        let request = match &which {
            Resolving::All => ai::RESOLVE_ALL_REQUEST.to_string(),
            Resolving::One(quote) => {
                let Some(note) = find_notes(&document.text).into_iter().find(|note| &note.quote == quote) else {
                    return;
                };
                ai::resolve_note_request(&note.quote, &note.text)
            }
        };
        let text = document.text.clone();
        let target = AskTarget { path: document.path.clone(), range: 0..text.len(), text, whole_file: true };
        let tool = self.settings.ai.default_tool;
        let pending = PendingRun {
            target,
            request,
            kind: AskKind::Rewrite,
            tool,
            skill: None,
            agent: None,
            resolving: Some(which),
            compare: None,
        };
        self.check_ai_tools();
        self.begin_run(ctx, pending);
    }

    /// The tool and seconds so far when a run is resolving `which` (a
    /// "resolve all" run counts for every note).
    pub(super) fn resolving_state(&self, which: &Resolving) -> Option<(AiTool, u64)> {
        let job = self.ask.job.as_ref().filter(|job| job.is_running())?;
        let resolving = job.pending.resolving.as_ref()?;
        (resolving == which || *resolving == Resolving::All).then(|| (job.pending.tool, job.elapsed_seconds()))
    }

    /// An AI run is going on.
    pub(super) fn ai_busy(&self) -> bool {
        self.ask.job.as_ref().is_some_and(AiJob::is_running)
    }

    /// Stop the running tool.
    pub(super) fn stop_ai(&mut self) {
        if let Some(job) = &mut self.ask.job {
            job.runs.iter_mut().for_each(|run| run.run.stop());
        }
    }
}

/// What the tool (and the one to compare with) get for `ask` about the
/// target's text, with the agent's personality, skills and model.
fn target_parts(
    settings: &AiSettings,
    all_skills: &[Skill],
    target: &AskTarget,
    ask: &AskRequest,
    (tool, compare): (AiTool, Option<AiTool>),
    agent: Option<&Agent>,
) -> PromptParts {
    let commands: Vec<(AiTool, String)> = std::iter::once(tool)
        .chain(compare.filter(|compare| *compare != tool))
        .map(|tool| (tool, run_command(settings, tool, agent)))
        .collect();
    let commands: Vec<(AiTool, &str)> = commands.iter().map(|(tool, command)| (*tool, command.as_str())).collect();
    let (skills, _missing) = run_skills(all_skills, agent, ask.skill.as_deref());
    prompt_parts(&ask.request, &target.text, ask.kind, target.whole_file, agent, &skills, &commands)
}

/// The command for `tool`: the agent's own, or the one from Settings with
/// the agent's model added.
fn run_command(settings: &AiSettings, tool: AiTool, agent: Option<&Agent>) -> String {
    let base = settings.command(tool);
    agent.map_or_else(|| base.to_string(), |agent| agent.command_for(tool, base))
}

/// The skills a run adds: the agent's, then the one picked in the bar. Also
/// returns the names of the agent's skills that were not found.
fn run_skills<'a>(all: &'a [Skill], agent: Option<&Agent>, picked: Option<&'a Skill>) -> (Vec<&'a Skill>, Vec<String>) {
    let (mut skills, missing) = agent.map(|agent| find_skills(agent, all)).unwrap_or_default();
    skills.extend(picked);
    (skills, missing)
}

/// True when a run's output is drawn as Markdown in the output panel: an
/// answer to read always is, and new text for a Markdown file is too, so
/// `**bold**`, headings, lists and code fences read as they will in the
/// preview. New text for any other file (code, plain text) is shown as it
/// is, since Markdown would change its look (a `# comment` as a heading).
fn shows_as_markdown(kind: AskKind, path: &Path) -> bool {
    match kind {
        AskKind::Answer => true,
        AskKind::Rewrite => is_markdown_path(path),
    }
}

/// Tools run in the file's folder, so they can read nearby notes.
fn run_folder(path: &Path) -> PathBuf {
    path.parent().map_or_else(|| PathBuf::from("/"), Path::to_path_buf)
}

fn short(request: &str) -> String {
    let flat = request.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > TITLE_REQUEST_CHARS {
        format!("{}…", flat.chars().take(TITLE_REQUEST_CHARS).collect::<String>())
    } else {
        flat
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_and_markdown_rewrites_are_drawn_as_markdown() {
        assert!(shows_as_markdown(AskKind::Answer, Path::new("/notes/main.rs")));
        assert!(shows_as_markdown(AskKind::Rewrite, Path::new("/notes/todo.md")));
        assert!(!shows_as_markdown(AskKind::Rewrite, Path::new("/notes/main.rs")));
        assert!(!shows_as_markdown(AskKind::Rewrite, Path::new("/notes/list.txt")));
    }
}
