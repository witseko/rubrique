//! The settings window: groups on the left, the open group on the right.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use eframe::egui::collapsing_header::CollapsingState;
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, WidgetInfo,
    pos2, vec2,
};

use crate::ai::agents::Agent;
use crate::ai::flags::{self, FlagKind};
use crate::ai::skills::{Skill, SourceFolder};
use crate::ai::{AiSettings, AiTool, CommandProblem, PROMPT, SYSTEM, check_command};
use crate::document::highlighting::{HighlightStyle, Syntax, highlighted};
use crate::install::view::{InstallAction, install_prompt};
use crate::install::{InstallStatus, Recipe};
use crate::settings::accessibility::{AccessibilitySettings, EDITOR_FONT_SIZES, TextSize};
use crate::settings::privacy::{LOCK_AFTER_CHOICES, PrivacySettings};
use crate::theme::catalog::{ThemeChoice, Themes};
use crate::theme::code_colors::CodeColors;
use crate::theme::palette::{Base, Palette, Rgb, Theme};
use crate::voice::VoiceSettings;

mod skill_list;

use skill_list::{SkillsInfo, skills_section};

/// Something the user asked for in the settings panel.
#[derive(Debug)]
pub enum SettingsAction {
    PickTheme(ThemeChoice),
    PickCodeColors(CodeColors),
    ReloadThemes,
    /// Text size, editor font, motion or focus outline changed.
    AccessibilityChanged,
    /// Pick another folder for scratch files.
    ChooseScratchFolder,
    /// Go back to the default scratch folder.
    ResetScratchFolder,
    /// Remote folders were turned on or off, or the ssh program changed.
    RemoteChanged,
    /// An AI command changed, so which tools are installed may have too.
    AiCommandChanged,
    /// Pick another skills folder.
    ChooseSkillsFolder,
    /// Go back to the default skills folder.
    ResetSkillsFolder,
    /// Read the skills folders again.
    ReloadSkills,
    /// Write the example skill into the skills folder.
    CreateExampleSkill,
    /// Pick another agents folder.
    ChooseAgentsFolder,
    /// Go back to the default agents folder.
    ResetAgentsFolder,
    /// Read the agents folders again.
    ReloadAgents,
    /// Write the example agent into the agents folder.
    CreateExampleAgent,
    /// Open this file in a tab (and close Settings).
    OpenFile(PathBuf),
    /// Show this folder in the system's file manager.
    ShowFolder(PathBuf),
    /// Ask again before the next run in every folder.
    ForgetAllowedFolders,
    /// The transcription command changed, so look for its program again.
    VoiceCommandChanged,
    /// Look for tmux again (after installing it in a terminal).
    CheckTerminals,
    /// Put this text on the clipboard.
    Copy(String),
    /// A click in an install prompt.
    Install(InstallAction),
    /// Choose the scratch password, which turns encryption on.
    SetScratchPassword,
    /// Choose a new scratch password and seal every scratch with it.
    ChangeScratchPassword,
    /// Enter the scratch password for this session.
    UnlockScratches,
    /// Forget the scratch password until it is entered again.
    LockScratches,
    /// Encrypt the plain scratches already in the folder.
    EncryptExistingScratches,
    /// Choose a PIN, or a new one.
    SetPin,
    /// Turn the PIN off (asks for it first).
    RemovePin,
    /// Lock the window now.
    LockNow,
    Close,
}

/// Voice memo settings to edit, and where the transcription program was found.
#[derive(Debug)]
pub struct VoiceInfo<'a> {
    pub settings: &'a mut VoiceSettings,
    pub found: Option<&'a Path>,
    /// How to install the transcription program, when it is missing.
    pub recipe: Option<&'a Recipe>,
    /// How to download the model, when it is missing.
    pub model_recipe: Option<&'a Recipe>,
    pub install: &'a InstallStatus,
}

/// Whether tmux was found, for the Terminals section.
#[derive(Clone, Copy, Debug)]
pub struct TerminalsInfo<'a> {
    /// Where tmux is; `None` when it was not found and Terminals are off.
    pub found: Option<&'a Path>,
    /// How to install tmux, when it is missing.
    pub recipe: Option<&'a Recipe>,
    pub install: &'a InstallStatus,
}

/// AI settings to edit, and which tools were found (in [`AiTool::ALL`] order).
#[derive(Debug)]
pub struct AiInfo<'a> {
    pub settings: &'a mut AiSettings,
    /// Where each tool's program was found, in [`AiTool::ALL`] order.
    pub found: &'a [Option<PathBuf>; 4],
    /// Skills found in the skills folders.
    pub skills: &'a [Skill],
    /// Every skill source's folder, in reading order, switched on or not.
    pub skill_folders: &'a [SourceFolder],
    /// Agents found in the agents folders.
    pub agents: &'a [Agent],
    /// Rubrique's agents folder (the chosen one or the default).
    pub agents_folder: Option<&'a Path>,
    /// How to install each missing tool, in [`AiTool::ALL`] order.
    pub recipes: &'a [Option<Recipe>; 4],
    /// How to download the local model, when Ollama is there but the model is not.
    pub local_model: Option<&'a Recipe>,
    pub install: &'a InstallStatus,
}

/// Scratch encryption and the PIN, for the Privacy section.
#[derive(Debug)]
pub struct PrivacyInfo<'a> {
    pub settings: &'a mut PrivacySettings,
    /// The scratch password was entered in this session.
    pub unlocked: bool,
}

/// Where scratch files go, and remote folder settings, for the Files section.
#[derive(Debug)]
pub struct ScratchInfo<'a> {
    pub folder: Option<&'a Path>,
    /// The folder is the default one (so "Use default" has nothing to do).
    pub is_default: bool,
    /// Remote folders over SSH.
    pub remote: &'a mut crate::remote::RemoteSettings,
}

const TITLE_SIZE: f32 = 16.0;
/// Space between theme cards.
const CARD_GAP: f32 = 10.0;
const CARD_PADDING: f32 = 6.0;
const CARD_RADIUS: u8 = 8;
const PREVIEW_HEIGHT: f32 = 64.0;
const PREVIEW_RADIUS: u8 = 5;
/// Height of the name row under a card's preview.
const NAME_ROW_HEIGHT: f32 = 22.0;
/// Width of the explorer strip in a card's preview, as a share of the preview.
const PREVIEW_SIDEBAR_SHARE: f32 = 0.26;
const PREVIEW_PADDING: f32 = 8.0;
/// Bars standing for text in a card's preview: width share, height, kind.
const PREVIEW_BARS: [(f32, f32, BarKind); 4] =
    [(0.55, 5.0, BarKind::Accent), (0.85, 4.0, BarKind::Text), (0.7, 4.0, BarKind::Text), (0.45, 4.0, BarKind::Weak)];
const PREVIEW_BAR_GAP: f32 = 5.0;
const CHECK_SIZE: f32 = 12.0;
/// The code in a Code colors card: a keyword, a defined name, a comment, a
/// string and a number, so each choice shows its main colors.
const CODE_PREVIEW_TEXT: &str = "def word_count(note):\n    # words in the body\n    return len(note.split(\" \")) + 1";
/// Font size and line height of the code in a Code colors card.
const CODE_PREVIEW_FONT_SIZE: f32 = 11.0;
const CODE_PREVIEW_LINE_HEIGHT: f32 = 16.0;

