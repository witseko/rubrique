//! The team editor's shared state and the small widgets every screen uses.
//!
//! The form edits [`TeamEditor::spec`]. Each frame the editor writes the
//! changed model into a copy of the text it was read from (see
//! [`TeamEditor::write_to`]), so the document holds the YAML the form shows
//! and the app's own save, drafts and dirty mark work unchanged. When the
//! text changes elsewhere (the YAML view, undo, a newer file on disk) the
//! model is read again (see [`TeamEditor::sync_from`]).

use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::Receiver;

use eframe::egui::{self, Context, Id, RichText, Ui};

use crate::app::openrig::{poll, work};
use crate::document::text_file::Document;
use crate::icons::{Icon, small_icon_button_on};
use crate::openrig::spec::{DELIVERY_HINTS, EnvVar, Issue, PERMISSION_POLICIES, RigSpec, Startup, StartupFile};
use crate::openrig::{Preflight, RigCallError, RigHost, SpecValidation, UpReply};
use crate::settings::view::{field_box, list_frame};

/// Size of the spec key shown after a field's label (`agent_ref`).
pub(in crate::app) const KEY_SIZE: f32 = 11.0;
/// Space between one labelled field and the next.
pub(in crate::app) const FIELD_GAP: f32 = 14.0;
/// Space between a field's label row and the field.
pub(in crate::app) const LABEL_GAP: f32 = 4.0;
/// Space between two cards.
pub(in crate::app) const CARD_GAP: f32 = 12.0;
/// Size of the remove button at the end of a list row.
pub(in crate::app) const REMOVE_SIZE: f32 = 18.0;
/// Corner rounding of a pill.
const PILL_RADIUS: u8 = 12;
/// Space left and right of a pill's text.
const PILL_PADDING_X: i8 = 10;
/// Space above and below a pill's text.
const PILL_PADDING_Y: i8 = 3;
/// Space around a row of a list (variables, startup files).
const ROW_MARGIN: i8 = 6;
/// Width of a variable name field.
const NAME_FIELD_WIDTH: f32 = 120.0;
/// Width of a drop-down in a list row.
const CHOICE_WIDTH: f32 = 140.0;
/// Labels of the permission pills, in the order of [`PERMISSION_POLICIES`].
const PERMISSION_LABELS: [&str; 5] = ["Locked", "Standard", "Open", "Yolo", "Auto"];

/// The three views under the header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) enum SubTab {
    /// Pod cards with their seats.
    #[default]
    Seats,
    /// "Who works with whom": the links.
    Links,
    /// Team settings.
    Team,
}

/// A seat by its place in the model: `spec.pods[pod].members[member]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct SeatRef {
    pub pod: usize,
    pub member: usize,
}

impl SeatRef {
    /// The validator path of this seat: `pods[0].members[1]`.
    pub(in crate::app) fn path(self) -> String {
        format!("pods[{}].members[{}]", self.pod, self.member)
    }
}

/// The pod whose card shows its id and label fields, and the id typed so
/// far. The id is applied when the field loses focus or on Done, not on
/// each key, so the links between pods are renamed once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct PodEdit {
    pub pod: usize,
    pub id_draft: String,
}

/// Something the form asks the app to do; the app handles these after the
/// frame (see `RubriqueApp::team_event`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum TeamEvent {
    /// Save the file, then run `rig spec validate` (and preflight after it).
    Validate,
    /// Save the file, then validate, preflight and preview the launch with
    /// `rig up --plan`. Starts nothing.
    PrepareLaunch,
    /// Launch the team with `rig up`. Only the Launch button under a
    /// launch plan asks for this.
    Launch,
    /// Save the file now.
    Save,
    /// Drop the unsaved edits and read the file again.
    Revert,
    /// Open this file in a tab (agent.yaml, the culture file).
    OpenPath(PathBuf),
    /// Copy this read-only team into the user's spec folder and open the copy.
    Duplicate,
    /// Show the file as YAML, for the fields the form does not edit.
    ShowYaml,
    /// Open Settings at the `OpenRig` page.
    OpenSettings,
}

