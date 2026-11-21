//! The Seats view: one card per pod with its seats, and the controls that
//! add, copy and remove seats and pods.
//!
//! Drawing only collects what the user asked for (an [`Action`]); the
//! change is made after the cards are drawn, in [`apply`], so the model is
//! never changed while it is being drawn.

use eframe::egui::{self, Align, Color32, Layout, Margin, RichText, Sense, Ui, UiBuilder};

use super::common::{
    CARD_GAP, PodEdit, SeatRef, TeamEditor, issue_line, kept_as_written, labelled_field, plural, text_field,
    warning_mark,
};
use super::links::kind_in_words;
use crate::openrig::spec::{self, Issue, Member, Pod, RigSpec};
use crate::settings::view::list_frame;

/// Height of the title row and of each seat row.
const ROW_HEIGHT: f32 = 36.0;
/// Space left and right of a row's contents, inside the card.
const ROW_MARGIN_X: i8 = 14;
/// Space above and below the links line and the pod edit fields.
const ROW_MARGIN_Y: i8 = 8;
/// Share of the row width each column takes: Seat, Runs on, Model, Agent, Profile.
const COLUMN_SHARES: [f32; 5] = [0.18, 0.13, 0.18, 0.35, 0.16];
/// The shares in a narrow row, which leaves out Model (as the design does
/// beside the seat inspector).
const NARROW_COLUMN_SHARES: [f32; 5] = [0.25, 0.24, 0.0, 0.33, 0.18];
/// Rows narrower than this leave out the Model column.
const NARROW_ROW_WIDTH: f32 = 720.0;
/// Width of the `...` menu at the end of each row.
const MENU_WIDTH: f32 = 28.0;
/// Corner rounding of the runtime tag.
const RUNTIME_RADIUS: u8 = 12;
/// How strongly the selected row takes the selection color.
const SELECTED_TINT: f32 = 0.45;
/// Id a new seat starts from (`seat`, `seat-2`, ...).
const NEW_SEAT_ID: &str = "seat";
/// Id and label a new pod starts from.
const NEW_POD_ID: &str = "pod";
const NEW_POD_LABEL: &str = "New pod";

/// What the user asked for this frame; at most one per frame.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    Select(SeatRef),
    AddSeat(usize),
    DuplicateSeat(SeatRef),
    RemoveSeat(SeatRef),
    AddPod,
    RemovePod(usize),
    /// Open the id and label fields in the pod's card.
    EditPod(usize),
    /// Give the pod this id (and rename the links between pods that name it).
    RenamePod(usize, String),
    /// Close the pod's id and label fields, applying the id typed there.
    FinishEdit,
    /// Show the file as YAML (for the pod's keys kept as written).
    ShowYaml,
}

/// Draw the Seats view (artboards `EditorStarter`, `EditorFactory`).
///
/// A column header row, then one card per pod: the pod's label, id, seat
/// count, "+ Add seat" and a `...` menu (Edit, Remove); one row per seat
/// (id, startup files, runtime, model, agent, profile and a `...` menu with
/// Duplicate and Remove); and a line with the links inside the pod. A click
/// on a row selects the seat, which opens the inspector. Below the cards,
/// "+ Add pod", and a hint while no seat is selected. A read-only team can
/// still select seats; every control that changes the team is disabled.
pub(super) fn show_seats(ui: &mut Ui, editor: &mut TeamEditor) {
    let mut pod_edit = editor.pod_edit.take().filter(|edit| edit.pod < editor.spec.pods.len() && !editor.read_only);
    let mut action = None;

    column_header(ui);
    for pod_index in 0..editor.spec.pods.len() {
        let card_edit = pod_edit.as_mut().filter(|edit| edit.pod == pod_index);
        pod_card(ui, editor, pod_index, card_edit, &mut action);
        ui.add_space(CARD_GAP);
    }
    if let Some(issue) = editor.issue("pods") {
        issue_line(ui, &issue.message);
    }
    if ui.add_enabled(!editor.read_only, egui::Button::new("+ Add pod")).clicked() {
        action = Some(Action::AddPod);
    }
    if editor.selected.is_none() {
        hint_card(ui, &editor.spec);
    }

    match action {
        Some(Action::FinishEdit) => {
            if let Some(edit) = pod_edit.take() {
                action = Some(Action::RenamePod(edit.pod, edit.id_draft));
            }
        }
        Some(Action::EditPod(pod)) => {
            let id_draft = editor.spec.pods[pod].id.clone();
            pod_edit = Some(PodEdit { pod, id_draft });
        }
        Some(Action::RemovePod(_)) => pod_edit = None,
        _ => {}
    }
    editor.pod_edit = pod_edit;
    let adding_pod = action == Some(Action::AddPod);
    if let Some(action) = action {
        apply(editor, action);
    }
    if adding_pod && !editor.read_only {
        let pod = editor.spec.pods.len() - 1;
        editor.pod_edit = Some(PodEdit { pod, id_draft: editor.spec.pods[pod].id.clone() });
    }
}

