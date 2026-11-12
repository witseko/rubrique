//! Remote folders in the app: the "Add remote folder" dialog, checking the
//! open remote file for changes in the background, and downloading the
//! images a remote note shows.

use std::io;
use std::path::{Component, Path, PathBuf};

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::{self, Context, Key, RichText};

use super::{OpenFile, RubriqueApp};
use crate::alert::{self, ButtonKind};
use crate::document::text_file::DiskState;
use crate::media::{MediaKind, image_uri, media_kind_of_url};
use crate::remote::task::{Task, spawn_with_repaint};
use crate::remote::{self, ProgramCheck, commands};
use crate::settings::view::SettingsGroup;
use crate::vfs::{self, Stamp};

/// Largest image downloaded for a remote note's preview.
const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// Most images downloaded for one note.
const MAX_IMAGES: usize = 60;
/// Largest image or video opened from a remote folder.
pub(super) const MAX_MEDIA_BYTES: u64 = 200 * 1024 * 1024;
/// Width of the "Add remote folder" dialog.
const DIALOG_WIDTH: f32 = 380.0;
/// Most height the folder buttons take before they scroll.
const FOLDER_LIST_HEIGHT: f32 = 120.0;
/// Size of the dialog's spinners.
const SPINNER_SIZE: f32 = 12.0;

/// Background work for remote folders.
#[derive(Debug, Default)]
pub(super) struct RemoteState {
    /// Checking the open remote file for outside changes.
    check: Option<(PathBuf, Task<io::Result<Option<Stamp>>>)>,
    /// Image downloads for remote notes; each gives the mirror paths it wrote.
    images: Vec<Task<Vec<PathBuf>>>,
    /// The "Add remote folder" dialog, while it is open.
    dialog: Option<AddRemote>,
}

/// State of the "Add remote folder" dialog.
#[derive(Debug, Default)]
struct AddRemote {
    host: String,
    folder: String,
    /// Hosts from `~/.ssh/config`, offered as buttons.
    known_hosts: Vec<String>,
    /// Why the last attempt failed.
    error: Option<String>,
    /// The error is about the ssh program setting: offer to open Settings.
    program_problem: bool,
    /// Connecting and finding the folder.
    connecting: Option<Task<io::Result<PathBuf>>>,
    /// The host whose folders are listed as the user types. Set when a host
    /// is picked with a button, confirmed with Enter or already in the
    /// explorer, so typing half a host name connects nowhere.
    active_host: Option<String>,
    /// The listing under way, with the host and typed folder it is for.
    listing: Option<(String, String, Task<io::Result<commands::Folders>>)>,
    /// The last (host, typed folder) a listing was started for, so a folder
    /// that does not exist is not asked for again every frame.
    attempted: Option<(String, String)>,
    /// The last listing that worked.
    folders: Option<Listed>,
}

/// The subfolders of one folder on a host, shown under the Folder field.
#[derive(Debug)]
struct Listed {
    host: String,
    /// The folder as typed (`~/dev/`): the start of the Folder text.
    parent: String,
    /// The same folder's absolute path on the host.
    path: PathBuf,
    /// Its subfolders' names, in name order.
    names: Vec<String>,
}

/// What the user asked for in the dialog this frame.
#[derive(Debug, Default)]
struct DialogEvents {
    add: bool,
    cancel: bool,
    /// Open Settings on the ssh program field.
    open_settings: bool,
}

impl AddRemote {
    /// Start listing folders on the typed host, after checking that the
    /// ssh program setting names a program at all (connecting would only
    /// turn that into a vaguer error).
    fn activate_host(&mut self, program: &str) {
        let host = self.host.trim().to_string();
        if !remote::mirror::is_valid_host(&host) {
            return;
        }
        if let ProgramCheck::Problem(message) = remote::check_ssh_program(program) {
            self.error = Some(message);
            self.program_problem = true;
            return;
        }
        self.error = None;
        self.program_problem = false;
        // The user asked to try now, even if the host failed a moment ago.
        remote::connection::forget_failure(&host);
        self.attempted = None;
        self.active_host = Some(host);
    }

