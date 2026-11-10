//! The Ask bar (Cmd+K) with what the tool gets, the output panel of a
//! running tool, and the dialog that asks before the first run in a folder
//! when a run does not start from the Ask bar.

use std::path::{Path, PathBuf};

use eframe::egui::text_edit::TextEditState;
use eframe::egui::{self, Align, Context, Key, Layout, Pos2, Response, RichText, Sense, Ui};

use super::agents::{self, Agent, MENTION_PREFIX};
use super::mention_list::{self, TypedMention};
use super::preview::PromptParts;
use super::skills::Skill;
use super::{AiTool, AskKind, QuickAction, format_elapsed};
use crate::icons::{Icon, icon_label, icon_text_button};
use crate::markdown::render::{CodeAction, render_answer, take_code_action};
use crate::settings::view::home_relative;

/// Width of the Ask bar.
const ASK_WIDTH: f32 = 580.0;
const ASK_RADIUS: u8 = 8;
/// Width of the tool picker in the Ask bar.
const TOOL_PICKER_WIDTH: f32 = 96.0;
/// Width of the "Compare with" picker.
const COMPARE_PICKER_WIDTH: f32 = 80.0;
const CONSENT_WIDTH: f32 = 560.0;
/// Longest request shown in the output panel's header.
const HEADER_REQUEST_CHARS: usize = 60;
/// Space between the command line, the output and any errors.
const OUTPUT_ITEM_GAP: f32 = 8.0;
/// Width of the labels in "What Claude gets".
const ROW_LABEL_WIDTH: f32 = 84.0;
/// Size of the small text in the Ask bar.
const SMALL_TEXT: f32 = 11.5;
/// Height of the Ask bar without "What Claude gets" rows, with some room below.
const ASK_CHROME_HEIGHT: f32 = 220.0;
/// Least height for the rows when the window is short.
const ROWS_MIN_HEIGHT: f32 = 120.0;
/// Size of commands and paths in "What Claude gets".
const CODE_TEXT: f32 = 12.0;
/// Size of the open/closed arrows.
const ARROW_SIZE: f32 = 9.0;

/// The open Ask bar.
#[derive(Debug)]
pub struct AskBar {
    pub request: String,
    pub tool: AiTool,
    /// A second tool to run at the same time, to compare the answers.
    pub compare: Option<AiTool>,
    /// What the request is about, for the footer ("2 lines selected").
    pub scope: String,
    /// A run asked for in a folder not allowed yet, waiting for Run.
    pub waiting: Option<AskRequest>,
    /// "Don't ask again for this folder".
    pub remember: bool,
    /// The agent picked with `@ai-name`, shown as a chip before the field.
    pub agent: Option<Box<Agent>>,
    /// Why the request cannot run yet (an unknown agent, say).
    error: Option<String>,
    /// Give the field focus on the next frame.
    focus: bool,
}

impl AskBar {
    pub fn new(tool: AiTool, scope: String) -> Self {
        AskBar {
            request: String::new(),
            tool,
            compare: None,
            scope,
            waiting: None,
            remember: true,
            agent: None,
            error: None,
            focus: true,
        }
    }

    /// Use `agent` for this request, and its tool when it has one that is installed.
    fn pick_agent(&mut self, agent: &Agent, installed: [bool; 4]) {
        if let Some(tool) = agent.tool
            && AiTool::ALL.into_iter().zip(installed).any(|(each, found)| each == tool && found)
        {
            self.tool = tool;
        }
        self.agent = Some(Box::new(agent.clone()));
        self.error = None;
        self.focus = true;
    }

    /// The `@word` being typed at the start of the field, while no agent is
    /// picked and no space follows it yet.
    fn typing_mention(&self) -> Option<&str> {
        if self.agent.is_some() {
            return None;
        }
        let (word, rest) = agents::leading_mention(&self.request)?;
        (rest.is_empty() && !self.request.ends_with(char::is_whitespace)).then_some(word)
    }

