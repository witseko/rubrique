//! The team editor's states around the form: a new team, a read-only
//! built-in team, validation and preflight results, `rig` missing, and the
//! unsaved-changes wording.
//!
//! New teams and copies are written into the user's spec folder
//! (`$OPENRIG_HOME/specs`, else `~/.openrig/specs`). The team name becomes a
//! folder name, so it is checked before any path is built from it, and every
//! folder and file is created with "fail if it exists": nothing is ever
//! overwritten.

use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::{Component, Path, PathBuf};

use eframe::egui::{self, Context, CursorIcon, RichText, Sense, Ui};

use super::common::{SeatRef, SubTab, TeamEditor, TeamEvent, is_built_in, is_safe_relative, plural};
use super::links::kind_in_words;
use crate::alert::{self, ButtonKind};
use crate::openrig::RigCallError;
use crate::openrig::spec::{CLAUDE_RUNTIME, Edge, Member, Pod, RigSpec, seat_address};
use crate::settings::view::path_box;

/// Width of the new team dialog.
const NEW_TEAM_WIDTH: f32 = 760.0;
/// The starter team as `OpenRig` ships it.
const STARTER_TEXT: &str = include_str!("../../openrig/spec/fixtures/starter.yaml");
/// The factory team as `OpenRig` ships it.
const FACTORY_TEXT: &str = include_str!("../../openrig/spec/fixtures/factory.yaml");
/// The smallest text that reads as a rig spec; a blank team is written into it.
const BLANK_TEXT: &str = "version: \"0.2\"\nname: blank\npods: []\nedges: []\n";
/// The spec file inside a team folder.
const SPEC_FILE: &str = "rig.yaml";
/// Longest team name accepted; it becomes a folder name and part of every seat address.
const MAX_NAME_LENGTH: usize = 64;
/// The name a new team starts with.
const DEFAULT_NAME: &str = "my-team";
/// How many `-copy-N` names a duplicate tries before giving up.
const MAX_COPY_NUMBER: usize = 99;
/// Seat addresses listed under the name field before "and N more".
const ADDRESSES_SHOWN: usize = 3;
/// Changes listed in the unsaved-changes dialog before "and N more".
const CHANGES_SHOWN: usize = 5;
/// Space inside a template card and a notice box.
const BOX_MARGIN: i8 = 12;
/// Corner rounding of a template card and a notice box.
const BOX_RADIUS: u8 = 8;
/// Size of the small text in cards and notices.
const SMALL_SIZE: f32 = 12.0;

/// What a new team starts from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) enum Template {
    /// One pod: a Claude Code builder and a Codex reviewer.
    #[default]
    Starter,
    /// Three pods: a lead, a PM advisor, build, QA, design and two reviewers.
    Factory,
    /// One pod and one empty seat.
    Blank,
}

impl Template {
    /// All templates, in the order the dialog shows them.
    const ALL: [Template; 3] = [Template::Starter, Template::Factory, Template::Blank];

    /// The card title.
    fn title(self) -> &'static str {
        match self {
            Template::Starter => "Starter",
            Template::Factory => "Factory",
            Template::Blank => "Blank",
        }
    }

    /// The line at the bottom of the card.
    fn note(self) -> &'static str {
        match self {
            Template::Starter => "A builder and a reviewer in one repository. Good first team.",
            Template::Factory => "A lead runs build, QA, design and two reviewers.",
            Template::Blank => "You choose the seat's agent before the first check.",
        }
    }

    /// The folder of this team in `OpenRig`'s built-in specs
    /// (`rigs/launch/<folder>`); none for a blank team.
    fn built_in_folder(self) -> Option<&'static str> {
        match self {
            Template::Starter => Some("starter"),
            Template::Factory => Some("factory"),
            Template::Blank => None,
        }
    }

    /// The template's model, named `name`. The shipped fixtures always parse
    /// (a test checks it), so a failure here gives an empty model.
    fn spec(self, name: &str) -> RigSpec {
        let mut spec = match self {
            Template::Starter => RigSpec::parse(STARTER_TEXT).unwrap_or_default(),
            Template::Factory => RigSpec::parse(FACTORY_TEXT).unwrap_or_default(),
            Template::Blank => blank_spec(),
        };
        name.clone_into(&mut spec.name);
        spec
    }
}

/// The new team dialog's choices while it is open.
#[derive(Debug)]
pub(in crate::app) struct NewTeamDialog {
    pub template: Template,
    /// The team name, which also names its folder in the user's spec folder.
    pub name: String,
    /// Rewrite the built-in agents' `local:` links to `path:` links, so they
    /// still find agent.yaml from the new folder.
    pub keep_builtin_agents: bool,
    /// The installed `OpenRig`'s spec folder (holding `rigs/` and
    /// `agents/`), looked up once when the dialog first draws.
    pub openrig_specs: Option<PathBuf>,
    /// True once `openrig_specs` was looked up.
    pub looked_for_openrig: bool,
}

impl Default for NewTeamDialog {
    fn default() -> Self {
        Self {
            template: Template::default(),
            name: DEFAULT_NAME.to_owned(),
            keep_builtin_agents: true,
            openrig_specs: None,
            looked_for_openrig: false,
        }
    }
}

/// How the new team dialog was answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum NewTeamChoice {
    /// Create the team (see [`create_team`]) and open it.
    Create,
    Cancel,
}

/// Draw the new team dialog (artboard `StateNewTeam`): three template
/// cards (Starter, Factory, Blank, each with its pods and seats), the team
/// name with the seat addresses it gives, where it is saved
/// (`~/.openrig/specs/<name>/rig.yaml`, with the files copied next to it),
/// the "Keep using the built-in agents" checkbox, then Cancel and "Create
/// and open". Returns the answer on the frame it is given.
pub(in crate::app) fn new_team_dialog(ctx: &Context, dialog: &mut NewTeamDialog) -> Option<NewTeamChoice> {
    if !dialog.looked_for_openrig {
        dialog.openrig_specs = installed_specs_folder();
        dialog.looked_for_openrig = true;
    }
    let spec_root = user_spec_folder();
    let modal = alert::show(ctx, "new-openrig-team", NEW_TEAM_WIDTH, |ui| {
        alert::title(ui, "New team");
        ui.label(RichText::new("Start from").strong());
        template_cards(ui, dialog);
        let problem = name_problem(&dialog.name, spec_root.as_deref());
        let name_field = ui.columns(2, |columns| {
            let field = name_column(&mut columns[0], dialog, problem.as_deref());
            saved_as_column(&mut columns[1], dialog, spec_root.as_deref());
            field
        });
        agents_choice(ui, dialog);
        ui.separator();
        let (cancel, create) = alert::button_row(
            ui,
            |_| (),
            |ui| {
                let create = ui
                    .add_enabled_ui(problem.is_none(), |ui| {
                        alert::button(ui, "Create and open", ButtonKind::Primary, true)
                    })
                    .inner;
                (alert::button(ui, "Cancel", ButtonKind::Secondary, false), create)
            },
        )
        .1;
        let entered = name_field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if cancel.clicked() {
            Some(NewTeamChoice::Cancel)
        } else if problem.is_none() && (create.clicked() || entered || alert::enter_for_default(ui)) {
            Some(NewTeamChoice::Create)
        } else {
            None
        }
    });
    let closed = modal.should_close();
    modal.inner.or(closed.then_some(NewTeamChoice::Cancel))
}