    /// The (host, typed folder) to list now, if any.
    fn wanted_listing(&self) -> Option<(String, String)> {
        let host = self.active_host.clone().filter(|active| active == self.host.trim())?;
        if self.listing.is_some() {
            return None;
        }
        let (parent, _) = split_typed(&self.folder);
        let wanted = (host, parent);
        (self.attempted.as_ref() != Some(&wanted)).then_some(wanted)
    }

    /// Pick up a finished listing.
    fn poll_listing(&mut self) {
        let Some((host, parent, task)) = &self.listing else {
            return;
        };
        let Some(result) = task.take() else {
            return;
        };
        let (host, parent) = (host.clone(), parent.clone());
        self.listing = None;
        match result {
            Ok(found) => {
                self.folders = Some(Listed { host, parent, path: found.path, names: found.names });
                self.error = None;
            }
            // The folder being typed does not exist (yet): keep the last list.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                self.folders = None;
                self.error = Some(error.to_string());
            }
        }
    }

    /// Pick up the finished connection: the mirror path to add, if it worked.
    fn poll_connecting(&mut self) -> Option<PathBuf> {
        let result = self.connecting.as_ref().and_then(Task::take)?;
        self.connecting = None;
        match result.map(|path| remote::to_mirror(self.host.trim(), &path)) {
            Ok(Some(mirror)) => return Some(mirror),
            Ok(None) => self.error = Some("That folder cannot be used".to_string()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.error = Some(format!("No folder \"{}\" on {}", self.folder.trim(), self.host.trim()));
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        None
    }
}

impl RubriqueApp {
    /// Each frame: pick up finished background work for remote files.
    pub(super) fn poll_remote(&mut self) {
        let mut fetched = Vec::new();
        self.remote.images.retain(|task| match task.take() {
            Some(paths) => {
                fetched.extend(paths);
                false
            }
            None => true,
        });
        for path in fetched {
            self.repaint.forget_image(&image_uri(&path));
        }

        let finished = self.remote.check.as_ref().and_then(|(path, task)| Some((path.clone(), task.take()?)));
        if let Some((path, result)) = finished {
            self.remote.check = None;
            let state = match &self.open_file {
                OpenFile::Text { document, .. } if document.path == path && self.merging.is_none() => {
                    Some(document.disk_state_from(result))
                }
                _ => None,
            };
            if let Some(state) = state {
                self.apply_disk_state(&state);
            }
        }
    }

    /// Ask the host in the background whether the open file changed.
    pub(super) fn start_remote_check(&mut self, path: &Path) {
        if self.remote.check.is_some() {
            return;
        }
        let task_path = path.to_path_buf();
        let task = spawn_with_repaint(&self.repaint, move || vfs::current_stamp(&task_path));
        self.remote.check = Some((path.to_path_buf(), task));
    }

    /// Download the images a remote note links to, so its preview shows them.
    pub(super) fn fetch_remote_images(&mut self, note: &Path, text: &str) {
        let Some(folder) = note.parent().filter(|_| remote::is_mirror(note)) else {
            return;
        };
        let mut wanted: Vec<PathBuf> = Vec::new();
        for url in image_links(text) {
            for path in image_paths(folder, &url) {
                if remote::is_mirror(&path) && !wanted.contains(&path) {
                    wanted.push(path);
                }
            }
        }
        wanted.truncate(MAX_IMAGES);
        if wanted.is_empty() {
            return;
        }
        let task = spawn_with_repaint(&self.repaint, move || download_images(&wanted));
        self.remote.images.push(task);
    }

    /// True (with a message) when the open note is remote: adding images,
    /// audio or voice memos to remote notes is not supported yet.
    pub(super) fn refuse_remote_note(&mut self, what: &str) -> bool {
        let is_remote = self.open_file.path().is_some_and(remote::is_mirror);
        if is_remote {
            self.show_status(format!("Adding {what} to notes in remote folders is not supported yet"));
        }
        is_remote
    }

    /// Open the "Add remote folder" dialog. A host that already has a folder
    /// in the explorer is picked first and its folders listed at once.
    pub(super) fn open_add_remote(&mut self) {
        let known_hosts = remote::ssh_config::user_host_names();
        let in_use: Vec<String> =
            self.roots.iter().filter_map(|root| remote::from_mirror(&root.path)).map(|found| found.host).collect();
        let host = known_hosts.iter().find(|name| in_use.contains(name)).or(known_hosts.first()).cloned();
        let host = host.unwrap_or_default();
        let mut dialog = AddRemote { host, folder: "~".to_string(), known_hosts, ..AddRemote::default() };
        if in_use.contains(&dialog.host) {
            dialog.activate_host(&self.settings.remote.ssh_program);
        }
        self.remote.dialog = Some(dialog);
    }

    /// Draw the "Add remote folder" dialog while it is open.
    pub(super) fn add_remote_dialog(&mut self, ctx: &Context) {
        let Some(dialog) = &mut self.remote.dialog else {
            return;
        };
        if let Some(mirror) = dialog.poll_connecting() {
            self.remote.dialog = None;
            self.add_folder(mirror.clone());
            self.show_status(format!("Added {}", remote::display(&mirror)));
            return;
        }
        dialog.poll_listing();
        if let Some((host, parent)) = dialog.wanted_listing() {
            dialog.attempted = Some((host.clone(), parent.clone()));
            let (task_host, task_parent) = (host.clone(), parent.clone());
            let task = spawn_with_repaint(ctx, move || commands::folders(&task_host, &task_parent));
            dialog.listing = Some((host, parent, task));
        }

        let program = self.settings.remote.ssh_program.clone();
        let connecting = dialog.connecting.is_some();
        let modal = alert::show(ctx, "add-remote-folder", DIALOG_WIDTH, |ui| draw_dialog(ui, dialog, &program));
        let should_close = modal.should_close();
        let events = modal.inner;
        if events.open_settings {
            self.remote.dialog = None;
            self.settings_group = SettingsGroup::Files;
            if !self.settings_open {
                self.toggle_settings();
            }
            return;
        }
        if events.cancel || (should_close && !connecting) {
            self.remote.dialog = None;
            return;
        }
        if events.add {
            let host = dialog.host.trim().to_string();
            let folder = dialog.folder.trim().to_string();
            if let ProgramCheck::Problem(message) = remote::check_ssh_program(&program) {
                dialog.error = Some(message);
                dialog.program_problem = true;
                return;
            }
            dialog.error = None;
            dialog.program_problem = false;
            // The user asked to try now, even if the host failed a moment ago.
            remote::connection::forget_failure(&host);
            dialog.connecting = Some(spawn_with_repaint(ctx, move || commands::resolve_folder(&host, &folder)));
        }
    }

    /// Keep the settings' folder list in step with the explorer. While remote
    /// folders are off, the remote ones in the settings are kept for later.
    pub(super) fn remember_roots(&mut self) {
        let mut folders: Vec<PathBuf> = self.roots.iter().map(|root| root.path.clone()).collect();
        if !self.settings.remote.enabled {
            let hidden =
                self.settings.folders.iter().filter(|folder| remote::is_mirror(folder) && !folders.contains(folder));
            folders.extend(hidden.cloned().collect::<Vec<_>>());
        }
        self.settings.folders = folders;
    }

    /// Settings → Files changed the remote folder settings.
    pub(super) fn remote_settings_changed(&mut self) {
        remote::set_ssh_program(&self.settings.remote.ssh_program);
        if self.settings.remote.enabled {
            // Bring back the remote folders kept while it was off.
            let missing: Vec<PathBuf> = self
                .settings
                .folders
                .iter()
                .filter(|folder| remote::is_mirror(folder) && !self.roots.iter().any(|root| root.path == **folder))
                .cloned()
                .collect();
            for folder in missing {
                self.add_folder(folder);
            }
        } else {
            self.remember_roots();
            self.show_status("Remote folders stay in the explorer until Rubrique restarts".to_string());
        }
    }
}

/// The dialog's contents; returns what the user asked for.
fn draw_dialog(ui: &mut egui::Ui, dialog: &mut AddRemote, program: &str) -> DialogEvents {
    let connecting = dialog.connecting.is_some();
    alert::title(ui, "Add remote folder");
    alert::body(ui, "A folder on another computer, opened over SSH.");
    let host_entered = draw_host_fields(ui, dialog, program);
    let folder_field = draw_folder_fields(ui, dialog);
    let mut events = DialogEvents::default();
    if let Some(error) = &dialog.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
        if dialog.program_problem && ui.link("Set the ssh program in Settings \u{2192} Files").clicked() {
            events.open_settings = true;
        }
    }
    let entered = folder_field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
    let host = dialog.host.trim();
    let host_ok = remote::mirror::is_valid_host(host);
    let ready = host_ok && !dialog.folder.trim().is_empty() && !connecting;
    let ((), (add, cancel)) = alert::button_row(
        ui,
        |ui| {
            if connecting {
                ui.add(egui::Spinner::new().size(SPINNER_SIZE));
                ui.label(RichText::new(format!("Connecting to {host}\u{2026}")).weak());
            } else if !host_ok && !host.is_empty() {
                ui.label(RichText::new("Use a host name, or user@host").weak());
            }
        },
        |ui| {
            let add =
                ui.add_enabled_ui(ready, |ui| alert::button(ui, &add_label(&dialog.folder), ButtonKind::Primary, true));
            let cancel = alert::button(ui, "Cancel", ButtonKind::Secondary, false);
            (add.inner.clicked(), cancel.clicked())
        },
    );
    // Enter in the Computer field lists that host's folders; it does not add.
    let default_enter = alert::enter_for_default(ui) && !host_entered;
    events.add = add || ((entered || default_enter) && ready);
    events.cancel = cancel;
    events
}