/// One background `rig` call and its last answer.
#[derive(Debug)]
pub(in crate::app) struct Job<T> {
    running: Option<Receiver<Result<T, RigCallError>>>,
    /// The last answer; `None` before the first run finishes.
    pub result: Option<Result<T, RigCallError>>,
}

impl<T> Default for Job<T> {
    fn default() -> Self {
        Self { running: None, result: None }
    }
}

impl<T: Send + 'static> Job<T> {
    /// Start `operation` on another thread, replacing any earlier answer.
    pub(in crate::app) fn start(
        &mut self,
        ctx: &Context,
        operation: impl FnOnce() -> Result<T, RigCallError> + Send + 'static,
    ) {
        self.result = None;
        self.running = Some(work(ctx, operation));
    }

    /// Take the answer once it arrives. Returns true on the frame it does.
    pub(in crate::app) fn poll(&mut self) -> bool {
        let Some(result) = poll(&mut self.running) else { return false };
        self.result = Some(result);
        true
    }

    /// True while the call runs.
    pub(in crate::app) fn is_running(&self) -> bool {
        self.running.is_some()
    }
}

/// The state of one rig spec shown as a form, kept with its tab.
#[derive(Debug)]
pub(in crate::app) struct TeamEditor {
    /// The model the form edits.
    pub spec: RigSpec,
    /// The text `spec` was read from; edits are written into a copy of it.
    base_text: String,
    /// The model as last written into the document.
    written: RigSpec,
    /// The document revision last read or written by the form; any other
    /// revision means the text changed elsewhere.
    synced_revision: u64,
    /// The file as last saved (`Document::base_text`), and its model, for
    /// the unsaved-changes line and dialog.
    saved_text: String,
    pub saved: Option<RigSpec>,
    /// Why the text cannot be shown as a form (a YAML error); the form
    /// waits until the YAML is fixed.
    pub parse_error: Option<String>,
    /// Why the last edit could not be written into the text.
    pub write_error: Option<String>,
    /// Problems the form finds itself (`RigSpec::check`), kept current.
    pub issues: Vec<Issue>,
    /// A built-in team: shown, never changed (see `is_read_only`).
    pub read_only: bool,
    /// The file is mirrored from another computer, so this computer's
    /// folder pickers cannot choose its paths.
    pub remote: bool,
    /// The folder holding the spec, which relative paths start from.
    pub spec_folder: PathBuf,
    pub tab: SubTab,
    /// The seat open in the inspector.
    pub selected: Option<SeatRef>,
    /// The pod whose id and label are being edited in its card.
    pub pod_edit: Option<PodEdit>,
    pub validation: Job<SpecValidation>,
    pub preflight: Job<Preflight>,
    /// The last `rig up --plan`, which the Launch button needs first.
    pub plan: Job<UpReply>,
    /// The last real `rig up`.
    pub launch: Job<UpReply>,
    /// Run the plan once preflight says the team is ready (set by
    /// [`TeamEvent::PrepareLaunch`]).
    pub plan_after_preflight: bool,
    /// What the app should do after this frame.
    pub events: Vec<TeamEvent>,
}

impl TeamEditor {
    /// The form for `document`, which must hold a rig spec.
    pub(in crate::app) fn new(document: &Document) -> Self {
        let parsed = RigSpec::parse(&document.text);
        let spec = parsed.as_ref().cloned().unwrap_or_default();
        Self {
            issues: spec.check(),
            written: spec.clone(),
            spec,
            base_text: document.text.clone(),
            synced_revision: document.revision(),
            saved_text: document.base_text().to_owned(),
            saved: RigSpec::parse(document.base_text()).ok(),
            parse_error: parsed.err(),
            write_error: None,
            read_only: is_read_only(&document.path),
            remote: crate::remote::mirror::from_mirror(&document.path).is_some(),
            spec_folder: document.path.parent().map(Path::to_path_buf).unwrap_or_default(),
            tab: SubTab::default(),
            selected: None,
            pod_edit: None,
            validation: Job::default(),
            preflight: Job::default(),
            plan: Job::default(),
            launch: Job::default(),
            plan_after_preflight: false,
            events: Vec::new(),
        }
    }