/// Make the change `action` asks for. A read-only team only selects.
fn apply(editor: &mut TeamEditor, action: Action) {
    match action {
        Action::Select(seat) => {
            editor.selected = Some(seat);
            return;
        }
        Action::ShowYaml => {
            editor.request(super::common::TeamEvent::ShowYaml);
            return;
        }
        _ if editor.read_only => return,
        _ => {}
    }
    match action {
        Action::AddSeat(pod) => add_seat(editor, pod),
        Action::DuplicateSeat(seat) => duplicate_seat(editor, seat),
        Action::RemoveSeat(seat) => editor.remove_seat(seat),
        Action::AddPod => {
            let id = unique_id(NEW_POD_ID, |id| editor.spec.pods.iter().any(|pod| pod.id == id));
            editor.spec.pods.push(Pod::new(&id, NEW_POD_LABEL));
        }
        Action::RemovePod(pod) => editor.remove_pod(pod),
        Action::RenamePod(pod, id) => editor.spec.rename_pod(pod, &id),
        Action::Select(_) | Action::ShowYaml | Action::EditPod(_) | Action::FinishEdit => {}
    }
}

/// Add an empty Claude Code seat at the end of `pod_index` and select it.
fn add_seat(editor: &mut TeamEditor, pod_index: usize) {
    let Some(pod) = editor.spec.pods.get_mut(pod_index) else { return };
    let id = unique_id(NEW_SEAT_ID, |id| pod.members.iter().any(|member| member.id == id));
    pod.members.push(Member::new(&id, spec::CLAUDE_RUNTIME));
    editor.selected = Some(SeatRef { pod: pod_index, member: pod.members.len() - 1 });
}

/// Add a copy of `seat` at the end of its pod, under a free id, and select it.
fn duplicate_seat(editor: &mut TeamEditor, seat: SeatRef) {
    let Some(pod) = editor.spec.pods.get_mut(seat.pod) else { return };
    let Some(original) = pod.members.get(seat.member) else { return };
    let mut copy = original.duplicate();
    copy.id = unique_id(&original.id, |id| pod.members.iter().any(|member| member.id == id));
    pod.members.push(copy);
    editor.selected = Some(SeatRef { pod: seat.pod, member: pod.members.len() - 1 });
}

/// The pod part of a link end between pods (`dev` in `dev.build`).
fn end_pod(end: &str) -> Option<&str> {
    end.split_once('.').map(|(pod, _)| pod)
}

/// `base`, or `base-2`, `base-3`, ... : the first one `taken` says is free.
fn unique_id(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    let mut number = 2;
    loop {
        let candidate = format!("{base}-{number}");
        if !taken(&candidate) {
            return candidate;
        }
        number += 1;
    }
}

/// The widths of the five columns, for a row `width` wide that also holds
/// the menu. A column left out (Model in a narrow row) has width 0.
fn column_widths(ui: &Ui, width: f32) -> [f32; 5] {
    let column_shares = if width < NARROW_ROW_WIDTH { NARROW_COLUMN_SHARES } else { COLUMN_SHARES };
    // One gap after each shown column, before the next column or the menu.
    let spacing: f32 = column_shares.iter().filter(|share| **share > 0.0).map(|_| ui.spacing().item_spacing.x).sum();
    let columns_width = (width - MENU_WIDTH - spacing).max(0.0);
    column_shares.map(|share| share * columns_width)
}