/// The Computer field and the buttons for the hosts in `~/.ssh/config`.
/// Picking a button, or Enter in the field, starts listing that host's folders.
/// Returns true when Enter was pressed in the field this frame.
fn draw_host_fields(ui: &mut egui::Ui, dialog: &mut AddRemote, program: &str) -> bool {
    let connecting = dialog.connecting.is_some();
    alert::field_label(ui, "Computer");
    let field = ui
        .add_enabled_ui(!connecting, |ui| {
            let edit = egui::TextEdit::singleline(&mut dialog.host)
                .hint_text("server or user@server")
                .desired_width(f32::INFINITY);
            alert::text_field(ui, edit)
        })
        .inner;
    let mut picked = None;
    if !dialog.known_hosts.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("From ~/.ssh/config:").weak());
            for name in &dialog.known_hosts {
                if ui.add_enabled(!connecting, egui::Button::selectable(dialog.host == *name, name)).clicked() {
                    picked = Some(name.clone());
                }
            }
        });
    }
    let typed_new_host = field.lost_focus() && dialog.active_host.as_deref() != Some(dialog.host.trim());
    if let Some(name) = picked {
        dialog.host = name;
        dialog.activate_host(program);
    } else if typed_new_host {
        dialog.activate_host(program);
    }
    field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter))
}