#[derive(Clone, Copy)]
enum BarKind {
    Accent,
    Text,
    Weak,
}

/// A group in the settings window's side list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SettingsGroup {
    #[default]
    Appearance,
    Accessibility,
    Files,
    Privacy,
    Ai,
    Skills,
    Agents,
    Terminals,
    VoiceMemos,
}

impl SettingsGroup {
    pub const ALL: [SettingsGroup; 9] = [
        Self::Appearance,
        Self::Accessibility,
        Self::Files,
        Self::Privacy,
        Self::Ai,
        Self::Skills,
        Self::Agents,
        Self::Terminals,
        Self::VoiceMemos,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Accessibility => "Accessibility",
            Self::Files => "Files",
            Self::Privacy => "Privacy",
            Self::Ai => "AI",
            Self::Skills => "Skills",
            Self::Agents => "Agents",
            Self::Terminals => "Terminals",
            Self::VoiceMemos => "Voice memos",
        }
    }
}

/// Share of the app window's width and height the settings window takes.
const WINDOW_SHARE: egui::Vec2 = vec2(0.7, 0.8);
/// Smallest size the settings window takes while the app window has room for it.
const WINDOW_MIN_SIZE: egui::Vec2 = vec2(780.0, 540.0);
/// Largest size of the settings window, so lines stay readable on big screens.
const WINDOW_MAX_SIZE: egui::Vec2 = vec2(1200.0, 900.0);
/// Space kept between the settings window and the app window's edges.
const WINDOW_MARGIN: f32 = 40.0;

/// Size of the settings window for the app window's content area.
fn window_size(ctx: &egui::Context) -> egui::Vec2 {
    window_size_for(ctx.content_rect().size())
}

/// Size of the settings window in an app window of `app_size` (in points):
/// a share of it, kept between [`WINDOW_MIN_SIZE`] and [`WINDOW_MAX_SIZE`].
/// A small app window (or large text) makes it smaller than the minimum so it
/// still fits inside the margin, down to room for the group list and a header.
fn window_size_for(app_size: egui::Vec2) -> egui::Vec2 {
    let room = app_size - vec2(WINDOW_MARGIN, WINDOW_MARGIN);
    let wanted = (app_size * WINDOW_SHARE).clamp(WINDOW_MIN_SIZE, WINDOW_MAX_SIZE);
    wanted.min(room).max(vec2(NAV_WIDTH * 2.0, HEADER_HEIGHT * 4.0))
}

/// Width of the group list on the left.
const NAV_WIDTH: f32 = 180.0;
const NAV_ITEM_HEIGHT: f32 = 30.0;
const HEADER_HEIGHT: f32 = 46.0;
/// Space around the content of a group.
const CONTENT_MARGIN: i8 = 22;

/// Draw the settings window's content: a title, the groups on the left and
/// the open group on the right. Returns what the user clicked.
#[expect(clippy::too_many_arguments, reason = "one borrowed view of each part of the settings")]
pub fn show_settings(
    ui: &mut Ui,
    group: &mut SettingsGroup,
    themes: &Themes,
    current: &ThemeChoice,
    code_colors: CodeColors,
    themes_folder: Option<&Path>,
    accessibility: &mut AccessibilitySettings,
    scratch: ScratchInfo,
    note_marks: &mut bool,
    privacy: PrivacyInfo,
    ai: AiInfo,
    voice: VoiceInfo,
    terminals: TerminalsInfo,
) -> Option<SettingsAction> {
    let mut action = None;
    let (outer, _) = ui.allocate_exact_size(window_size(ui.ctx()), Sense::hover());
    let header = Rect::from_min_size(outer.min, vec2(outer.width(), HEADER_HEIGHT));
    let nav = Rect::from_min_max(pos2(outer.left(), header.bottom()), pos2(outer.left() + NAV_WIDTH, outer.bottom()));
    let content = Rect::from_min_max(pos2(nav.right(), header.bottom()), outer.max);
    let visuals = ui.visuals().clone();
    let line = visuals.widgets.noninteractive.bg_stroke;
    let painter = ui.painter();
    painter.rect_filled(nav, egui::CornerRadius { sw: 12, ..Default::default() }, visuals.panel_fill);
    painter.hline(outer.x_range(), header.bottom(), line);
    painter.vline(nav.right(), nav.y_range(), line);

    let header_inner = header.shrink2(vec2(20.0, 0.0)).with_max_x(header.right() - 10.0);
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(header_inner).layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.label(RichText::new("Settings").size(TITLE_SIZE).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("×").on_hover_text("Close (Esc)").clicked() {
                    action = Some(SettingsAction::Close);
                }
            });
        },
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(nav.shrink2(vec2(8.0, 10.0))), |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for each in SettingsGroup::ALL {
            if nav_item(ui, each.name(), *group == each).clicked() {
                *group = each;
            }
        }
        version_label(ui);
    });
    ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
        egui::ScrollArea::vertical().id_salt(("settings-group", *group)).auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::new().inner_margin(CONTENT_MARGIN).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                let picked = match *group {
                    SettingsGroup::Appearance => appearance_section(ui, themes, current, code_colors, themes_folder),
                    SettingsGroup::Accessibility => accessibility_section(ui, accessibility),
                    SettingsGroup::Files => files_section(ui, scratch, note_marks),
                    SettingsGroup::Privacy => privacy_section(ui, privacy),
                    SettingsGroup::Ai => ai_section(ui, ai),
                    SettingsGroup::Skills => {
                        let AiInfo { settings, skills, skill_folders, .. } = ai;
                        skills_section(ui, SkillsInfo { settings, skills, folders: skill_folders })
                    }
                    SettingsGroup::Agents => {
                        let AiInfo { settings, agents, agents_folder, .. } = ai;
                        agents_section(ui, settings, agents, agents_folder)
                    }
                    SettingsGroup::Terminals => terminals_section(ui, terminals),
                    SettingsGroup::VoiceMemos => voice_section(ui, voice),
                };
                if picked.is_some() {
                    action = picked;
                }
            });
        });
    });
    action
}

/// One group in the side list: full width, filled when it is the open one.
fn nav_item(ui: &mut Ui, name: &str, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), NAV_ITEM_HEIGHT), Sense::click());
    let visuals = ui.visuals().clone();
    let fill = if selected {
        visuals.selection.bg_fill
    } else if response.hovered() {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 7, fill);
    let color = if selected { visuals.strong_text_color() } else { visuals.text_color() };
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.painter().text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, name, font, color);
    response.widget_info(|| WidgetInfo::selected(egui::Role::Tab, true, selected, name));
    response
}

/// The app's version, small and dim at the bottom of the group list, so a
/// user can read it off before reporting a problem or checking for a
/// newer release.
fn version_label(ui: &mut Ui) {
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            // Lined up with the group names, which start 10 points in (see `nav_item`).
            ui.add_space(10.0);
            ui.label(RichText::new(concat!("Rubrique ", env!("CARGO_PKG_VERSION"))).weak().small())
                .on_hover_text("The version of Rubrique you are running");
        });
    });
}