    /// The `@` word to suggest agents for: one at the start of the field
    /// (any `@`), or an `@ai-…` word at `cursor` (a char index) anywhere else.
    fn suggesting_for(&self, cursor: Option<usize>) -> Option<TypedMention> {
        if self.agent.is_some() {
            return None;
        }
        if let Some(word) = self.typing_mention() {
            let start = self.request.len() - self.request.trim_start().len();
            return Some(TypedMention { range: start..self.request.len(), typed: word.to_string() });
        }
        mention_list::typed_mention(&self.request, cursor?, &[agents::MENTION_PREFIX])
    }

    /// Use the suggested `agent` and take its `@` word out of the request.
    fn pick_suggested(&mut self, agent: &Agent, mention: &TypedMention, installed: [bool; 4]) {
        self.pick_agent(agent, installed);
        self.request = mention_list::remove_mention(&self.request, mention.range.clone());
    }

    /// Turn a finished `@ai-name ` at the start of the field into the
    /// agent chip. An unknown `@ai-` name leaves the text and sets an error.
    fn take_mention(&mut self, agents: &[Agent], installed: [bool; 4]) {
        self.error = None;
        if self.agent.is_some() || self.typing_mention().is_some() {
            return;
        }
        let Some((word, rest)) = agents::leading_mention(&self.request) else {
            return;
        };
        if !word.to_lowercase().starts_with(MENTION_PREFIX) {
            return;
        }
        match agents::find_by_mention(agents, word) {
            Some(agent) => {
                let rest = rest.to_string();
                self.pick_agent(agent, installed);
                self.request = rest;
            }
            None => self.error = Some(format!("No agent called @{word}. See Settings → Agents.")),
        }
    }

    /// Hold `ask` until the user presses Run (or Enter) in a folder not
    /// allowed yet.
    pub fn wait(&mut self, ask: AskRequest) {
        self.waiting = Some(ask);
        self.focus = true;
    }

    /// What Run would send now: the waiting run, else the typed request.
    pub fn shown(&self) -> AskRequest {
        self.waiting.clone().unwrap_or_else(|| AskRequest {
            request: self.request.trim().to_string(),
            kind: AskKind::Rewrite,
            skill: None,
        })
    }
}

/// A request from the Ask bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskRequest {
    pub request: String,
    pub kind: AskKind,
    pub skill: Option<Box<Skill>>,
}

/// What the user did in the Ask bar.
#[derive(Debug, PartialEq, Eq)]
pub enum AskEvent {
    /// Run `ask`. `confirmed` is set by Run (or Enter), which also answers
    /// the question for a folder not allowed yet.
    Run {
        ask: AskRequest,
        confirmed: bool,
    },
    Close,
}

/// What the Ask bar shows besides its own fields.
#[derive(Debug)]
pub struct AskBarInfo<'a> {
    /// Which tools are installed, in [`AiTool::ALL`] order.
    pub installed: [bool; 4],
    pub skills: &'a [Skill],
    pub agents: &'a [Agent],
    /// What Run would send now (see [`AskBar::shown`]).
    pub parts: &'a PromptParts,
    /// Where the tool runs.
    pub folder: &'a Path,
    /// AI runs in `folder` were allowed before.
    pub allowed: bool,
}