/// The three template cards side by side; a click picks one.
fn template_cards(ui: &mut Ui, dialog: &mut NewTeamDialog) {
    ui.columns(Template::ALL.len(), |columns| {
        for (column, template) in columns.iter_mut().zip(Template::ALL) {
            if template_card(column, template, dialog.template == template).clicked() {
                dialog.template = template;
            }
        }
    });
}

/// One template card: title and counts, its pods and seats, and a note.
fn template_card(ui: &mut Ui, template: Template, selected: bool) -> egui::Response {
    let visuals = ui.visuals();
    let (fill, stroke) = if selected {
        (visuals.selection.bg_fill, visuals.selection.stroke)
    } else {
        (visuals.faint_bg_color, visuals.widgets.noninteractive.bg_stroke)
    };
    let spec = template.spec("");
    let frame = egui::Frame::new().fill(fill).stroke(stroke).corner_radius(BOX_RADIUS).inner_margin(BOX_MARGIN);
    let response = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.horizontal(|ui| {
            ui.label(RichText::new(template.title()).strong());
            ui.label(RichText::new(count_line(&spec)).size(SMALL_SIZE).weak());
        });
        match template {
            Template::Starter => seat_lines(ui, &spec),
            Template::Factory => pod_lines(ui, &spec),
            Template::Blank => {
                ui.label(RichText::new("One pod and one empty seat.").weak());
            }
        }
        ui.label(RichText::new(template.note()).size(SMALL_SIZE).weak());
    });
    response.response.interact(Sense::click()).on_hover_cursor(CursorIcon::PointingHand)
}

/// "1 pod, 2 seats".
fn count_line(spec: &RigSpec) -> String {
    let pods = spec.pods.len();
    let seats = spec.seat_count();
    format!("{pods} {}, {seats} {}", plural(pods, "pod", "pods"), plural(seats, "seat", "seats"))
}

/// Each seat with its runtime, for a one-pod team.
fn seat_lines(ui: &mut Ui, spec: &RigSpec) {
    for member in spec.pods.iter().flat_map(|pod| &pod.members) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&member.id).monospace());
            crate::settings::view::tag(ui, &member.runtime);
        });
    }
}

/// Each pod with its seat ids: "Development: build, qa, design".
fn pod_lines(ui: &mut Ui, spec: &RigSpec) {
    for pod in &spec.pods {
        let seats: Vec<&str> = pod.members.iter().map(|member| member.id.as_str()).collect();
        ui.label(RichText::new(format!("{}: {}", pod.name(), seats.join(", "))).weak());
    }
}

/// The team name field, then the seat addresses it gives or why the name
/// cannot be used. Returns the field's response.
fn name_column(ui: &mut Ui, dialog: &mut NewTeamDialog, problem: Option<&str>) -> egui::Response {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Team name").strong());
        ui.label(RichText::new("*").color(ui.visuals().hyperlink_color).strong());
        ui.label(RichText::new("name").monospace().size(SMALL_SIZE).weak());
    });
    let edit = egui::TextEdit::singleline(&mut dialog.name).hint_text(DEFAULT_NAME).desired_width(f32::INFINITY);
    let field = alert::text_field(ui, edit);
    if let Some(problem) = problem {
        ui.label(RichText::new(problem).size(SMALL_SIZE).color(ui.visuals().error_fg_color));
    } else {
        let line = format!("Seats will be {}", address_list(&dialog.template.spec(&dialog.name)));
        ui.label(RichText::new(line).size(SMALL_SIZE).weak());
    }
    field
}

/// Every seat address of `spec`, in file order.
fn seat_addresses(spec: &RigSpec) -> Vec<String> {
    spec.pods
        .iter()
        .flat_map(|pod| pod.members.iter().map(|member| seat_address(&pod.id, &member.id, &spec.name)))
        .collect()
}

/// The first seat addresses of `spec`, then "and N more".
fn address_list(spec: &RigSpec) -> String {
    let addresses = seat_addresses(spec);
    let shown = addresses.iter().take(ADDRESSES_SHOWN).cloned().collect::<Vec<_>>().join(", ");
    match addresses.len().checked_sub(ADDRESSES_SHOWN) {
        Some(more) if more > 0 => format!("{shown} and {more} more"),
        _ => shown,
    }
}

/// Where the team is saved, and which files come with it.
fn saved_as_column(ui: &mut Ui, dialog: &NewTeamDialog, spec_root: Option<&Path>) {
    ui.label(RichText::new("Saved as").strong());
    let Some(spec_root) = spec_root else {
        let message = "Your home folder is not known (HOME is not set), so the team has nowhere to go.";
        ui.label(RichText::new(message).size(SMALL_SIZE).color(ui.visuals().error_fg_color));
        return;
    };
    let name = if dialog.name.is_empty() { DEFAULT_NAME } else { dialog.name.as_str() };
    path_box(ui, &spec_root.join(name).join(SPEC_FILE));
    if dialog.template == Template::Blank {
        return;
    }
    if dialog.openrig_specs.is_some() {
        ui.label(RichText::new("Copies CULTURE.md and guidance/ next to it.").size(SMALL_SIZE).weak());
    } else {
        let message = "OpenRig's built-in teams were not found on this computer (rig is not on PATH). The team is \
                       still created, but its agent links must be set by hand, and CULTURE.md and guidance/ are \
                       not copied.";
        ui.label(RichText::new(message).size(SMALL_SIZE).color(ui.visuals().warn_fg_color));
    }
}

/// "Keep using the built-in agents", for a template from the installed `OpenRig`.
fn agents_choice(ui: &mut Ui, dialog: &mut NewTeamDialog) {
    if dialog.template == Template::Blank {
        return;
    }
    let found = dialog.openrig_specs.is_some();
    ui.add_enabled(found, egui::Checkbox::new(&mut dialog.keep_builtin_agents, "Keep using the built-in agents"));
    let explanation = if found {
        "Agent links become full path: links, so they still find agent.yaml from the new folder."
    } else {
        "Needs OpenRig installed: the links stay local: links that only work inside OpenRig's own folder."
    };
    ui.label(RichText::new(explanation).size(SMALL_SIZE).weak());
}

/// Why `name` cannot name a new team in `spec_root`, or `None` when it can.
fn name_problem(name: &str, spec_root: Option<&Path>) -> Option<String> {
    if let Err(problem) = check_team_name(name) {
        return Some(problem);
    }
    let taken = spec_root.is_some_and(|root| fs::symlink_metadata(root.join(name)).is_ok());
    taken.then(|| format!("A team folder called \u{201c}{name}\u{201d} already exists."))
}

/// Check a team name before it becomes a folder name: not empty, at most
/// [`MAX_NAME_LENGTH`] characters, only letters, digits, `_`, `.` and `-`,
/// and not starting with a dot (which also refuses `.` and `..`).
fn check_team_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Give the team a name.".to_owned());
    }
    if name.len() > MAX_NAME_LENGTH {
        return Err(format!("Use at most {MAX_NAME_LENGTH} characters."));
    }
    if !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')) {
        return Err("Use letters, digits, _, . and - only.".to_owned());
    }
    if name.starts_with('.') {
        return Err("Start the name with a letter or digit.".to_owned());
    }
    Ok(())
}