/// Appearance: the theme cards, the code colors and the user's own themes.
fn appearance_section(
    ui: &mut Ui,
    themes: &Themes,
    current: &ThemeChoice,
    code_colors: CodeColors,
    themes_folder: Option<&Path>,
) -> Option<SettingsAction> {
    let mut action = None;
    ui.label(RichText::new("Theme").strong());
    if let Some(choice) = theme_cards(ui, themes, current) {
        action = Some(SettingsAction::PickTheme(choice));
    }
    ui.add_space(6.0);
    ui.separator();
    ui.label(RichText::new("Code colors").strong());
    ui.label(
        RichText::new(
            "How code, Markdown and YAML are colored in the editor and preview. \
             VS Code uses its Dark+ colors on dark themes and Light+ on light ones.",
        )
        .weak(),
    );
    if let Some(choice) = code_color_cards(ui, code_colors) {
        action = Some(SettingsAction::PickCodeColors(choice));
    }
    ui.add_space(10.0);
    ui.label(RichText::new("Your themes").strong());
    ui.label(RichText::new("Add .theme files to this folder, then reload.").weak());
    if let Some(folder) = themes_folder {
        path_box(ui, folder);
    }
    if ui.button("Reload themes").clicked() {
        action = Some(SettingsAction::ReloadThemes);
    }
    action
}

/// Accessibility: text size, editor font size, motion and focus outlines.
fn accessibility_section(ui: &mut Ui, settings: &mut AccessibilitySettings) -> Option<SettingsAction> {
    let mut changed = false;
    ui.label(RichText::new("Text size").strong());
    ui.label(RichText::new("Scales the whole window. Cmd/Ctrl + plus, minus and 0 change it too.").weak());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for size in TextSize::ALL {
            let percent = format!("{}%", (size.scale() * 100.0).round());
            changed |= ui.selectable_value(&mut settings.text_size, size, size.name()).on_hover_text(percent).changed();
        }
    });

    ui.add_space(6.0);
    ui.separator();
    ui.label(RichText::new("Editor font size").strong());
    ui.label(RichText::new("The Markdown editor and other monospace text, before the text size.").weak());
    let slider = egui::Slider::new(&mut settings.editor_font_size, EDITOR_FONT_SIZES).suffix(" pt");
    changed |= ui.add(slider).changed();
    ui.add_space(4.0);
    changed |= ui
        .checkbox(&mut settings.line_numbers, "Line numbers")
        .on_hover_text("Number the lines in the source editor")
        .changed();

    ui.add_space(6.0);
    ui.separator();
    ui.label(RichText::new("Motion and focus").strong());
    changed |= ui.checkbox(&mut settings.reduce_motion, "Reduce motion").on_hover_text("Turn off animations").changed();
    changed |= ui
        .checkbox(&mut settings.strong_focus, "Stronger focus outline")
        .on_hover_text("A thicker outline on the focused button or text field")
        .changed();
    changed.then_some(SettingsAction::AccessibilityChanged)
}

/// Files: where scratch files go, and the explorer's note marker.
fn files_section(ui: &mut Ui, scratch: ScratchInfo, note_marks: &mut bool) -> Option<SettingsAction> {
    let mut action = None;
    ui.label(RichText::new("Explorer").strong());
    ui.checkbox(note_marks, "Mark files that have notes")
        .on_hover_text("A small speech bubble before the name of a Markdown file with notes");
    ui.add_space(6.0);
    ui.separator();
    ui.label(RichText::new("Scratches").strong());
    ui.label(RichText::new("New scratch files (Cmd+N) go to this folder.").weak());
    if let Some(folder) = scratch.folder {
        path_box(ui, folder);
    }
    ui.horizontal(|ui| {
        if ui.button("Change…").clicked() {
            action = Some(SettingsAction::ChooseScratchFolder);
        }
        if ui.add_enabled(!scratch.is_default, egui::Button::new("Use default")).clicked() {
            action = Some(SettingsAction::ResetScratchFolder);
        }
    });
    ui.add_space(16.0);
    ui.label(RichText::new("Remote folders").strong());
    let remote = scratch.remote;
    if ui.checkbox(&mut remote.enabled, "Open folders on other computers over SSH").changed() {
        action = Some(SettingsAction::RemoteChanged);
    }
    ui.label(
        RichText::new(
            "Adds \"Add remote folder\" to the explorer. Uses your ssh setup (~/.ssh/config, keys, ssh-agent). \
             Password logins are not supported: use a key or ssh-agent.",
        )
        .weak(),
    );
    if remote.enabled {
        // A few file checks on this computer; it never connects anywhere.
        let check = crate::remote::check_ssh_program(&remote.ssh_program);
        ui.horizontal(|ui| {
            ui.label("ssh program");
            let field = ui.add(egui::TextEdit::singleline(&mut remote.ssh_program).desired_width(220.0));
            if field.lost_focus() || (field.changed() && !field.has_focus()) {
                action = Some(SettingsAction::RemoteChanged);
            }
            if let crate::remote::ProgramCheck::Found(path) = &check {
                ui.label(RichText::new(format!("Found: {}", path.display())).weak());
            }
        });
        if let crate::remote::ProgramCheck::Problem(message) = &check {
            ui.colored_label(ui.visuals().warn_fg_color, message);
        }
    }
    action
}

/// Width of the "Lock after" drop-down.
const LOCK_PICKER_WIDTH: f32 = 120.0;

/// "Off", "1 minute", "15 minutes".
fn lock_after_label(minutes: u32) -> String {
    match minutes {
        0 => "Off".to_string(),
        1 => "1 minute".to_string(),
        _ => format!("{minutes} minutes"),
    }
}

/// A button that can be greyed out; true when clicked.
fn enabled_button(ui: &mut Ui, label: &str, enabled: bool) -> bool {
    ui.add_enabled(enabled, egui::Button::new(label)).clicked()
}

