//! The "Add seat" and "Seat setup" dialogs for `OpenRig`: add a seat to a
//! rig with its provider, model, reasoning effort, permissions, working
//! folder and first instructions, or change an existing seat's settings.
//! rig runs on a background thread; the dialog waits with a spinner and, on
//! a failure, says which step failed.

use eframe::egui::{self, Context, RichText, Ui};

use super::RubriqueApp;
use super::seats::SeatPlace;
use crate::alert::{self, ButtonKind};
use crate::openrig::seat_setup::{
    PermissionMode, Runtime, SeatSettings, SeatSetup, SeatState, validate, validate_settings,
};
use crate::openrig::{Rig, RigHost};
use crate::remote::task::{Task, spawn_with_repaint};
use crate::terminal::groups::seat_of;

/// Width of the dialog.
const DIALOG_WIDTH: f32 = 480.0;
/// Visible lines of the first instructions field.
const INSTRUCTION_ROWS: usize = 4;
/// Label of the effort choice that leaves effort to the runtime.
const DEFAULT_EFFORT: &str = "Runtime default";

/// State of the "Add seat" or "Seat setup" dialog while it is open.
pub(super) struct SeatSetupDialog {
    /// The machine the rig runs on.
    host: RigHost,
    target: Target,
    form: Form,
    /// Why the last attempt failed, shown above the buttons.
    error: Option<String>,
    /// The rig work in flight.
    job: Option<Job>,
}

impl std::fmt::Debug for SeatSetupDialog {
    /// Leave out the first instructions, which can hold private task text.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("SeatSetupDialog").field("host", &self.host).finish_non_exhaustive()
    }
}

/// What the dialog sets up.
enum Target {
    /// A new seat in the rig named `rig_name`, which has these pods.
    Add { rig_name: String, pods: Vec<String> },
    /// Seat `seat`, with its setup once it has been read.
    Edit { seat: String, before: Option<SeatState> },
}

/// Rig work running for the dialog.
enum Job {
    /// Reading the seat's setup to fill the form.
    Load(Task<Result<SeatState, String>>),
    /// Adding or saving; the result is the status bar message.
    Save(Task<Result<String, String>>),
}

/// A finished [`Job`]'s result.
enum Finished {
    Loaded(Result<SeatState, String>),
    Saved(Result<String, String>),
}

/// The values typed and chosen in the dialog.
struct Form {
    name: String,
    /// The chosen existing pod's index, or `None` for a new pod.
    pod: Option<usize>,
    /// The new pod's name.
    new_pod: String,
    runtime: Runtime,
    model: String,
    /// `None` for the runtime's default effort.
    effort: Option<String>,
    /// `None` for the rig's default (adding) or unchanged (editing).
    permissions: Option<PermissionMode>,
    folder: String,
    instructions: String,
    /// Restart the seat after saving (editing only).
    restart: bool,
}

impl Form {
    /// An empty form, in the first pod when there is one.
    fn new(has_pods: bool) -> Self {
        Form {
            name: String::new(),
            pod: has_pods.then_some(0),
            new_pod: String::new(),
            runtime: Runtime::ClaudeCode,
            model: String::new(),
            effort: None,
            permissions: None,
            folder: String::new(),
            instructions: String::new(),
            restart: false,
        }
    }
}

/// What the user did in the dialog this frame.
#[derive(Clone, Copy, Default)]
struct DialogInput {
    save: bool,
    cancel: bool,
}

impl RubriqueApp {
    /// Open "Add seat" for the rig of `seats` (its seats' session names),
    /// offering the pods those seats are in.
    pub(super) fn start_add_seat(&mut self, seats: &[String]) {
        let Some(place) = seats.first().and_then(|name| self.seat_place(name)) else {
            return;
        };
        let Some((_, rig_name)) = place.address.split_once('@') else {
            return;
        };
        let mut pods: Vec<String> =
            seats.iter().filter_map(|name| seat_of(name)).map(|(_, pod)| pod.to_owned()).collect();
        pods.sort();
        pods.dedup();
        let form = Form::new(!pods.is_empty());
        self.seat_setup = Some(SeatSetupDialog {
            host: place.host.map_or(RigHost::Local, RigHost::Ssh),
            target: Target::Add { rig_name: rig_name.to_owned(), pods },
            form,
            error: None,
            job: None,
        });
    }

