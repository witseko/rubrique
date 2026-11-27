//! Terminals in the explorer: the Terminals section (docked at the bottom
//! of the panel, foldable and resizable), and the rows of `.tmux` files. Each row shows the terminal icon, the
//! name, then a colored status dot with the agent's state and when the
//! session was opened.

use eframe::egui::text::LayoutJob;
use eframe::egui::{self, Align, Color32, FontSelection, RichText, Ui, Visuals};

use super::reorder::{ListDrag, draggable};
use super::section::{SECTION_HEADER_HEIGHT, empty_space_double_clicked, paint_fold_arrow, section_header};
use super::selection::FileRows;
use super::view::{
    ExplorerAction, HEADER_BUTTON_SIZE, ICON_GAP, InlineRename, NOTE_MARK_SIZE, SeatCommand, SessionEdit, TreeContext,
    file_click, file_menu, is_revealed, paint_root_bar, rename_row, scroll_if_revealed, tree_row,
};
use crate::explorer::tree::FsNode;
use crate::icons::{Icon, paint_icon, small_icon_button, small_icon_button_on};
use crate::settings::layout::LayoutSettings;
use crate::terminal::agent_sessions::AgentSession;
use crate::terminal::groups::{TerminalGroup, seat_of};
use crate::terminal::monitor::{TerminalRow, ago_label, opened_label};
use crate::terminal::status::TerminalStatus;
use crate::terminal::tmux::session_name_for_file;
use crate::theme::style::group_color;

/// Height of the open Terminals section until the user drags its edge.
const TERMINALS_DEFAULT_HEIGHT: f32 = 180.0;
/// The open section keeps room for its header and about two rows.
const TERMINALS_MIN_HEIGHT: f32 = 90.0;
/// Space above the Terminals and `OpenRig` headers, under the panel's dividing line.
pub(super) const SECTION_GAP: i8 = 4;
/// Status color of an idle agent: the green used for "all good".
const IDLE_COLOR: Color32 = Color32::from_rgb(0x34, 0xa8, 0x53);
/// One more space before the name than tree rows have: the terminal icon
/// is wider than a folder arrow and would touch the name otherwise.
const ICON_ROOM: &str = " ";
/// Spaces left after the name for the status dot to be painted in.
const DOT_ROOM: &str = "     ";
const DOT_RADIUS: f32 = 3.5;
/// Space between a rig's header row and its "Add seat" button.
const ADD_SEAT_GAP: f32 = 4.0;

/// The Terminals section docked at the bottom of the explorer: a panel the
/// user can resize by its top edge, or only its header while folded.
/// Nothing when tmux is not installed.
pub(super) fn show_docked_terminals(
    ui: &mut Ui,
    context: &TreeContext,
    layout: &mut LayoutSettings,
    rename: &mut Option<InlineRename<SessionEdit>>,
    action: &mut Option<ExplorerAction>,
) {
    if context.terminals.is_none() {
        return;
    }
    let collapsed = layout.terminals_collapsed;
    let frame = egui::Frame::new().inner_margin(egui::Margin { top: SECTION_GAP, ..egui::Margin::ZERO });
    // Folded and open use their own ids, so folding keeps the height the
    // user dragged the open section to.
    let panel = if collapsed {
        egui::Panel::bottom("terminals-folded").exact_size(SECTION_HEADER_HEIGHT + f32::from(SECTION_GAP))
    } else {
        egui::Panel::bottom("terminals")
            .resizable(true)
            .default_size(TERMINALS_DEFAULT_HEIGHT)
            .size_range(TERMINALS_MIN_HEIGHT..=f32::INFINITY)
    };
    panel.frame(frame).show(ui, |ui| show_terminals_section(ui, context, layout, rename, action));
}