/// Privacy: encrypted scratches and the app PIN.
fn privacy_section(ui: &mut Ui, privacy: PrivacyInfo) -> Option<SettingsAction> {
    let PrivacyInfo { settings, unlocked } = privacy;
    let mut action = None;

    ui.label(RichText::new("Encrypted scratches").strong());
    ui.label(
        RichText::new(
            "Encrypted scratches are saved with a password. They open only after you enter it, and AI \
             tools never see them. Each scratch can be encrypted on its own, at any time: the lock button \
             in the status bar, right-click it in the explorer, or Cmd+Shift+N for a new encrypted one.",
        )
        .weak(),
    );
    let has_password = settings.vault.is_some();
    let mut encrypt = settings.encrypt_scratches;
    if ui.checkbox(&mut encrypt, "Encrypt every new scratch (Cmd+N)").changed() {
        if encrypt && !has_password {
            action = Some(SettingsAction::SetScratchPassword);
        } else {
            settings.encrypt_scratches = encrypt;
        }
    }
    if has_password {
        ui.label(if unlocked { "Unlocked for this session." } else { "Locked: enter the password to read them." });
        ui.horizontal_wrapped(|ui| {
            let (label, toggle) = if unlocked {
                ("Lock scratches", SettingsAction::LockScratches)
            } else {
                ("Unlock…", SettingsAction::UnlockScratches)
            };
            if enabled_button(ui, label, true) {
                action = Some(toggle);
            }
            if enabled_button(ui, "Change password…", true) {
                action = Some(SettingsAction::ChangeScratchPassword);
            }
            if enabled_button(ui, "Encrypt existing scratches", unlocked) {
                action = Some(SettingsAction::EncryptExistingScratches);
            }
        });
        ui.label(
            RichText::new("If you forget the password, encrypted scratches cannot be read. There is no way back.")
                .color(ui.visuals().warn_fg_color),
        );
    }

    ui.add_space(12.0);
    ui.label(RichText::new("App PIN").strong());
    ui.label(
        RichText::new(
            "Ask for a PIN when Rubrique opens and after it sits idle. The PIN locks the window only: \
             it does not encrypt your files.",
        )
        .weak(),
    );
    let has_pin = settings.pin_hash.is_some();
    ui.horizontal_wrapped(|ui| {
        if enabled_button(ui, if has_pin { "Change PIN…" } else { "Set PIN…" }, true) {
            action = Some(SettingsAction::SetPin);
        }
        if enabled_button(ui, "Remove PIN…", has_pin) {
            action = Some(SettingsAction::RemovePin);
        }
        if enabled_button(ui, "Lock app now", has_pin) {
            action = Some(SettingsAction::LockNow);
        }
    });
    ui.add_enabled_ui(has_pin, |ui| {
        ui.horizontal(|ui| {
            ui.label("Lock after");
            egui::ComboBox::from_id_salt("privacy-lock-after")
                .selected_text(lock_after_label(settings.lock_after_minutes))
                .width(LOCK_PICKER_WIDTH)
                .show_ui(ui, |ui| {
                    for minutes in LOCK_AFTER_CHOICES {
                        ui.selectable_value(&mut settings.lock_after_minutes, minutes, lock_after_label(minutes));
                    }
                });
            ui.label(RichText::new("without use").weak());
        });
    });
    action
}

/// Size of help lines under a command.
const SMALL_SIZE: f32 = 11.5;
/// Size of the Installed / Not found text.
const STATUS_SIZE: f32 = 12.0;
/// Size of the Ask and Summaries tags on a tool row.
const TAG_SIZE: f32 = 11.0;
/// Corner radius of the box around the tool rows.
const LIST_RADIUS: u8 = 6;
const TAG_RADIUS: u8 = 8;
/// Radius of the dot that marks a tool that was not found.
const MISSING_DOT_RADIUS: f32 = 3.5;
/// Space inside one tool row.
const ROW_MARGIN: egui::Margin = egui::Margin { left: 12, right: 10, top: 8, bottom: 8 };
/// What the Ask picker offers, in the order of the tools list.
const ASK_CHOICES: [Option<AiTool>; 4] =
    [Some(AiTool::Claude), Some(AiTool::Codex), Some(AiTool::Grok), Some(AiTool::Local)];
/// What the summaries picker offers: the local model first (text stays on
/// this computer), then the others, then Off.
const SUMMARY_CHOICES: [Option<AiTool>; 5] =
    [Some(AiTool::Local), Some(AiTool::Claude), Some(AiTool::Codex), Some(AiTool::Grok), None];

/// The AI section: which tool answers Ask and which writes summaries, then
/// every tool with its status, install help and (folded) command, then safety.
fn ai_section(ui: &mut Ui, ai: AiInfo) -> Option<SettingsAction> {
    let AiInfo { settings, found, recipes, local_model, install, .. } = ai;
    let mut action = None;

    role_pickers(ui, settings, found);

    ui.add_space(6.0);
    section_title(ui, "Tools", "Programs already on this computer. Rubrique starts them for you, no API keys.");
    list_frame(ui, |ui| {
        for (index, tool) in AiTool::ALL.into_iter().enumerate() {
            if index > 0 {
                ui.add(egui::Separator::default().spacing(0.0));
            }
            let row = ToolRow {
                tool,
                found: found[index].as_deref(),
                recipe: recipes[index].as_ref().filter(|_| found[index].is_none()),
                model_recipe: local_model.filter(|_| tool == AiTool::Local),
                install,
            };
            if let Some(picked) = tool_row(ui, settings, &row) {
                action = Some(picked);
            }
        }
    });

    ui.add_space(6.0);
    if let Some(picked) = safety_row(ui, settings) {
        action = Some(picked);
    }
    action
}

/// A section's name and, under it, what it is for.
fn section_title(ui: &mut Ui, title: &str, hint: &str) {
    ui.label(RichText::new(title).strong());
    ui.label(RichText::new(hint).weak());
}

/// The two choices that matter most, side by side: the tool behind Ask and
/// the tool behind summaries.
fn role_pickers(ui: &mut Ui, settings: &mut AiSettings, found: &[Option<PathBuf>; 4]) {
    let is_found = |tool: AiTool| found[tool_index(tool)].is_some();
    ui.columns(2, |columns| {
        let ui = &mut columns[0];
        ui.label(RichText::new("Ask and notes").strong());
        ui.label(RichText::new("Cmd+K, Resolve notes, Compare").weak().size(SMALL_SIZE));
        // The Ask tool is never Off, so it borrows the Option-based picker.
        let mut choice = Some(settings.default_tool);
        tool_picker(ui, "ai-ask-tool", &ASK_CHOICES, &mut choice, &is_found);
        if let Some(tool) = choice {
            settings.default_tool = tool;
        }

        let ui = &mut columns[1];
        ui.label(RichText::new("Summaries and names").strong());
        ui.label(RichText::new("Hover summaries in the explorer, scratch names").weak().size(SMALL_SIZE));
        tool_picker(ui, "ai-summary-tool", &SUMMARY_CHOICES, &mut settings.summary_tool, &is_found);
    });
}

/// A drop-down over `choices`, as wide as the space it gets. Tools that were
/// not found cannot be picked; `None` is Off.
fn tool_picker(
    ui: &mut Ui,
    salt: &str,
    choices: &[Option<AiTool>],
    choice: &mut Option<AiTool>,
    is_found: &dyn Fn(AiTool) -> bool,
) {
    let selected = match *choice {
        Some(tool) if !is_found(tool) => format!("{} (not found)", tool.full_name()),
        other => choice_name(other).to_string(),
    };
    egui::ComboBox::from_id_salt(salt).selected_text(selected).width(ui.available_width()).show_ui(ui, |ui| {
        for each in choices {
            if each.is_none() {
                ui.separator();
            }
            let enabled = each.is_none_or(is_found);
            ui.add_enabled_ui(enabled, |ui| ui.selectable_value(choice, *each, choice_name(*each)))
                .response
                .on_disabled_hover_text("Not found. See Tools below.");
        }
    });
}

fn choice_name(choice: Option<AiTool>) -> &'static str {
    choice.map_or("Off", AiTool::full_name)
}

/// The outlined box around the tool rows; the rows draw their own separators.
fn list_frame(ui: &mut Ui, add_rows: impl FnOnce(&mut Ui)) {
    egui::Frame::new().stroke(ui.visuals().widgets.noninteractive.bg_stroke).corner_radius(LIST_RADIUS).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add_rows(ui);
        },
    );
}