/// Write the new team's folder (`~/.openrig/specs/<name>/rig.yaml` and,
/// for a template, its culture file and guidance) and return the spec's
/// path to open. The starter and factory come from the installed `OpenRig`;
/// with `keep_builtin_agents` their `local:` agent links become absolute
/// `path:` links. Refuses a name that is not a valid id or a folder that
/// exists.
///
/// # Errors
///
/// Fails with a message for the dialog when the team cannot be written.
pub(in crate::app) fn create_team(dialog: &NewTeamDialog) -> Result<PathBuf, String> {
    let spec_root = user_spec_folder().ok_or("Your home folder is not known (HOME is not set).")?;
    let installed = if dialog.looked_for_openrig { dialog.openrig_specs.clone() } else { installed_specs_folder() };
    create_team_in(dialog, &spec_root, installed.as_deref())
}

/// [`create_team`] with the user's spec folder and the installed `OpenRig`
/// spec folder (if any) given, so tests never touch the real ones.
fn create_team_in(dialog: &NewTeamDialog, spec_root: &Path, installed: Option<&Path>) -> Result<PathBuf, String> {
    let name = dialog.name.as_str();
    check_team_name(name)?;
    let base_text = match dialog.template {
        Template::Starter => STARTER_TEXT,
        Template::Factory => FACTORY_TEXT,
        Template::Blank => BLANK_TEXT,
    };
    let source_folder = dialog
        .template
        .built_in_folder()
        .zip(installed)
        .map(|(folder, installed)| installed.join("rigs").join("launch").join(folder));
    let mut spec = dialog.template.spec(name);
    if dialog.keep_builtin_agents
        && let Some(source_folder) = &source_folder
    {
        rewrite_agent_links(&mut spec, source_folder);
    }
    let text = spec.write_into(base_text)?;
    write_team_folder(spec_root, name, &text, &companion_files(&spec), source_folder.as_deref())
        .map_err(|error| folder_error(spec_root, name, &error))
}

/// The blank team: one pod with one Claude Code seat whose agent the user
/// picks.
fn blank_spec() -> RigSpec {
    let mut spec = RigSpec::parse(BLANK_TEXT).unwrap_or_default();
    let mut pod = Pod::new("dev", "Project");
    let mut seat = Member::new("build", CLAUDE_RUNTIME);
    "default".clone_into(&mut seat.profile);
    pod.members.push(seat);
    spec.pods.push(pod);
    spec
}

/// Copy the read-only team at `spec_path` (its current `text`) into the
/// user's spec folder, rewriting `local:` agent links to `path:` links so
/// they still resolve, and return the copy's path to open.
///
/// # Errors
///
/// Fails with a message for the status bar when the copy cannot be written.
pub(in crate::app) fn duplicate_team(spec_path: &Path, text: &str) -> Result<PathBuf, String> {
    let spec_root = user_spec_folder().ok_or("Your home folder is not known (HOME is not set).")?;
    duplicate_team_in(spec_path, text, &spec_root)
}

/// [`duplicate_team`] into `spec_root`. The copy is named after the team:
/// `<name>-copy`, then `<name>-copy-2` and on, taking the first free name.
fn duplicate_team_in(spec_path: &Path, text: &str, spec_root: &Path) -> Result<PathBuf, String> {
    let original = RigSpec::parse(text)?;
    let source_folder = spec_path.parent().filter(|folder| folder.is_absolute());
    let base = if check_team_name(&original.name).is_ok() { original.name.as_str() } else { "team" };
    for number in 1..=MAX_COPY_NUMBER {
        let name = if number == 1 { format!("{base}-copy") } else { format!("{base}-copy-{number}") };
        if check_team_name(&name).is_err() {
            break;
        }
        let mut spec = original.clone();
        spec.name.clone_from(&name);
        if let Some(source_folder) = source_folder {
            rewrite_agent_links(&mut spec, source_folder);
        }
        let copy = spec.write_into(text)?;
        match write_team_folder(spec_root, &name, &copy, &companion_files(&spec), source_folder) {
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            written => return written.map_err(|error| folder_error(spec_root, &name, &error)),
        }
    }
    Err(format!("No free name for a copy of \u{201c}{base}\u{201d} in {}.", spec_root.display()))
}

/// A plain message for a team folder that could not be written.
fn folder_error(spec_root: &Path, name: &str, error: &io::Error) -> String {
    let folder = spec_root.join(name);
    if error.kind() == ErrorKind::AlreadyExists {
        format!("{} already exists; choose another name.", folder.display())
    } else {
        format!("Could not create {}: {error}", folder.display())
    }
}

/// Create `<spec_root>/<name>/` holding `text` as `rig.yaml`, with each of
/// `companions` (relative paths) copied from `source_folder` when it has
/// them. `name` must have passed [`check_team_name`].
///
/// The team folder must not exist yet: it is created with "fail if it
/// exists", so an existing folder or a link of that name is never written
/// through. When a later step fails the new folder is removed again.
///
/// # Errors
///
/// [`ErrorKind::AlreadyExists`] when the folder exists; any other I/O error
/// from writing.
fn write_team_folder(
    spec_root: &Path,
    name: &str,
    text: &str,
    companions: &[String],
    source_folder: Option<&Path>,
) -> io::Result<PathBuf> {
    fs::create_dir_all(spec_root)?;
    let folder = spec_root.join(name);
    fs::create_dir(&folder)?;
    let filled = fill_team_folder(&folder, text, companions, source_folder);
    if let Err(error) = filled {
        // The folder is new and holds only what this call wrote.
        let _ = fs::remove_dir_all(&folder);
        return Err(error);
    }
    Ok(folder.join(SPEC_FILE))
}

/// Write `rig.yaml` and the companion files into the new `folder`.
fn fill_team_folder(folder: &Path, text: &str, companions: &[String], source_folder: Option<&Path>) -> io::Result<()> {
    write_new_file(&folder.join(SPEC_FILE), text.as_bytes())?;
    let Some(source_folder) = source_folder else { return Ok(()) };
    for relative in companions {
        let source = source_folder.join(relative);
        // Only regular files: a link in the source folder is not followed.
        if !fs::symlink_metadata(&source).is_ok_and(|metadata| metadata.is_file()) {
            continue;
        }
        let target = folder.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        write_new_file(&target, &fs::read(&source)?)?;
    }
    Ok(())
}

/// Write `contents` to a file that must not exist yet.
fn write_new_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::io::Write;
    OpenOptions::new().write(true).create_new(true).open(path)?.write_all(contents)
}

/// The files a team names next to its spec (culture file, startup files,
/// docs), each once, keeping only plain relative paths that stay inside the
/// team folder.
fn companion_files(spec: &RigSpec) -> Vec<String> {
    let startup = spec
        .startup
        .files
        .iter()
        .chain(spec.pods.iter().flat_map(|pod| &pod.members).flat_map(|member| &member.startup.files))
        .map(|file| &file.path);
    let mut files: Vec<String> = Vec::new();
    for path in std::iter::once(&spec.culture_file).chain(startup).chain(&spec.docs) {
        if is_safe_relative(path) && !files.contains(path) {
            files.push(path.clone());
        }
    }
    files
}

/// Turn each `local:` agent link, which is relative to `source_folder`,
/// into a `path:` link with the absolute folder, so it still resolves from
/// another folder. Other links stay as they are.
fn rewrite_agent_links(spec: &mut RigSpec, source_folder: &Path) {
    for member in spec.pods.iter_mut().flat_map(|pod| &mut pod.members) {
        let Some(relative) = member.agent_ref.strip_prefix("local:") else { continue };
        if Path::new(relative).is_relative() {
            member.agent_ref = format!("path:{}", join_lexically(source_folder, relative).display());
        }
    }
}