/// The Folder field and, under it, the folders inside the typed path on the
/// host: a button per folder (click to look inside), "up", and a filter by
/// the name being typed. Tab completes the one name left.
fn draw_folder_fields(ui: &mut egui::Ui, dialog: &mut AddRemote) -> egui::Response {
    let connecting = dialog.connecting.is_some();
    alert::field_label(ui, "Folder");
    // Focus stays locked so Tab completes a name instead of leaving the field.
    let output = ui
        .add_enabled_ui(!connecting, |ui| {
            let output = egui::TextEdit::singleline(&mut dialog.folder)
                .hint_text("~/notes")
                .desired_width(f32::INFINITY)
                .lock_focus(true)
                .margin(egui::vec2(10.0, 7.0))
                .show(ui);
            alert::field_ring(ui, &output.response);
            output
        })
        .inner;
    let (typed_parent, typed_name) = split_typed(&dialog.folder);
    let tab = output.response.has_focus() && ui.input(|input| input.key_pressed(Key::Tab));
    let mut go_to = None;
    if let Some(listed) = &dialog.folders {
        // The name filters only the folder it is typed in.
        let filter = if listed.parent == typed_parent { typed_name } else { "" };
        let names: Vec<&str> = listed
            .names
            .iter()
            .filter(|name| name.to_lowercase().starts_with(&filter.to_lowercase()))
            .map(String::as_str)
            .collect();
        ui.add_space(4.0);
        egui::ScrollArea::vertical().max_height(FOLDER_LIST_HEIGHT).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let starting = if filter.is_empty() { String::new() } else { format!(", starting with \"{filter}\"") };
                let heading = format!("In {} on {}{starting}:", shown_parent(&listed.parent), listed.host);
                ui.label(RichText::new(heading).weak());
                if listed.path.parent().is_some() && ui.add(egui::Button::selectable(false, "\u{2191} up")).clicked() {
                    go_to = Some(up(&listed.parent, &listed.path));
                }
                for name in &names {
                    if ui.button(*name).clicked() {
                        go_to = Some(enter(&listed.parent, name));
                    }
                }
            });
        });
        if tab && names.len() == 1 {
            go_to = Some(enter(&listed.parent, names[0]));
        }
        ui.label(RichText::new("Click a folder to look inside it, or type a path.").weak().small());
    } else if dialog.listing.is_some() {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(SPINNER_SIZE));
            ui.label(RichText::new(format!("Looking at {}\u{2026}", dialog.host.trim())).weak());
        });
    }
    if let Some(text) = go_to {
        dialog.folder = text;
        // Keep typing at the end of the new path.
        let mut state = output.state;
        let end = CCursor::new(dialog.folder.chars().count());
        state.cursor.set_char_range(Some(CCursorRange::one(end)));
        state.store(ui.ctx(), output.response.id);
        output.response.request_focus();
    }
    output.response.response
}