/// Draw the Ask bar with its top-left corner at `at`. `open` is whether
/// "What Claude gets" is expanded.
pub fn show_ask_bar(ctx: &Context, at: Pos2, bar: &mut AskBar, info: &AskBarInfo, open: &mut bool) -> Option<AskEvent> {
    let mut event = None;
    bar.take_mention(info.agents, info.installed);
    // The cursor from the last frame: the field is drawn below.
    let cursor = TextEditState::load(ctx, ask_field_id())
        .and_then(|state| state.cursor.char_range())
        .map(|range| range.primary.index.0);
    let typing = bar.suggesting_for(cursor);
    let suggestions: Vec<&Agent> =
        typing.as_ref().map(|mention| agents::matching(info.agents, &mention.typed)).unwrap_or_default();
    egui::Area::new(egui::Id::unique("ask-bar")).order(egui::Order::Foreground).fixed_pos(at).show(ctx, |ui| {
        let visuals = ui.visuals().clone();
        egui::Frame::popup(ui.style()).fill(visuals.extreme_bg_color).corner_radius(ASK_RADIUS).inner_margin(10).show(
            ui,
            |ui| {
                ui.set_width(ASK_WIDTH);
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.horizontal(|ui| {
                    icon_label(ui, Icon::Sparkle);
                    if let Some(agent) = &bar.agent {
                        let chip = ui
                            .small_button(format!("{} ×", agent.mention()))
                            .on_hover_text(format!("{}: {}\nClick to remove", agent.name, agent.description));
                        if chip.clicked() {
                            bar.agent = None;
                            bar.focus = true;
                        }
                    }
                    let field_width = ui.available_width() - TOOL_PICKER_WIDTH - ui.spacing().item_spacing.x;
                    let hint = bar.agent.as_ref().map_or_else(
                        || "What should change? Type @ for agents".to_string(),
                        |agent| format!("What should {} do?", agent.name),
                    );
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut bar.request)
                            .id(ask_field_id())
                            .hint_text(hint)
                            .desired_width(field_width),
                    );
                    if bar.focus {
                        field.request_focus();
                        bar.focus = false;
                    }
                    if field.changed() {
                        bar.waiting = None;
                    }
                    let entered = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
                    if entered
                        && let Some(first) = suggestions.first()
                        && let Some(mention) = &typing
                    {
                        // Enter while typing @… picks the first agent listed.
                        bar.pick_suggested(first, mention, info.installed);
                    } else if entered && (bar.error.is_some() || typing.is_some()) {
                        bar.focus = true;
                    } else if entered && let Some(ask) = runnable(bar) {
                        event = Some(AskEvent::Run { ask, confirmed: true });
                    } else if entered {
                        bar.focus = true;
                    }
                    tool_picker(ui, &mut bar.tool, info.installed);
                });
                if let Some(mention) = &typing
                    && let Some(picked) = agent_suggestions(ui, &mention.typed, &suggestions)
                {
                    bar.pick_suggested(picked, mention, info.installed);
                }
                if let Some(error) = &bar.error {
                    ui.label(RichText::new(error).size(SMALL_TEXT).color(ui.visuals().error_fg_color));
                }
                if let Some(action_event) = action_row(ui, bar, info) {
                    event = Some(action_event);
                }
                ui.separator();
                let waiting = bar.waiting.is_some();
                if *open || waiting {
                    prompt_header(ui, info.parts, (!waiting).then_some(open));
                    // Keep the bar on screen in a short window; the rows scroll.
                    let room = (ctx.content_rect().bottom() - at.y - ASK_CHROME_HEIGHT).max(ROWS_MIN_HEIGHT);
                    egui::ScrollArea::vertical().id_salt("ask-prompt-rows").max_height(room).show(ui, |ui| {
                        prompt_rows(ui, info.parts, info.folder, "ask-bar");
                    });
                    ui.separator();
                } else {
                    closed_header(ui, info.parts, &bar.scope, open);
                }
                if let Some(footer_event) = ask_footer(ui, bar, info) {
                    event = Some(footer_event);
                }
            },
        );
    });
    if ctx.input(|input| input.key_pressed(Key::Escape)) {
        event = Some(AskEvent::Close);
    }
    event
}

/// The Ask bar's request field, so its cursor can be read before it is drawn.
fn ask_field_id() -> egui::Id {
    egui::Id::unique("ask-bar-request")
}

/// What Run or Enter would run, if there is a request.
fn runnable(bar: &AskBar) -> Option<AskRequest> {
    let ask = bar.shown();
    (!ask.request.is_empty()).then_some(ask)
}

