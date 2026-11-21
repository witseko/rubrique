//! The Team view: the settings that apply to the whole team.
//!
//! Two columns of cards (name, summary and culture file; permissions;
//! workspace; environment), then a closed "Advanced" card for the rarer
//! keys. Keys the form does not edit are named there as kept as written.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, FontSelection, Id, RichText, Ui, WidgetText, text::LayoutJob};

use super::common::{
    CARD_GAP, FIELD_GAP, KEY_SIZE, LABEL_GAP, REMOVE_SIZE, TeamEditor, TeamEvent, choice, env_list, field_label, hint,
    is_safe_relative, issue_at, issue_line, kept_as_written, labelled_field, permissions_field, pill, pill_row,
    remove_button, remove_id, startup_file_list, text_field,
};
use crate::openrig::spec::{CLAUDE_INSTRUCTION_FILES, EnvVar, Issue, REPO_KINDS, Repo, Workspace, seat_address};
use crate::settings::view::{field_box, list_frame};

/// Space inside a card.
const CARD_MARGIN: i8 = 14;
/// Corner rounding of a card.
const CARD_RADIUS: u8 = 8;
/// Space around a row of a list (repositories).
const ROW_MARGIN: i8 = 6;
/// Lines the summary field shows.
const SUMMARY_ROWS: usize = 3;
/// Width of a short field in a list row (a repository name or path).
const SHORT_FIELD_WIDTH: f32 = 120.0;
/// The kind a new repository starts with.
const NEW_REPO_KIND: &str = "project";
/// Who owns the team view's remove buttons (see `remove_id`).
const OWNER: &str = "team";

/// Draw the Team view (artboard `TeamSettings`). Fields show their
/// problems; nothing can change when `editor.read_only`, but the culture
/// file still opens and Advanced still unfolds.
pub(super) fn show_team(ui: &mut Ui, editor: &mut TeamEditor) {
    let issues = editor.issues_under("");
    let enabled = !editor.read_only;
    ui.columns(2, |columns| {
        let [left, right] = columns else { return };
        identity_card(left, editor, &issues, enabled);
        left.add_space(CARD_GAP);
        card(left, |ui| {
            ui.add_enabled_ui(enabled, |ui| permissions(ui, &mut editor.spec.permission_policy, &issues));
        });
        card(right, |ui| workspace(ui, &mut editor.spec.workspace, &issues, enabled));
        right.add_space(CARD_GAP);
        card(right, |ui| ui.add_enabled_ui(enabled, |ui| environment(ui, &mut editor.spec.env, &issues)));
    });
    ui.add_space(CARD_GAP);
    let show_yaml = card(ui, |ui| advanced(ui, editor, &issues, enabled));
    if show_yaml {
        editor.request(TeamEvent::ShowYaml);
    }
}

/// An outlined card filling the width.
fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(CARD_RADIUS)
        .inner_margin(CARD_MARGIN)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Problems under a list row, each prefixed with the field it is about.
fn row_issues(ui: &mut Ui, issues: &[Issue], fields: &[(&str, String)]) {
    for (name, path) in fields {
        if let Some(issue) = issue_at(issues, path) {
            issue_line(ui, &format!("{name}: {}", issue.message));
        }
    }
}

/// A one-line monospace field of `width` in a list row.
fn row_field(ui: &mut Ui, value: &mut String, hint_text: &str, width: f32) -> egui::Response {
    ui.add(egui::TextEdit::singleline(value).hint_text(hint_text).font(egui::TextStyle::Monospace).desired_width(width))
}

