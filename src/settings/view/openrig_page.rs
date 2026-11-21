//! The `OpenRig` settings group: the remote rigs, and the SSH host field it
//! shares with Send to `OpenRig`.

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Ui, vec2};

use super::{SMALL_SIZE, SettingsAction};
use crate::icons::paint_openrig_mark;
use crate::remote::mirror::is_valid_host;

/// `OpenRig`'s website (the `homepage` of its npm package).
const WEBSITE: &str = "https://openrig.dev";
/// Side of the logo tile in the About card, and how much larger than the
/// explorer icon the mark is drawn in it.
const LOGO_SIZE: f32 = 44.0;
const LOGO_SCALE: f32 = 2.4;
/// Status dot of a remote rig: green for ready, like an idle agent.
const READY_COLOR: Color32 = Color32::from_rgb(0x34, 0xa8, 0x53);
const DOT_RADIUS: f32 = 4.0;

/// The remote rigs, their last check and the host being typed.
#[derive(Debug)]
pub struct OpenRigInfo<'a> {
    /// `OpenRig` is turned on (the switch at the top of the group).
    pub enabled: &'a mut bool,
    pub rigs: Vec<RemoteRigRow<'a>>,
    /// The text in the "Add a remote rig" field.
    pub new_host: &'a mut String,
    /// The host names in `~/.ssh/config`, offered as buttons.
    pub known_hosts: &'a [String],
}

/// One remote rig and what its last check found.
#[derive(Debug)]
pub struct RemoteRigRow<'a> {
    pub host: &'a str,
    pub check: RigCheck<'a>,
}

/// The state of a remote rig's check in this session.
#[derive(Debug)]
pub enum RigCheck<'a> {
    NotChecked,
    Checking,
    /// Seats were found; the text names them.
    Ready(&'a str),
    /// What is missing: the SSH login, rig on the host, or a running team.
    Problem(&'a str),
}

/// What the user did in the SSH host field.
#[derive(Debug, Clone, Copy)]
pub struct HostField {
    /// Add the typed host: "Add rig" was clicked, or Enter pressed in the
    /// field, with a valid host that is not in the list yet.
    pub add: bool,
}

/// Space between the blocks of the group (card, switch, rigs, add field).
const BLOCK_GAP: f32 = 16.0;
/// Indent of the switch's description, so it lines up with the label.
const CHECKBOX_INDENT: f32 = 26.0;
/// Corner radius of the card, the rig list and the off hint.
const BOX_RADIUS: u8 = 10;
/// Padding inside a row of the rig list.
const ROW_MARGIN: egui::Margin = egui::Margin { left: 14, right: 14, top: 10, bottom: 10 };
/// Room kept at a rig row's right for "Check" and "Remove".
const ROW_BUTTONS_WIDTH: f32 = 150.0;
/// Height of a rig row's content (the host and its status line), so the
/// dot and the buttons center on it.
const ROW_HEIGHT: f32 = 40.0;
/// Height of the host field and "Add rig", corner radius and padding of the field.
const FIELD_HEIGHT: f32 = 32.0;
const FIELD_RADIUS: u8 = 7;
const FIELD_MARGIN: egui::Margin = egui::Margin { left: 10, right: 10, top: 5, bottom: 5 };

/// The `OpenRig` group: the About card with the logo and website, the
/// switch, then (when on) each remote rig with Check and Remove, and a field
/// to add one.
pub(super) fn openrig_section(ui: &mut Ui, info: OpenRigInfo) -> Option<SettingsAction> {
    let OpenRigInfo { enabled, rigs, new_host, known_hosts } = info;
    let mut action = None;
    about_card(ui);
    ui.add_space(BLOCK_GAP);
    if ui.checkbox(enabled, RichText::new("Use OpenRig").strong()).changed() {
        action = Some(SettingsAction::OpenRigToggled);
    }
    ui.horizontal(|ui| {
        ui.add_space(CHECKBOX_INDENT);
        weak_text(
            ui,
            "Seats get their own section in the explorer, and Send to OpenRig in a document's Tasks panel \
             hands them a step. Off, seats show as plain terminals.",
        );
    });
    ui.add_space(BLOCK_GAP);
    if !*enabled {
        off_hint(ui);
        return action;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Remote rigs").strong());
        ui.label(
            RichText::new("Logged in with your ssh: keys and ~/.ssh/config work, passwords do not.")
                .size(SMALL_SIZE)
                .weak(),
        );
    });
    if rigs.is_empty() {
        ui.label(RichText::new("No remote rigs yet.").size(SMALL_SIZE).weak());
    } else if let Some(clicked) = rig_list(ui, &rigs) {
        action = Some(clicked);
    }
    ui.add_space(BLOCK_GAP);
    ui.label(RichText::new("Add a remote rig").strong());
    let hosts: Vec<&str> = rigs.iter().map(|row| row.host).collect();
    if rig_host_field(ui, new_host, known_hosts, &hosts).add {
        action = Some(SettingsAction::AddRemoteRig(new_host.trim().to_owned()));
    }
    action
}