/// `base` joined with the relative `relative`, with `.` and `..` resolved
/// on the text (the target need not exist).
fn join_lexically(base: &Path, relative: &str) -> PathBuf {
    let mut joined = base.to_path_buf();
    for component in Path::new(relative).components() {
        match component {
            Component::ParentDir => {
                joined.pop();
            }
            Component::Normal(part) => joined.push(part),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    joined
}

/// The user's spec folder: `$OPENRIG_HOME/specs`, else `~/.openrig/specs`
/// (where `OpenRig` keeps the teams its users write).
fn user_spec_folder() -> Option<PathBuf> {
    let openrig_home = std::env::var_os("OPENRIG_HOME").filter(|home| !home.is_empty()).map(PathBuf::from);
    let home =
        || std::env::var_os("HOME").filter(|home| !home.is_empty()).map(|home| Path::new(&home).join(".openrig"));
    Some(openrig_home.or_else(home)?.join("specs"))
}

/// The installed `OpenRig`'s spec folder, found from the `rig` command on
/// `PATH` (see [`specs_folder_for_rig`]).
fn installed_specs_folder() -> Option<PathBuf> {
    specs_folder_for_rig(&crate::ai::find_on_path("rig")?)
}

/// The spec folder of the `OpenRig` that `rig` belongs to. `rig` is a link
/// to `<package>/dist/bin-wrapper.js` of `@openrig/cli`; an npm install
/// keeps the specs in `<package>/daemon/specs`, a checkout of the
/// repository in `packages/daemon/specs` next to the package. The folder
/// must hold `rigs/launch` and `agents`.
fn specs_folder_for_rig(rig: &Path) -> Option<PathBuf> {
    let wrapper = fs::canonicalize(rig).ok()?;
    let package = wrapper.parent()?.parent()?;
    let candidates = [package.join("daemon").join("specs"), package.parent()?.join("daemon").join("specs")];
    candidates.into_iter().find(|folder| folder.join("rigs").join("launch").is_dir() && folder.join("agents").is_dir())
}

/// A filled box with a border in `edge` color, holding `add`.
fn notice_box(ui: &mut Ui, edge: egui::Color32, add: impl FnOnce(&mut Ui)) {
    let frame = egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(egui::Stroke::new(1.0, edge))
        .corner_radius(BOX_RADIUS)
        .inner_margin(BOX_MARGIN);
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 6.0;
        add(ui);
    });
}

/// The banner over a read-only team (artboard `StateReadOnly`): why it
/// cannot be changed, and "Duplicate to edit", which requests
/// `TeamEvent::Duplicate`.
pub(super) fn read_only_banner(ui: &mut Ui, editor: &mut TeamEditor) {
    let why = if is_built_in(&editor.spec_folder) {
        "Built-in team that comes with OpenRig. You can read it and launch it. To change it, make your own copy."
    } else {
        "This file cannot be written. You can read it and launch it. To change it, make your own copy."
    };
    let mut duplicate = false;
    notice_box(ui, ui.visuals().widgets.noninteractive.bg_stroke.color, |ui| {
        ui.horizontal(|ui| {
            ui.label(why);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                duplicate = ui.button("Duplicate to edit").clicked();
            });
        });
    });
    if duplicate {
        editor.request(TeamEvent::Duplicate);
    }
}

/// Shown instead of the form while the YAML cannot be read: the error and
/// a button to the YAML view.
pub(super) fn yaml_error_notice(ui: &mut Ui, editor: &mut TeamEditor) {
    let Some(error) = &editor.parse_error else { return };
    let mut show_yaml = false;
    notice_box(ui, ui.visuals().warn_fg_color, |ui| {
        ui.label(RichText::new("This file cannot be shown as a form").strong());
        ui.label(RichText::new(error).monospace().color(ui.visuals().warn_fg_color));
        let hint = "Fix it in the YAML view. The form comes back when the YAML reads cleanly.";
        ui.label(RichText::new(hint).size(SMALL_SIZE).weak());
        show_yaml = ui.button("Show YAML").clicked();
    });
    if show_yaml {
        editor.request(TeamEvent::ShowYaml);
    }
}

/// The notice when `rig` is not installed (artboard `StateNoRig`), shown
/// when a validation, preflight, plan or launch answered
/// [`RigCallError::NotInstalled`]: what is missing and how to install it.
/// The form stays usable.
pub(super) fn rig_missing_notice(ui: &mut Ui, editor: &mut TeamEditor) {
    let missing = [
        editor.validation.result.as_ref().and_then(|result| result.as_ref().err()),
        editor.preflight.result.as_ref().and_then(|result| result.as_ref().err()),
        editor.plan.result.as_ref().and_then(|result| result.as_ref().err()),
        editor.launch.result.as_ref().and_then(|result| result.as_ref().err()),
    ]
    .into_iter()
    .flatten()
    .find_map(|error| match error {
        RigCallError::NotInstalled(message) => Some(message.clone()),
        RigCallError::Failed(_) => None,
    });
    let Some(message) = missing else { return };
    let mut open_settings = false;
    notice_box(ui, ui.visuals().warn_fg_color, |ui| {
        ui.label(RichText::new("OpenRig's rig command was not found").strong().color(ui.visuals().warn_fg_color));
        ui.label("You can still edit and save this team. Checking and launching need rig.");
        ui.label(RichText::new(message).size(SMALL_SIZE).weak());
        let after = "Settings > OpenRig shows whether rig is found. After installing it, press Validate to look again.";
        ui.label(RichText::new(after).size(SMALL_SIZE).weak());
        open_settings = ui.button("Open Settings").clicked();
    });
    if open_settings {
        editor.request(TeamEvent::OpenSettings);
    }
}

/// Where a validator path points in the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// A seat or one of its fields.
    Seat(SeatRef),
    /// A link inside the pod at this index.
    PodLink(usize),
    /// The pod at this index (its id or label).
    Pod(usize),
    /// A link between pods, at this index.
    TeamLink(usize),
    /// A team setting.
    Team,
    /// The message names no place.
    Nowhere,
}

/// The place a validator path (`pods[1].members[2].id`) points at.
fn locate(path: &str) -> Place {
    if path.is_empty() {
        return Place::Nowhere;
    }
    if let Some((index, _)) = take_index(path, "edges") {
        return Place::TeamLink(index);
    }
    let Some((pod, rest)) = take_index(path, "pods") else { return Place::Team };
    let rest = rest.strip_prefix('.').unwrap_or(rest);
    if let Some((member, _)) = take_index(rest, "members") {
        Place::Seat(SeatRef { pod, member })
    } else if take_index(rest, "edges").is_some() {
        Place::PodLink(pod)
    } else {
        Place::Pod(pod)
    }
}

/// For `path` starting with `key[<n>]`: `n` and the text after the `]`.
fn take_index<'a>(path: &'a str, key: &str) -> Option<(usize, &'a str)> {
    let inside = path.strip_prefix(key)?.strip_prefix('[')?;
    let (number, rest) = inside.split_once(']')?;
    Some((number.parse().ok()?, rest))
}