/// The first card: team name with an example seat address, summary, and
/// culture file with Open (which works even on a read-only team).
fn identity_card(ui: &mut Ui, editor: &mut TeamEditor, issues: &[Issue], enabled: bool) {
    let spec = &mut editor.spec;
    let example = spec
        .pods
        .first()
        .and_then(|pod| pod.members.first().map(|member| seat_address(&pod.id, &member.id, &spec.name)));
    let culture_path = culture_path(&editor.spec_folder, &spec.culture_file);
    let mut open = false;
    card(ui, |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            let issue = issue_at(issues, "name");
            labelled_field(ui, "Team name", "name", true, issue, |ui| {
                text_field(ui, &mut spec.name, "my-team", issue.is_some());
                if let Some(example) = &example {
                    let text = format!(
                        "Part of every seat address: {example}. Changing it renames the team on its next launch."
                    );
                    hint(ui, &text);
                }
            });
            labelled_field(ui, "Summary", "summary", false, issue_at(issues, "summary"), |ui| {
                multiline_field(ui, &mut spec.summary);
            });
        });
        let issue = issue_at(issues, "culture_file");
        labelled_field(ui, "Culture file", "culture_file", false, issue, |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    open = ui.add_enabled(culture_path.is_some(), egui::Button::new("Open")).clicked();
                    ui.add_enabled_ui(enabled, |ui| {
                        text_field(ui, &mut spec.culture_file, "CULTURE.md", issue.is_some());
                    });
                });
            });
            hint(ui, "Shared rules every seat reads at start. A path inside the team folder.");
        });
    });
    if open && let Some(path) = culture_path {
        editor.request(TeamEvent::OpenPath(path));
    }
}

/// The culture file to open, when `culture_file` is a safe relative path.
fn culture_path(spec_folder: &Path, culture_file: &str) -> Option<PathBuf> {
    is_safe_relative(culture_file).then(|| spec_folder.join(culture_file))
}

/// A multi-line text field filling the width, in the Settings field box.
fn multiline_field(ui: &mut Ui, value: &mut String) {
    let visuals = ui.visuals().clone();
    field_box(ui, &visuals, false, |ui| {
        let edit = egui::TextEdit::multiline(value)
            .frame(egui::Frame::NONE)
            .desired_rows(SUMMARY_ROWS)
            .desired_width(f32::INFINITY);
        ui.add(edit);
    });
}

/// "Permissions for every seat": Not set, the built-in policies, or a
/// policy file, and the value that gets written.
fn permissions(ui: &mut Ui, policy: &mut String, issues: &[Issue]) {
    field_label(ui, "Permissions for every seat", "permission_policy", false);
    ui.add_space(LABEL_GAP);
    let unset = "Not set uses OpenRig's default floor. A seat can choose its own.";
    permissions_field(ui, policy, "Not set", unset, issue_at(issues, "permission_policy"));
}

/// The Workspace card: "+ Add workspace" when there is none, else its
/// fields and "Remove workspace".
fn workspace(ui: &mut Ui, workspace: &mut Option<Workspace>, issues: &[Issue], enabled: bool) {
    ui.horizontal(|ui| {
        field_label(ui, "Workspace", "workspace", false);
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new("Optional").weak().size(KEY_SIZE));
        });
    });
    ui.add_space(LABEL_GAP);
    ui.add_enabled_ui(enabled, |ui| {
        let Some(fields) = workspace else {
            hint(ui, "No workspace. Add one to give the seats a root folder and named repositories.");
            if ui.add(egui::Button::new("+ Add workspace").frame(false)).clicked() {
                *workspace = Some(Workspace::default());
            }
            return;
        };
        if workspace_fields(ui, fields, issues) {
            *workspace = None;
        }
    });
}

/// Root folder, repositories, default repo and knowledge folder. Returns
/// true when "Remove workspace" was clicked.
fn workspace_fields(ui: &mut Ui, workspace: &mut Workspace, issues: &[Issue]) -> bool {
    let issue = issue_at(issues, "workspace.workspace_root");
    labelled_field(ui, "Root folder", "workspace_root", true, issue, |ui| {
        text_field(ui, &mut workspace.workspace_root, "~/code/shop", issue.is_some())
    });
    field_label(ui, "Repositories", "repos", false);
    ui.add_space(LABEL_GAP);
    repositories(ui, &mut workspace.repos, issues);
    ui.add_space(FIELD_GAP);
    let issue = issue_at(issues, "workspace.default_repo");
    labelled_field(ui, "Default repo", "default_repo", false, issue, |ui| {
        default_repo_choice(ui, &mut workspace.default_repo, &workspace.repos);
    });
    let issue = issue_at(issues, "workspace.knowledge_root");
    labelled_field(ui, "Knowledge folder", "knowledge_root", false, issue, |ui| {
        text_field(ui, &mut workspace.knowledge_root, "none", issue.is_some())
    });
    ui.add(egui::Button::new("Remove workspace").frame(false)).clicked()
}