    /// Read the model again when the document's text changed elsewhere.
    pub(in crate::app) fn sync_from(&mut self, document: &Document) {
        if self.saved_text != document.base_text() {
            document.base_text().clone_into(&mut self.saved_text);
            self.saved = RigSpec::parse(&self.saved_text).ok();
        }
        if document.revision() == self.synced_revision {
            return;
        }
        self.synced_revision = document.revision();
        self.base_text.clone_from(&document.text);
        self.plan = Job::default();
        match RigSpec::parse(&document.text) {
            Ok(spec) => {
                self.issues = spec.check();
                self.written = spec.clone();
                self.spec = spec;
                self.parse_error = None;
                self.write_error = None;
            }
            Err(error) => self.parse_error = Some(error),
        }
        let pod_count = self.spec.pods.len();
        let seat_exists =
            |seat: &SeatRef| self.spec.pods.get(seat.pod).is_some_and(|pod| seat.member < pod.members.len());
        self.selected = self.selected.filter(seat_exists);
        self.pod_edit = self.pod_edit.take().filter(|edit| edit.pod < pod_count);
    }

    /// Write the model into the document when the form changed it. A
    /// read-only team takes back any change instead.
    pub(in crate::app) fn write_to(&mut self, document: &mut Document) {
        if self.spec == self.written || self.parse_error.is_some() {
            return;
        }
        if self.read_only {
            self.spec = self.written.clone();
            return;
        }
        self.issues = self.spec.check();
        match self.spec.write_into(&self.base_text) {
            Ok(text) => {
                self.written = self.spec.clone();
                self.write_error = None;
                if text != document.text {
                    document.text = text;
                    document.on_text_changed();
                    // The plan was for the text before this edit.
                    self.plan = Job::default();
                }
                self.synced_revision = document.revision();
            }
            Err(error) => self.write_error = Some(error),
        }
    }

    /// The problem at `path` (`pods[0].members[1].id`): from the last
    /// `rig spec validate` first, else from the form's own checks.
    pub(in crate::app) fn issue(&self, path: &str) -> Option<&Issue> {
        self.validated_issues()
            .iter()
            .find(|issue| issue.path == path)
            .or_else(|| self.issues.iter().find(|issue| issue.path == path))
    }

    /// Every problem at `prefix` or under it (`pods[0].members[1]` takes
    /// `pods[0].members[1].id` too), one per path, each as [`Self::issue`]
    /// gives it. An empty `prefix` takes them all. The screens copy these so
    /// they can change the spec while they show them.
    pub(in crate::app) fn issues_under(&self, prefix: &str) -> Vec<Issue> {
        let under = |path: &str| {
            prefix.is_empty() || path.strip_prefix(prefix).is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
        };
        let mut found: Vec<Issue> = Vec::new();
        for issue in self.validated_issues().iter().chain(&self.issues) {
            if under(&issue.path) && !found.iter().any(|kept| kept.path == issue.path) {
                found.push(issue.clone());
            }
        }
        found
    }

    /// The errors of the last `rig spec validate`, if it answered.
    fn validated_issues(&self) -> &[Issue] {
        match &self.validation.result {
            Some(Ok(validation)) => &validation.errors,
            _ => &[],
        }
    }

    /// Ask the app to do `event` after this frame.
    pub(in crate::app) fn request(&mut self, event: TeamEvent) {
        if !self.events.contains(&event) {
            self.events.push(event);
        }
    }

    /// Remove `seat` and its links, keeping the selection on the same seat
    /// (or clearing it when it was this one).
    pub(in crate::app) fn remove_seat(&mut self, seat: SeatRef) {
        self.spec.remove_seat(seat.pod, seat.member);
        self.selected = match self.selected {
            Some(selected) if selected == seat => None,
            Some(selected) if selected.pod == seat.pod && selected.member > seat.member => {
                Some(SeatRef { member: selected.member - 1, ..selected })
            }
            other => other,
        };
    }