/// The "TERMINALS" header (with a count, status dots while folded, and
/// "New terminal"), then one row per tmux session in its own scroll area:
/// in the order the user dragged them into, new ones on top. With more than
/// one group (folders, "Other", rigs) each group has a header and folds. `rename` is the name field for a new session (shown at the
/// top) or for a session being renamed in its row, if any.
/// Nothing when tmux is not installed.
pub(super) fn show_terminals_section(
    ui: &mut Ui,
    context: &TreeContext,
    layout: &mut LayoutSettings,
    rename: &mut Option<InlineRename<SessionEdit>>,
    action: &mut Option<ExplorerAction>,
) {
    let Some(all_rows) = context.terminals else {
        return;
    };
    // `OpenRig` seats have their own section.
    let rows: Vec<&TerminalRow> =
        all_rows.iter().filter(|row| !(context.openrig && seat_of(&row.info.name).is_some())).collect();
    let expanded = !layout.terminals_collapsed;
    let clicks = section_header(
        ui,
        "TERMINALS",
        expanded,
        |ui| header_details(ui, &rows, expanded),
        |ui, _| {
            let tip = "New terminal: a tmux session in the first folder";
            if small_icon_button(ui, Icon::Plus, HEADER_BUTTON_SIZE, "New terminal", tip).clicked() {
                *action = Some(ExplorerAction::NewTerminal);
            }
        },
    );
    if clicks.background_double {
        layout.toggle_terminals_focus();
        return;
    }
    if clicks.title {
        layout.terminals_collapsed = expanded;
    }
    if !expanded {
        return;
    }
    let naming_new = rename.as_ref().is_some_and(|rename| rename.target == SessionEdit::New);
    if rows.is_empty() && !naming_new {
        let hint = "No tmux sessions yet. Start one with +, or right-click a folder for a .tmux file.";
        ui.add(egui::Label::new(RichText::new(hint).small().weak()).wrap());
        if empty_space_double_clicked(ui, "terminals-empty-space") {
            layout.toggle_terminals_focus();
        }
        return;
    }
    egui::ScrollArea::both().id_salt("terminal-list").auto_shrink([false, false]).show(ui, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        if naming_new {
            rename_row(ui, rename);
        }
        match context.terminal_groups {
            // One folder and no rig: the plain list, without a header.
            [group] if group.children.is_empty() => group_rows(ui, group, context, rename, action),
            groups => {
                for group in groups {
                    terminal_group(ui, group, context, rename, action);
                }
            }
        }
        if empty_space_double_clicked(ui, "terminals-empty-space") {
            layout.toggle_terminals_focus();
        }
    });
}

/// One group of the list: its header, then (unless folded) its sessions
/// and its sub-groups (pods) indented under it.
pub(super) fn terminal_group(
    ui: &mut Ui,
    group: &TerminalGroup,
    context: &TreeContext,
    rename: &mut Option<InlineRename<SessionEdit>>,
    action: &mut Option<ExplorerAction>,
) {
    let folded = context.folded_terminal_groups.contains(&group.key);
    // Folder colors show only with several folders, as on the tabs.
    let color = group.root.filter(|_| context.several_roots).map(group_color);
    let header = group_header(ui, group, folded, color);
    if header.clicked() {
        *action = Some(ExplorerAction::ToggleTerminalGroup(group.key.clone()));
    }
    if is_rig_group(group) {
        add_seat_controls(ui, &header, group, context, action);
    }
    if folded {
        return;
    }
    ui.indent(("terminal-group", &group.key), |ui| {
        group_rows(ui, group, context, rename, action);
        for child in &group.children {
            terminal_group(ui, child, context, rename, action);
        }
    });
}

/// A rig's group in the `OpenRig` list (not one of its pods).
fn is_rig_group(group: &TerminalGroup) -> bool {
    group.key.strip_prefix("rig:").is_some_and(|rig| !rig.contains('/'))
}