/// The repository rows (name, path, kind, remove) and "+ Add repository".
fn repositories(ui: &mut Ui, repos: &mut Vec<Repo>, issues: &[Issue]) {
    let mut remove = None;
    if !repos.is_empty() {
        list_frame(ui, |ui| {
            for (index, repo) in repos.iter_mut().enumerate() {
                if index > 0 {
                    ui.separator();
                }
                if repository_row(ui, repo, index, issues) {
                    remove = Some(index);
                }
            }
        });
    }
    if let Some(index) = remove {
        repos.remove(index);
    }
    ui.horizontal_wrapped(|ui| {
        if ui.add(egui::Button::new("+ Add repository").frame(false)).clicked() {
            repos.push(Repo::new("", "", NEW_REPO_KIND));
        }
        hint(ui, &format!("Kind: {}.", REPO_KINDS.join(", ")));
    });
}

/// One repository row with its problems. Returns true when remove was clicked.
fn repository_row(ui: &mut Ui, repo: &mut Repo, index: usize, issues: &[Issue]) -> bool {
    let mut removed = false;
    egui::Frame::new().inner_margin(ROW_MARGIN).show(ui, |ui| {
        ui.horizontal(|ui| {
            row_field(ui, &mut repo.name, "name", SHORT_FIELD_WIDTH);
            row_field(ui, &mut repo.path, "path", SHORT_FIELD_WIDTH);
            choice(ui, Id::unique(("team-repo-kind", index)), &mut repo.kind, &REPO_KINDS.map(|kind| (kind, kind)));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let id = remove_id(OWNER, "repos", index);
                removed = remove_button(ui, id, "Remove repository", "Remove this repository");
            });
        });
        let path = format!("workspace.repos[{index}]");
        let fields =
            [("Name", format!("{path}.name")), ("Path", format!("{path}.path")), ("Kind", format!("{path}.kind"))];
        row_issues(ui, issues, &fields);
    });
    removed
}

/// The default repo drop-down: none, or one of the named repositories.
fn default_repo_choice(ui: &mut Ui, default_repo: &mut String, repos: &[Repo]) {
    let mut choices = vec![("None", "")];
    choices
        .extend(repos.iter().filter(|repo| !repo.name.is_empty()).map(|repo| (repo.name.as_str(), repo.name.as_str())));
    choice(ui, Id::unique("team-default-repo"), default_repo, &choices);
}

/// The Environment card: one row per variable and "+ Add variable".
fn environment(ui: &mut Ui, env: &mut Vec<EnvVar>, issues: &[Issue]) {
    field_label(ui, "Environment", "env", false);
    hint(ui, "Every seat gets these; a seat's own win by name. Names in capitals, OPENRIG_ names are reserved.");
    ui.add_space(LABEL_GAP);
    env_list(ui, env, issues, "env", OWNER);
}

/// The closed "Advanced" card: docs, team startup files, the Claude
/// instruction file, launch warnings, and the keys kept as written.
/// Returns true when "Edit in YAML" was clicked.
fn advanced(ui: &mut Ui, editor: &mut TeamEditor, issues: &[Issue], enabled: bool) -> bool {
    let spec = &mut editor.spec;
    let header = advanced_header(ui.style(), &spec.yaml_only);
    let mut show_yaml = false;
    egui::CollapsingHeader::new(header).id_salt("team-advanced").default_open(false).show(ui, |ui| {
        ui.add_space(LABEL_GAP);
        ui.add_enabled_ui(enabled, |ui| {
            docs(ui, &mut spec.docs, issues);
            field_label(ui, "Team startup files", "startup", false);
            hint(ui, "Every seat gets these at launch, before its own.");
            ui.add_space(LABEL_GAP);
            show_yaml |= startup_file_list(ui, &mut spec.startup, issues, "", OWNER);
            ui.add_space(FIELD_GAP);
            instruction_file(ui, &mut spec.claude_instruction_file);
            launch_warnings(ui, &mut spec.non_interruptive);
        });
        show_yaml |= kept_as_written(ui, &spec.yaml_only);
    });
    show_yaml
}