    /// Open "Seat setup" for the seat at `place` and read its setup in the background.
    pub(super) fn start_seat_setup(&mut self, ctx: &Context, place: SeatPlace) {
        let host = place.host.map_or(RigHost::Local, RigHost::Ssh);
        let (task_host, seat) = (host.clone(), place.address.clone());
        let task = spawn_with_repaint(ctx, move || Rig::open(&task_host)?.read_setup(&seat));
        self.seat_setup = Some(SeatSetupDialog {
            host,
            target: Target::Edit { seat: place.address, before: None },
            form: Form::new(false),
            error: None,
            job: Some(Job::Load(task)),
        });
    }

    /// Draw the seat setup dialog while it is open, and pick up finished work.
    pub(super) fn seat_setup_dialog(&mut self, ctx: &Context) {
        self.finish_seat_setup_job();
        let Some(dialog) = &mut self.seat_setup else {
            return;
        };
        let modal = alert::show(ctx, "seat-setup", DIALOG_WIDTH, |ui| dialog_contents(ui, dialog));
        // A started add or save cannot be stopped halfway, so the dialog waits for it.
        if matches!(dialog.job, Some(Job::Save(_))) {
            return;
        }
        if modal.inner.cancel || modal.should_close() {
            self.seat_setup = None;
        } else if modal.inner.save {
            dialog.start_save(ctx);
        }
    }

    /// Fill the form once the seat's setup is read; close the dialog and
    /// refresh the seat list once an add or save is done.
    fn finish_seat_setup_job(&mut self) {
        let Some(dialog) = &mut self.seat_setup else {
            return;
        };
        let finished = match &dialog.job {
            Some(Job::Load(task)) => task.take().map(Finished::Loaded),
            Some(Job::Save(task)) => task.take().map(Finished::Saved),
            None => None,
        };
        let Some(finished) = finished else {
            return;
        };
        dialog.job = None;
        match finished {
            Finished::Loaded(result) => dialog.loaded(result),
            Finished::Saved(Err(error)) => dialog.error = Some(error),
            Finished::Saved(Ok(message)) => {
                self.seat_setup = None;
                self.show_status(message);
                if let Some(monitor) = &self.terminals {
                    monitor.refresh();
                }
            }
        }
    }
}

