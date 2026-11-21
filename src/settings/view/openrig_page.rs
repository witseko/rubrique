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
    /// The typed host is valid and not in the list yet.
    pub addable: bool,
    /// Enter was pressed in the field.
    pub entered: bool,
}

/// The `OpenRig` group: the About card with the logo and website, the
/// switch, then (when on) each remote rig with Check and Remove, and a field
/// to add one.
pub(super) fn openrig_section(ui: &mut Ui, info: OpenRigInfo) -> Option<SettingsAction> {
    let OpenRigInfo { enabled, rigs, new_host, known_hosts } = info;
    let mut action = None;
    about_card(ui);
    ui.add_space(6.0);
    if ui.checkbox(enabled, "Use OpenRig").changed() {
        action = Some(SettingsAction::OpenRigToggled);
    }
    weak_text(
        ui,
        "Seats get their own section in the explorer, and Send to OpenRig in a document's Tasks panel hands \
         them a step. Off, seats show as plain terminals.",
    );
    if !*enabled {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Turn on OpenRig to list remote rigs here. New to it?").weak());
            ui.hyperlink_to("Read how to start a team", WEBSITE);
        });
        return action;
    }
    ui.add_space(12.0);
    ui.label(RichText::new("Remote rigs").strong());
    weak_text(
        ui,
        "Rubrique logs in with your ssh (keys and ~/.ssh/config work, passwords do not) and runs rig there, \
         so this computer needs no OpenRig CLI.",
    );
    if rigs.is_empty() {
        ui.label(RichText::new("No remote rigs yet.").size(SMALL_SIZE).weak());
    }
    for row in &rigs {
        ui.horizontal(|ui| {
            status_dot(ui, &row.check);
            ui.label(RichText::new(row.host).monospace().strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Remove").on_hover_text("Remove this rig from the list").clicked() {
                    action = Some(SettingsAction::RemoveRemoteRig(row.host.to_owned()));
                }
                let checking = matches!(row.check, RigCheck::Checking);
                let check = ui.add_enabled(!checking, egui::Button::new("Check"));
                if check.on_hover_text("Log in and list the OpenRig seats there").clicked() {
                    action = Some(SettingsAction::CheckRemoteRig(row.host.to_owned()));
                }
            });
        });
        check_line(ui, &row.check);
    }
    ui.add_space(6.0);
    ui.label("Add a remote rig");
    let hosts: Vec<&str> = rigs.iter().map(|row| row.host).collect();
    let field = rig_host_field(ui, new_host, known_hosts, &hosts);
    let add = ui.add_enabled(field.addable, egui::Button::new("Add rig")).clicked();
    if field.addable && (add || field.entered) {
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
        .corner_radius(10)
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
                    ui.hyperlink_to("openrig.dev ↗", WEBSITE).on_hover_text(WEBSITE);
                });
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
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(small("Checking…").weak());
            });
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

/// The SSH host field for a new remote rig, with a button for each host in
/// `~/.ssh/config` that is not in `rigs` yet, and a hint when the typed host
/// cannot be added. The caller draws its own Add button.
pub fn rig_host_field(ui: &mut Ui, host: &mut String, known_hosts: &[String], rigs: &[&str]) -> HostField {
    let field =
        ui.add(egui::TextEdit::singleline(host).hint_text("server or user@server").desired_width(f32::INFINITY));
    let offered: Vec<&String> = known_hosts.iter().filter(|name| !rigs.contains(&name.as_str())).collect();
    if !offered.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("From ~/.ssh/config:").size(SMALL_SIZE).weak());
            for name in offered {
                if ui.small_button(name).clicked() {
                    host.clone_from(name);
                }
            }
        });
    }
    let typed = host.trim();
    let valid = is_valid_host(typed);
    let listed = rigs.contains(&typed);
    if !typed.is_empty() && !valid {
        ui.weak("Use a host name, or user@host");
    } else if listed {
        ui.weak("This rig is in the list already.");
    }
    let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    HostField { addable: valid && !listed, entered }
}