/// The card at the top: the `OpenRig` mark, its name and what it is, and a
/// link to its website.
fn about_card(ui: &mut Ui) {
    let visuals = ui.visuals().clone();
    egui::Frame::new()
        .fill(visuals.panel_fill)
        .stroke(visuals.widgets.noninteractive.bg_stroke)
        .corner_radius(BOX_RADIUS)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (tile, _) = ui.allocate_exact_size(vec2(LOGO_SIZE, LOGO_SIZE), Sense::hover());
                ui.painter().rect_filled(tile, 10, visuals.faint_bg_color);
                let center = tile.center() + vec2(0.0, 0.5 * LOGO_SCALE);
                paint_openrig_mark(ui.painter(), center, LOGO_SCALE, visuals.text_color(), visuals.hyperlink_color);
                ui.vertical(|ui| {
                    ui.label(RichText::new("OpenRig").size(16.0).strong());
                    ui.label(
                        RichText::new("Teams of coding agents (Claude Code, Codex) working together on your projects.")
                            .weak(),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let link = RichText::new("openrig.dev ↗").color(visuals.hyperlink_color);
                    let button = egui::Button::new(link)
                        .fill(Color32::TRANSPARENT)
                        .stroke(visuals.widgets.noninteractive.bg_stroke)
                        .min_size(vec2(0.0, FIELD_HEIGHT));
                    if ui.add(button).on_hover_text(WEBSITE).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(WEBSITE));
                    }
                });
            });
        });
}

/// The remote rigs in a bordered box, one row each with a line between:
/// the status dot, the host with its last check under it, then "Check"
/// and "Remove". Returns what was clicked.
fn rig_list(ui: &mut Ui, rigs: &[RemoteRigRow]) -> Option<SettingsAction> {
    let mut action = None;
    let line = ui.visuals().widgets.noninteractive.bg_stroke;
    egui::Frame::new().stroke(line).corner_radius(BOX_RADIUS).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (index, row) in rigs.iter().enumerate() {
            if index > 0 {
                let y = ui.cursor().top();
                ui.painter().hline(ui.max_rect().x_range(), y, line);
            }
            egui::Frame::new().inner_margin(ROW_MARGIN).show(ui, |ui| {
                let size = vec2(ui.available_width(), ROW_HEIGHT);
                ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    status_dot(ui, &row.check);
                    let text_width = (ui.available_width() - ROW_BUTTONS_WIDTH).max(0.0);
                    let column = Layout::top_down(Align::Min).with_main_align(Align::Center);
                    ui.allocate_ui_with_layout(vec2(text_width, ROW_HEIGHT), column, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new(row.host).monospace().strong());
                        check_line(ui, &row.check);
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let remove = egui::Button::new(RichText::new("Remove").weak()).frame(false);
                        if ui.add(remove).on_hover_text("Remove this rig from the list").clicked() {
                            action = Some(SettingsAction::RemoveRemoteRig(row.host.to_owned()));
                        }
                        let checking = matches!(row.check, RigCheck::Checking);
                        let check = ui.add_enabled(!checking, egui::Button::new("Check"));
                        if check.on_hover_text("Log in and list the OpenRig seats there").clicked() {
                            action = Some(SettingsAction::CheckRemoteRig(row.host.to_owned()));
                        }
                    });
                });
            });
        }
    });
    action
}

/// With `OpenRig` off: a framed line saying how to list remote rigs, with a
/// link to its website.
fn off_hint(ui: &mut Ui) {
    let line = ui.visuals().widgets.noninteractive.bg_stroke;
    egui::Frame::new().stroke(line).corner_radius(BOX_RADIUS).inner_margin(14).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Turn on OpenRig to list remote rigs here. New to it?").weak());
            ui.hyperlink_to("Read how to start a team", WEBSITE);
        });
    });
}

