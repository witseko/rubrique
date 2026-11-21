//! The `OpenRig` section of the explorer: the seats of the rigs on this
//! computer and on the remote rigs, one group per rig with its pods, docked
//! at the bottom under Terminals.

use eframe::egui::{self, RichText, Ui};

use super::section::{SECTION_HEADER_HEIGHT, empty_space_double_clicked, section_header};
use super::terminals::{SECTION_GAP, header_details, terminal_group};
use super::view::{ExplorerAction, HEADER_BUTTON_SIZE, InlineRename, SessionEdit, TreeContext};
use crate::icons::{Icon, small_icon_button};
use crate::settings::layout::LayoutSettings;
use crate::terminal::monitor::TerminalRow;

/// Height of the open section until the user drags its edge.
const DEFAULT_HEIGHT: f32 = 160.0;
/// The open section keeps room for its header and about two rows.
const MIN_HEIGHT: f32 = 90.0;

/// The section as a panel docked at the very bottom of the explorer, which
/// the user can resize by its top edge, or only its header while folded.
/// Nothing when tmux is not installed, or no rig has seats and no remote
/// rig was added.
pub(super) fn show_docked_openrig(
    ui: &mut Ui,
    context: &TreeContext,
    layout: &mut LayoutSettings,
    rename: &mut Option<InlineRename<SessionEdit>>,
    action: &mut Option<ExplorerAction>,
) {
    if !shows(context) {
        return;
    }
    let frame = egui::Frame::new().inner_margin(egui::Margin { top: SECTION_GAP, ..egui::Margin::ZERO });
    // Folded and open use their own ids, so folding keeps the dragged height.
    let panel = if layout.openrig_collapsed {
        egui::Panel::bottom("openrig-folded").exact_size(SECTION_HEADER_HEIGHT + f32::from(SECTION_GAP))
    } else {
        egui::Panel::bottom("openrig")
            .resizable(true)
            .default_size(DEFAULT_HEIGHT)
            .size_range(MIN_HEIGHT..=f32::INFINITY)
    };
    panel.frame(frame).show(ui, |ui| openrig_section(ui, context, layout, rename, action));
}

/// The section filling the rest of the explorer's height (when Files and
/// Terminals are folded). Nothing when [`show_docked_openrig`] shows nothing.
pub(super) fn show_openrig_section(
    ui: &mut Ui,
    context: &TreeContext,
    layout: &mut LayoutSettings,
    rename: &mut Option<InlineRename<SessionEdit>>,
    action: &mut Option<ExplorerAction>,
) {
    if shows(context) {
        openrig_section(ui, context, layout, rename, action);
    }
}

/// The section shows when tmux is installed and some rig has seats or a
/// remote rig was added.
fn shows(context: &TreeContext) -> bool {
    context.openrig && context.terminals.is_some() && (!context.rig_groups.is_empty() || context.remote_rigs)
}

/// The "OPENRIG" header (with a seat count, status dots while folded, and
/// "Remote rigs"), then one group per rig in its own scroll area.
fn openrig_section(
    ui: &mut Ui,
    context: &TreeContext,
    layout: &mut LayoutSettings,
    rename: &mut Option<InlineRename<SessionEdit>>,
    action: &mut Option<ExplorerAction>,
) {
    let all_rows = context.terminals.unwrap_or_default();
    let seats: Vec<&TerminalRow> = context
        .rig_groups
        .iter()
        .flat_map(|rig| rig.children.iter().flat_map(|pod| pod.rows.iter()))
        .filter_map(|&index| all_rows.get(index))
        .collect();
    let expanded = !layout.openrig_collapsed;
    let clicks = section_header(
        ui,
        "OPENRIG",
        expanded,
        |ui| header_details(ui, &seats, expanded),
        |ui| {
            let tip = "Add, check or remove rigs on other computers (Settings)";
            if small_icon_button(ui, Icon::OpenRig, HEADER_BUTTON_SIZE, "Remote rigs", tip).clicked() {
                *action = Some(ExplorerAction::OpenRigSettings);
            }
        },
    );
    if clicks.background_double {
        layout.toggle_openrig_focus();
        return;
    }
    if clicks.title {
        layout.openrig_collapsed = expanded;
    }
    if !expanded {
        return;
    }
    if context.rig_groups.is_empty() {
        let hint = "No OpenRig seats running. Start a team, or check the remote rigs in Settings.";
        ui.add(egui::Label::new(RichText::new(hint).small().weak()).wrap());
        if empty_space_double_clicked(ui, "openrig-empty-space") {
            layout.toggle_openrig_focus();
        }
        return;
    }
    egui::ScrollArea::both().id_salt("openrig-list").auto_shrink([false, false]).show(ui, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        for rig in context.rig_groups {
            terminal_group(ui, rig, context, rename, action);
        }
        if empty_space_double_clicked(ui, "openrig-empty-space") {
            layout.toggle_openrig_focus();
        }
    });
}