/// Where a problem is, in words: "Development, seat qa", "Team".
fn describe_place(spec: &RigSpec, place: Place) -> String {
    let pod_label =
        |index: usize| spec.pods.get(index).map_or_else(|| format!("Pod {}", index + 1), |pod| pod.name().to_owned());
    match place {
        Place::Seat(seat) => {
            let member = spec.pods.get(seat.pod).and_then(|pod| pod.members.get(seat.member));
            let seat_name = member.map_or_else(|| format!("{}", seat.member + 1), |member| member.id.clone());
            format!("{}, seat {seat_name}", pod_label(seat.pod))
        }
        Place::PodLink(pod) => format!("{}, a link", pod_label(pod)),
        Place::Pod(pod) => pod_label(pod),
        Place::TeamLink(index) => format!("Link between pods {}", index + 1),
        Place::Team | Place::Nowhere => "Team".to_owned(),
    }
}

/// Show the field at `place`: the view that holds it, and the seat in the inspector.
fn reveal(editor: &mut TeamEditor, place: Place) {
    match place {
        Place::Seat(seat) => {
            editor.tab = SubTab::Seats;
            editor.selected = Some(seat);
        }
        Place::Pod(_) => editor.tab = SubTab::Seats,
        Place::PodLink(_) | Place::TeamLink(_) => editor.tab = SubTab::Links,
        Place::Team => editor.tab = SubTab::Team,
        Place::Nowhere => {}
    }
}

/// The result of the last `rig spec validate` (artboard `StateValidation`):
/// valid, or the errors as a list where a click on one with a path reveals
/// its field (switch `editor.tab`, set `editor.selected`). The fields show
/// their own errors through `TeamEditor::issue`.
pub(super) fn validation_summary(ui: &mut Ui, editor: &mut TeamEditor) {
    let validation = match &editor.validation.result {
        Some(Ok(validation)) => Some(validation.clone()),
        Some(Err(RigCallError::Failed(message))) => {
            ui.label(RichText::new(format!("Validation could not run: {message}")).color(ui.visuals().warn_fg_color));
            None
        }
        _ => None,
    };
    let mut problems = validation.as_ref().map(|validation| validation.errors.clone()).unwrap_or_default();
    for issue in &editor.issues {
        if !problems.contains(issue) {
            problems.push(issue.clone());
        }
    }
    if problems.is_empty() {
        if let Some(validation) = validation.filter(|validation| validation.valid) {
            valid_notice(ui, &validation.advisories);
        }
        return;
    }
    let source = if validation.is_some() {
        "from rig spec validate and the form. Launch waits until they are fixed."
    } else {
        "found by the form. Validate asks OpenRig."
    };
    let mut clicked = None;
    notice_box(ui, ui.visuals().error_fg_color, |ui| {
        ui.horizontal(|ui| {
            let count = format!("{} {}", problems.len(), plural(problems.len(), "problem", "problems"));
            ui.label(RichText::new(count).strong().color(ui.visuals().error_fg_color));
            ui.label(RichText::new(source).size(SMALL_SIZE).weak());
        });
        for issue in &problems {
            let place = locate(&issue.path);
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{}: {}", describe_place(&editor.spec, place), issue.message));
                if !issue.path.is_empty() {
                    ui.label(RichText::new(&issue.path).monospace().size(SMALL_SIZE).weak());
                }
                if place != Place::Nowhere && ui.link("Show").clicked() {
                    clicked = Some(place);
                }
            });
        }
    });
    if let Some(place) = clicked {
        reveal(editor, place);
    }
}

/// "Valid" after a clean `rig spec validate`, with its advisories.
fn valid_notice(ui: &mut Ui, advisories: &[String]) {
    ui.label(RichText::new("Valid: rig spec validate found no problems.").color(ui.visuals().hyperlink_color));
    for advisory in advisories {
        ui.label(RichText::new(advisory).size(SMALL_SIZE).color(ui.visuals().warn_fg_color));
    }
}

/// The result of the last preflight (artboard `StatePreflight`): ready,
/// with the launch plan when one was asked for, or the problems and "Check
/// again". The only control that launches the team for real is the Launch
/// button under a plan that found nothing in the way.
pub(super) fn preflight_result(ui: &mut Ui, editor: &mut TeamEditor) {
    let preflight = match &editor.preflight.result {
        Some(Ok(preflight)) => preflight.clone(),
        Some(Err(RigCallError::Failed(message))) => {
            ui.label(RichText::new(format!("Preflight could not run: {message}")).color(ui.visuals().warn_fg_color));
            return;
        }
        _ => return,
    };
    let visuals = ui.visuals().clone();
    let (title, edge) = if preflight.ready {
        ("Ready to launch".to_owned(), visuals.hyperlink_color)
    } else {
        let count = preflight.errors.len();
        (format!("{count} {} before launch", plural(count, "problem", "problems")), visuals.error_fg_color)
    };
    let mut event = None;
    notice_box(ui, edge, |ui| {
        ui.label(RichText::new(title).strong().color(edge));
        for error in &preflight.errors {
            ui.label(RichText::new(format!("\u{2022} {error}")).color(visuals.error_fg_color));
        }
        for warning in &preflight.warnings {
            ui.label(RichText::new(format!("\u{2022} {warning}")).color(visuals.warn_fg_color));
        }
        if preflight.ready {
            event = launch_plan(ui, editor);
        } else {
            ui.label(RichText::new("From rig spec preflight.").size(SMALL_SIZE).weak());
            if ui.button("Check again").clicked() {
                event = Some(TeamEvent::Validate);
            }
        }
    });
    if let Some(event) = event {
        editor.request(event);
    }
}

/// Under a ready preflight: the launch plan (`rig up --plan`) with the
/// seats it starts and the Launch button, or why there is none yet.
/// Returns the event a click asked for.
fn launch_plan(ui: &mut Ui, editor: &TeamEditor) -> Option<TeamEvent> {
    let note = |ui: &mut Ui, text: &str| ui.label(RichText::new(text).size(SMALL_SIZE).weak());
    if editor.plan.is_running() {
        note(ui, "Asking rig up --plan what the launch starts. Nothing starts yet.");
        return None;
    }
    let plan = match &editor.plan.result {
        None => {
            note(ui, "From rig spec preflight. Nothing has started yet.");
            let show = ui.button("Show the launch plan").clicked();
            return show.then_some(TeamEvent::PrepareLaunch);
        }
        Some(Err(error)) => {
            ui.label(
                RichText::new(format!("The launch plan could not run: {error}")).color(ui.visuals().warn_fg_color),
            );
            return None;
        }
        Some(Ok(plan)) => plan,
    };
    let visuals = ui.visuals().clone();
    if !plan.ok {
        ui.label(RichText::new("The launch plan found problems").strong().color(visuals.error_fg_color));
        for line in plan.message.iter().chain(&plan.errors) {
            ui.label(RichText::new(format!("\u{2022} {line}")).color(visuals.error_fg_color));
        }
        return ui.button("Check again").clicked().then_some(TeamEvent::PrepareLaunch);
    }
    let addresses = seat_addresses(&editor.spec);
    let count = addresses.len();
    ui.label(RichText::new(format!("Launch will start {count} {}", plural(count, "seat", "seats"))).strong());
    ui.horizontal_wrapped(|ui| {
        for address in &addresses {
            ui.label(RichText::new(address).monospace());
        }
    });
    for warning in &plan.warnings {
        ui.label(RichText::new(format!("\u{2022} {warning}")).color(visuals.warn_fg_color));
    }
    note(ui, "From rig spec preflight and rig up --plan. Nothing has started yet.");
    let launching = editor.launch.is_running();
    let label = if launching { "Launching".to_owned() } else { format!("Launch {}", editor.spec.name) };
    ui.add_enabled(!launching, egui::Button::new(label)).clicked().then_some(TeamEvent::Launch)
}