    /// Remove pod `pod` and its links, keeping the selection on the same
    /// seat (or clearing it when it was in this pod).
    pub(in crate::app) fn remove_pod(&mut self, pod: usize) {
        self.spec.remove_pod(pod);
        self.pod_edit = None;
        self.selected = match self.selected {
            Some(selected) if selected.pod == pod => None,
            Some(selected) if selected.pod > pod => Some(SeatRef { pod: selected.pod - 1, ..selected }),
            other => other,
        };
    }
}

/// Where `rig` runs for the spec at `path`, and the spec's path there: a
/// file mirrored from another computer is checked by that computer's rig.
pub(in crate::app) fn rig_target(path: &Path) -> (RigHost, PathBuf) {
    match crate::remote::mirror::from_mirror(path) {
        Some(remote) => (RigHost::Ssh(remote.host), remote.path),
        None => (RigHost::Local, path.to_path_buf()),
    }
}

/// A team the form shows but does not change: one of the teams `OpenRig`
/// ships (see [`is_built_in`]), or a file that cannot be written.
pub(in crate::app) fn is_read_only(path: &Path) -> bool {
    let unwritable = std::fs::metadata(path).is_ok_and(|metadata| metadata.permissions().readonly());
    is_built_in(path) || unwritable
}

/// True for a path inside the teams `OpenRig` ships, which live under
/// `daemon/specs/rigs` in its installation (`packages/daemon/specs/rigs` in
/// the repository, `@openrig/daemon/specs/rigs` in an npm install).
pub(in crate::app) fn is_built_in(path: &Path) -> bool {
    let names: Vec<_> = path.components().map(Component::as_os_str).collect();
    names.windows(3).any(|window| window == ["daemon", "specs", "rigs"])
}

/// A relative path that stays inside its folder: no root, no `..`. The
/// team's own files (culture file, startup files, docs) must be such paths
/// before the editor opens or copies them.
pub(in crate::app) fn is_safe_relative(path: &str) -> bool {
    !path.trim().is_empty()
        && Path::new(path).components().all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

/// `one` for a count of one, else `many`.
pub(in crate::app) fn plural(count: usize, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 { one } else { many }
}

/// The problem at `path` in `issues`.
pub(in crate::app) fn issue_at<'a>(issues: &'a [Issue], path: &str) -> Option<&'a Issue> {
    issues.iter().find(|issue| issue.path == path)
}

/// A field's label row: the label, a required mark, and the spec key small
/// after it (`Agent * agent_ref`).
pub(in crate::app) fn field_label(ui: &mut Ui, label: &str, key: &str, required: bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new(label).strong());
        if required {
            required_mark(ui);
        }
        spec_key(ui, key);
    });
}

/// The `*` after a required field's label.
pub(in crate::app) fn required_mark(ui: &mut Ui) {
    ui.label(RichText::new("*").color(ui.visuals().hyperlink_color).strong());
}

/// A spec key in small monospace, as shown after labels and section titles.
pub(in crate::app) fn spec_key(ui: &mut Ui, key: &str) {
    ui.label(RichText::new(key).monospace().size(KEY_SIZE).weak());
}

/// A small, weak line of help under a field.
pub(in crate::app) fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).small().weak());
}

/// A problem under a field, in the warning color.
pub(in crate::app) fn issue_line(ui: &mut Ui, message: &str) {
    ui.label(RichText::new(message).color(ui.visuals().warn_fg_color).small());
}

/// A small mark in the warning color; hovering it shows the problem.
pub(in crate::app) fn warning_mark(ui: &mut Ui, issue: &Issue) {
    ui.label(RichText::new("!").strong().color(ui.visuals().warn_fg_color)).on_hover_text(&issue.message);
}

/// The "Edit in YAML" link. Returns true when clicked.
pub(in crate::app) fn yaml_link(ui: &mut Ui) -> bool {
    ui.link(RichText::new("Edit in YAML").small()).clicked()
}

/// A labelled field: the label row, the field `add` draws, then the
/// problem at that field (if any) in the warning color. Returns what `add`
/// returns.
pub(in crate::app) fn labelled_field<R>(
    ui: &mut Ui,
    label: &str,
    key: &str,
    required: bool,
    issue: Option<&Issue>,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    field_label(ui, label, key, required);
    ui.add_space(LABEL_GAP);
    let inner = add(ui);
    if let Some(issue) = issue {
        issue_line(ui, &issue.message);
    }
    ui.add_space(FIELD_GAP);
    inner
}