/// The folder to list and the name being typed, from the Folder text:
/// `~/dev/pe` lists `~/dev/` filtered by `pe`; `dev` lists `~` filtered by
/// `dev`; `~`, `~/` and `/` list themselves.
fn split_typed(typed: &str) -> (String, &str) {
    let typed = typed.trim();
    match typed.rfind('/') {
        Some(at) => (typed[..=at].to_string(), &typed[at + 1..]),
        None if typed == "~" => ("~".to_string(), ""),
        None => ("~".to_string(), typed),
    }
}

/// The Folder text for `name` inside the listed `parent`, ready to list it.
fn enter(parent: &str, name: &str) -> String {
    if parent.ends_with('/') { format!("{parent}{name}/") } else { format!("{parent}/{name}/") }
}

/// The Folder text one level up from the listed `parent`: `~/dev/` gives
/// `~/`; `~`, a relative name and `/` need the host's real parent, from `path`.
fn up(parent: &str, path: &Path) -> String {
    let trimmed = parent.trim_end_matches('/');
    if let Some(at) = trimmed.rfind('/') {
        return trimmed[..=at].to_string();
    }
    let mut above = path.parent().unwrap_or(Path::new("/")).display().to_string();
    if !above.ends_with('/') {
        above.push('/');
    }
    above
}

/// A listed folder as the heading shows it: `~/dev/` as `~/dev`.
fn shown_parent(parent: &str) -> &str {
    if parent.len() > 1 { parent.trim_end_matches('/') } else { parent }
}

/// "Add ~/dev" for the Folder text `~/dev/`.
fn add_label(folder: &str) -> String {
    let folder = folder.trim();
    let folder = if folder.len() > 1 { folder.trim_end_matches('/') } else { folder };
    if folder.is_empty() { "Add".to_string() } else { format!("Add {folder}") }
}

/// Fetch images from their hosts into the mirror. Returns the mirror paths
/// that were written. Images that cannot be read are skipped.
fn download_images(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut hosts: Vec<String> = Vec::new();
    for path in paths {
        if let Some(found) = remote::from_mirror(path)
            && !hosts.contains(&found.host)
        {
            hosts.push(found.host);
        }
    }
    let mut written = Vec::new();
    for host in hosts {
        let mirrors: Vec<&PathBuf> =
            paths.iter().filter(|path| remote::from_mirror(path).is_some_and(|found| found.host == host)).collect();
        let remote_paths: Vec<PathBuf> =
            mirrors.iter().filter_map(|path| remote::from_mirror(path).map(|found| found.path)).collect();
        let Ok(files) = commands::read_many(&host, &remote_paths, MAX_IMAGE_BYTES) else {
            continue;
        };
        for (mirror, bytes) in mirrors.into_iter().zip(files) {
            if let Some(bytes) = bytes {
                vfs::write_mirror(mirror, &bytes);
                written.push(mirror.clone());
            }
        }
    }
    written
}

