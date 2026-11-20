//! Compose and track `OpenRig` handoffs from a document's Tasks panel.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::{self, Context, Ui};

use super::{OpenFile, RubriqueApp, char_to_byte};
use crate::document::{preview_selection, view::editor_selection};
use crate::openrig::{Inventory, MAX_BODY_BYTES, Rig, TaskLink, TaskStatus, store};
use crate::settings::openrig_tasks_folder;

const REFRESH_INTERVAL: Duration = Duration::from_secs(8);

/// The draft shown before the user sends any text.
struct Draft {
    document: PathBuf,
    body: String,
    inventory: Option<Result<Inventory, String>>,
    seat: Option<usize>,
    error: Option<String>,
}

/// A saved link and its latest observed state.
struct TrackedTask {
    link: TaskLink,
    status: Option<TaskStatus>,
    error: Option<String>,
    next_refresh: Option<Instant>,
}

impl TrackedTask {
    /// Refresh a saved task the first time it appears.
    fn new(link: TaskLink) -> Self {
        Self { link, status: None, error: None, next_refresh: Some(Instant::now()) }
    }

    /// Store the latest result, keeping older status visible during failures.
    fn update(&mut self, result: Result<TaskStatus, String>) {
        self.next_refresh = Some(Instant::now() + REFRESH_INTERVAL);
        match result {
            Ok(status) => {
                if status.is_closed() {
                    self.next_refresh = None;
                }
                self.status = Some(status);
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
    }
}

/// `OpenRig`'s seat picker and the task links for open documents.
#[derive(Default)]
pub(super) struct OpenRigPanel {
    draft: Option<Draft>,
    tasks: Option<Result<Vec<TrackedTask>, String>>,
    discover: Option<Receiver<Result<Inventory, String>>>,
    submit: Option<QueueRequest>,
    refresh: Option<QueueRequest>,
}

impl std::fmt::Debug for OpenRigPanel {
    /// Exclude task text and responses from application debug output.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("OpenRigPanel").finish_non_exhaustive()
    }
}

/// A worker reading or submitting one saved queue link.
struct QueueRequest {
    id: String,
    response: Receiver<Result<TaskStatus, String>>,
}

impl RubriqueApp {
    /// Show the composer and the current document's saved tasks.
    pub(super) fn openrig_tasks(&mut self, ui: &mut Ui) {
        let OpenFile::Text { document, .. } = &self.open_file else { return };
        if document.is_encrypted() || crate::remote::is_mirror(&document.path) {
            self.openrig.draft = None;
            return;
        }
        let path = document.path.clone();
        self.openrig.prepare(ui.ctx(), &path);
        if self.openrig.draft.is_some() {
            self.openrig.show_draft(ui);
        } else {
            let button = ui.button("Send to OpenRig").on_hover_text("Select text in the document to fill in the step.");
            let visible = button.rect.intersect(ui.clip_rect());
            preview_selection::set_openrig_button_rect(ui.ctx(), visible);
            if button.clicked() {
                let body = self.selected_openrig_step(ui.ctx()).unwrap_or_default();
                self.openrig.compose(ui.ctx(), path.clone(), body);
            }
        }
        self.openrig.show_tasks(ui, &path);
    }

    /// Capture only the current editor or document preview selection.
    fn selected_openrig_step(&self, ctx: &Context) -> Option<String> {
        let OpenFile::Text { document, .. } = &self.open_file else { return None };
        if self.showing_preview() && self.block_edit.is_none() {
            let selection = preview_selection::current(ctx)?;
            return self.ask.page_rect.filter(|page| page.contains(selection.pointer)).map(|_| selection.text);
        }
        if self.block_edit.is_some() {
            return None;
        }
        let (first, last) = editor_selection(ctx)?;
        let range = char_to_byte(&document.text, first)..char_to_byte(&document.text, last);
        document.text.get(range).map(str::to_owned)
    }
}

impl OpenRigPanel {
    /// Load the task links, then collect and schedule work for the visible document.
    fn prepare(&mut self, ctx: &Context, path: &Path) {
        if self.draft.as_ref().is_some_and(|draft| draft.document != path) {
            self.draft = None;
            self.discover = None;
        }
        if let Some(reply) = poll(&mut self.discover)
            && let Some(draft) = &mut self.draft
        {
            draft.inventory = Some(reply);
        }
        let tasks = self.tasks.get_or_insert_with(load_tasks);
        if let Ok(tasks) = tasks {
            collect(&mut self.submit, tasks);
            collect(&mut self.refresh, tasks);
            if self.refresh.is_none() {
                self.refresh = refresh_task(ctx, path, tasks, self.submit.as_ref());
            }
        }
    }