/// "Advanced" with what it holds in small text after it.
fn advanced_header(style: &egui::Style, yaml_only: &[String]) -> WidgetText {
    let kept =
        if yaml_only.is_empty() { String::new() } else { format!(", kept as written: {}", yaml_only.join(", ")) };
    let contents = format!("docs, team startup files, Claude instruction file, launch warnings{kept}");
    let mut job = LayoutJob::default();
    RichText::new("Advanced  ").strong().append_to(&mut job, style, FontSelection::Default, Align::Center);
    RichText::new(contents).weak().small().append_to(&mut job, style, FontSelection::Default, Align::Center);
    WidgetText::from(job)
}

/// The `docs` paths: one row each, and "+ Add doc".
fn docs(ui: &mut Ui, docs: &mut Vec<String>, issues: &[Issue]) {
    field_label(ui, "Docs", "docs", false);
    hint(ui, "Files that travel with the team for people and agents to read. OpenRig does not load them.");
    ui.add_space(LABEL_GAP);
    let mut remove = None;
    for (index, path) in docs.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let width = (ui.available_width() - REMOVE_SIZE - ui.spacing().item_spacing.x).max(0.0);
            row_field(ui, path, "docs/setup.md", width);
            if remove_button(ui, remove_id(OWNER, "docs", index), "Remove doc", "Remove this doc") {
                remove = Some(index);
            }
        });
        row_issues(ui, issues, &[("Path", format!("docs[{index}].path"))]);
    }
    if let Some(index) = remove {
        docs.remove(index);
    }
    if ui.add(egui::Button::new("+ Add doc").frame(false)).clicked() {
        docs.push(String::new());
    }
    ui.add_space(FIELD_GAP);
}

/// Which file gets Claude's managed blocks (`managed_blocks.claude-code`).
fn instruction_file(ui: &mut Ui, file: &mut String) {
    field_label(ui, "Claude instruction file", "managed_blocks", false);
    ui.add_space(LABEL_GAP);
    let [shared, local] = CLAUDE_INSTRUCTION_FILES;
    pill_row(ui, file, &[("Default", ""), (shared, shared), (local, local)]);
    hint(ui, "Default writes to CLAUDE.md. Codex, Pi and OMP seats use AGENTS.md.");
    ui.add_space(FIELD_GAP);
}

/// The `non_interruptive` launch default: not set, hide or show warnings.
fn launch_warnings(ui: &mut Ui, non_interruptive: &mut Option<bool>) {
    field_label(ui, "Launch warnings", "non_interruptive", false);
    ui.add_space(LABEL_GAP);
    ui.horizontal(|ui| {
        for (label, value) in [("Not set", None), ("Hide", Some(true)), ("Show", Some(false))] {
            if pill(ui, label, *non_interruptive == value).clicked() {
                *non_interruptive = value;
            }
        }
    });
    hint(ui, "Hide accepts or hides harness warnings at launch for full-bypass seats. It grants no permissions.");
    ui.add_space(FIELD_GAP);
}

#[cfg(test)]
mod tests {
    use super::super::test_driver::Driver;
    use super::*;

    const FACTORY: &str = include_str!("../../openrig/spec/fixtures/factory.yaml");
    const STARTER: &str = include_str!("../../openrig/spec/fixtures/starter.yaml");

    /// Click the remove button of item `index` in the list `list`.
    fn click_remove(driver: &mut Driver, list: &str, index: usize) {
        let response = driver.ctx.read_response(remove_id(OWNER, list, index)).expect("the remove button is drawn");
        driver.click_at(response.rect.center());
    }