/// The quick actions and the Skills… and Agents… menus.
fn action_row(ui: &mut Ui, bar: &mut AskBar, info: &AskBarInfo) -> Option<AskEvent> {
    let mut event = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for action in QuickAction::ALL {
            if ui.small_button(action.label()).clicked() {
                if let Some(request) = action.request() {
                    let ask = AskRequest { request: request.to_string(), kind: action.kind(), skill: None };
                    event = Some(AskEvent::Run { ask, confirmed: false });
                } else {
                    "Translate to ".clone_into(&mut bar.request);
                    bar.waiting = None;
                    bar.focus = true;
                }
            }
        }
        if let Some(skill_event) = skills_menu(ui, &bar.request, info.skills) {
            event = Some(skill_event);
        }
        if let Some(picked) = agents_menu(ui, info.agents) {
            bar.pick_agent(picked, info.installed);
        }
    });
    event
}

/// The agents matching the `@word` being typed, one per row; a click picks one.
fn agent_suggestions<'a>(ui: &mut Ui, typing: &str, matches: &[&'a Agent]) -> Option<&'a Agent> {
    let mut picked = None;
    if matches.is_empty() {
        let text = format!("No agent matches @{typing}. Add agents in Settings → Agents.");
        ui.label(RichText::new(text).size(SMALL_TEXT).weak());
        return None;
    }
    for (index, agent) in matches.iter().enumerate() {
        let row = ui.horizontal(|ui| {
            let mention = RichText::new(agent.mention()).monospace();
            let clicked = ui.selectable_label(index == 0, mention).clicked();
            ui.label(&agent.name);
            ui.add(egui::Label::new(RichText::new(&agent.description).size(SMALL_TEXT).weak()).truncate());
            clicked
        });
        if row.inner {
            picked = Some(*agent);
        }
    }
    ui.label(RichText::new("Enter picks the first one").size(SMALL_TEXT).weak());
    picked
}

/// The Agents… menu: pick an agent for the request.
fn agents_menu<'a>(ui: &mut Ui, agents: &'a [Agent]) -> Option<&'a Agent> {
    let mut picked = None;
    ui.menu_button(RichText::new("Agents…").weak(), |ui| {
        if agents.is_empty() {
            ui.label(RichText::new("No agents yet. Add them in Settings → Agents.").weak());
        }
        for agent in agents {
            let label = format!("{}  {}", agent.mention(), agent.name);
            if ui.button(label).on_hover_text(&agent.description).clicked() {
                picked = Some(agent);
                ui.close();
            }
        }
    });
    picked
}

/// "What Claude gets ▸" with the scope at the right; a click opens it.
fn closed_header(ui: &mut Ui, parts: &PromptParts, scope: &str, open: &mut bool) {
    ui.horizontal(|ui| {
        let title = RichText::new(gets_title(parts)).size(SMALL_TEXT).weak();
        if fold_label(ui, title, false).on_hover_text("See what will be sent").clicked() {
            *open = true;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(scope).size(SMALL_TEXT).weak());
        });
    });
}

/// "What Claude gets" and, when it can be closed, Hide.
fn prompt_header(ui: &mut Ui, parts: &PromptParts, open: Option<&mut bool>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(gets_title(parts)).strong());
        if let Some(open) = open {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.link("Hide").clicked() {
                    *open = false;
                }
            });
        }
    });
}

/// "What Claude gets", or "What Claude and Codex get".
fn gets_title(parts: &PromptParts) -> String {
    let verb = if parts.commands.len() > 1 { "get" } else { "gets" };
    format!("What {} {verb}", parts.tools())
}

/// The Ask bar's last line: the folder question when waiting, Compare
/// with, Cancel and Run.
fn ask_footer(ui: &mut Ui, bar: &mut AskBar, info: &AskBarInfo) -> Option<AskEvent> {
    let mut event = None;
    ui.horizontal(|ui| {
        if bar.waiting.is_some() || !info.allowed {
            ui.checkbox(&mut bar.remember, RichText::new("Don't ask again for this folder").size(SMALL_TEXT));
        }
        ui.label(RichText::new("Compare with").size(SMALL_TEXT).weak());
        compare_picker(ui, bar, info.installed);
        ui.label(RichText::new("· Esc to close").size(SMALL_TEXT).weak());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let ask = runnable(bar);
            let visuals = ui.visuals().clone();
            let run = egui::Button::new(RichText::new("Run").color(visuals.strong_text_color()))
                .fill(visuals.selection.bg_fill);
            let tip = if ask.is_some() { "Run the request (Enter)" } else { "Type a request first" };
            if ui.add_enabled(ask.is_some(), run).on_hover_text(tip).on_disabled_hover_text(tip).clicked()
                && let Some(ask) = ask
            {
                event = Some(AskEvent::Run { ask, confirmed: true });
            }
            if ui.button("Cancel").clicked() {
                event = Some(AskEvent::Close);
            }
        });
    });
    event
}