/// One column of a row: exactly `width` wide, contents centered vertically.
/// Contents wider than the column are clipped: a column that grew would
/// widen the card, and every row and card after it. A column of width 0 is
/// left out.
fn cell(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui)) {
    if width <= 0.0 {
        return;
    }
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), Sense::hover());
    let mut cell_ui = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)));
    cell_ui.shrink_clip_rect(rect);
    add(&mut cell_ui);
}

/// The column names above the cards, lined up with the seat rows.
fn column_header(ui: &mut Ui) {
    egui::Frame::new().inner_margin(Margin::symmetric(ROW_MARGIN_X, 0)).show(ui, |ui| {
        let widths = column_widths(ui, ui.available_width());
        ui.horizontal(|ui| {
            for (name, width) in ["Seat", "Runs on", "Model", "Agent", "Profile"].into_iter().zip(widths) {
                cell(ui, width, |ui| {
                    ui.label(RichText::new(name).small().weak());
                });
            }
        });
    });
}

/// One pod's card: its title row (or its id and label fields while it is
/// being edited), one row per seat and the links line.
fn pod_card(
    ui: &mut Ui,
    editor: &mut TeamEditor,
    pod_index: usize,
    pod_edit: Option<&mut PodEdit>,
    action: &mut Option<Action>,
) {
    list_frame(ui, |ui| {
        match pod_edit {
            Some(edit) => pod_edit_fields(ui, editor, edit, action),
            None => pod_title_row(ui, editor, pod_index, action),
        }
        for member in 0..editor.spec.pods[pod_index].members.len() {
            ui.add(egui::Separator::default().spacing(0.0));
            seat_row(ui, editor, SeatRef { pod: pod_index, member }, action);
        }
        ui.add(egui::Separator::default().spacing(0.0));
        links_line(ui, &editor.spec, pod_index);
    });
}

/// The pod's label, id and seat count, a warning mark for a problem with
/// the id or label, then "+ Add seat" and the pod's `...` menu.
fn pod_title_row(ui: &mut Ui, editor: &TeamEditor, pod_index: usize, action: &mut Option<Action>) {
    let pod = &editor.spec.pods[pod_index];
    let issue =
        editor.issue(&format!("pods[{pod_index}].id")).or_else(|| editor.issue(&format!("pods[{pod_index}].label")));
    egui::Frame::new().inner_margin(Margin::symmetric(ROW_MARGIN_X, 0)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_height(ROW_HEIGHT);
            // The buttons go first, from the right, so the names get only the
            // room left and are cut instead of running under the buttons.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_enabled_ui(!editor.read_only, |ui| {
                    ui.menu_button("…", |ui| {
                        if ui.button("Edit id and label").clicked() {
                            *action = Some(Action::EditPod(pod_index));
                        }
                        if ui.button("Remove pod").clicked() {
                            *action = Some(Action::RemovePod(pod_index));
                        }
                    });
                    let add = RichText::new("+ Add seat").color(ui.visuals().hyperlink_color);
                    if ui.add(egui::Button::new(add).frame(false)).clicked() {
                        *action = Some(Action::AddSeat(pod_index));
                    }
                });
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(pod.name()).strong()).truncate());
                    ui.add(egui::Label::new(RichText::new(&pod.id).monospace().weak()).truncate());
                    let count = pod.members.len();
                    let count = RichText::new(format!("{count} {}", plural(count, "seat", "seats"))).small().weak();
                    ui.add(egui::Label::new(count).truncate());
                    if let Some(issue) = issue {
                        warning_mark(ui, issue);
                    }
                });
            });
        });
    });
}

/// The pod's label and id fields with their problems, and Done. The label
/// changes as it is typed; the id when its field loses focus or on Done.
fn pod_edit_fields(ui: &mut Ui, editor: &mut TeamEditor, edit: &mut PodEdit, action: &mut Option<Action>) {
    let label_issue = editor.issue(&format!("pods[{}].label", edit.pod)).cloned();
    let id_issue = editor.issue(&format!("pods[{}].id", edit.pod)).cloned();
    egui::Frame::new().inner_margin(Margin::symmetric(ROW_MARGIN_X, ROW_MARGIN_Y)).show(ui, |ui| {
        let pod = &mut editor.spec.pods[edit.pod];
        labelled_field(ui, "Label", "label", true, label_issue.as_ref(), |ui| {
            text_field(ui, &mut pod.label, "Development", label_issue.is_some());
        });
        let id_field = labelled_field(ui, "Pod id", "id", true, id_issue.as_ref(), |ui| {
            text_field(ui, &mut edit.id_draft, "dev", id_issue.is_some())
        });
        if kept_as_written(ui, &pod.yaml_only) {
            *action = Some(Action::ShowYaml);
        }
        if ui.button("Done").clicked() {
            *action = Some(Action::FinishEdit);
        } else if id_field.lost_focus() && edit.id_draft != pod.id {
            *action = Some(Action::RenamePod(edit.pod, edit.id_draft.clone()));
        }
    });
}