/// "Add seat" for a rig: a + button just after its header row, and the
/// same item in the header's right-click menu.
fn add_seat_controls(
    ui: &Ui,
    header: &egui::Response,
    group: &TerminalGroup,
    context: &TreeContext,
    action: &mut Option<ExplorerAction>,
) {
    let size = header.rect.height().min(HEADER_BUTTON_SIZE);
    let rect = egui::Rect::from_min_size(
        egui::pos2(header.rect.right() + ADD_SEAT_GAP, header.rect.center().y - size / 2.0),
        egui::vec2(size, size),
    );
    let tip = "Add a seat to this rig";
    let plus = small_icon_button_on(ui, rect, header.id.with("add-seat"), Icon::Plus, "Add seat", tip);
    let mut clicked = plus.clicked();
    header.context_menu(|ui| {
        if ui.button("Add seat…").on_hover_text(tip).clicked() {
            clicked = true;
            ui.close();
        }
    });
    if clicked {
        *action = Some(ExplorerAction::AddSeat { seats: group_session_names(group, context) });
    }
}

/// The session names of a group's rows and its sub-groups' rows.
fn group_session_names(group: &TerminalGroup, context: &TreeContext) -> Vec<String> {
    let rows = context.terminals.unwrap_or_default();
    let mut names: Vec<String> =
        group.rows.iter().filter_map(|&index| rows.get(index)).map(|row| row.info.name.clone()).collect();
    for child in &group.children {
        names.extend(group_session_names(child, context));
    }
    names
}

/// A group's header row: the fold arrow, the title, the session count, and
/// for a folder a bar of its color.
fn group_header(ui: &mut Ui, group: &TerminalGroup, folded: bool, color: Option<Color32>) -> egui::Response {
    let mut job = LayoutJob::default();
    let style = ui.style();
    RichText::new(format!("{ICON_GAP}{}", group.title)).strong().append_to(
        &mut job,
        style,
        FontSelection::Default,
        Align::Center,
    );
    RichText::new(format!("  {}", group.session_count())).small().weak().append_to(
        &mut job,
        style,
        FontSelection::Default,
        Align::Center,
    );
    let response = tree_row(ui, false, job);
    let center = egui::pos2(
        response.rect.left() + ui.spacing().button_padding.x + NOTE_MARK_SIZE / 2.0,
        response.rect.center().y,
    );
    let arrow_color = ui.style().interact(&response).fg_stroke.color;
    paint_fold_arrow(ui.painter(), center, !folded, arrow_color);
    if let Some(color) = color {
        paint_root_bar(ui, &response, color);
    }
    let tip = if folded { "Show these terminals" } else { "Hide these terminals" };
    response.on_hover_text(tip)
}

/// The sessions of one group, which can be dragged up and down inside it.
fn group_rows(
    ui: &mut Ui,
    group: &TerminalGroup,
    context: &TreeContext,
    rename: &mut Option<InlineRename<SessionEdit>>,
    action: &mut Option<ExplorerAction>,
) {
    let Some(rows) = context.terminals else {
        return;
    };
    let group_rows: Vec<&TerminalRow> = group.rows.iter().filter_map(|&index| rows.get(index)).collect();
    let mut drag = ListDrag::default();
    for row in &group_rows {
        let top = ui.cursor().top();
        let response = terminal_row(ui, row, context, rename, action);
        drag.add(ui, top, response.as_ref());
    }
    if let Some((from, to)) = drag.finish(ui)
        && let Some(moved) = move_action(&group_rows, from, to)
    {
        *action = Some(moved);
    }
}

/// The drag of row `from` of a group to go before row `to` (the row count
/// for the end), as a move by session names.
fn move_action(group_rows: &[&TerminalRow], from: usize, to: usize) -> Option<ExplorerAction> {
    let name = group_rows.get(from)?.info.name.clone();
    let (anchor, after) = match group_rows.get(to) {
        Some(row) => (row, false),
        None => (group_rows.last()?, true),
    };
    Some(ExplorerAction::MoveTerminal { name, anchor: anchor.info.name.clone(), after })
}