/// The Skills… menu: run a skill on the text, with the typed request (if
/// any) as extra detail.
fn skills_menu(ui: &mut Ui, typed: &str, skills: &[Skill]) -> Option<AskEvent> {
    let mut event = None;
    ui.menu_button(RichText::new("Skills…").weak(), |ui| {
        if skills.is_empty() {
            ui.label(RichText::new("No skills yet. Add them in Settings → Skills.").weak());
        }
        for skill in skills {
            if ui.button(&skill.name).on_hover_text(&skill.description).clicked() {
                let typed = typed.trim();
                let request = if typed.is_empty() { skill.description.clone() } else { typed.to_string() };
                let ask = AskRequest { request, kind: skill.kind, skill: Some(Box::new(skill.clone())) };
                event = Some(AskEvent::Run { ask, confirmed: false });
                ui.close();
            }
        }
    });
    event
}

/// The rows of "What Claude gets": request, text, instructions, skill,
/// command(s) and folder. `salt` keeps each place's open rows apart.
pub fn prompt_rows(ui: &mut Ui, parts: &PromptParts, folder: &Path, salt: &str) {
    ui.spacing_mut().item_spacing.y = 6.0;
    row(ui, "Request", |ui| {
        if parts.request.is_empty() {
            ui.label(RichText::new("Type a request, or pick an action above.").weak());
        } else {
            ui.add(egui::Label::new(&parts.request).wrap());
        }
    });
    row(ui, "Text", |ui| text_row(ui, parts));
    fold_row(ui, egui::Id::unique((salt, "instructions")), "Instructions", false, |ui, open| {
        if open {
            boxed(ui, |ui| render_answer(ui, &parts.system));
        } else {
            ui.add(egui::Label::new(RichText::new(&parts.system).weak()).truncate());
        }
        for shown in parts.commands.iter().filter(|shown| shown.instructions_in_prompt) {
            let note = format!("{} gets these at the start of the request.", shown.tool.name());
            ui.label(RichText::new(note).size(SMALL_TEXT).weak());
        }
    });
    row(ui, "Agent", |ui| match &parts.agent {
        Some(agent) => ui.label(agent),
        None => ui.label(RichText::new("None").weak()),
    });
    let label = if parts.skills.len() > 1 { "Skills" } else { "Skill" };
    row(ui, label, |ui| {
        if parts.skills.is_empty() { ui.label(RichText::new("None").weak()) } else { ui.label(parts.skills.join(", ")) }
    });
    let label = if parts.commands.len() > 1 { "Commands" } else { "Command" };
    fold_row(ui, egui::Id::unique((salt, "command")), label, true, |ui, open| {
        for shown in &parts.commands {
            if parts.commands.len() > 1 {
                ui.label(RichText::new(shown.tool.name()).size(SMALL_TEXT).weak());
            }
            command_box(ui, &shown.command, open);
        }
    });
    row(ui, "Runs in", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(home_relative(folder)).monospace().size(CODE_TEXT));
            if !parts.commands.is_empty() && parts.commands.iter().all(|shown| shown.read_only) {
                ui.label(RichText::new("· read-only").weak())
                    .on_hover_text("The command keeps the tool from changing files");
            }
        });
    });
}