/// What one row of the tools list shows.
struct ToolRow<'a> {
    tool: AiTool,
    /// Where the program was found.
    found: Option<&'a Path>,
    /// How to install the program, when it is missing.
    recipe: Option<&'a Recipe>,
    /// How to download the local model, when Ollama is there but the model is not.
    model_recipe: Option<&'a Recipe>,
    install: &'a InstallStatus,
}

/// One tool: a status mark, the name, Ask and Summaries tags, where it was
/// found and a Command button. Under that, an install prompt when the program
/// or model is missing, and the command field when opened. A broken command
/// stays open until it is fixed.
fn tool_row(ui: &mut Ui, settings: &mut AiSettings, row: &ToolRow) -> Option<SettingsAction> {
    let mut action = None;
    let problem = check_command(settings.command(row.tool));
    let id = ui.make_persistent_id(("ai-command", row.tool.id()));
    let mut open = CollapsingState::load_with_default_open(ui.ctx(), id, false);
    if problem.is_some() {
        open.set_open(true);
    }
    egui::Frame::new().inner_margin(ROW_MARGIN).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.horizontal(|ui| {
            status_mark(ui, row.found.is_some());
            ui.label(RichText::new(row.tool.full_name()).strong());
            if settings.default_tool == row.tool {
                tag(ui, "Ask");
            }
            if settings.summary_tool == Some(row.tool) {
                tag(ui, "Summaries");
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if open.is_open() { "Hide command" } else { "Command" };
                if ui.add_enabled(problem.is_none(), egui::Button::new(label).small()).clicked() {
                    open.toggle(ui);
                }
                found_text(ui, row.found);
            });
        });
        if let Some(clicked) = install_lines(ui, row, problem.is_none()) {
            action = Some(SettingsAction::Install(clicked));
        }
        let body = open.show_body_unindented(ui, |ui| command_editor(ui, settings, row.tool, problem));
        if body.is_some_and(|body| body.inner) {
            action = Some(SettingsAction::AiCommandChanged);
        }
    });
    action
}

/// A check for a tool that was found, a warning dot for one that was not.
fn status_mark(ui: &mut Ui, found: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(CHECK_SIZE, CHECK_SIZE), Sense::hover());
    let visuals = ui.visuals();
    if found {
        paint_check(ui.painter(), rect.center(), visuals.hyperlink_color);
    } else {
        ui.painter().circle_filled(rect.center(), MISSING_DOT_RADIUS, visuals.warn_fg_color);
    }
}

/// A small tag on a tool row: "Ask" or "Summaries".
fn tag(ui: &mut Ui, text: &str) {
    egui::Frame::new()
        .fill(ui.visuals().selection.bg_fill)
        .corner_radius(TAG_RADIUS)
        .inner_margin(egui::Margin::symmetric(7, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(TAG_SIZE).strong());
        });
}

/// Where the program was found, as a short path (the full one on hover), or
/// "Not found".
fn found_text(ui: &mut Ui, found: Option<&Path>) {
    match found {
        Some(path) => {
            let short = RichText::new(home_relative(path)).monospace().size(SMALL_SIZE).weak();
            ui.add(egui::Label::new(short).truncate())
                .on_hover_text(RichText::new(path.display().to_string()).monospace());
        }
        None => {
            ui.label(RichText::new("Not found").size(STATUS_SIZE).color(ui.visuals().warn_fg_color));
        }
    }
}

/// Under a row: how to install a missing program, or download the missing
/// local model. Nothing while the command is broken: that comes first.
fn install_lines(ui: &mut Ui, row: &ToolRow, command_ok: bool) -> Option<InstallAction> {
    if !command_ok {
        return None;
    }
    let mut clicked = None;
    match (row.found, row.recipe) {
        (Some(_), _) => {}
        (None, Some(recipe)) => {
            let message = format!("{} is not installed, or not on PATH.", recipe.name);
            clicked = prompt(ui, &message, recipe, row.install);
        }
        (None, None) => {
            ui.label(RichText::new(missing_help(row.tool)).size(SMALL_SIZE).weak());
        }
    }
    if let Some(recipe) = row.model_recipe {
        let message = format!("Model {} is not downloaded yet. Ollama gets it on first use, or now:", recipe.name);
        if let Some(choice) = prompt(ui, &message, recipe, row.install) {
            clicked = Some(choice);
        }
    }
    clicked
}

/// What to do about a program that was not found and has no install recipe.
fn missing_help(tool: AiTool) -> String {
    match tool {
        AiTool::Local => {
            "Ollama is not on PATH. Install Ollama or llama.cpp, or put the full path in the command.".to_string()
        }
        AiTool::Grok => "No Grok tool on PATH. Install one, then set its command.".to_string(),
        tool => format!("{} is not on PATH. Install it, or put the full path in the command.", tool.full_name()),
    }
}

/// The command field, Reset when it was changed, and a line under it: the
/// problem, or what the placeholders mean. Returns whether the command changed.
fn command_editor(ui: &mut Ui, settings: &mut AiSettings, tool: AiTool, problem: Option<CommandProblem>) -> bool {
    let mut changed = flag_chips(ui, settings, tool);
    let visuals = ui.visuals().clone();
    field_box(ui, &visuals, problem.is_some(), |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Command").size(SMALL_SIZE).weak());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if settings.is_custom(tool) && ui.small_button("Reset").on_hover_text(tool.default_command()).clicked()
                {
                    settings.reset_command(tool);
                    changed = true;
                }
                let edit = egui::TextEdit::singleline(settings.command_mut(tool))
                    .font(FontId::monospace(12.0))
                    .frame(egui::Frame::NONE)
                    .hint_text(format!("{} command with {PROMPT}", tool.id()))
                    .desired_width(ui.available_width());
                changed |= ui.add(edit).changed();
            });
        });
    });
    let (text, color) = match problem {
        Some(problem) => (problem.message().to_string(), visuals.warn_fg_color),
        None => (command_hint(tool, settings.command(tool)), visuals.weak_text_color()),
    };
    ui.label(RichText::new(text).size(SMALL_SIZE).color(color));
    changed
}

/// Width of the model field in a chip.
const MODEL_CHIP_WIDTH: f32 = 110.0;
/// Width of a choice drop-down in a chip.
const CHOICE_CHIP_WIDTH: f32 = 150.0;
/// What a choice chip says when the option is not in the command.
const TOOL_DEFAULT: &str = "tool default";

/// The flags Rubrique knows for `tool`, as chips above the command field:
/// a model field and the tool's choice options. A chip rewrites its flag in
/// the command; the rest of the command stays as typed. Returns whether the
/// command changed.
fn flag_chips(ui: &mut Ui, settings: &mut AiSettings, tool: AiTool) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for flag in flags::known_flags(tool) {
            ui.label(RichText::new(flag.label).size(SMALL_SIZE).weak());
            let current = flags::value(settings.command(tool), tool, flag);
            let edit = match flag.kind {
                FlagKind::Model => model_chip(ui, tool, current),
                FlagKind::Choice { values, .. } => choice_chip(ui, tool, flag.label, values, current.as_deref()),
            };
            if let Some(edit) = edit {
                let command = flags::with_value(settings.command(tool), tool, flag, edit.value());
                settings.set_command(tool, &command);
                changed = true;
            }
            ui.add_space(6.0);
        }
    });
    changed
}