/// The targets of `![alt](url)` images and `<img src="url">` tags.
fn image_links(text: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("![") {
        rest = &rest[start + 2..];
        let Some(close) = rest.find("](") else {
            break;
        };
        let after = &rest[close + 2..];
        let target = after[..after.find(')').unwrap_or(after.len())].trim();
        let url = match target.strip_prefix('<').and_then(|inner| inner.split_once('>')) {
            Some((url, _)) => url,
            // A title may follow the URL: ![a](cat.png "Cat").
            None => target.split_whitespace().next().unwrap_or(""),
        };
        links.push(url.to_string());
        rest = after;
    }
    for (start, _) in text.match_indices("<img") {
        let tag = &text[start..start + text[start..].find('>').unwrap_or(text.len() - start)];
        let Some(at) = tag.find("src=") else {
            continue;
        };
        let value = &tag[at + 4..];
        let Some(quote) = value.chars().next().filter(|quote| matches!(quote, '"' | '\'')) else {
            continue;
        };
        if let Some(end) = value[1..].find(quote) {
            links.push(value[1..=end].to_string());
        }
    }
    links
}

/// Where a relative image link points, under `folder`: as written and,
/// when it has `%20`-style escapes, decoded too. Web links, absolute paths
/// (which mean this computer to the preview) and non-images give nothing.
fn image_paths(folder: &Path, url: &str) -> Vec<PathBuf> {
    let url = url.split(['?', '#']).next().unwrap_or(url);
    if url.is_empty() || url.contains(':') || url.starts_with('/') || media_kind_of_url(url) != Some(MediaKind::Image) {
        return Vec::new();
    }
    let mut paths = vec![normalize(&folder.join(url))];
    let decoded = crate::voice::decode_url(url);
    if decoded != url {
        paths.push(normalize(&folder.join(decoded)));
    }
    paths
}

/// `a/b/../c` as `a/c`, without looking at the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                normal.pop();
            }
            Component::CurDir => {}
            other => normal.push(other),
        }
    }
    normal
}