    /// Only a relative path inside the team folder opens.
    #[test]
    fn opens_only_paths_inside_the_team_folder() {
        let folder = Path::new("/specs/team");
        assert_eq!(culture_path(folder, "CULTURE.md"), Some(folder.join("CULTURE.md")));
        assert_eq!(culture_path(folder, "culture/./RULES.md"), Some(folder.join("culture/./RULES.md")));
        for unsafe_path in ["", "  ", "../other/CULTURE.md", "/etc/passwd", "culture/../../x.md"] {
            assert_eq!(culture_path(folder, unsafe_path), None, "{unsafe_path:?}");
        }
    }

    /// Name, permissions and environment change through the form.
    #[test]
    fn edits_team_fields() {
        let mut driver = Driver::new(FACTORY, false, show_team);
        driver.click_text("factory");
        driver.replace_text("crew");
        assert_eq!(driver.editor.spec.name, "crew");

        driver.click_text("Locked");
        assert_eq!(driver.editor.spec.permission_policy, "builtin:locked");
        driver.click_text("or a policy file");
        driver.replace_text("policies/mine.md");
        assert_eq!(driver.editor.spec.permission_policy, "policies/mine.md");
        driver.click_text("Not set");
        assert_eq!(driver.editor.spec.permission_policy, "");

        driver.click_text("+ Add variable");
        driver.click_text("NAME");
        driver.replace_text("log_level");
        assert_eq!(driver.editor.spec.env, [EnvVar { name: "LOG_LEVEL".into(), value: String::new() }]);
        click_remove(&mut driver, "env", 0);
        assert_eq!(driver.editor.spec.env, []);
    }

    /// A workspace is added with a repository, which is removed again.
    #[test]
    fn edits_the_workspace() {
        let mut driver = Driver::new(STARTER, false, show_team);
        assert_eq!(driver.editor.spec.workspace, None);
        driver.click_text("+ Add workspace");
        driver.click_text("+ Add repository");
        let repos = &driver.editor.spec.workspace.as_ref().expect("a workspace").repos;
        assert_eq!(repos, &[Repo::new("", "", "project")]);
        driver.click_text("project");
        driver.click_text("knowledge");
        assert_eq!(driver.editor.spec.workspace.as_ref().expect("a workspace").repos[0].kind, "knowledge");
        click_remove(&mut driver, "repos", 0);
        assert_eq!(driver.editor.spec.workspace.as_ref().expect("a workspace").repos, []);
        driver.click_text("Remove workspace");
        assert_eq!(driver.editor.spec.workspace, None);
    }

    /// Advanced opens on a click and edits docs, the instruction file and launch warnings.
    #[test]
    fn edits_advanced_fields() {
        let mut driver = Driver::new(FACTORY, false, show_team);
        assert!(driver.find("+ Add doc").is_empty(), "Advanced starts closed");
        driver.click_text("Advanced");
        driver.click_text("+ Add doc");
        assert_eq!(driver.editor.spec.docs, [String::new()]);
        driver.click_text("CLAUDE.local.md");
        assert_eq!(driver.editor.spec.claude_instruction_file, "CLAUDE.local.md");
        // The pill, not the hint under it that starts with the same word.
        driver.click_nth_text("Hide", 0);
        assert_eq!(driver.editor.spec.non_interruptive, Some(true));
        driver.click_text("+ Add startup file");
        assert_eq!(driver.editor.spec.startup.files.len(), 1);
    }

    /// A built-in team's settings cannot change, but its culture file opens
    /// and Advanced unfolds.
    #[test]
    fn read_only_blocks_team_edits() {
        let mut driver = Driver::new(FACTORY, true, show_team);
        let before = driver.editor.spec.clone();
        driver.click_text("factory");
        driver.replace_text("crew");
        driver.click_text("Locked");
        driver.click_text("+ Add variable");
        driver.click_text("+ Add workspace");
        driver.click_text("Advanced");
        driver.click_text("+ Add doc");
        driver.click_text("CLAUDE.local.md");
        assert_eq!(driver.editor.spec, before);

        // The first "Open" is the culture file's button; the last is a permission pill.
        driver.click_nth_text("Open", 0);
        let culture = driver.editor.spec_folder.join("CULTURE.md");
        assert_eq!(driver.editor.events, [TeamEvent::OpenPath(culture)]);
    }
}