/// What a chip asks for.
enum ChipEdit {
    /// Set the flag to this value.
    Set(String),
    /// Take the flag out, back to the tool's own default.
    Clear,
}

impl ChipEdit {
    fn value(&self) -> Option<&str> {
        match self {
            ChipEdit::Set(value) => Some(value),
            ChipEdit::Clear => None,
        }
    }
}

/// A small text field with the model. Returns the edit when the field was
/// typed in; an emptied field clears the flag.
fn model_chip(ui: &mut Ui, tool: AiTool, current: Option<String>) -> Option<ChipEdit> {
    let mut text = current.unwrap_or_default();
    let edit = egui::TextEdit::singleline(&mut text)
        .id_salt(("ai-model-chip", tool.id()))
        .font(FontId::monospace(12.0))
        .hint_text(TOOL_DEFAULT)
        .desired_width(MODEL_CHIP_WIDTH);
    let response = ui.add(edit);
    if !response.changed() {
        return None;
    }
    let text = text.trim();
    Some(if text.is_empty() { ChipEdit::Clear } else { ChipEdit::Set(text.to_string()) })
}

/// A drop-down over the option's values, plus the tool's default (the
/// option taken out). Returns the pick when it differs from `current`.
fn choice_chip(ui: &mut Ui, tool: AiTool, label: &str, values: &[&str], current: Option<&str>) -> Option<ChipEdit> {
    let mut choice = current;
    egui::ComboBox::from_id_salt(("ai-choice-chip", tool.id(), label))
        .selected_text(choice.unwrap_or(TOOL_DEFAULT))
        .width(CHOICE_CHIP_WIDTH)
        .show_ui(ui, |ui| {
            for value in values {
                ui.selectable_value(&mut choice, Some(*value), *value);
            }
            ui.separator();
            ui.selectable_value(&mut choice, None, TOOL_DEFAULT);
        });
    if choice == current {
        return None;
    }
    Some(choice.map_or(ChipEdit::Clear, |value| ChipEdit::Set(value.to_string())))
}

/// What the placeholders in `command` mean, and for the local model which
/// programs work.
fn command_hint(tool: AiTool, command: &str) -> String {
    let mut hint = format!("{PROMPT} becomes your request");
    if command.contains(SYSTEM) {
        let _ = write!(hint, ", {SYSTEM} Rubrique's instructions");
    }
    hint.push('.');
    if tool == AiTool::Local {
        let _ = write!(hint, " Ollama or llama.cpp (llama-cli -m model.gguf -p {PROMPT}).");
    }
    hint
}

/// Safety: how many folders run without asking, and a way to forget them.
fn safety_row(ui: &mut Ui, settings: &AiSettings) -> Option<SettingsAction> {
    section_title(ui, "Safety", "You start every run. The first run in a folder shows the command and asks once.");
    let count = settings.allowed_folders.len();
    let text = match count {
        0 => "No folder runs without asking yet".to_string(),
        1 => "1 folder runs without asking".to_string(),
        count => format!("{count} folders run without asking"),
    };
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(text);
        if ui.add_enabled(count > 0, egui::Button::new("Forget allowed folders")).clicked() {
            action = Some(SettingsAction::ForgetAllowedFolders);
        }
    });
    action
}

/// The install prompt for `recipe`, with how its install went.
fn prompt(ui: &mut Ui, message: &str, recipe: &Recipe, install: &InstallStatus) -> Option<InstallAction> {
    install_prompt(ui, message, recipe, install.of(recipe), install.busy)
}

fn tool_index(tool: AiTool) -> usize {
    AiTool::ALL.iter().position(|each| *each == tool).unwrap_or_default()
}

/// Agents: the folder, Claude Code's agents, and every agent found, each
/// with how to call it and an Open button.
fn agents_section(
    ui: &mut Ui,
    settings: &mut AiSettings,
    agents: &[Agent],
    folder: Option<&Path>,
) -> Option<SettingsAction> {
    let mut action = None;
    ui.label(RichText::new("Agents").strong());
    ui.label(
        RichText::new(
            "Named helpers with their own personality, skills and AI tool. Call one in Ask (Cmd+K) \
             with @ai- and its handle, for example @ai-editor. One folder per agent with an AGENT.md file.",
        )
        .weak(),
    );
    if let Some(folder) = folder {
        path_box(ui, folder);
    }
    ui.horizontal(|ui| {
        if ui.button("Change…").clicked() {
            action = Some(SettingsAction::ChooseAgentsFolder);
        }
        if ui.add_enabled(settings.agents_folder.is_some(), egui::Button::new("Use default")).clicked() {
            action = Some(SettingsAction::ResetAgentsFolder);
        }
    });
    if ui.checkbox(&mut settings.use_claude_agents, "Also use Claude's agents (~/.claude/agents)").changed() {
        action = Some(SettingsAction::ReloadAgents);
    }
    ui.add_space(6.0);
    ui.separator();
    if agents.is_empty() {
        ui.label(RichText::new("No agents found yet.").weak());
    }
    for agent in agents {
        ui.horizontal(|ui| {
            ui.label(RichText::new(agent.mention()).monospace().strong());
            ui.label(&agent.name);
            ui.label(RichText::new(format!("· {}", agent.engine())).size(SMALL_SIZE).weak());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Open").on_hover_text(agent.path.display().to_string()).clicked() {
                    action = Some(SettingsAction::OpenFile(agent.path.clone()));
                }
            });
        });
        ui.add(egui::Label::new(RichText::new(&agent.description).size(SMALL_SIZE).weak()).wrap());
        if !agent.skills.is_empty() {
            let skills = format!("Skills: {}", agent.skills.join(", "));
            ui.label(RichText::new(skills).size(SMALL_SIZE).weak());
        }
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button("Reload").clicked() {
            action = Some(SettingsAction::ReloadAgents);
        }
        if ui
            .button("Create example agent")
            .on_hover_text("Adds an \"editor\" agent (@ai-editor) to the folder")
            .clicked()
        {
            action = Some(SettingsAction::CreateExampleAgent);
        }
    });
    action
}