/// One seat: id, startup files, runtime, model, agent and profile, then
/// its `...` menu. A click on the row (not the menu) selects the seat.
fn seat_row(ui: &mut Ui, editor: &TeamEditor, seat: SeatRef, action: &mut Option<Action>) {
    let member = &editor.spec.pods[seat.pod].members[seat.member];
    let issues = editor.issues_under(&seat.path());
    let issue = issues.first();
    let fill = if editor.selected == Some(seat) {
        ui.visuals().selection.bg_fill.gamma_multiply(SELECTED_TINT)
    } else {
        Color32::TRANSPARENT
    };
    // The row's click sense is registered before its contents, so the menu
    // button drawn inside sits on top and gets its own clicks.
    let row = ui.scope_builder(UiBuilder::new().sense(Sense::click()), |ui| {
        egui::Frame::new()
            .fill(fill)
            .inner_margin(Margin::symmetric(ROW_MARGIN_X, 0))
            .show(ui, |ui| {
                ui.style_mut().interaction.selectable_labels = false;
                let widths = column_widths(ui, ui.available_width());
                ui.horizontal(|ui| {
                    cell(ui, widths[0], |ui| seat_id_cell(ui, member, issue));
                    cell(ui, widths[1], |ui| runtime_tag(ui, &member.runtime));
                    cell(ui, widths[2], |ui| model_cell(ui, &member.model));
                    cell(ui, widths[3], |ui| plain_cell(ui, &short_agent(&member.agent_ref)));
                    cell(ui, widths[4], |ui| plain_cell(ui, &member.profile));
                    let mut menu_clicked = false;
                    cell(ui, MENU_WIDTH, |ui| menu_clicked = seat_menu(ui, editor.read_only, seat, action));
                    menu_clicked
                })
                .inner
            })
            .inner
    });
    let menu_clicked = row.inner;
    if row.response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !menu_clicked {
        *action = Some(Action::Select(seat));
    }
}

/// The seat id in monospace, a "N startup file(s)" tag, and a warning mark
/// when the seat has a problem.
fn seat_id_cell(ui: &mut Ui, member: &Member, issue: Option<&Issue>) {
    ui.add(egui::Label::new(RichText::new(&member.id).monospace().strong()).truncate());
    let files = member.startup.files.len();
    if files > 0 {
        quiet_tag(ui, RichText::new(format!("{files} startup {}", plural(files, "file", "files"))).small().weak());
    }
    if let Some(issue) = issue {
        warning_mark(ui, issue);
    }
}

/// The seat's runtime in a rounded tag, like the "Runs on" pills.
fn runtime_tag(ui: &mut Ui, runtime: &str) {
    if runtime.is_empty() {
        not_set(ui);
        return;
    }
    quiet_tag(ui, RichText::new(runtime).monospace().small());
}

/// `text` in a quiet rounded tag as tall as the text, cut to the room left
/// in the column.
fn quiet_tag(ui: &mut Ui, text: RichText) {
    // In the cell's centered row a frame would take the full row height;
    // its own row keeps it as tall as the text.
    ui.horizontal(|ui| {
        egui::Frame::new()
            .fill(ui.visuals().widgets.inactive.weak_bg_fill)
            .corner_radius(RUNTIME_RADIUS)
            .inner_margin(Margin::symmetric(8, 1))
            .show(ui, |ui| {
                ui.add(egui::Label::new(text).truncate());
            });
    });
}

/// The model, or "runtime default" when the seat names none.
fn model_cell(ui: &mut Ui, model: &str) {
    if model.is_empty() {
        ui.add(egui::Label::new(RichText::new("runtime default").weak()).truncate());
    } else {
        plain_cell(ui, model);
    }
}