impl SeatSetupDialog {
    /// Fill the form from the seat's setup, or show why it could not be read.
    fn loaded(&mut self, result: Result<SeatState, String>) {
        let Target::Edit { before, .. } = &mut self.target else {
            return;
        };
        match result {
            Ok(state) => {
                let settings = &state.settings;
                self.form.model = settings.model.clone().unwrap_or_default();
                self.form.effort.clone_from(&settings.effort);
                self.form.permissions = settings.permissions;
                self.form.folder = settings.folder.clone().unwrap_or_default();
                if let Some(runtime) = Runtime::from_id(&state.runtime) {
                    self.form.runtime = runtime;
                }
                *before = Some(state);
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// Start adding the seat or saving its changes in the background.
    fn start_save(&mut self, ctx: &Context) {
        let host = self.host.clone();
        let task = match &self.target {
            Target::Add { rig_name, pods } => {
                let (rig_name, setup) = (rig_name.clone(), add_setup(&self.form, pods));
                spawn_with_repaint(ctx, move || {
                    let seat = Rig::open(&host)?.add_seat(&rig_name, &setup)?;
                    Ok(format!("Added seat {seat}"))
                })
            }
            Target::Edit { seat, before: Some(state) } => {
                let (seat, before) = (seat.clone(), state.settings.clone());
                let (after, restart) = (form_settings(&self.form), self.form.restart);
                spawn_with_repaint(ctx, move || {
                    let changed = Rig::open(&host)?.save_setup(&seat, &before, &after, restart)?;
                    Ok(saved_message(&seat, changed, restart))
                })
            }
            Target::Edit { before: None, .. } => return,
        };
        self.error = None;
        self.job = Some(Job::Save(task));
    }

    /// The first value that cannot be saved, or `None` when the form is
    /// ready. An empty text means not ready, with nothing to say yet: a new
    /// seat without a name, or an edit whose setup is not read.
    fn problem(&self) -> Option<String> {
        match &self.target {
            Target::Add { .. } if self.form.name.trim().is_empty() => Some(String::new()),
            Target::Add { pods, .. } => validate(&add_setup(&self.form, pods)).err(),
            Target::Edit { before: Some(_), .. } => validate_settings(&form_settings(&self.form)).err(),
            Target::Edit { before: None, .. } => Some(String::new()),
        }
    }
}

/// The new seat the form describes.
fn add_setup(form: &Form, pods: &[String]) -> SeatSetup {
    let (pod, new_pod) = match form.pod.and_then(|index| pods.get(index)) {
        Some(pod) => (pod.clone(), false),
        None => (form.new_pod.trim().to_owned(), true),
    };
    SeatSetup {
        name: form.name.trim().to_owned(),
        pod,
        new_pod,
        runtime: form.runtime,
        settings: form_settings(form),
        instructions: form.instructions.clone(),
    }
}

/// The settings the form describes; an empty field is `None`.
fn form_settings(form: &Form) -> SeatSettings {
    SeatSettings {
        model: filled(&form.model),
        effort: form.effort.clone(),
        permissions: form.permissions,
        folder: filled(&form.folder),
    }
}

/// `text` without surrounding spaces, or `None` when that leaves nothing.
fn filled(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// The status bar message after a save.
fn saved_message(seat: &str, changed: bool, restart: bool) -> String {
    match (changed, restart) {
        (false, _) => format!("No changes to save for {seat}"),
        (true, true) => format!("Saved the setup of {seat} and restarted it"),
        (true, false) => format!("Saved the setup of {seat}. It applies at the seat's next launch"),
    }
}

/// The dialog's title, fields and buttons; returns what the user did.
fn dialog_contents(ui: &mut Ui, dialog: &mut SeatSetupDialog) -> DialogInput {
    let busy = dialog.job.is_some();
    heading(ui, dialog);
    let problem = dialog.problem();
    let ready = !matches!(dialog.target, Target::Edit { before: None, .. });
    if ready {
        ui.add_enabled_ui(!busy, |ui| form_fields(ui, dialog));
        if let Some(problem) = problem.as_deref().filter(|problem| !problem.is_empty()) {
            ui.weak(problem);
        }
    }
    progress(ui, dialog);
    if let Some(error) = &dialog.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    let can_save = !busy && problem.is_none();
    let label = if matches!(dialog.target, Target::Add { .. }) { "Add seat" } else { "Save" };
    let ((), (save, cancel)) = alert::button_row(
        ui,
        |_| (),
        |ui| {
            let save = ui.add_enabled_ui(can_save, |ui| alert::button(ui, label, ButtonKind::Primary, true)).inner;
            let cancel = alert::button(ui, "Cancel", ButtonKind::Secondary, false);
            (save.clicked(), cancel.clicked())
        },
    );
    DialogInput { save: can_save && (save || alert::enter_for_default(ui)), cancel }
}

/// The title and the line under it.
fn heading(ui: &mut Ui, dialog: &SeatSetupDialog) {
    let place = match &dialog.host {
        RigHost::Local => String::new(),
        RigHost::Ssh(host) => format!(" on {host}"),
    };
    match &dialog.target {
        Target::Add { rig_name, .. } => {
            alert::title(ui, &format!("Add seat to {rig_name}"));
            alert::body(ui, &format!("OpenRig{place} adds the seat, saves these settings and starts its agent."));
        }
        Target::Edit { seat, .. } => {
            alert::title(ui, &format!("Seat setup for {seat}"));
            alert::body(ui, &format!("OpenRig{place} keeps these settings for the seat."));
        }
    }
}

/// A spinner with what the dialog waits for, while rig works.
fn progress(ui: &mut Ui, dialog: &SeatSetupDialog) {
    let text = match (&dialog.job, &dialog.target) {
        (None, _) => return,
        (Some(Job::Load(_)), _) => "Reading the seat's setup…",
        (Some(Job::Save(_)), Target::Add { .. }) => "Adding the seat. Starting its agent can take a minute…",
        (Some(Job::Save(_)), Target::Edit { .. }) => "Saving…",
    };
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(12.0));
        ui.label(RichText::new(text).weak());
    });
}

/// The form: name, pod and provider for a new seat (the provider only shown
/// for an existing one), then the settings, then the first instructions
/// (new seat) or the restart choice (existing seat).
fn form_fields(ui: &mut Ui, dialog: &mut SeatSetupDialog) {
    let SeatSetupDialog { target, form, .. } = dialog;
    let (runtime, editing) = match target {
        Target::Add { pods, .. } => {
            name_and_pod_fields(ui, form, pods);
            (Some(form.runtime), false)
        }
        Target::Edit { before: Some(state), .. } => {
            provider_line(ui, &state.runtime);
            (Runtime::from_id(&state.runtime), true)
        }
        Target::Edit { before: None, .. } => return,
    };
    ui.columns(2, |columns| {
        if editing {
            model_field(&mut columns[0], form, editing);
        } else {
            runtime_field(&mut columns[0], form);
            model_field(&mut columns[1], form, editing);
        }
    });
    ui.columns(2, |columns| {
        effort_field(&mut columns[0], form, runtime);
        permissions_field(&mut columns[1], form, editing);
    });
    folder_field(ui, form, editing);
    if editing {
        restart_choice(ui, form);
    } else {
        instructions_field(ui, form);
    }
}

/// The seat's name, and its pod: one of the rig's pods or a new one.
fn name_and_pod_fields(ui: &mut Ui, form: &mut Form, pods: &[String]) {
    ui.columns(2, |columns| {
        let ui = &mut columns[0];
        alert::field_label(ui, "Name");
        alert::text_field(
            ui,
            egui::TextEdit::singleline(&mut form.name).hint_text("impl").desired_width(f32::INFINITY),
        );
        let ui = &mut columns[1];
        alert::field_label(ui, "Pod");
        let selected = form.pod.and_then(|index| pods.get(index)).map_or("New pod", String::as_str);
        egui::ComboBox::from_id_salt("seat-setup-pod").selected_text(selected).width(ui.available_width()).show_ui(
            ui,
            |ui| {
                for (index, pod) in pods.iter().enumerate() {
                    ui.selectable_value(&mut form.pod, Some(index), pod);
                }
                ui.selectable_value(&mut form.pod, None, "New pod…");
            },
        );
    });
    if form.pod.is_none() {
        alert::field_label(ui, "New pod name");
        alert::text_field(ui, egui::TextEdit::singleline(&mut form.new_pod).desired_width(f32::INFINITY));
    }
}

/// An existing seat's provider, which this dialog does not change.
fn provider_line(ui: &mut Ui, runtime: &str) {
    let name = Runtime::from_id(runtime).map_or(runtime, |known| known.name());
    alert::field_label(ui, "Provider");
    ui.label(RichText::new(name).strong());
    ui.weak("Changing the provider is a handover in OpenRig (rig seat set-runtime).");
}

/// The provider of a new seat. Choosing another one drops an effort level
/// the new provider does not take.
fn runtime_field(ui: &mut Ui, form: &mut Form) {
    alert::field_label(ui, "Provider");
    egui::ComboBox::from_id_salt("seat-setup-runtime")
        .selected_text(form.runtime.name())
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for runtime in Runtime::ALL {
                ui.selectable_value(&mut form.runtime, runtime, runtime.name());
            }
        });
    let levels = form.runtime.effort_levels();
    if form.effort.as_deref().is_some_and(|level| !levels.contains(&level)) {
        form.effort = None;
    }
}