/// A wrapped paragraph in the weak text color.
fn weak_text(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).weak()).wrap());
}

/// A dot before a rig's name in the color of its last check: green when it
/// is ready, the accent while checking, the warning color for a problem.
fn status_dot(ui: &mut Ui, check: &RigCheck) {
    let visuals = ui.visuals();
    let color = match check {
        RigCheck::NotChecked => visuals.weak_text_color(),
        RigCheck::Checking => visuals.hyperlink_color,
        RigCheck::Ready(_) => READY_COLOR,
        RigCheck::Problem(_) => visuals.warn_fg_color,
    };
    let (rect, _) = ui.allocate_exact_size(vec2(DOT_RADIUS * 2.0, DOT_RADIUS * 2.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), DOT_RADIUS, color);
}

/// The last check's result under a rig's name.
fn check_line(ui: &mut Ui, check: &RigCheck) {
    let small = |text: &str| RichText::new(text).size(SMALL_SIZE);
    match check {
        RigCheck::NotChecked => {
            ui.label(small("Not checked yet.").weak());
        }
        RigCheck::Checking => {
            ui.label(small("Checking…").weak());
        }
        RigCheck::Ready(text) => {
            ui.label(small(text).weak());
        }
        RigCheck::Problem(text) => {
            let color = ui.visuals().warn_fg_color;
            ui.add(egui::Label::new(small(text).color(color)).wrap());
        }
    }
}

/// The SSH host field for a new remote rig with "Add rig" beside it, then a
/// button for each host in `~/.ssh/config` that is not in `rigs` yet, and a
/// hint when the typed host cannot be added.
pub fn rig_host_field(ui: &mut Ui, host: &mut String, known_hosts: &[String], rigs: &[&str]) -> HostField {
    let addable = |host: &str| is_valid_host(host.trim()) && !rigs.contains(&host.trim());
    let enabled = addable(host);
    let row = vec2(ui.available_width(), FIELD_HEIGHT);
    let (field, clicked) = ui
        .allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let visuals = ui.visuals().clone();
            let label = RichText::new("Add rig").strong().color(visuals.panel_fill);
            let button = egui::Button::new(label)
                .fill(visuals.hyperlink_color)
                .corner_radius(FIELD_RADIUS)
                .min_size(vec2(0.0, FIELD_HEIGHT));
            let clicked = ui.add_enabled(enabled, button).clicked();
            // The field draws its own box: background, border and rounded corners.
            let field = egui::Frame::new()
                .fill(visuals.faint_bg_color)
                .stroke(visuals.widgets.noninteractive.bg_stroke)
                .corner_radius(FIELD_RADIUS)
                .inner_margin(FIELD_MARGIN)
                .show(ui, |ui| {
                    let edit = egui::TextEdit::singleline(host)
                        .hint_text(RichText::new("server or user@server").monospace())
                        .font(egui::TextStyle::Monospace)
                        .frame(egui::Frame::NONE)
                        .align(egui::Align2::LEFT_CENTER)
                        .min_size(vec2(0.0, FIELD_HEIGHT - f32::from(FIELD_MARGIN.top + FIELD_MARGIN.bottom)))
                        .desired_width(ui.available_width());
                    ui.add(edit)
                })
                .inner;
            (field, clicked)
        })
        .inner;
    let offered: Vec<&String> = known_hosts.iter().filter(|name| !rigs.contains(&name.as_str())).collect();
    if !offered.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("From ~/.ssh/config:").size(SMALL_SIZE).weak());
            let line = ui.visuals().widgets.noninteractive.bg_stroke;
            for name in offered {
                let chip = egui::Button::new(RichText::new(name).monospace().size(SMALL_SIZE))
                    .fill(Color32::TRANSPARENT)
                    .stroke(line)
                    .corner_radius(12);
                if ui.add(chip).clicked() {
                    host.clone_from(name);
                }
            }
        });
    }
    let typed = host.trim();
    if !typed.is_empty() && !is_valid_host(typed) {
        ui.weak("Use a host name, or user@host");
    } else if rigs.contains(&typed) {
        ui.weak("This rig is in the list already.");
    }
    let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    HostField { add: addable(host) && (clicked || entered) }
}