    /// Show the body preview and start finding available seats.
    fn compose(&mut self, ctx: &Context, document: PathBuf, body: String) {
        self.draft = Some(Draft { document, body, inventory: None, seat: None, error: None });
        self.discover = Some(work(ctx, || Rig::find()?.inventory()));
    }

    /// Render the composer and handle its buttons.
    fn show_draft(&mut self, ui: &mut Ui) {
        let Some(draft) = &mut self.draft else { return };
        ui.strong("Send to OpenRig");
        seat_picker(ui, draft);
        ui.label("Document step");
        ui.add(
            egui::TextEdit::multiline(&mut draft.body)
                .desired_rows(4)
                .desired_width(f32::INFINITY)
                .hint_text("Paste or type the step to send"),
        );
        ui.weak("OpenRig saves this text in its queue. The seat works with its configured permissions.");
        if draft.body.len() > MAX_BODY_BYTES {
            ui.label("The step is too long. Use up to 120 KB.");
        }
        if let Some(error) = &draft.error {
            ui.label(error);
        }
        let can_send = draft.seat.is_some()
            && !draft.body.trim().is_empty()
            && draft.body.len() <= MAX_BODY_BYTES
            && self.submit.is_none();
        let (send, cancel, reload) = ui
            .horizontal_wrapped(|ui| {
                let send = ui.add_enabled(can_send, egui::Button::new("Send")).clicked();
                let cancel = ui.button("Cancel").clicked();
                let reload = ui.add_enabled(self.discover.is_none(), egui::Button::new("Reload seats")).clicked();
                (send, cancel, reload)
            })
            .inner;
        if send {
            self.submit_draft(ui.ctx());
        } else if cancel {
            self.draft = None;
            self.discover = None;
        } else if reload {
            draft.seat = None;
            draft.inventory = None;
            self.discover = Some(work(ui.ctx(), || Rig::find()?.inventory()));
        }
        ui.separator();
    }

    /// Save the task link before launching the one queue create request.
    fn submit_draft(&mut self, ctx: &Context) {
        let Some(draft) = &mut self.draft else { return };
        let link = match save_draft(draft) {
            Ok(link) => link,
            Err(error) => {
                draft.error = Some(error);
                return;
            }
        };
        let body = std::mem::take(&mut draft.body);
        self.draft = None;
        if !matches!(self.tasks, Some(Ok(_))) {
            self.tasks = Some(load_tasks());
        }
        if let Some(Ok(tasks)) = &mut self.tasks
            && !tasks.iter().any(|task| task.link.id == link.id)
        {
            tasks.insert(0, TrackedTask::new(link.clone()));
        }
        self.submit =
            Some(QueueRequest { id: link.id.clone(), response: work(ctx, move || Rig::find()?.send(&link, body)) });
    }

    /// Show the durable references and their current state for this file.
    fn show_tasks(&mut self, ui: &mut Ui, path: &Path) {
        match &mut self.tasks {
            Some(Ok(tasks)) => {
                for task in tasks.iter_mut().filter(|task| task.link.document == path) {
                    let sending = self.submit.as_ref().is_some_and(|request| request.id == task.link.id);
                    let refreshing = self.refresh.as_ref().is_some_and(|request| request.id == task.link.id);
                    task_card(ui, task, sending, refreshing);
                }
            }
            Some(Err(error)) => {
                ui.label(error.as_str());
                if ui.button("Reload saved tasks").clicked() {
                    self.tasks = Some(load_tasks());
                }
            }
            None => {}
        }
    }
}

/// Render the discovered seats and any partial inventory result.
fn seat_picker(ui: &mut Ui, draft: &mut Draft) {
    match &draft.inventory {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Finding OpenRig seats");
            });
        }
        Some(Err(error)) => {
            ui.label(error);
        }
        Some(Ok(inventory)) => {
            if inventory.partial {
                ui.weak("OpenRig returned a partial seat list.");
            }
            if inventory.seats.is_empty() {
                ui.label("Start an OpenRig team, then reload seats.");
                return;
            }
            let selected = draft.seat.map_or("Choose a seat", |index| &inventory.seats[index].address);
            egui::ComboBox::from_id_salt("openrig-seat").selected_text(selected).show_ui(ui, |ui| {
                for (index, seat) in inventory.seats.iter().enumerate() {
                    ui.selectable_value(&mut draft.seat, Some(index), &seat.address);
                }
            });
        }
    }
}