impl DiskState {
    /// True when the file changed or vanished.
    #[cfg(test)]
    fn is_news(&self) -> bool {
        !matches!(self, DiskState::Unchanged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::text_file::{Document, Opened};
    use crate::remote::testing::mirror_of;
    use crate::test_support::TempFolder;

    #[test]
    fn splits_the_typed_folder_into_parent_and_name() {
        assert_eq!(split_typed("~/dev/pe"), ("~/dev/".to_string(), "pe"));
        assert_eq!(split_typed("~/dev/"), ("~/dev/".to_string(), ""));
        assert_eq!(split_typed("dev"), ("~".to_string(), "dev"));
        assert_eq!(split_typed(" ~ "), ("~".to_string(), ""));
        assert_eq!(split_typed(""), ("~".to_string(), ""));
        assert_eq!(split_typed("/"), ("/".to_string(), ""));
        assert_eq!(split_typed("/srv/no"), ("/srv/".to_string(), "no"));
    }

    #[test]
    fn moves_into_and_out_of_folders() {
        assert_eq!(enter("~/dev/", "personal"), "~/dev/personal/");
        assert_eq!(enter("~", "dev"), "~/dev/");
        assert_eq!(enter("/", "srv"), "/srv/");
        let home = Path::new("/home/me");
        assert_eq!(up("~/dev/", &home.join("dev")), "~/");
        assert_eq!(up("~/", home), "/home/");
        assert_eq!(up("~", home), "/home/");
        assert_eq!(up("dev/", &home.join("dev")), "/home/me/");
        assert_eq!(up("/srv/", Path::new("/srv")), "/");
        assert_eq!(up("/", Path::new("/")), "/");
    }

    #[test]
    fn labels_follow_the_typed_folder() {
        assert_eq!(add_label("~/dev/"), "Add ~/dev");
        assert_eq!(add_label("~"), "Add ~");
        assert_eq!(add_label("/"), "Add /");
        assert_eq!(add_label("  "), "Add");
        assert_eq!(shown_parent("~/dev/"), "~/dev");
        assert_eq!(shown_parent("~"), "~");
        assert_eq!(shown_parent("/"), "/");
    }

    #[test]
    fn lists_only_for_a_picked_host_and_each_folder_once() {
        let mut dialog = AddRemote { host: "box".to_string(), folder: "~/dev/x".to_string(), ..AddRemote::default() };
        assert_eq!(dialog.wanted_listing(), None);
        dialog.activate_host("sh");
        assert_eq!(dialog.wanted_listing(), Some(("box".to_string(), "~/dev/".to_string())));
        dialog.attempted = dialog.wanted_listing();
        assert_eq!(dialog.wanted_listing(), None);
        dialog.folder = "~/dev/x/".to_string();
        assert_eq!(dialog.wanted_listing(), Some(("box".to_string(), "~/dev/x/".to_string())));
        // Typing another host lists nothing until it is picked.
        dialog.host = "other".to_string();
        assert_eq!(dialog.wanted_listing(), None);
    }

    #[test]
    fn a_broken_ssh_program_setting_is_reported_before_connecting() {
        let mut dialog = AddRemote { host: "box".to_string(), ..AddRemote::default() };
        dialog.activate_host("ssh box");
        assert!(dialog.program_problem);
        assert!(dialog.error.as_deref().unwrap_or_default().starts_with("Not a program"));
        assert_eq!(dialog.active_host, None);
    }

    #[test]
    fn finds_image_links() {
        let text = "![cat](images/cat.png) and ![dog](<my dog.jpg> \"Dog\")\n\
            ![t](pics/a.gif \"title\") ![web](https://x.org/a.png)\n<img alt=\"x\" src='html.webp'>";
        assert_eq!(
            image_links(text),
            ["images/cat.png", "my dog.jpg", "pics/a.gif", "https://x.org/a.png", "html.webp"]
        );
    }

    #[test]
    fn resolves_relative_image_paths_only() {
        let folder = Path::new("/m/host/notes");
        assert_eq!(image_paths(folder, "images/cat.png"), [PathBuf::from("/m/host/notes/images/cat.png")]);
        assert_eq!(image_paths(folder, "../shared/a.png?v=2"), [PathBuf::from("/m/host/shared/a.png")]);
        assert_eq!(
            image_paths(folder, "my%20cat.png"),
            [PathBuf::from("/m/host/notes/my%20cat.png"), PathBuf::from("/m/host/notes/my cat.png")]
        );
        assert_eq!(image_paths(folder, "https://x.org/a.png"), Vec::<PathBuf>::new());
        assert_eq!(image_paths(folder, "/etc/a.png"), Vec::<PathBuf>::new());
        assert_eq!(image_paths(folder, "notes.md"), Vec::<PathBuf>::new());
    }

    #[test]
    fn downloads_images_into_the_mirror() {
        let temp = TempFolder::new();
        temp.write("notes/images/cat.png", "meow");
        let note = mirror_of("img-host", &temp.path().join("notes/a.md"));
        let folder = note.parent().unwrap();
        let wanted: Vec<PathBuf> =
            ["images/cat.png", "images/missing.png"].iter().flat_map(|url| image_paths(folder, url)).collect();
        let written = download_images(&wanted);
        assert_eq!(written, [folder.join("images/cat.png")]);
        assert_eq!(std::fs::read(folder.join("images/cat.png")).unwrap(), b"meow");
    }

    #[test]
    fn remote_document_notices_outside_changes() {
        let temp = TempFolder::new();
        let real = temp.write("a.md", "first");
        let mirror = mirror_of("doc-host", &real);
        let Opened::Text(mut document) = Document::open(&mirror, None).unwrap() else {
            panic!("text expected");
        };
        assert_eq!(document.text, "first");
        assert!(!document.disk_state().is_news());

        document.text.push_str(" edit");
        document.on_text_changed();
        document.save().unwrap();
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "first edit");
        assert!(!document.disk_state().is_news());

        std::fs::write(&real, "changed elsewhere").unwrap();
        assert_eq!(document.disk_state(), DiskState::Modified);
        assert_eq!(document.read_disk_text().unwrap(), "changed elsewhere");
        document.accept_disk_text("changed elsewhere".to_string());
        assert!(!document.disk_state().is_news());

        std::fs::remove_file(&real).unwrap();
        assert_eq!(document.disk_state(), DiskState::Deleted);
        // No answer from the host is no news, not "deleted".
        let unreachable = document.disk_state_from(Err(io::Error::other("timeout")));
        assert_eq!(unreachable, DiskState::Unchanged);
    }
}