/// The model id. Empty means the runtime's default for a new seat, and no
/// change for an existing one.
fn model_field(ui: &mut Ui, form: &mut Form, editing: bool) {
    let hint = match (editing, form.runtime) {
        (true, _) => "unchanged",
        (false, Runtime::Pi) => "provider/model",
        (false, _) => "runtime default",
    };
    alert::field_label(ui, "Model");
    alert::text_field(ui, egui::TextEdit::singleline(&mut form.model).hint_text(hint).desired_width(f32::INFINITY));
}

/// The reasoning effort: the runtime's default or one of its levels.
fn effort_field(ui: &mut Ui, form: &mut Form, runtime: Option<Runtime>) {
    alert::field_label(ui, "Reasoning effort");
    let Some(runtime) = runtime else {
        ui.weak("Not available for this provider.");
        return;
    };
    egui::ComboBox::from_id_salt("seat-setup-effort")
        .selected_text(form.effort.as_deref().unwrap_or(DEFAULT_EFFORT))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut form.effort, None, DEFAULT_EFFORT);
            for level in runtime.effort_levels() {
                ui.selectable_value(&mut form.effort, Some((*level).to_owned()), *level);
            }
        });
}

/// The permission mode, or the rig's default (adding) or no change (editing).
fn permissions_field(ui: &mut Ui, form: &mut Form, editing: bool) {
    let unset = if editing { "Unchanged" } else { "Rig default" };
    alert::field_label(ui, "Permissions");
    egui::ComboBox::from_id_salt("seat-setup-permissions")
        .selected_text(form.permissions.map_or(unset, PermissionMode::name))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut form.permissions, None, unset);
            for mode in PermissionMode::ALL {
                ui.selectable_value(&mut form.permissions, Some(mode), mode.name());
            }
        });
}