/// The result of the last real launch (`rig up`): the seats started, or why not.
pub(super) fn launch_result(ui: &mut Ui, editor: &mut TeamEditor) {
    let reply = match &editor.launch.result {
        Some(Ok(reply)) => reply,
        Some(Err(RigCallError::Failed(message))) => {
            ui.label(RichText::new(format!("rig up could not run: {message}")).color(ui.visuals().warn_fg_color));
            return;
        }
        _ => return,
    };
    let visuals = ui.visuals().clone();
    let (title, edge) = match (&reply.rig_result, reply.ok) {
        (Some(result), true) => (format!("Launched: {}", result.replace('_', " ")), visuals.hyperlink_color),
        (None, true) => ("Launched".to_owned(), visuals.hyperlink_color),
        (_, false) => ("The team did not launch".to_owned(), visuals.error_fg_color),
    };
    notice_box(ui, edge, |ui| {
        ui.label(RichText::new(title).strong().color(edge));
        if let Some(message) = &reply.message {
            ui.label(message);
        }
        for error in &reply.errors {
            ui.label(RichText::new(format!("\u{2022} {error}")).color(visuals.error_fg_color));
        }
        for warning in &reply.warnings {
            ui.label(RichText::new(format!("\u{2022} {warning}")).color(visuals.warn_fg_color));
        }
    });
}

/// The changes between the saved team and the form, one short line each
/// ("Seat dev.build: model changed", "Pod review added"), for the header's
/// "N unsaved changes" and the unsaved-changes dialog. Pods match by id,
/// seats by id within their pod, links by kind and ends.
pub(in crate::app) fn describe_changes(saved: &RigSpec, current: &RigSpec) -> Vec<String> {
    if saved == current {
        return Vec::new();
    }
    let mut changes = Vec::new();
    if let Some(fields) = changed_fields(&team_fields(saved), &team_fields(current)) {
        changes.push(format!("Team: {fields} changed"));
    }
    for pod in &saved.pods {
        if !current.pods.iter().any(|other| other.id == pod.id) {
            changes.push(format!("Pod {} removed", pod.id));
        }
    }
    for pod in &current.pods {
        match saved.pods.iter().find(|other| other.id == pod.id) {
            Some(before) => describe_pod_changes(&mut changes, before, pod),
            None => changes.push(format!("Pod {} added", pod.id)),
        }
    }
    describe_link_changes(&mut changes, "Link", &saved.edges, &current.edges);
    if changes.is_empty() {
        // Something the lines above do not name, such as the order of items.
        changes.push("Team changed".to_owned());
    }
    changes
}

/// The changes inside one pod that is in both versions.
fn describe_pod_changes(changes: &mut Vec<String>, before: &Pod, after: &Pod) {
    let pod_fields = |pod: &Pod| vec![("label", pod.label.clone()), ("summary", pod.summary.clone())];
    if let Some(fields) = changed_fields(&pod_fields(before), &pod_fields(after)) {
        changes.push(format!("Pod {}: {fields} changed", after.id));
    }
    for member in &before.members {
        if !after.members.iter().any(|other| other.id == member.id) {
            changes.push(format!("Seat {}.{} removed", after.id, member.id));
        }
    }
    for member in &after.members {
        match before.members.iter().find(|other| other.id == member.id) {
            Some(old) => {
                if let Some(fields) = changed_fields(&member_fields(old), &member_fields(member)) {
                    changes.push(format!("Seat {}.{}: {fields} changed", after.id, member.id));
                }
            }
            None => changes.push(format!("Seat {}.{} added", after.id, member.id)),
        }
    }
    describe_link_changes(changes, &format!("Pod {}: link", after.id), &before.edges, &after.edges);
}

/// Links in `before` but not `after` (removed) and the other way (added).
fn describe_link_changes(changes: &mut Vec<String>, prefix: &str, before: &[Edge], after: &[Edge]) {
    let same = |one: &Edge, other: &Edge| (&one.kind, &one.from, &one.to) == (&other.kind, &other.from, &other.to);
    let words = |edge: &Edge| format!("{} {} {}", edge.from, kind_in_words(&edge.kind), edge.to);
    for edge in before.iter().filter(|edge| !after.iter().any(|other| same(edge, other))) {
        changes.push(format!("{prefix} {} removed", words(edge)));
    }
    for edge in after.iter().filter(|edge| !before.iter().any(|other| same(edge, other))) {
        changes.push(format!("{prefix} {} added", words(edge)));
    }
}

/// The team settings the form edits, by their name in a change line. Each
/// value is its debug text, which is enough to tell equal from changed.
fn team_fields(spec: &RigSpec) -> Vec<(&'static str, String)> {
    vec![
        ("name", spec.name.clone()),
        ("version", spec.version.clone()),
        ("summary", spec.summary.clone()),
        ("culture file", spec.culture_file.clone()),
        ("permissions", spec.permission_policy.clone()),
        ("non-interruptive", format!("{:?}", spec.non_interruptive)),
        ("managed blocks", spec.claude_instruction_file.clone()),
        ("environment", format!("{:?}", spec.env)),
        ("workspace", format!("{:?}", spec.workspace)),
        ("docs", format!("{:?}", spec.docs)),
        ("startup", format!("{:?}", spec.startup)),
    ]
}

/// A seat's fields, by their name in a change line (see [`team_fields`]).
fn member_fields(member: &Member) -> Vec<(&'static str, String)> {
    vec![
        ("label", member.label.clone()),
        ("agent", member.agent_ref.clone()),
        ("profile", member.profile.clone()),
        ("runtime", member.runtime.clone()),
        ("model", member.model.clone()),
        ("effort", member.effort.clone()),
        ("folder", member.cwd.clone()),
        ("role", member.role.clone()),
        ("permissions", member.permission_policy.clone()),
        ("restart policy", member.restore_policy.clone()),
        ("compaction", member.compaction_strategy.clone()),
        ("mechanic", member.mechanic.clone()),
        ("environment", format!("{:?}", member.env)),
        ("startup files", format!("{:?}", member.startup)),
        ("Claude Code settings", format!("{:?}", member.claude)),
        ("Codex settings", format!("{:?}", member.codex)),
    ]
}

/// The names of the fields that differ, as "model and effort", or `None`.
fn changed_fields(before: &[(&'static str, String)], after: &[(&'static str, String)]) -> Option<String> {
    let names: Vec<&str> = before.iter().zip(after).filter(|(old, new)| old.1 != new.1).map(|(old, _)| old.0).collect();
    match names.as_slice() {
        [] => None,
        [one] => Some((*one).to_owned()),
        [rest @ .., last] => Some(format!("{} and {last}", rest.join(", "))),
    }
}

