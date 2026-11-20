//! Compose and track `OpenRig` handoffs from a document's Tasks panel.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::{self, Context, Ui};

use super::{OpenFile, RubriqueApp, char_to_byte};
use crate::document::{preview_selection, view::editor_selection};
use crate::openrig::{Inventory, MAX_BODY_BYTES, Rig, RigHost, TaskLink, TaskStatus, store};
use crate::remote;
use crate::remote::mirror::is_valid_host;
use crate::settings::openrig_tasks_folder;

const REFRESH_INTERVAL: Duration = Duration::from_secs(8);

/// The draft shown before the user sends any text.
struct Draft {
    document: PathBuf,
    body: String,
    /// The machine whose seats are listed.
    rig: RigHost,
    /// The inline "Add remote rig" field, while it is open.
    new_rig: Option<NewRig>,
    inventory: Option<Result<Inventory, String>>,
    seat: Option<usize>,
    error: Option<String>,
}

/// The SSH host typed for a new remote rig, and the hosts from `~/.ssh/config` to offer.
struct NewRig {
    host: String,
    known_hosts: Vec<String>,
}

/// What the user did in the rig picker.
enum RigEvent {
    /// List the seats of another rig.
    Choose(RigHost),
    /// Add a remote rig by SSH host name and switch to it.
    Add(String),
    /// Remove a remote rig from the list.
    Remove(String),
}

/// What the user did in the inline "Add remote rig" field.
enum NewRigAction {
    /// Still typing.
    None,
    /// Add the typed host.
    Add,
    /// Close the field without adding.
    Cancel,
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
    /// The rig chosen last in this session; the next draft starts on it.
    rig: RigHost,
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
            if self.openrig.show_draft(ui, &mut self.settings.openrig_rigs) {
                self.write_settings();
            }
        } else {
            let button = ui.button("Send to OpenRig").on_hover_text("Select text in the document to fill in the step.");
            let visible = button.rect.intersect(ui.clip_rect());
            preview_selection::set_openrig_button_rect(ui.ctx(), visible);
            if button.clicked() {
                let body = self.selected_openrig_step(ui.ctx()).unwrap_or_default();
                self.openrig.compose(ui.ctx(), path.clone(), body, &self.settings.openrig_rigs);
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

    /// Show the body preview and start finding available seats on the last chosen rig.
    fn compose(&mut self, ctx: &Context, document: PathBuf, body: String, rigs: &[String]) {
        if let RigHost::Ssh(host) = &self.rig
            && !rigs.contains(host)
        {
            self.rig = RigHost::Local;
        }
        let rig = self.rig.clone();
        self.draft =
            Some(Draft { document, body, rig: rig.clone(), new_rig: None, inventory: None, seat: None, error: None });
        self.discover = Some(find_seats(ctx, rig));
    }

    /// Render the composer and handle its buttons. Returns true when the list of remote rigs changed.
    fn show_draft(&mut self, ui: &mut Ui, rigs: &mut Vec<String>) -> bool {
        let Some(draft) = &mut self.draft else { return false };
        ui.strong("Send to OpenRig");
        let event = rig_picker(ui, draft, rigs);
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
            self.discover = Some(find_seats(ui.ctx(), draft.rig.clone()));
        }
        ui.separator();
        event.is_some_and(|event| self.apply_rig_event(ui.ctx(), event, rigs))
    }

    /// Switch to another rig, adding or removing a remote one first.
    /// Returns true when the list of remote rigs changed.
    fn apply_rig_event(&mut self, ctx: &Context, event: RigEvent, rigs: &mut Vec<String>) -> bool {
        let (rig, changed) = match event {
            RigEvent::Choose(rig) => (rig, false),
            RigEvent::Add(host) => {
                let added = !rigs.contains(&host);
                if added {
                    rigs.push(host.clone());
                }
                (RigHost::Ssh(host), added)
            }
            RigEvent::Remove(host) => {
                rigs.retain(|known| *known != host);
                (RigHost::Local, true)
            }
        };
        self.rig = rig.clone();
        if let Some(draft) = &mut self.draft {
            draft.rig = rig.clone();
            draft.seat = None;
            draft.inventory = None;
            draft.error = None;
            self.discover = Some(find_seats(ctx, rig));
        }
        changed
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
        self.submit = Some(QueueRequest {
            id: link.id.clone(),
            response: work(ctx, move || Rig::open(&link.host)?.send(&link, body)),
        });
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

/// Render the rig choice: this computer, the added remote rigs, and a way to add or remove one.
fn rig_picker(ui: &mut Ui, draft: &mut Draft, rigs: &[String]) -> Option<RigEvent> {
    let mut event = None;
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_id_salt("openrig-rig").selected_text(draft.rig.label()).show_ui(ui, |ui| {
            let choices = std::iter::once(RigHost::Local).chain(rigs.iter().cloned().map(RigHost::Ssh));
            for choice in choices {
                let clicked = ui.selectable_label(choice == draft.rig, choice.label()).clicked();
                if clicked && choice != draft.rig {
                    event = Some(RigEvent::Choose(choice));
                }
            }
            if ui.selectable_label(false, "Add remote rig…").clicked() {
                draft.new_rig =
                    Some(NewRig { host: String::new(), known_hosts: remote::ssh_config::user_host_names() });
            }
        });
        if let RigHost::Ssh(host) = &draft.rig
            && ui.small_button("Remove").on_hover_text("Remove this remote rig from the list").clicked()
        {
            event = Some(RigEvent::Remove(host.clone()));
        }
    });
    if let Some(new_rig) = &mut draft.new_rig {
        match new_rig_fields(ui, new_rig, rigs) {
            NewRigAction::Add => {
                event = Some(RigEvent::Add(new_rig.host.trim().to_owned()));
                draft.new_rig = None;
            }
            NewRigAction::Cancel => draft.new_rig = None,
            NewRigAction::None => {}
        }
    }
    event
}