/// A one-line text field filling the width, in the Settings field box
/// (warning border when `warn`). `hint` shows while it is empty.
pub(in crate::app) fn text_field(ui: &mut Ui, value: &mut String, hint: &str, warn: bool) -> egui::Response {
    let visuals = ui.visuals().clone();
    let mut response = None;
    field_box(ui, &visuals, warn, |ui| {
        let edit =
            egui::TextEdit::singleline(value).hint_text(hint).frame(egui::Frame::NONE).desired_width(f32::INFINITY);
        response = Some(ui.add(edit));
    });
    response.expect("field_box always runs its contents")
}

/// A one-line monospace field of `width` in a list row.
fn row_field(ui: &mut Ui, value: &mut String, hint_text: &str, width: f32) -> egui::Response {
    ui.add(egui::TextEdit::singleline(value).hint_text(hint_text).font(egui::TextStyle::Monospace).desired_width(width))
}

/// A row of pills, one per `(label, value)` choice, the one equal to
/// `current` filled. A click sets `current`. Returns true when it changed.
pub(in crate::app) fn pill_row(ui: &mut Ui, current: &mut String, choices: &[(&str, &str)]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (label, value) in choices {
            if pill(ui, label, current == value).clicked() && current != value {
                (*value).clone_into(current);
                changed = true;
            }
        }
    });
    changed
}

/// One pill: filled with the selection color when `selected`.
pub(in crate::app) fn pill(ui: &mut Ui, label: &str, selected: bool) -> egui::Response {
    let visuals = ui.visuals();
    let (fill, stroke) = if selected {
        (visuals.selection.bg_fill, visuals.selection.stroke)
    } else {
        (visuals.widgets.inactive.weak_bg_fill, visuals.widgets.noninteractive.bg_stroke)
    };
    let text = RichText::new(label).monospace();
    let button =
        egui::Button::new(text).fill(fill).stroke(stroke).corner_radius(PILL_RADIUS).min_size(egui::vec2(0.0, 0.0));
    // Set the padding on `ui` itself, not in a `scope`: a scope takes the
    // rest of the line, so a wrapping row could not move the pill to the
    // next line and would grow wider instead.
    let padding = std::mem::replace(
        &mut ui.spacing_mut().button_padding,
        egui::vec2(f32::from(PILL_PADDING_X), f32::from(PILL_PADDING_Y)),
    );
    let response = ui.add(button);
    ui.spacing_mut().button_padding = padding;
    response
}

/// A drop-down of `(label, value)` choices that sets `value`. A value not
/// in the list shows as written.
pub(in crate::app) fn choice(ui: &mut Ui, id: Id, value: &mut String, choices: &[(&str, &str)]) {
    let selected = choices.iter().find(|(_, choice)| choice == value).map_or(value.as_str(), |(label, _)| label);
    egui::ComboBox::from_id_salt(id).selected_text(RichText::new(selected).monospace()).width(CHOICE_WIDTH).show_ui(
        ui,
        |ui| {
            for (label, choice) in choices {
                if ui.selectable_label(value == choice, RichText::new(*label).monospace()).clicked() {
                    (*choice).clone_into(value);
                }
            }
        },
    );
}

/// A small cross button with a fixed `id` (so a test can find it).
/// Returns true when clicked. `label` is what screen readers announce.
pub(in crate::app) fn remove_button(ui: &mut Ui, id: Id, label: &str, tip: &str) -> bool {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(REMOVE_SIZE, REMOVE_SIZE), egui::Sense::hover());
    small_icon_button_on(ui, rect, id, Icon::Close, label, tip).clicked()
}

/// The id of the remove button of item `index` in the list `list` of the
/// screen part `owner` (`team`, `seat`).
pub(in crate::app) fn remove_id(owner: &str, list: &str, index: usize) -> Id {
    Id::unique(("team-editor-remove", owner, list, index))
}