/// "3 lines selected, 42 words", the text itself when short, and a warning
/// when it is too long to send.
fn text_row(ui: &mut Ui, parts: &PromptParts) {
    ui.label(parts.text.describe());
    if let Some(shown) = parts.text.shown.as_deref().filter(|shown| !shown.is_empty()) {
        boxed(ui, |ui| render_answer(ui, shown));
    }
    if parts.too_long {
        ui.label(RichText::new("Too long to send; select a smaller part.").color(ui.visuals().error_fg_color));
    }
}

/// The command in a code box: one cut-short line when closed, wrapped when open.
fn command_box(ui: &mut Ui, command: &str, open: bool) {
    boxed(ui, |ui| {
        let text = RichText::new(command).monospace().size(CODE_TEXT);
        let label = egui::Label::new(text).selectable(true);
        ui.add(if open { label.wrap() } else { label.truncate() });
    });
}

/// A labelled row: the label at the left, `contents` in the space left.
fn row<R>(ui: &mut Ui, label: &str, contents: impl FnOnce(&mut Ui) -> R) {
    ui.horizontal_top(|ui| {
        let size = egui::vec2(ROW_LABEL_WIDTH, ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Min), |ui| {
            ui.set_min_width(ROW_LABEL_WIDTH);
            ui.add(egui::Label::new(RichText::new(label).weak()).selectable(false));
        });
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            contents(ui);
        });
    });
}

/// A row whose label opens and closes it; `contents` gets whether it is open.
/// The choice is kept for the session under `id`.
fn fold_row(ui: &mut Ui, id: egui::Id, label: &str, open_first: bool, contents: impl FnOnce(&mut Ui, bool)) {
    let mut open = ui.data_mut(|data| *data.get_temp_mut_or(id, open_first));
    ui.horizontal_top(|ui| {
        let size = egui::vec2(ROW_LABEL_WIDTH, ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Min), |ui| {
            ui.set_min_width(ROW_LABEL_WIDTH);
            let tip = if open { "Show less" } else { "Show all" };
            if fold_label(ui, RichText::new(label).weak(), open).on_hover_text(tip).clicked() {
                open = !open;
            }
        });
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            contents(ui, open);
        });
    });
    ui.data_mut(|data| data.insert_temp(id, open));
}

/// `text` with an arrow after it (right when closed, down when open); both click.
fn fold_label(ui: &mut Ui, text: RichText, open: bool) -> Response {
    ui.spacing_mut().item_spacing.x = 4.0;
    let label = ui.add(egui::Label::new(text).selectable(false).sense(Sense::click()));
    let (_, arrow) = ui.allocate_exact_size(egui::vec2(ARROW_SIZE, ARROW_SIZE), Sense::click());
    egui::collapsing_header::paint_default_icon(ui, if open { 1.0 } else { 0.0 }, &arrow);
    let response = label | arrow;
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// `contents` on the code background, full width.
fn boxed(ui: &mut Ui, contents: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(ui.visuals().code_bg_color)
        .corner_radius(5)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            contents(ui);
        });
}

/// "Compare with": none, or a second tool (not the first one).
fn compare_picker(ui: &mut Ui, bar: &mut AskBar, installed: [bool; 4]) {
    if bar.compare == Some(bar.tool) {
        bar.compare = None;
    }
    let label = bar.compare.map_or("None", AiTool::name);
    egui::ComboBox::from_id_salt("ask-compare")
        .selected_text(label)
        .width(COMPARE_PICKER_WIDTH)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut bar.compare, None, "None");
            for (choice, found) in AiTool::ALL.into_iter().zip(installed) {
                if choice == bar.tool {
                    continue;
                }
                ui.add_enabled_ui(found, |ui| ui.selectable_value(&mut bar.compare, Some(choice), choice.name()))
                    .response
                    .on_disabled_hover_text(format!(
                        "{} is not installed. Install it in Settings → AI.",
                        choice.name()
                    ));
            }
        })
        .response
        .on_hover_text("Run a second tool at the same time and compare the answers in the merge screen");
}