/// One session in the list: a click shows it in a tab, a double click
/// renames it in place, a right click offers "Show in files", "Rename" and
/// "Exit terminal" (an `OpenRig` seat gets [`seat_menu`] instead, and keeps
/// its name), and a drag moves it up or down the list. Returns its row, or
/// `None` while it shows a name field instead.
fn terminal_row(
    ui: &mut Ui,
    row: &TerminalRow,
    context: &TreeContext,
    rename: &mut Option<InlineRename<SessionEdit>>,
    action: &mut Option<ExplorerAction>,
) -> Option<egui::Response> {
    let renaming = |rename: &InlineRename<SessionEdit>| rename.target == SessionEdit::Rename(row.info.name.clone());
    if rename.as_ref().is_some_and(renaming) {
        rename_row(ui, rename);
        return None;
    }
    let name = &row.info.name;
    let sessions = context.agent_history.map(|history| history.of(name)).unwrap_or_default();
    let expanded = context.expanded_terminals.contains(name);
    let selected = context.open_terminal == Some(name.as_str());
    let label = terminal_label(ui, name, Some(row));
    let response = draggable(&tree_row(ui, selected, label)).on_hover_text(terminal_tip(Some(row)));
    paint_status_dot(ui, &response, name, Some(&row.status));
    // A terminal that ran agent sessions folds them open and closed with an
    // arrow in place of its icon, like a folder.
    if sessions.is_empty() {
        paint_terminal_icon(ui, &response);
    } else if fold_arrow(ui, &response, expanded) {
        *action = Some(ExplorerAction::ToggleAgentSessions(name.clone()));
        return Some(response);
    }
    let is_seat = context.openrig && seat_of(name).is_some();
    if response.double_clicked() && !is_seat {
        *action = Some(ExplorerAction::RenameTerminal(row.info.name.clone()));
    } else if response.clicked() {
        *action = Some(ExplorerAction::OpenTerminal(row.info.name.clone()));
    }
    if is_seat {
        response.context_menu(|ui| seat_menu(ui, name, action));
    } else {
        terminal_menu(&response, name, action);
    }
    if expanded && !sessions.is_empty() {
        ui.indent(("agent-sessions", name), |ui| {
            for session in sessions {
                agent_session_row(ui, session, action);
            }
        });
    }
    Some(response)
}

/// The right-click menu of an `OpenRig` seat: send it a step, copy its
/// address or attach command, open it in the terminal app, change its
/// setup, or stop it.
fn seat_menu(ui: &mut Ui, name: &str, action: &mut Option<ExplorerAction>) {
    let items = [
        (
            "Send step to this seat…",
            "Open Send to OpenRig for the open document, with this seat chosen",
            SeatCommand::SendStep,
        ),
        ("Copy address", "Copy the seat's OpenRig address (seat@rig)", SeatCommand::CopyAddress),
        ("Copy attach command", "Copy the shell command that attaches to this seat", SeatCommand::CopyAttach),
        (
            "Open in Terminal app",
            "Attach to this seat in a window of the system's terminal app",
            SeatCommand::OpenInTerminalApp,
        ),
        ("Seat setup…", "Change the seat's model, reasoning effort, permissions or working folder", SeatCommand::Setup),
    ];
    for (label, tip, command) in items {
        if ui.button(label).on_hover_text(tip).clicked() {
            *action = Some(ExplorerAction::Seat(name.to_owned(), command));
            ui.close();
        }
    }
    ui.separator();
    if ui.button("Stop seat…").on_hover_text("End the seat's tmux session and the agent in it").clicked() {
        *action = Some(ExplorerAction::Seat(name.to_owned(), SeatCommand::Stop));
        ui.close();
    }
}

/// The right-click menu of a plain terminal: show in files, rename or exit.
fn terminal_menu(response: &egui::Response, name: &str, action: &mut Option<ExplorerAction>) {
    response.context_menu(|ui| {
        let tip = "Open the file tree down to this terminal's .tmux file or folder";
        if ui.button("Show in files").on_hover_text(tip).clicked() {
            *action = Some(ExplorerAction::ShowTerminalInFiles(name.to_owned()));
            ui.close();
        }
        if ui.button("Rename").on_hover_text("Or double-click it").clicked() {
            *action = Some(ExplorerAction::RenameTerminal(name.to_owned()));
            ui.close();
        }
        if ui.button("Exit terminal").on_hover_text("End the session and everything running in it").clicked() {
            *action = Some(ExplorerAction::ExitTerminal(name.to_owned()));
            ui.close();
        }
    });
}