/// A monospace value cut to the column, or "not set" when empty.
fn plain_cell(ui: &mut Ui, value: &str) {
    if value.is_empty() {
        not_set(ui);
    } else {
        ui.add(egui::Label::new(RichText::new(value).monospace()).truncate()).on_hover_text(value);
    }
}

/// The placeholder for a required value the seat lacks.
fn not_set(ui: &mut Ui) {
    ui.label(RichText::new("not set").weak().italics());
}

/// The seat's `...` menu: Duplicate and Remove. Returns true when the menu
/// button itself was clicked this frame.
fn seat_menu(ui: &mut Ui, read_only: bool, seat: SeatRef, action: &mut Option<Action>) -> bool {
    ui.add_enabled_ui(!read_only, |ui| {
        ui.menu_button("…", |ui| {
            if ui.button("Duplicate seat").clicked() {
                *action = Some(Action::DuplicateSeat(seat));
            }
            if ui.button("Remove seat").clicked() {
                *action = Some(Action::RemoveSeat(seat));
            }
        })
        .response
        .clicked()
    })
    .inner
}

/// The agent link cut to its last two path parts
/// (`local:../../agents/development/qa` shows as `…/development/qa`).
/// A link with no folders (`builtin:terminal`) shows whole.
fn short_agent(agent_ref: &str) -> String {
    let path = agent_ref.split_once(':').map_or(agent_ref, |(_, path)| path);
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    match parts.as_slice() {
        [.., parent, last] if parts.len() > 2 => format!("…/{parent}/{last}"),
        _ if path.contains('/') => path.to_owned(),
        _ => agent_ref.to_owned(),
    }
}

/// Under the seats: the links inside the pod in words ("Inside this pod:
/// build delegates to review"), and how many links join it to other pods.
fn links_line(ui: &mut Ui, spec: &RigSpec, pod_index: usize) {
    let pod = &spec.pods[pod_index];
    let names_pod = |end: &str| end_pod(end) == Some(pod.id.as_str());
    let between = spec.edges.iter().filter(|edge| names_pod(&edge.from) || names_pod(&edge.to)).count();
    egui::Frame::new().inner_margin(Margin::symmetric(ROW_MARGIN_X, ROW_MARGIN_Y)).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if pod.edges.is_empty() {
                ui.label(RichText::new("No links inside this pod.").small().weak());
            } else {
                ui.label(RichText::new("Inside this pod:").small().weak());
                for (index, edge) in pod.edges.iter().enumerate() {
                    if index > 0 {
                        ui.label(RichText::new(",").small().weak());
                    }
                    ui.label(RichText::new(&edge.from).monospace().small());
                    ui.label(RichText::new(kind_in_words(&edge.kind)).small().weak());
                    ui.label(RichText::new(&edge.to).monospace().small());
                }
            }
            if between > 0 {
                let links = plural(between, "link", "links");
                let text = format!("{between} {links} with other pods: see Who works with whom.");
                ui.label(RichText::new(text).small().weak());
            }
        });
    });
}