/// A small drop-down of the tools; ones that were not found are disabled.
fn tool_picker(ui: &mut Ui, tool: &mut AiTool, installed: [bool; 4]) {
    egui::ComboBox::from_id_salt("ask-tool").selected_text(tool.name()).width(TOOL_PICKER_WIDTH).show_ui(ui, |ui| {
        for (choice, found) in AiTool::ALL.into_iter().zip(installed) {
            ui.add_enabled_ui(found, |ui| ui.selectable_value(tool, choice, choice.name()))
                .response
                .on_disabled_hover_text(format!("{} is not installed. Install it in Settings → AI.", choice.name()));
        }
    });
}

/// Where a run stands, for the output panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputState {
    Running { seconds: u64 },
    Done { seconds: u64 },
    Failed { seconds: u64, code: Option<i32> },
    Stopped { seconds: u64 },
}

/// One tool's run in the output panel.
#[derive(Debug)]
pub struct RunView {
    pub tool: AiTool,
    /// The command as run, prompt shortened.
    pub command: String,
    pub output: String,
    pub errors: String,
    pub state: OutputState,
    /// Draw the output as Markdown (an answer to read, or new text for a
    /// Markdown file); otherwise it is shown as it is, in monospace.
    pub formatted: bool,
}

/// What the output panel shows: one run, or two when comparing tools.
#[derive(Debug)]
pub struct OutputInfo<'a> {
    pub request: &'a str,
    pub runs: Vec<RunView>,
    /// The answers can go to the merge screen: "Review in merge" for one,
    /// "Compare in merge" for two.
    pub reviewable: usize,
}

/// What the user clicked in the output panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputAction {
    Stop,
    Copy,
    /// Copy one code block of a formatted answer.
    CopyCode(String),
    Review,
    Close,
}

/// The output panel: header with state and buttons, then what each tool printed.
pub fn show_output(ui: &mut Ui, info: &OutputInfo) -> Option<OutputAction> {
    let mut action = None;
    let visuals = ui.visuals().clone();
    let running = info.runs.iter().any(|run| matches!(run.state, OutputState::Running { .. }));
    let tools = info.runs.iter().map(|run| run.tool.name()).collect::<Vec<_>>().join(" and ");
    egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            icon_label(ui, Icon::Sparkle);
            ui.label(RichText::new(tools).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if running {
                    let tip = if info.runs.len() > 1 { "Stop both tools" } else { "Stop the tool" };
                    if icon_text_button(ui, Icon::Stop, "Stop", tip, false).clicked() {
                        action = Some(OutputAction::Stop);
                    }
                    ui.add(egui::Spinner::new().size(12.0));
                } else {
                    if ui.button("×").on_hover_text("Close").clicked() {
                        action = Some(OutputAction::Close);
                    }
                    if info.reviewable > 0 {
                        let label = if info.reviewable > 1 { "Compare in merge" } else { "Review in merge" };
                        let review = egui::Button::new(RichText::new(label).color(visuals.strong_text_color()))
                            .fill(visuals.selection.bg_fill);
                        if ui.add(review).on_hover_text("See each change and decide").clicked() {
                            action = Some(OutputAction::Review);
                        }
                    }
                    if ui.button("Copy").clicked() {
                        action = Some(OutputAction::Copy);
                    }
                }
                for run in info.runs.iter().rev() {
                    let (text, color) = state_text(run, info.runs.len() > 1, &visuals);
                    ui.label(RichText::new(text).color(color));
                }
                // The request fills the space left, cut short with "…" when needed.
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(shorten(info.request)).weak()).truncate());
                });
            });
        });
    });
    ui.separator();
    egui::ScrollArea::vertical().id_salt("ai-output").auto_shrink([false, false]).stick_to_bottom(true).show(
        ui,
        |ui| {
            egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = OUTPUT_ITEM_GAP;
                if let [only] = info.runs.as_slice() {
                    run_output(ui, only, false);
                } else {
                    ui.columns(info.runs.len(), |columns| {
                        for (column, run) in columns.iter_mut().zip(&info.runs) {
                            column.with_layout(Layout::top_down(Align::LEFT), |ui| run_output(ui, run, true));
                        }
                    });
                }
            });
        },
    );
    if let Some(CodeAction::Copy(code)) = take_code_action(ui.ctx()) {
        action = Some(OutputAction::CopyCode(code));
    }
    action
}