/// The inline SSH host field for a new remote rig, offering the hosts from `~/.ssh/config`.
fn new_rig_fields(ui: &mut Ui, new_rig: &mut NewRig, rigs: &[String]) -> NewRigAction {
    ui.label("SSH host of the remote rig");
    let field = ui.add(
        egui::TextEdit::singleline(&mut new_rig.host).hint_text("server or user@server").desired_width(f32::INFINITY),
    );
    let offered: Vec<&String> = new_rig.known_hosts.iter().filter(|name| !rigs.contains(name)).collect();
    if !offered.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for name in offered {
                if ui.small_button(name).clicked() {
                    new_rig.host.clone_from(name);
                }
            }
        });
    }
    let host = new_rig.host.trim();
    let valid = is_valid_host(host);
    let listed = rigs.iter().any(|known| known == host);
    if !host.is_empty() && !valid {
        ui.weak("Use a host name, or user@host");
    } else if listed {
        ui.weak("This rig is in the list already.");
    }
    let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    let (add, cancel) = ui
        .horizontal(|ui| {
            let add = ui.add_enabled(valid && !listed, egui::Button::new("Add rig")).clicked();
            (add, ui.button("Cancel").clicked())
        })
        .inner;
    if valid && !listed && (add || entered) {
        NewRigAction::Add
    } else if cancel {
        NewRigAction::Cancel
    } else {
        NewRigAction::None
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
        host: inventory.host.clone(),
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
    Some(QueueRequest { id: link.id.clone(), response: work(ctx, move || Rig::open(&link.host)?.read(&link)) })
}

/// List the seats of `rig` in the background. The user asked for it, so a
/// remote host that failed a moment ago is tried again at once.
fn find_seats(ctx: &Context, rig: RigHost) -> Receiver<Result<Inventory, String>> {
    if let RigHost::Ssh(host) = &rig {
        remote::connection::forget_failure(host);
    }
    work(ctx, move || Rig::open(&rig)?.inventory())
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
    if let RigHost::Ssh(host) = &task.link.host {
        ui.weak(format!("On {host}"));
    }
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