/// The working folder on the rig's computer.
fn folder_field(ui: &mut Ui, form: &mut Form, editing: bool) {
    let hint = if editing { "unchanged" } else { "/home/me/project (rig default when empty)" };
    alert::field_label(ui, "Working folder");
    let edit = egui::TextEdit::singleline(&mut form.folder)
        .hint_text(hint)
        .font(egui::TextStyle::Monospace)
        .desired_width(f32::INFINITY);
    alert::text_field(ui, edit);
}

/// The text sent to a new seat once its agent runs.
fn instructions_field(ui: &mut Ui, form: &mut Form) {
    alert::field_label(ui, "First instructions");
    let edit = egui::TextEdit::multiline(&mut form.instructions)
        .hint_text("Optional: what the seat should know or do first")
        .desired_rows(INSTRUCTION_ROWS)
        .desired_width(f32::INFINITY);
    alert::text_field(ui, edit);
    ui.weak("Sent as a message once the agent starts. OpenRig has no system prompt setting.");
}

/// Restart now with a blank conversation, or let the change wait for the
/// seat's next launch.
fn restart_choice(ui: &mut Ui, form: &mut Form) {
    ui.checkbox(&mut form.restart, "Restart the seat now (blank conversation)");
    if !form.restart {
        ui.weak("Saved changes apply at the seat's next launch.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new seat takes the chosen pod, or the typed new pod, and leaves
    /// empty fields to `OpenRig`.
    #[test]
    fn the_form_becomes_a_new_seat() {
        let pods = vec!["dev".to_owned(), "qa".to_owned()];
        let mut form = Form::new(true);
        form.name = " impl ".into();
        form.pod = Some(1);
        form.model = "  ".into();
        form.folder = " /work/app ".into();
        let setup = add_setup(&form, &pods);
        assert_eq!((setup.name.as_str(), setup.pod.as_str(), setup.new_pod), ("impl", "qa", false));
        assert_eq!(setup.settings.model, None);
        assert_eq!(setup.settings.folder.as_deref(), Some("/work/app"));
        form.pod = None;
        form.new_pod = "review".into();
        let setup = add_setup(&form, &pods);
        assert_eq!((setup.pod.as_str(), setup.new_pod), ("review", true));
    }

    /// The status bar says whether anything changed and when it applies.
    #[test]
    fn saved_messages_say_when_the_change_applies() {
        assert_eq!(saved_message("dev@demo", false, true), "No changes to save for dev@demo");
        assert!(saved_message("dev@demo", true, true).ends_with("restarted it"));
        assert!(saved_message("dev@demo", true, false).ends_with("next launch"));
    }
}