/// The fold arrow in the icon's place; returns true when it was clicked.
fn fold_arrow(ui: &Ui, response: &egui::Response, expanded: bool) -> bool {
    let left = response.rect.left();
    let width = ui.spacing().button_padding.x * 2.0 + NOTE_MARK_SIZE;
    let area = response.rect.with_max_x(left + width);
    let arrow = ui.interact(area, response.id.with("fold"), egui::Sense::click());
    let center = egui::pos2(left + ui.spacing().button_padding.x + NOTE_MARK_SIZE / 2.0, response.rect.center().y);
    let color = if arrow.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() };
    paint_fold_arrow(ui.painter(), center, expanded, color);
    let tip = if expanded { "Hide its agent sessions" } else { "Show its agent sessions" };
    arrow.on_hover_text(tip).clicked()
}

/// One agent session under its terminal: a click resumes it there, a right
/// click offers a new terminal or its id.
fn agent_session_row(ui: &mut Ui, session: &AgentSession, action: &mut Option<ExplorerAction>) {
    let mut job = LayoutJob::default();
    let style = ui.style();
    RichText::new(format!("{} · {}", session.agent, session.label())).append_to(
        &mut job,
        style,
        FontSelection::Default,
        Align::Center,
    );
    RichText::new(format!("  {}", ago_label(session.last_used))).small().weak().append_to(
        &mut job,
        style,
        FontSelection::Default,
        Align::Center,
    );
    let response = tree_row(ui, false, job).on_hover_text(format!(
        "Click to resume it here:\n{}\nin {}",
        session.resume_command(),
        session.folder.display()
    ));
    let resume = |new_terminal: bool| ExplorerAction::ResumeAgentSession {
        terminal: session.terminal.clone(),
        id: session.id.clone(),
        new_terminal,
    };
    if response.clicked() {
        *action = Some(resume(false));
    }
    response.context_menu(|ui| {
        if ui.button("Resume here").clicked() {
            *action = Some(resume(false));
            ui.close();
        }
        if ui.button("Resume in new terminal").clicked() {
            *action = Some(resume(true));
            ui.close();
        }
        if ui.button("Copy session id").on_hover_text(&session.id).clicked() {
            *action = Some(ExplorerAction::Copy(session.id.clone()));
            ui.close();
        }
    });
}

/// Next to the title: how many sessions there are and, while the section
/// is folded, a dot and count for agents that need you or are working, so
/// nothing important hides behind the fold.
pub(super) fn header_details(ui: &mut Ui, rows: &[&TerminalRow], expanded: bool) {
    if rows.is_empty() {
        return;
    }
    ui.label(RichText::new(rows.len().to_string()).small().weak());
    if expanded {
        return;
    }
    let needs_you = rows.iter().filter(|row| matches!(row.status, TerminalStatus::NeedsYou(_))).count();
    let working = rows.iter().filter(|row| matches!(row.status, TerminalStatus::Working(_))).count();
    let visuals = ui.visuals().clone();
    for (count, color, meaning) in
        [(needs_you, visuals.warn_fg_color, "need you"), (working, visuals.hyperlink_color, "working")]
    {
        if count == 0 {
            continue;
        }
        let (rect, response) = ui.allocate_exact_size(egui::vec2(DOT_RADIUS * 2.0 + 4.0, 12.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), DOT_RADIUS, color);
        response.on_hover_text(format!("{count} {meaning}"));
        ui.label(RichText::new(count.to_string()).small().color(color));
    }
}