/// Terminals: what they are, whether tmux was found, and how to install it
/// when it is missing (Terminals stay off until it is there).
fn terminals_section(ui: &mut Ui, terminals: TerminalsInfo) -> Option<SettingsAction> {
    let TerminalsInfo { found, recipe, install } = terminals;
    let visuals = ui.visuals().clone();
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Terminals").strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(path) = found {
                let (check, _) = ui.allocate_exact_size(vec2(CHECK_SIZE, CHECK_SIZE), Sense::hover());
                paint_check(ui.painter(), check.center(), visuals.hyperlink_color);
                let status = ui.label(RichText::new("Installed").size(STATUS_SIZE).weak());
                status.on_hover_text(RichText::new(path.display().to_string()).monospace());
            } else {
                ui.label(RichText::new("Not found").size(STATUS_SIZE).color(visuals.warn_fg_color));
            }
        });
    });
    ui.add(
        egui::Label::new(
            RichText::new(
                "Shells and agents (claude, codex) running in tmux sessions, listed under your files. \
                 They keep running when Rubrique closes, and you can hand one a step with @tmux-name.",
            )
            .weak(),
        )
        .wrap(),
    );
    let small = |text: &str| RichText::new(text).size(SMALL_SIZE).weak();
    if let Some(path) = found {
        path_box(ui, path);
        ui.label(small("Start one with New → Terminal in the file list, or open a .tmux file."));
        return None;
    }
    let message = "tmux is not installed, or not on PATH. Install it to turn on Terminals.";
    match recipe {
        Some(recipe) => {
            if let Some(clicked) = prompt(ui, message, recipe, install) {
                action = Some(SettingsAction::Install(clicked));
            }
            if recipe.run.is_some() {
                ui.label(small("Terminals turn on as soon as the install finishes, no restart."));
            } else {
                ui.label(small(
                    "Run the command in a terminal (it may ask for your password), then click Check again.",
                ));
            }
        }
        None => {
            ui.label(RichText::new(message).size(SMALL_SIZE).color(visuals.warn_fg_color));
        }
    }
    if ui.button("Check again").on_hover_text("Look for tmux again").clicked() {
        action = Some(SettingsAction::CheckTerminals);
    }
    action
}

/// Voice memos: where they go, dropped files, and transcription.
fn voice_section(ui: &mut Ui, voice: VoiceInfo) -> Option<SettingsAction> {
    let VoiceInfo { settings, found, recipe, model_recipe, install } = voice;
    let visuals = ui.visuals().clone();
    let mut action = None;
    ui.label(RichText::new("Memos folder").strong());
    ui.label(RichText::new("Recordings and dropped audio go here: a folder next to the note, or a full path.").weak());
    field_box(ui, &visuals, false, |ui| {
        let edit = egui::TextEdit::singleline(&mut settings.folder)
            .font(FontId::monospace(12.0))
            .frame(egui::Frame::NONE)
            .hint_text("next to the note")
            .desired_width(f32::INFINITY);
        ui.add(edit);
    });
    ui.checkbox(&mut settings.copy_dropped, "Copy audio from other folders into it")
        .on_hover_text("Off: link the file where it is");
    if cfg!(target_os = "macos") {
        ui.label(RichText::new("macOS asks once for the microphone, the first time you record.").weak());
    }

    ui.add_space(6.0);
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Transcription").strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(path) = found {
                let status = ui.label(RichText::new("Installed").size(STATUS_SIZE).weak());
                let (check, _) = ui.allocate_exact_size(vec2(CHECK_SIZE, CHECK_SIZE), Sense::hover());
                paint_check(ui.painter(), check.center(), visuals.hyperlink_color);
                status.on_hover_text(RichText::new(path.display().to_string()).monospace());
            } else {
                ui.label(RichText::new("Not found").size(STATUS_SIZE).color(visuals.warn_fg_color));
            }
            if settings.is_custom() && ui.small_button("Reset").on_hover_text(crate::voice::DEFAULT_COMMAND).clicked() {
                settings.reset_command();
                action = Some(SettingsAction::VoiceCommandChanged);
            }
        });
    });
    ui.label(
        RichText::new("Runs on this computer. {audio} becomes the memo file; the text goes under the memo.").weak(),
    );
    let problem = settings.check_command();
    field_box(ui, &visuals, problem.is_some(), |ui| {
        let edit = egui::TextEdit::singleline(&mut settings.command)
            .font(FontId::monospace(12.0))
            .frame(egui::Frame::NONE)
            .hint_text("command with {audio}")
            .desired_width(f32::INFINITY);
        if ui.add(edit).changed() {
            action = Some(SettingsAction::VoiceCommandChanged);
        }
    });
    let small = |text: String, warn: bool| {
        let color = if warn { visuals.warn_fg_color } else { visuals.weak_text_color() };
        RichText::new(text).size(SMALL_SIZE).color(color)
    };
    if let Some(problem) = problem {
        ui.label(small(problem.message().to_string(), true));
    } else if let Some(recipe) = recipe.filter(|_| found.is_none()) {
        let message = format!("{} is not installed, or not on PATH.", recipe.name);
        if let Some(clicked) = prompt(ui, &message, recipe, install) {
            action = Some(SettingsAction::Install(clicked));
        }
    } else if found.is_none() {
        ui.label(small(
            "Install whisper.cpp (brew install whisper-cpp, or the whisper.cpp package on Linux), \
             or use another tool, like yap transcribe {audio} on macOS 26."
                .to_string(),
            false,
        ));
    }
    if let Some(recipe) = model_recipe {
        let model = settings.model_path().map(|model| home_relative(&model)).unwrap_or_default();
        let message = format!("Model not found: {model}");
        if let Some(clicked) = prompt(ui, &message, recipe, install) {
            action = Some(SettingsAction::Install(clicked));
        }
    } else if settings.model_missing()
        && let Some(model) = settings.model_path()
    {
        ui.label(small(format!("Model not found: {}", home_relative(&model)), true));
        if let Some(download) = settings.model_download_command() {
            ui.label(small("Download it (466 MB) in a terminal:".to_string(), false));
            if ui.button("Copy download command").on_hover_text(download.as_str()).clicked() {
                action = Some(SettingsAction::Copy(download));
            }
        }
    }
    ui.checkbox(&mut settings.auto_transcribe, "Transcribe after recording");
    action
}

/// A text field's outlined box; a warning border when `warn`.
fn field_box(ui: &mut Ui, visuals: &egui::Visuals, warn: bool, add: impl FnOnce(&mut Ui)) {
    let border = if warn { Stroke::new(1.0, visuals.warn_fg_color) } else { visuals.widgets.noninteractive.bg_stroke };
    egui::Frame::new()
        .fill(visuals.text_edit_bg_color())
        .stroke(border)
        .corner_radius(PREVIEW_RADIUS)
        .inner_margin(egui::Margin::symmetric(7, 4))
        .show(ui, add);
}

/// A folder path in a small outlined box, home folder shown as `~`.
pub fn path_box(ui: &mut Ui, folder: &Path) {
    let path = home_relative(folder);
    egui::Frame::new()
        .fill(ui.visuals().text_edit_bg_color())
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(PREVIEW_RADIUS)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(path).monospace().size(12.0));
        });
}

/// One card for "System" and one per theme, two per row. Returns the clicked choice.
fn theme_cards(ui: &mut Ui, themes: &Themes, current: &ThemeChoice) -> Option<ThemeChoice> {
    let light = themes.for_base(Base::Light);
    let dark = themes.for_base(Base::Dark);
    let mut cards = vec![(ThemeChoice::System, "System".to_string(), CardLook::Split(&light, &dark))];
    cards.extend(
        themes
            .list
            .iter()
            .map(|theme| (ThemeChoice::Named(theme.id.clone()), theme.name.clone(), CardLook::One(theme))),
    );
    // A saved theme that no longer exists shows as System, like the app uses it.
    let selected = match current {
        ThemeChoice::Named(id) if themes.find(id).is_some() => current.clone(),
        _ => ThemeChoice::System,
    };
    let card_width = ((ui.available_width() - CARD_GAP) / 2.0).floor();
    let mut picked = None;
    for row in cards.chunks(2) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = CARD_GAP;
            for (choice, name, look) in row {
                if theme_card(ui, card_width, name, look, *choice == selected).clicked() {
                    picked = Some(choice.clone());
                }
            }
        });
    }
    picked
}