/// Save a link for the selected seat without saving the document step.
fn save_draft(draft: &Draft) -> Result<TaskLink, String> {
    let Some(Ok(inventory)) = &draft.inventory else { return Err("Reload the OpenRig seats first.".into()) };
    let seat = draft.seat.and_then(|index| inventory.seats.get(index)).ok_or("Choose an OpenRig seat.")?;
    let folder = openrig_tasks_folder().ok_or("Cannot find the Rubrique settings folder.")?;
    let link = TaskLink {
        id: store::new_id()?,
        document: draft.document.clone(),
        endpoint: inventory.endpoint,
        seat: seat.clone(),
    };
    store::save(&folder, &link)
        .map_err(|_| "Cannot save the OpenRig task link. Check the Rubrique settings folder.")?;
    Ok(link)
}

/// Read saved task references once when the Tasks panel opens.
fn load_tasks() -> Result<Vec<TrackedTask>, String> {
    let folder = openrig_tasks_folder().ok_or("Cannot find the Rubrique settings folder.")?;
    let links = store::load(&folder).map_err(|_| "Cannot read the saved OpenRig task links.")?;
    Ok(links.into_iter().map(TrackedTask::new).collect())
}

/// Record a worker's response even if the user switched documents.
fn collect(request: &mut Option<QueueRequest>, tasks: &mut [TrackedTask]) {
    let Some(job) = request else { return };
    if let Some(reply) = receive(&job.response) {
        if let Some(task) = tasks.iter_mut().find(|task| task.link.id == job.id) {
            task.update(reply);
        }
        *request = None;
    }
}

/// Refresh one due task at a time. The timer runs only while the panel is visible.
fn refresh_task(
    ctx: &Context,
    path: &Path,
    tasks: &[TrackedTask],
    submitting: Option<&QueueRequest>,
) -> Option<QueueRequest> {
    let (next, link) = tasks
        .iter()
        .filter(|task| task.link.document == path)
        .filter(|task| submitting.is_none_or(|job| job.id != task.link.id))
        .filter_map(|task| Some((task.next_refresh?, &task.link)))
        .min_by_key(|(next, _)| *next)?;
    if next > Instant::now() {
        ctx.request_repaint_after(next.saturating_duration_since(Instant::now()));
        return None;
    }
    let link = link.clone();
    Some(QueueRequest { id: link.id.clone(), response: work(ctx, move || Rig::find()?.read(&link)) })
}

/// Run a CLI operation away from the UI and redraw when it returns.
fn work<T: Send + 'static>(
    ctx: &Context,
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Receiver<Result<T, String>> {
    let (sender, receiver) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let _ = sender.send(operation());
        ctx.request_repaint();
    });
    receiver
}

/// Read a worker response without blocking.
fn receive<T>(receiver: &Receiver<Result<T, String>>) -> Option<Result<T, String>> {
    match receiver.try_recv() {
        Ok(result) => Some(result),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(Err("The OpenRig request ended unexpectedly.".into())),
    }
}

/// Consume a completed one-off request.
fn poll<T>(request: &mut Option<Receiver<Result<T, String>>>) -> Option<Result<T, String>> {
    let reply = receive(request.as_ref()?)?;
    *request = None;
    Some(reply)
}

/// Show the latest queue state and allow an immediate refresh.
fn task_card(ui: &mut Ui, task: &mut TrackedTask, sending: bool, refreshing: bool) {
    ui.separator();
    ui.strong(&task.link.seat.address);
    ui.label(egui::RichText::new(&task.link.id).monospace()).on_hover_text("OpenRig queue ID");
    if sending {
        ui.weak("Sending to OpenRig");
        return;
    }
    if let Some(status) = &task.status {
        show_status(ui, status, &task.link);
    } else {
        ui.weak("Waiting for OpenRig confirmation");
    }
    if let Some(error) = &task.error {
        ui.weak(error);
    }
    let check = egui::Button::new(if refreshing { "Checking…" } else { "Check now" }).small();
    if ui.add_enabled(!refreshing, check).clicked() {
        task.next_refresh = Some(Instant::now());
        ui.ctx().request_repaint();
    }
}

/// Show the latest seat, state and resolution as plain text.
fn show_status(ui: &mut Ui, status: &TaskStatus, link: &TaskLink) {
    let state = match status.state.as_str() {
        "pending" => "Queued",
        "in-progress" if status.claimed => "Claimed",
        "in-progress" => "In progress",
        "blocked" => "Blocked",
        "done" => "Done",
        "denied" => "Denied",
        "canceled" => "Canceled",
        "failed" => "Failed",
        "handed-off" => "Handed off",
        other => other,
    };
    ui.label(state);
    if status.id != link.id {
        ui.weak(format!("Handed to {}", status.seat));
    }
    if let Some(closure) = &status.closure {
        ui.weak(closure.replace(['-', '_'], " "));
    }
    if let Some(resolution) = &status.resolution {
        ui.label(resolution);
    }
    if let Some(note) = &status.handoff_note {
        ui.weak(note);
    }
}