/// "Done in 0:12" and the like; with the tool name when there are two.
fn state_text(run: &RunView, named: bool, visuals: &egui::Visuals) -> (String, egui::Color32) {
    let name = if named { format!("{} ", run.tool.name()) } else { String::new() };
    match run.state {
        OutputState::Running { seconds } => (format!("{name}{}", format_elapsed(seconds)), visuals.weak_text_color()),
        OutputState::Done { seconds } => {
            let done = if named { name } else { "Done in ".to_string() };
            (format!("{done}{}", format_elapsed(seconds)), visuals.weak_text_color())
        }
        OutputState::Stopped { seconds } => {
            (format!("{name}stopped at {}", format_elapsed(seconds)), visuals.weak_text_color())
        }
        OutputState::Failed { code, .. } => (
            code.map_or_else(|| format!("{name}failed"), |code| format!("{name}failed (exit code {code})")),
            visuals.error_fg_color,
        ),
    }
}

/// What one tool printed, under its command.
fn run_output(ui: &mut Ui, run: &RunView, titled: bool) {
    let visuals = ui.visuals().clone();
    if titled {
        ui.label(RichText::new(run.tool.name()).strong());
    }
    ui.label(RichText::new(format!("$ {}", run.command)).monospace().weak());
    if run.formatted && !run.output.trim().is_empty() {
        render_answer(ui, &run.output);
    } else if !run.output.is_empty() {
        ui.add(egui::Label::new(RichText::new(&run.output).monospace()).selectable(true));
    }
    let failed = matches!(run.state, OutputState::Failed { .. });
    if !run.errors.trim().is_empty() && (failed || run.output.trim().is_empty()) {
        let errors = RichText::new(run.errors.trim_end()).monospace().color(visuals.error_fg_color);
        ui.add(egui::Label::new(errors).selectable(true));
    }
    if matches!(run.state, OutputState::Done { .. }) && run.output.trim().is_empty() {
        ui.label(RichText::new("The tool printed nothing.").weak());
    }
}

/// `request` cut to fit the header.
fn shorten(request: &str) -> String {
    let flat = request.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > HEADER_REQUEST_CHARS {
        format!("{}…", flat.chars().take(HEADER_REQUEST_CHARS).collect::<String>())
    } else {
        flat
    }
}

/// The question before the first run in a folder, for runs that do not
/// start from the Ask bar (Resolve notes).
#[derive(Debug)]
pub struct Consent {
    /// What the run will send.
    pub parts: PromptParts,
    pub folder: PathBuf,
    /// "Don't ask again for this folder".
    pub remember: bool,
}

/// The user's answer in the consent dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsentChoice {
    Run,
    Cancel,
}

/// Ask whether to run the tool(s) in the folder, showing what they get.
pub fn show_consent(ctx: &Context, consent: &mut Consent) -> Option<ConsentChoice> {
    let modal = egui::Modal::new(egui::Id::unique("ai-consent")).show(ctx, |ui| {
        ui.set_width(CONSENT_WIDTH);
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.heading(format!("Run {} in this folder?", consent.parts.tools()));
        egui::ScrollArea::vertical().max_height(ctx.content_rect().height() * 0.6).show(ui, |ui| {
            prompt_rows(ui, &consent.parts, &consent.folder, "consent");
        });
        ui.label(
            RichText::new("Your file changes only when you press Apply in the merge screen.").size(SMALL_TEXT).weak(),
        );
        ui.checkbox(&mut consent.remember, "Don't ask again for this folder");
        ui.add_space(4.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Run").clicked() {
                Some(ConsentChoice::Run)
            } else if ui.button("Cancel").clicked() {
                Some(ConsentChoice::Cancel)
            } else {
                None
            }
        })
        .inner
    });
    if modal.should_close() { Some(ConsentChoice::Cancel) } else { modal.inner }
}