/// One card per code colors choice, side by side. Returns the clicked choice.
fn code_color_cards(ui: &mut Ui, current: CodeColors) -> Option<CodeColors> {
    let card_width = ((ui.available_width() - CARD_GAP) / 2.0).floor();
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = CARD_GAP;
        for choice in CodeColors::ALL {
            if theme_card(ui, card_width, choice.name(), &CardLook::Code(choice), choice == current).clicked() {
                picked = Some(choice);
            }
        }
    });
    picked
}

/// What a card's preview shows.
enum CardLook<'a> {
    One(&'a Theme),
    /// Light on the left, dark on the right.
    Split(&'a Theme, &'a Theme),
    /// A few lines of code in these code colors, on the current theme.
    Code(CodeColors),
}

/// A clickable card: a small picture of the theme (or code colors) and its name.
fn theme_card(ui: &mut Ui, width: f32, name: &str, look: &CardLook, selected: bool) -> Response {
    let size = vec2(width, CARD_PADDING * 3.0 + PREVIEW_HEIGHT + NAME_ROW_HEIGHT - CARD_PADDING);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::selected(egui::Role::RadioButton, true, selected, name));
    let visuals = ui.visuals();
    let accent = visuals.hyperlink_color;
    let ring = if selected {
        Stroke::new(2.0, accent)
    } else if response.hovered() {
        Stroke::new(1.0, visuals.weak_text_color())
    } else {
        visuals.widgets.noninteractive.bg_stroke
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CARD_RADIUS, visuals.text_edit_bg_color());
    painter.rect_stroke(rect, CARD_RADIUS, ring, StrokeKind::Inside);

    let preview = Rect::from_min_size(
        rect.min + vec2(CARD_PADDING, CARD_PADDING),
        vec2(width - 2.0 * CARD_PADDING, PREVIEW_HEIGHT),
    );
    match look {
        CardLook::One(theme) => paint_preview(painter, preview, &theme.palette, true),
        CardLook::Split(left, right) => {
            let (left_rect, right_rect) = preview.split_left_right_at_fraction(0.5);
            paint_preview(painter, left_rect, &left.palette, true);
            paint_preview(painter, right_rect, &right.palette, false);
        }
        CardLook::Code(choice) => paint_code_preview(ui, preview, *choice),
    }
    painter.rect_stroke(preview, 0, visuals.widgets.noninteractive.bg_stroke, StrokeKind::Inside);

    let name_y = preview.bottom() + NAME_ROW_HEIGHT / 2.0 + 2.0;
    let name_color = if selected { visuals.strong_text_color() } else { visuals.text_color() };
    painter.text(pos2(preview.left() + 2.0, name_y), Align2::LEFT_CENTER, name, FontId::proportional(14.0), name_color);
    if selected {
        paint_check(painter, pos2(preview.right() - CHECK_SIZE / 2.0 - 2.0, name_y), accent);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A tiny window: an explorer strip (when `sidebar`) and a page with text bars.
fn paint_preview(painter: &egui::Painter, rect: Rect, palette: &Palette, sidebar: bool) {
    let page = if sidebar {
        let (strip, page) = rect.split_left_right_at_fraction(PREVIEW_SIDEBAR_SHARE);
        painter.rect_filled(strip, 0, color(palette.sidebar));
        page
    } else {
        rect
    };
    painter.rect_filled(page, 0, color(palette.background));
    let mut y = page.top() + PREVIEW_PADDING;
    let inner_width = page.width() - 2.0 * PREVIEW_PADDING;
    for (share, height, kind) in PREVIEW_BARS {
        let bar_color = match kind {
            BarKind::Accent => palette.accent,
            BarKind::Text => palette.text,
            BarKind::Weak => palette.text_weak,
        };
        let bar = Rect::from_min_size(pos2(page.left() + PREVIEW_PADDING, y), vec2(inner_width * share, height));
        painter.rect_filled(bar, height / 2.0, color(bar_color));
        y += height + PREVIEW_BAR_GAP;
    }
}

/// Python colored with `choice` on the current theme's page color, clipped to `rect`.
fn paint_code_preview(ui: &Ui, rect: Rect, choice: CodeColors) {
    let style =
        HighlightStyle::with_code_colors(ui, choice).with_size(CODE_PREVIEW_FONT_SIZE, CODE_PREVIEW_LINE_HEIGHT);
    let job = highlighted(ui.ctx(), CODE_PREVIEW_TEXT, &style, Some(Syntax::Python));
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0, ui.visuals().text_edit_bg_color());
    let galley = painter.layout_job(job);
    let top = (rect.height() - galley.size().y) / 2.0;
    painter.galley(rect.min + vec2(PREVIEW_PADDING, top), galley, ui.visuals().text_color());
}

/// A check mark centered on `center`.
fn paint_check(painter: &egui::Painter, center: egui::Pos2, color: Color32) {
    let box_rect = Rect::from_center_size(center, vec2(CHECK_SIZE, CHECK_SIZE));
    let points = [(0.16, 0.52), (0.4, 0.76), (0.86, 0.26)].map(|(x, y)| box_rect.lerp_inside(vec2(x, y)));
    painter.line(points.to_vec(), Stroke::new(1.7, color));
}

/// `folder` with the home folder shown as `~`, to keep it short.
pub fn home_relative(folder: &Path) -> String {
    if crate::remote::is_mirror(folder) {
        return format!("local copy of {}", crate::remote::display(folder));
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match home.as_deref().and_then(|home| folder.strip_prefix(home).ok()) {
        Some(rest) => Path::new("~").join(rest).display().to_string(),
        None => folder.display().to_string(),
    }
}

fn color(Rgb(red, green, blue): Rgb) -> Color32 {
    Color32::from_rgb(red, green, blue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_app_window_gets_the_minimum_width_and_a_share_of_the_height() {
        assert_eq!(window_size_for(vec2(1100.0, 720.0)), vec2(780.0, 576.0));
    }

    #[test]
    fn big_app_window_gets_a_share_up_to_the_maximum() {
        assert_eq!(window_size_for(vec2(1600.0, 1000.0)), vec2(1120.0, 800.0));
        assert_eq!(window_size_for(vec2(3840.0, 2160.0)), WINDOW_MAX_SIZE);
    }

    #[test]
    fn small_app_window_keeps_the_margin() {
        assert_eq!(window_size_for(vec2(700.0, 500.0)), vec2(660.0, 460.0));
    }

    #[test]
    fn tiny_app_window_keeps_room_for_the_group_list() {
        assert_eq!(window_size_for(vec2(200.0, 100.0)), vec2(NAV_WIDTH * 2.0, HEADER_HEIGHT * 4.0));
    }
}