/// Permissions (`permission_policy`): a pill for "not set" (labelled
/// `unset_label`), one per built-in policy, and a field for a policy file;
/// then the value that gets written, or `unset_hint` when there is none.
pub(in crate::app) fn permissions_field(
    ui: &mut Ui,
    policy: &mut String,
    unset_label: &str,
    unset_hint: &str,
    issue: Option<&Issue>,
) {
    let mut choices = vec![(unset_label, "")];
    choices.extend(PERMISSION_LABELS.into_iter().zip(PERMISSION_POLICIES));
    pill_row(ui, policy, &choices);
    ui.add_space(LABEL_GAP);
    let is_file = !policy.is_empty() && !PERMISSION_POLICIES.contains(&policy.as_str());
    let mut file = if is_file { policy.clone() } else { String::new() };
    if text_field(ui, &mut file, "or a policy file", issue.is_some()).changed() {
        *policy = file;
    }
    if let Some(issue) = issue {
        issue_line(ui, &issue.message);
    }
    if policy.is_empty() {
        hint(ui, unset_hint);
    } else {
        ui.horizontal(|ui| {
            hint(ui, "Saves as");
            spec_key(ui, policy);
        });
    }
}

/// An environment list (`env`): one row per variable (name turned to
/// capitals as typed, value, remove) with its problem, and "+ Add
/// variable". `prefix` is the list's validator path (`env`,
/// `pods[0].members[1].env`) and `owner` names the screen part for the
/// remove buttons' ids.
pub(in crate::app) fn env_list(ui: &mut Ui, env: &mut Vec<EnvVar>, issues: &[Issue], prefix: &str, owner: &str) {
    let mut remove = None;
    if !env.is_empty() {
        list_frame(ui, |ui| {
            for (index, var) in env.iter_mut().enumerate() {
                if index > 0 {
                    ui.separator();
                }
                if variable_row(ui, var, remove_id(owner, "env", index)) {
                    remove = Some(index);
                }
                if let Some(issue) = issue_at(issues, &format!("{prefix}.{}", var.name)) {
                    egui::Frame::new().inner_margin(ROW_MARGIN).show(ui, |ui| issue_line(ui, &issue.message));
                }
            }
        });
    }
    if let Some(index) = remove {
        env.remove(index);
    }
    if ui.add(egui::Button::new("+ Add variable").frame(false)).clicked() {
        env.push(EnvVar::default());
    }
}

/// One variable: name, `=`, value filling the row, and remove. Returns true
/// when remove was clicked.
fn variable_row(ui: &mut Ui, var: &mut EnvVar, remove: Id) -> bool {
    let mut removed = false;
    egui::Frame::new().inner_margin(ROW_MARGIN).show(ui, |ui| {
        ui.horizontal(|ui| {
            if row_field(ui, &mut var.name, "NAME", NAME_FIELD_WIDTH).changed() {
                var.name = var.name.to_uppercase();
            }
            ui.label("=");
            let spacing = ui.spacing().item_spacing.x;
            let value_width = (ui.available_width() - REMOVE_SIZE - 2.0 * spacing).max(0.0);
            row_field(ui, &mut var.value, "value", value_width);
            removed = remove_button(ui, remove, "Remove variable", "Remove this variable");
        });
    });
    removed
}

/// A startup file list (`startup.files`): per file its path and remove,
/// then how it is delivered and whether it is required, with its problems;
/// "+ Add startup file"; and the number of startup actions, which are
/// edited in YAML. `prefix` is the validator path of the `startup` block's
/// owner (empty for the team). Returns true when "Edit in YAML" was clicked.
pub(in crate::app) fn startup_file_list(
    ui: &mut Ui,
    startup: &mut Startup,
    issues: &[Issue],
    prefix: &str,
    owner: &str,
) -> bool {
    let mut remove = None;
    if !startup.files.is_empty() {
        list_frame(ui, |ui| {
            for (index, file) in startup.files.iter_mut().enumerate() {
                if index > 0 {
                    ui.separator();
                }
                if startup_file_row(ui, file, (owner, index)) {
                    remove = Some(index);
                }
                let path = format!("{prefix}startup.files[{index}]");
                egui::Frame::new().inner_margin(ROW_MARGIN).show(ui, |ui| {
                    for key in ["path", "delivery_hint", "required", "applies_on"] {
                        if let Some(issue) = issue_at(issues, &format!("{path}.{key}")) {
                            issue_line(ui, &format!("{key}: {}", issue.message));
                        }
                    }
                });
            }
        });
    }
    if let Some(index) = remove {
        startup.files.remove(index);
    }
    let mut show_yaml = false;
    ui.horizontal_wrapped(|ui| {
        if ui.add(egui::Button::new("+ Add startup file").frame(false)).clicked() {
            startup.files.push(StartupFile::new(""));
        }
        if startup.action_count > 0 {
            let actions = plural(startup.action_count, "action", "actions");
            hint(ui, &format!("{} startup {actions}, kept as written.", startup.action_count));
            show_yaml = yaml_link(ui);
        }
    });
    show_yaml
}