/// A `.tmux` file's row: the terminal icon, its name and its session's
/// status. A click opens it, a double click renames it.
pub(super) fn show_terminal_file(
    ui: &mut Ui,
    file: &FsNode,
    context: &TreeContext,
    files: &mut FileRows,
    action: &mut Option<ExplorerAction>,
) {
    let session = session_name_for_file(&file.path);
    let row = context.terminals.and_then(|rows| rows.iter().find(|row| row.info.name == session));
    let is_open = context.open_file == Some(file.path.as_path()) || is_revealed(context, &file.path);
    let label = terminal_label(ui, &file.name, row);
    let response = tree_row(ui, files.highlights(&file.path, is_open), label).on_hover_text(terminal_tip(row));
    scroll_if_revealed(ui, context, &file.path, &response);
    paint_terminal_icon(ui, &response);
    paint_status_dot(ui, &response, &file.name, row.map(|row| &row.status));
    file_click(&response, &file.path, files, action);
    file_menu(&response, &file.path, context, Some((&session, row.is_some())), files, action);
}

/// The name, room for the status dot, then the status and opening time in
/// small text: "build  ●  claude · working · 14:02".
fn terminal_label(ui: &Ui, name: &str, row: Option<&TerminalRow>) -> egui::WidgetText {
    let mut job = LayoutJob::default();
    let style = ui.style();
    RichText::new(format!("{ICON_GAP}{ICON_ROOM}{name}{DOT_ROOM}")).append_to(
        &mut job,
        style,
        FontSelection::Default,
        Align::Center,
    );
    let details = match row {
        Some(row) => format!("{} · {}", row.status.label(), opened_label(row.info.created)),
        None => "not running".to_string(),
    };
    RichText::new(details).small().weak().append_to(&mut job, style, FontSelection::Default, Align::Center);
    job.into()
}

/// The hover text: when it was opened, its last output, and its folder.
fn terminal_tip(row: Option<&TerminalRow>) -> String {
    let Some(row) = row else {
        return "Not running: opening it starts a tmux session in this folder".to_string();
    };
    format!(
        "Session {}\nOpened {} ({})\nLast output {}\nRunning {} in {}",
        row.info.name,
        opened_label(row.info.created),
        ago_label(row.info.created),
        ago_label(row.info.last_output),
        row.info.command,
        row.info.folder.display(),
    )
}

/// Paint the terminal icon in the gap before the name, in one quiet color:
/// the status dot carries the color.
fn paint_terminal_icon(ui: &Ui, response: &egui::Response) {
    let visuals = ui.visuals();
    let center = egui::pos2(
        response.rect.left() + ui.spacing().button_padding.x + NOTE_MARK_SIZE / 2.0,
        response.rect.center().y,
    );
    let behind = if response.hovered() { visuals.widgets.hovered.weak_bg_fill } else { visuals.panel_fill };
    let color = visuals.weak_text_color();
    paint_icon(ui.painter(), Icon::Terminal, center, color, color, behind);
}

/// Paint the status dot in the room left after `name`: filled in the
/// status color, or a hollow gray ring for a terminal that is not running.
fn paint_status_dot(ui: &Ui, response: &egui::Response, name: &str, status: Option<&TerminalStatus>) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let width_of =
        |text: String| ui.fonts_mut(|fonts| fonts.layout_no_wrap(text, font.clone(), Color32::WHITE).size().x);
    let name_width = width_of(format!("{ICON_GAP}{ICON_ROOM}{name}"));
    let room_width = width_of(DOT_ROOM.to_string());
    let x = response.rect.left() + ui.spacing().button_padding.x + name_width + room_width / 2.0;
    let center = egui::pos2(x, response.rect.center().y);
    let visuals = ui.visuals();
    if let Some(status) = status {
        ui.painter().circle_filled(center, DOT_RADIUS, status_color(status, visuals));
    } else {
        let ring = egui::Stroke::new(1.2, visuals.weak_text_color());
        ui.painter().circle_stroke(center, DOT_RADIUS - 0.6, ring);
    }
}

/// Blue while an agent works, orange when it needs you, green when it is
/// idle, gray for shells and other programs.
fn status_color(status: &TerminalStatus, visuals: &Visuals) -> Color32 {
    match status {
        TerminalStatus::Working(_) => visuals.hyperlink_color,
        TerminalStatus::NeedsYou(_) => visuals.warn_fg_color,
        TerminalStatus::Idle(_) => IDLE_COLOR,
        TerminalStatus::Shell | TerminalStatus::Program(_) => visuals.weak_text_color(),
    }
}