/// The hint shown while no seat is selected: click a seat to edit it, with
/// the address of the first seat as an example.
fn hint_card(ui: &mut Ui, spec: &RigSpec) {
    let Some((pod, member)) = spec.pods.iter().find_map(|pod| pod.members.first().map(|member| (pod, member))) else {
        return;
    };
    let address = spec::seat_address(&pod.id, &member.id, &spec.name);
    ui.add_space(CARD_GAP);
    list_frame(ui, |ui| {
        egui::Frame::new().inner_margin(Margin::symmetric(ROW_MARGIN_X, ROW_MARGIN_Y)).show(ui, |ui| {
            ui.label(RichText::new("Click a seat to edit it").strong());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let intro = format!(
                    "The seat form opens on the right. Seat addresses follow pod-seat@team, so {} is ",
                    member.id
                );
                ui.label(RichText::new(intro).weak());
                ui.label(RichText::new(address).monospace().color(ui.visuals().hyperlink_color));
                ui.label(RichText::new(".").weak());
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Event, PointerButton, Pos2, RawInput, Rect, Shape, epaint::ClippedShape, vec2};

    use super::*;
    use crate::document::text_file::{Document, Opened};
    use crate::test_support::TempFolder;

    const FACTORY: &str = include_str!("../../openrig/spec/fixtures/factory.yaml");

    /// A form for the factory team, saved in the user's folder (or in the
    /// built-in folder when `built_in`, which makes it read-only).
    fn factory_editor(built_in: bool) -> (TempFolder, TeamEditor) {
        let temp = TempFolder::new();
        let file = if built_in { "openrig/packages/daemon/specs/rigs/factory/rig.yaml" } else { "factory/rig.yaml" };
        let path = temp.write(file, FACTORY);
        let Opened::Text(document) = Document::open(&path, None).unwrap() else { panic!("a text file") };
        let editor = TeamEditor::new(&document);
        assert_eq!(editor.read_only, built_in);
        (temp, editor)
    }

    /// Draw the Seats view once with the pointer at `pointer` (pressed or
    /// released when `button` says so). Returns where each text was drawn.
    fn frame(ctx: &egui::Context, editor: &mut TeamEditor, pointer: Pos2, button: Option<bool>) -> Vec<(String, Rect)> {
        let mut input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 2000.0))), ..Default::default() };
        input.events.push(Event::PointerMoved(pointer));
        if let Some(pressed) = button {
            input.events.push(Event::PointerButton {
                pos: pointer,
                button: PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
        let mut output = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| show_seats(ui, editor));
        });
        output.textures_delta.clear();
        output
            .shapes
            .into_iter()
            .filter_map(|ClippedShape { shape, .. }| match shape {
                Shape::Text(text) => Some((text.galley.text().to_owned(), text.visual_bounding_rect())),
                _ => None,
            })
            .collect()
    }

    /// Click on the first text drawn as exactly `text`.
    fn click_text(ctx: &egui::Context, editor: &mut TeamEditor, text: &str) {
        let texts = frame(ctx, editor, Pos2::ZERO, None);
        let (_, rect) = texts.iter().find(|(drawn, _)| drawn == text).unwrap_or_else(|| panic!("no text {text:?}"));
        let at = rect.center();
        frame(ctx, editor, at, None);
        frame(ctx, editor, at, Some(true));
        frame(ctx, editor, at, Some(false));
    }

    #[test]
    fn clicking_a_seat_row_selects_it() {
        let (_temp, mut editor) = factory_editor(false);
        let ctx = egui::Context::default();
        click_text(&ctx, &mut editor, "…/development/qa");
        assert_eq!(editor.selected, Some(SeatRef { pod: 1, member: 1 }));
    }

    #[test]
    fn add_seat_adds_a_selected_seat_to_its_pod() {
        let (_temp, mut editor) = factory_editor(false);
        let ctx = egui::Context::default();
        click_text(&ctx, &mut editor, "+ Add seat");
        let orchestration = &editor.spec.pods[0];
        assert_eq!(orchestration.members.len(), 3);
        assert_eq!(orchestration.members[2].id, "seat");
        assert_eq!(orchestration.members[2].runtime, spec::CLAUDE_RUNTIME);
        assert_eq!(editor.selected, Some(SeatRef { pod: 0, member: 2 }));
    }

    #[test]
    fn add_pod_adds_an_empty_pod_under_a_free_id() {
        let (_temp, mut editor) = factory_editor(false);
        let ctx = egui::Context::default();
        click_text(&ctx, &mut editor, "+ Add pod");
        click_text(&ctx, &mut editor, "Done");
        click_text(&ctx, &mut editor, "+ Add pod");
        let ids: Vec<&str> = editor.spec.pods.iter().map(|pod| pod.id.as_str()).collect();
        assert_eq!(ids, ["orch", "dev", "review", "pod", "pod-2"]);
        assert_eq!(editor.spec.pods[3].members.len(), 0);
    }

    #[test]
    fn a_read_only_team_can_be_browsed_but_not_changed() {
        let (_temp, mut editor) = factory_editor(true);
        let ctx = egui::Context::default();
        click_text(&ctx, &mut editor, "+ Add seat");
        click_text(&ctx, &mut editor, "+ Add pod");
        assert_eq!(editor.spec.seat_count(), 7);
        assert_eq!(editor.spec.pods.len(), 3);
        click_text(&ctx, &mut editor, "…/development/qa");
        assert_eq!(editor.selected, Some(SeatRef { pod: 1, member: 1 }));
        apply(&mut editor, Action::RemovePod(0));
        assert_eq!(editor.spec.pods.len(), 3);
    }

    /// The pod being edited belongs to its team: another team's tab, drawn
    /// with the same egui context, shows its pods closed.
    #[test]
    fn a_pod_edit_stays_with_its_team() {
        let (_temp, mut editor) = factory_editor(false);
        let (_other_temp, mut other) = factory_editor(false);
        let ctx = egui::Context::default();
        click_text(&ctx, &mut editor, "+ Add pod");
        assert!(editor.pod_edit.is_some());
        let texts = frame(&ctx, &mut other, Pos2::ZERO, None);
        assert!(texts.iter().all(|(text, _)| text != "Done"), "the other team opens no pod fields");
        assert_eq!(other.pod_edit, None);
    }

    #[test]
    fn removing_a_seat_removes_its_links_and_keeps_the_selection_on_its_seat() {
        let (_temp, mut editor) = factory_editor(false);
        editor.selected = Some(SeatRef { pod: 1, member: 2 });
        apply(&mut editor, Action::RemoveSeat(SeatRef { pod: 1, member: 0 }));
        let development = &editor.spec.pods[1];
        assert_eq!(development.members.iter().map(|member| member.id.as_str()).collect::<Vec<_>>(), ["qa", "design"]);
        assert_eq!(development.edges.len(), 0, "build delegates to qa is gone");
        assert!(editor.spec.edges.iter().all(|edge| edge.from != "dev.build" && edge.to != "dev.build"));
        assert!(editor.spec.edges.iter().any(|edge| edge.to == "dev.design"));
        assert_eq!(editor.selected, Some(SeatRef { pod: 1, member: 1 }), "still design");

        apply(&mut editor, Action::RemoveSeat(SeatRef { pod: 1, member: 1 }));
        assert_eq!(editor.selected, None);
    }

    #[test]
    fn removing_a_pod_removes_its_links_between_pods() {
        let (_temp, mut editor) = factory_editor(false);
        editor.selected = Some(SeatRef { pod: 2, member: 1 });
        apply(&mut editor, Action::RemovePod(1));
        assert_eq!(editor.spec.pods.iter().map(|pod| pod.id.as_str()).collect::<Vec<_>>(), ["orch", "review"]);
        assert!(
            editor.spec.edges.iter().all(|edge| end_pod(&edge.from) != Some("dev") && end_pod(&edge.to) != Some("dev"))
        );
        assert!(editor.spec.edges.iter().any(|edge| edge.to == "review.r1"));
        assert_eq!(editor.selected, Some(SeatRef { pod: 1, member: 1 }), "still r2");

        apply(&mut editor, Action::RemovePod(1));
        assert_eq!(editor.selected, None);
    }

    #[test]
    fn duplicating_a_seat_adds_a_copy_under_a_free_id() {
        let (_temp, mut editor) = factory_editor(false);
        apply(&mut editor, Action::DuplicateSeat(SeatRef { pod: 1, member: 0 }));
        let development = &editor.spec.pods[1];
        assert_eq!(development.members.len(), 4);
        assert_eq!(development.members[3].id, "build-2");
        assert_eq!(development.members[3].agent_ref, development.members[0].agent_ref);
        assert_eq!(editor.selected, Some(SeatRef { pod: 1, member: 3 }));
    }

    #[test]
    fn renaming_a_pod_renames_its_links_between_pods() {
        let (_temp, mut editor) = factory_editor(false);
        apply(&mut editor, Action::RenamePod(1, "build".into()));
        assert_eq!(editor.spec.pods[1].id, "build");
        assert!(editor.spec.edges.iter().any(|edge| edge.to == "build.design"));
        assert!(editor.spec.edges.iter().all(|edge| end_pod(&edge.to) != Some("dev")));
    }

    #[test]
    fn agent_links_show_their_last_two_folders() {
        assert_eq!(short_agent("local:../../../agents/development/qa"), "…/development/qa");
        assert_eq!(short_agent("path:/agents/qa"), "/agents/qa");
        assert_eq!(short_agent("builtin:terminal"), "builtin:terminal");
    }
}