/// One startup file: path and remove on the first line, delivery and
/// required on the second. `place` is the owner and index, for the ids.
/// Returns true when remove was clicked.
fn startup_file_row(ui: &mut Ui, file: &mut StartupFile, place: (&str, usize)) -> bool {
    let (owner, index) = place;
    let mut removed = false;
    egui::Frame::new().inner_margin(ROW_MARGIN).show(ui, |ui| {
        ui.horizontal(|ui| {
            let width = (ui.available_width() - REMOVE_SIZE - ui.spacing().item_spacing.x).max(0.0);
            row_field(ui, &mut file.path, "guidance/first-move.md", width);
            removed = remove_button(ui, remove_id(owner, "startup", index), "Remove startup file", "Remove this file");
        });
        ui.horizontal(|ui| {
            // An absent `delivery_hint` means auto.
            let mut hints = vec![("auto", "")];
            hints.extend(DELIVERY_HINTS.iter().filter(|hint| **hint != "auto").map(|hint| (*hint, *hint)));
            choice(ui, Id::unique(("team-editor-delivery", owner, index)), &mut file.delivery_hint, &hints);
            // An absent `required` means true; unticking writes `false`.
            let mut required = file.required.unwrap_or(true);
            if ui.checkbox(&mut required, "Required").changed() {
                file.required = if required { None } else { Some(false) };
            }
        });
    });
    removed
}

/// "Kept as written": the keys under this part that the form leaves to the
/// YAML, with "Edit in YAML". Draws nothing when there are none. Returns
/// true when the link was clicked.
pub(in crate::app) fn kept_as_written(ui: &mut Ui, keys: &[String]) -> bool {
    if keys.is_empty() {
        return false;
    }
    let mut show_yaml = false;
    ui.horizontal_wrapped(|ui| {
        hint(ui, "Kept as written:");
        for key in keys {
            spec_key(ui, key);
        }
        show_yaml = yaml_link(ui);
    });
    show_yaml
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Teams shipped inside `OpenRig` are read-only; a user's are not.
    #[test]
    fn built_in_teams_are_read_only() {
        assert!(is_read_only(Path::new("/opt/openrig/packages/daemon/specs/rigs/launch/starter/rig.yaml")));
        assert!(is_read_only(Path::new("/usr/lib/node_modules/@openrig/daemon/specs/rigs/launch/factory/rig.yaml")));
        assert!(!is_read_only(Path::new("/home/me/.openrig/specs/my-team/rig.yaml")));
    }

    /// A local file is checked by this computer's rig.
    #[test]
    fn local_specs_use_the_local_rig() {
        let path = Path::new("/home/me/.openrig/specs/my-team/rig.yaml");
        assert_eq!(rig_target(path), (RigHost::Local, path.to_path_buf()));
    }

    /// Only relative paths that stay inside their folder are safe.
    #[test]
    fn safe_paths_stay_inside_the_folder() {
        for path in ["CULTURE.md", "culture/./RULES.md", "guidance/first.md"] {
            assert!(is_safe_relative(path), "{path}");
        }
        for path in ["", "  ", "../other/CULTURE.md", "/etc/passwd", "culture/../../x.md"] {
            assert!(!is_safe_relative(path), "{path:?}");
        }
    }
}