/// The body of the unsaved-changes dialog for a team (artboard
/// `StateUnsaved`): what changed, and that comments and the fields the form
/// does not show stay as they are.
pub(in crate::app) fn unsaved_changes_text(editor: &TeamEditor) -> String {
    let changes = editor.saved.as_ref().map(|saved| describe_changes(saved, &editor.spec)).unwrap_or_default();
    let mut text = if changes.is_empty() {
        "You changed the team.".to_owned()
    } else {
        let mut lines: Vec<String> = changes.iter().take(CHANGES_SHOWN).map(|change| format!("{change}.")).collect();
        if changes.len() > CHANGES_SHOWN {
            let more = changes.len() - CHANGES_SHOWN;
            lines.push(format!("And {more} more {}.", plural(more, "change", "changes")));
        }
        lines.join(" ")
    };
    text.push_str(" Comments and fields the form does not show stay as they are.");
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openrig::spec::EDGE_KINDS;
    use crate::test_support::TempFolder;

    /// A dialog asking for `template` named `name`.
    fn request(template: Template, name: &str, keep_builtin_agents: bool) -> NewTeamDialog {
        NewTeamDialog { template, name: name.to_owned(), keep_builtin_agents, ..NewTeamDialog::default() }
    }

    /// A fake `OpenRig` install: its spec folder with the starter's files
    /// and an agents folder.
    fn fake_install(temp: &TempFolder) -> PathBuf {
        temp.write("openrig/daemon/specs/rigs/launch/starter/rig.yaml", STARTER_TEXT);
        temp.write("openrig/daemon/specs/rigs/launch/starter/CULTURE.md", "# Culture\n");
        temp.write("openrig/daemon/specs/rigs/launch/starter/guidance/lead-first-move.md", "First move.\n");
        temp.write("openrig/daemon/specs/agents/development/implementer/agent.yaml", "name: implementer\n");
        temp.path().join("openrig/daemon/specs")
    }

    /// The dialog and every notice draw without an answer or a panic, and a
    /// read-only team asks for nothing until its button is pressed.
    #[test]
    fn states_draw() {
        use crate::document::text_file::{Document, Opened};
        use crate::openrig::spec::Issue;
        use crate::openrig::{Preflight, SpecValidation};

        let temp = TempFolder::new();
        let path = temp.write("openrig/daemon/specs/rigs/launch/starter/rig.yaml", STARTER_TEXT);
        let Opened::Text(document) = Document::open(&path, None).unwrap() else { panic!("a text file") };
        let mut editor = TeamEditor::new(&document);
        let validation = SpecValidation {
            valid: false,
            errors: vec![Issue::at("pods[0].members[1].id", "two seats are called build")],
            advisories: Vec::new(),
        };
        editor.validation.result = Some(Ok(validation));
        let preflight = Preflight { ready: false, warnings: vec!["no policy".into()], errors: vec!["no agent".into()] };
        editor.preflight.result = Some(Ok(preflight));
        editor.launch.result = Some(Err(RigCallError::NotInstalled("Install rig.".into())));
        let mut dialog = NewTeamDialog { looked_for_openrig: true, ..NewTeamDialog::default() };

        let ctx = Context::default();
        let mut answer = Some(NewTeamChoice::Create);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            answer = new_team_dialog(ui.ctx(), &mut dialog);
            read_only_banner(ui, &mut editor);
            editor.parse_error = Some("bad indent".into());
            yaml_error_notice(ui, &mut editor);
            rig_missing_notice(ui, &mut editor);
            validation_summary(ui, &mut editor);
            preflight_result(ui, &mut editor);
            launch_result(ui, &mut editor);
        });
        output.textures_delta.clear();
        assert_eq!(answer, None);
        assert_eq!(editor.events, []);
    }

    /// A ready preflight offers the launch plan; only a plan that found
    /// nothing in the way shows the seats and the Launch button.
    #[test]
    fn launch_button_needs_a_clean_plan() {
        use super::super::test_driver::Driver;
        use crate::openrig::{Preflight, UpReply};

        let mut driver = Driver::new(STARTER_TEXT, false, preflight_result);
        let ready = Preflight { ready: true, warnings: Vec::new(), errors: Vec::new() };
        driver.editor.preflight.result = Some(Ok(ready));
        driver.frame(Vec::new());
        assert!(driver.find("Launch starter").is_empty(), "no Launch before a plan");
        driver.click_text("Show the launch plan");
        assert_eq!(driver.editor.events, [TeamEvent::PrepareLaunch]);
        driver.editor.events.clear();

        let plan = UpReply { ok: true, rig_result: None, message: None, errors: Vec::new(), warnings: Vec::new() };
        driver.editor.plan.result = Some(Ok(UpReply { ok: false, ..plan.clone() }));
        driver.frame(Vec::new());
        assert!(driver.find("Launch starter").is_empty(), "no Launch after a blocked plan");

        driver.editor.plan.result = Some(Ok(plan));
        driver.frame(Vec::new());
        assert_eq!(driver.find("dev-review@starter").len(), 1);
        driver.click_text("Launch starter");
        assert_eq!(driver.editor.events, [TeamEvent::Launch]);
    }

    /// Valid names pass; empty, odd characters, traversal and hidden names do not.
    #[test]
    fn checks_team_names() {
        for name in ["my-team", "team_2", "v1.0-crew", "A"] {
            assert_eq!(check_team_name(name), Ok(()), "{name}");
        }
        for name in ["", ".", "..", "../evil", "a/b", "a\\b", "/abs", ".hidden", "with space", "caf\u{e9}", "x\0y", "~"]
        {
            assert!(check_team_name(name).is_err(), "{name:?} should be refused");
        }
        assert!(check_team_name(&"a".repeat(MAX_NAME_LENGTH)).is_ok());
        assert!(check_team_name(&"a".repeat(MAX_NAME_LENGTH + 1)).is_err());
    }

    /// The starter from an install gets the new name, absolute agent links
    /// and its culture file and guidance; comments in the text survive.
    #[test]
    fn creates_a_starter_team_from_the_install() {
        let temp = TempFolder::new();
        let installed = fake_install(&temp);
        let spec_root = temp.path().join("home/.openrig/specs");
        let path = create_team_in(&request(Template::Starter, "crew", true), &spec_root, Some(&installed)).unwrap();
        assert_eq!(path, spec_root.join("crew/rig.yaml"));
        let spec = RigSpec::parse(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(spec.name, "crew");
        let implementer = installed.join("agents/development/implementer");
        assert_eq!(spec.pods[0].members[0].agent_ref, format!("path:{}", implementer.display()));
        assert_eq!(fs::read_to_string(spec_root.join("crew/CULTURE.md")).unwrap(), "# Culture\n");
        assert!(spec_root.join("crew/guidance/lead-first-move.md").is_file());
        assert_eq!(spec.check(), []);
    }

    /// Without an install (or without the checkbox) the links stay as
    /// written and the team is still created.
    #[test]
    fn creates_a_team_without_an_install() {
        let temp = TempFolder::new();
        let spec_root = temp.path().join("specs");
        let path = create_team_in(&request(Template::Factory, "plant", true), &spec_root, None).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text, FACTORY_TEXT.replacen("name: factory", "name: plant", 1));
        assert!(!spec_root.join("plant/CULTURE.md").exists());

        let installed = fake_install(&temp);
        let path = create_team_in(&request(Template::Starter, "own", false), &spec_root, Some(&installed)).unwrap();
        let spec = RigSpec::parse(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(spec.pods[0].members[0].agent_ref, "local:../../../agents/development/implementer");
    }

    /// A blank team has one pod and one seat, and only the agent is left to fill.
    #[test]
    fn creates_a_blank_team() {
        let temp = TempFolder::new();
        let path = create_team_in(&request(Template::Blank, "solo", true), temp.path(), None).unwrap();
        let spec = RigSpec::parse(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!((spec.name.as_str(), spec.pods.len(), spec.seat_count()), ("solo", 1, 1));
        let paths: Vec<String> = spec.check().into_iter().map(|issue| issue.path).collect();
        assert_eq!(paths, ["pods[0].members[0].agent_ref"]);
    }

    /// Names that YAML could read as numbers or booleans still read back as text.
    #[test]
    fn odd_names_stay_text() {
        let temp = TempFolder::new();
        for name in ["1.0", "true", "null", "1e3"] {
            let path = create_team_in(&request(Template::Blank, name, true), temp.path(), None).unwrap();
            let text = fs::read_to_string(path).unwrap();
            let quoted = [format!("name: \"{name}\"\n"), format!("name: '{name}'\n")];
            assert!(quoted.iter().any(|line| text.contains(line.as_str())), "{text}");
            assert_eq!(RigSpec::parse(&text).unwrap().name, name);
        }
    }

    /// An existing folder, file or link of that name is never written over,
    /// and a bad name never reaches the file system.
    #[test]
    fn never_overwrites_or_escapes() {
        let temp = TempFolder::new();
        let spec_root = temp.path().join("specs");
        temp.write("specs/taken/rig.yaml", "mine");
        let error = create_team_in(&request(Template::Starter, "taken", true), &spec_root, None).unwrap_err();
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(fs::read_to_string(spec_root.join("taken/rig.yaml")).unwrap(), "mine");

        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, spec_root.join("linked")).unwrap();
        assert!(create_team_in(&request(Template::Blank, "linked", true), &spec_root, None).is_err());
        assert!(!outside.join("rig.yaml").exists());

        for name in ["../escape", "..", "a/b"] {
            assert!(create_team_in(&request(Template::Blank, name, true), &spec_root, None).is_err());
        }
        assert!(!temp.path().join("escape").exists());
        assert_eq!(name_problem("taken", Some(&spec_root)).map(|_| ()), Some(()));
        assert_eq!(name_problem("free", Some(&spec_root)), None);
    }

    /// Companion files must be plain relative paths inside the team folder.
    #[test]
    fn copies_only_files_inside_the_team() {
        let mut spec = RigSpec::parse(STARTER_TEXT).unwrap();
        spec.docs = vec!["../secret.md".into(), "/etc/passwd".into(), "docs/a.md".into(), "CULTURE.md".into()];
        assert_eq!(companion_files(&spec), ["CULTURE.md", "guidance/lead-first-move.md", "docs/a.md"]);
    }

    /// A copy takes the first free `-copy` name, keeps the text around the
    /// changes and points its agent links at the original folder.
    #[test]
    fn duplicates_under_a_free_name() {
        let temp = TempFolder::new();
        let installed = fake_install(&temp);
        let source = installed.join("rigs/launch/starter/rig.yaml");
        let spec_root = temp.path().join("specs");
        let text = STARTER_TEXT.replace("pods:\n", "# the team\npods:\n");
        let first = duplicate_team_in(&source, &text, &spec_root).unwrap();
        assert_eq!(first, spec_root.join("starter-copy/rig.yaml"));
        let second = duplicate_team_in(&source, &text, &spec_root).unwrap();
        assert_eq!(second, spec_root.join("starter-copy-2/rig.yaml"));

        let written = fs::read_to_string(&first).unwrap();
        assert!(written.contains("# the team\npods:\n"), "{written}");
        let spec = RigSpec::parse(&written).unwrap();
        assert_eq!(spec.name, "starter-copy");
        let qa = installed.join("agents/development/qa");
        assert_eq!(spec.pods[0].members[1].agent_ref, format!("path:{}", qa.display()));
        assert!(spec_root.join("starter-copy/guidance/lead-first-move.md").is_file());
    }

    /// `local:` links resolve against the folder; other links stay.
    #[test]
    fn rewrites_local_links_only() {
        let mut spec = RigSpec::parse(STARTER_TEXT).unwrap();
        spec.pods[0].members[1].agent_ref = "path:/agents/qa".into();
        rewrite_agent_links(&mut spec, Path::new("/opt/openrig/daemon/specs/rigs/launch/starter"));
        assert_eq!(spec.pods[0].members[0].agent_ref, "path:/opt/openrig/daemon/specs/agents/development/implementer");
        assert_eq!(spec.pods[0].members[1].agent_ref, "path:/agents/qa");
    }

    /// The install is found through the `rig` link in either layout.
    #[test]
    fn finds_the_install_from_rig() {
        let temp = TempFolder::new();
        let installed = fake_install(&temp);
        let wrapper = temp.write("openrig/dist/bin-wrapper.js", "");
        let link = temp.path().join("bin/rig");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&wrapper, &link).unwrap();
        assert_eq!(specs_folder_for_rig(&link), Some(installed));

        let checkout = temp.write("repo/packages/cli/dist/bin-wrapper.js", "");
        temp.write("repo/packages/daemon/specs/rigs/launch/starter/rig.yaml", STARTER_TEXT);
        temp.write("repo/packages/daemon/specs/agents/qa/agent.yaml", "");
        assert_eq!(specs_folder_for_rig(&checkout), Some(temp.path().join("repo/packages/daemon/specs")));
        assert_eq!(specs_folder_for_rig(&temp.path().join("nowhere")), None);
    }

    /// The card counts come from the templates themselves.
    #[test]
    fn templates_parse_and_count() {
        assert_eq!(count_line(&Template::Starter.spec("x")), "1 pod, 2 seats");
        assert_eq!(count_line(&Template::Factory.spec("x")), "3 pods, 7 seats");
        assert_eq!(count_line(&Template::Blank.spec("x")), "1 pod, 1 seat");
        assert_eq!(address_list(&Template::Factory.spec("x")), "orch-lead@x, orch-advisor@x, dev-build@x and 4 more");
    }

    /// Changes read as short lines naming the pod, seat or link.
    #[test]
    fn describes_changes() {
        let saved = RigSpec::parse(FACTORY_TEXT).unwrap();
        assert_eq!(describe_changes(&saved, &saved), Vec::<String>::new());

        let mut current = saved.clone();
        current.pods[1].members[0].model = "opus".into();
        current.pods[1].members[0].effort = "high".into();
        current.pods[1].members.remove(2);
        current.pods[1].members.push(Member::new("docs", CLAUDE_RUNTIME));
        current.pods.push(Pod::new("ops", "Operations"));
        current.pods.remove(2);
        current.summary = "New summary.".into();
        current.pods[0].edges.push(Edge::new(EDGE_KINDS[3], "lead", "advisor"));
        let changes = describe_changes(&saved, &current);
        assert_eq!(
            changes,
            [
                "Team: summary changed",
                "Pod review removed",
                "Pod orch: link lead collaborates with advisor added",
                "Seat dev.design removed",
                "Seat dev.build: model and effort changed",
                "Seat dev.docs added",
                "Pod ops added",
            ]
        );
    }

    /// Validator paths map to the place in the form and a readable prefix.
    #[test]
    fn locates_validator_paths() {
        let spec = RigSpec::parse(FACTORY_TEXT).unwrap();
        let seat = locate("pods[1].members[2].id");
        assert_eq!(seat, Place::Seat(SeatRef { pod: 1, member: 2 }));
        assert_eq!(describe_place(&spec, seat), "Development, seat design");
        assert_eq!(locate("pods[2].label"), Place::Pod(2));
        assert_eq!(locate("pods[0].edges[1].to"), Place::PodLink(0));
        assert_eq!(locate("edges[3].from"), Place::TeamLink(3));
        assert_eq!(locate("culture_file"), Place::Team);
        assert_eq!(locate(""), Place::Nowhere);
        assert_eq!(locate("pods[x].id"), Place::Team);
    }
}
