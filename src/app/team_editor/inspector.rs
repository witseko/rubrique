//! The seat inspector: the form for one seat, opened on the right of the tab.
//!
//! Basic fields come first; the rarely changed ones sit in a closed
//! "Advanced" section, with the Claude Code only or Codex only block for the
//! seat's runtime. A runtime change clears what the new runtime may not
//! have, so the spec stays valid (see [`apply_runtime_change`]).

use std::path::{Component, Path, PathBuf};

use eframe::egui::{self, RichText, Ui};

use super::common::{
    FIELD_GAP, KEY_SIZE, LABEL_GAP, REMOVE_SIZE, SeatRef, TeamEditor, TeamEvent, env_list, field_label, hint, issue_at,
    kept_as_written, labelled_field, permissions_field, pill_row, startup_file_list, text_field,
};
use crate::icons::{Icon, small_icon_button};
use crate::openrig::spec::{
    self, CLAUDE_RUNTIME, CODEX_APPROVALS, CODEX_RUNTIME, CODEX_SANDBOXES, COMPACTION_STRATEGIES, ClaudeFields,
    CodexFields, Issue, Member, RESTORE_POLICIES, RUNTIMES, TERMINAL_AGENT, TERMINAL_PROFILE, TERMINAL_RUNTIME,
};
use crate::settings::view::list_frame;

/// Who owns the inspector's remove buttons (see `remove_id`).
const OWNER: &str = "seat";
/// The Codex sandbox that allows network access and extra writable folders.
const WORKSPACE_WRITE: &str = CODEX_SANDBOXES[0];
/// The compaction strategy that asks a mechanic.
const APPRENTICE_HANDOVER: &str = COMPACTION_STRATEGIES[3];
/// `claude_mcp` value that turns MCP servers off.
const MCP_OFF: &str = "none";
/// Room kept on the right of a field for its Choose or Browse button.
const SIDE_BUTTON_WIDTH: f32 = 72.0;
/// Space inside the terminal note and the tool rule rows.
const BOX_MARGIN: i8 = 8;
/// Corner rounding of the terminal note.
const NOTE_RADIUS: u8 = 6;
/// Space before a section heading inside Advanced.
const SECTION_GAP: f32 = 8.0;

/// What the seat's form needs besides the seat itself.
struct SeatForm<'a> {
    /// The seat's validator path: `pods[0].members[1]`.
    prefix: &'a str,
    /// Problems under `prefix`, as `TeamEditor::issue` reports them.
    issues: &'a [Issue],
    spec_folder: &'a Path,
    /// The team's `permission_policy`, for the "Same as team" hint.
    team_policy: &'a str,
    address: &'a str,
    /// False for a read-only team.
    editable: bool,
    /// True when this computer's folder picker can choose the seat's paths
    /// (not for a team mirrored from another computer).
    can_pick_folders: bool,
}

impl SeatForm<'_> {
    /// The problem at the seat's `key` (`id`, `env.NAME`), if any.
    fn issue(&self, key: &str) -> Option<&Issue> {
        issue_at(self.issues, &format!("{}.{key}", self.prefix))
    }
}

/// What a click in the inspector asks for; done after drawing, when the
/// seat is no longer borrowed.
#[derive(Default)]
struct Requests {
    event: Option<TeamEvent>,
    close: bool,
    remove: bool,
    /// The id typed this frame, applied with its links after drawing.
    rename_to: Option<String>,
}

/// Draw the inspector for the seat `seat` (artboards `SeatInspector`,
/// `SeatAdvanced`, `StateTerminal`). The root calls it inside a right-hand
/// panel of the tab, only for a seat that exists.
///
/// Draws the title row with the live address and a close button, the Basic
/// fields, the closed "Advanced" section, the keys kept as written and
/// "Remove seat" (which also removes the links naming the seat). Each field
/// shows `editor.issue` for its path. Nothing changes when
/// `editor.read_only`, but the inspector still opens agent.yaml and closes.
pub(super) fn show_inspector(ui: &mut Ui, editor: &mut TeamEditor, seat: SeatRef) {
    let prefix = seat.path();
    let issues = editor.issues_under(&prefix);
    let pod = &editor.spec.pods[seat.pod];
    let title = format!("{}.{}", pod.id, pod.members[seat.member].id);
    let address = spec::seat_address(&pod.id, &pod.members[seat.member].id, &editor.spec.name);
    let mut requests = Requests::default();
    title_row(ui, &title, &address, &mut requests);
    ui.separator();
    let form = SeatForm {
        prefix: &prefix,
        issues: &issues,
        spec_folder: &editor.spec_folder,
        team_policy: &editor.spec.permission_policy,
        address: &address,
        editable: !editor.read_only,
        can_pick_folders: !editor.remote,
    };
    let member = &mut editor.spec.pods[seat.pod].members[seat.member];
    if member.runtime == TERMINAL_RUNTIME {
        terminal_note(ui);
    }
    basic(ui, &form, member, &mut requests);
    if advanced(ui, &form, member) {
        requests.event = Some(TeamEvent::ShowYaml);
    }
    ui.add_space(SECTION_GAP);
    if kept_as_written(ui, &member.yaml_only) {
        requests.event = Some(TeamEvent::ShowYaml);
    }
    ui.separator();
    ui.add_enabled_ui(form.editable, |ui| {
        let remove = RichText::new("Remove seat").color(ui.visuals().error_fg_color);
        requests.remove = ui.link(remove).clicked();
    });
    apply_requests(editor, seat, requests);
}

/// Do what the inspector's clicks asked for.
fn apply_requests(editor: &mut TeamEditor, seat: SeatRef, requests: Requests) {
    if let Some(event) = requests.event {
        editor.request(event);
    }
    if editor.read_only {
        if requests.close {
            editor.selected = None;
        }
        return;
    }
    if let Some(new_id) = requests.rename_to {
        editor.spec.rename_seat(seat.pod, seat.member, &new_id);
    }
    if requests.remove {
        editor.remove_seat(seat);
    } else if requests.close {
        editor.selected = None;
    }
}

/// "Seat dev.build", the live address and the close button.
fn title_row(ui: &mut Ui, title: &str, address: &str, requests: &mut Requests) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Seat").strong());
        ui.label(RichText::new(title).monospace().weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            requests.close = small_icon_button(ui, Icon::Close, REMOVE_SIZE, "Close", "Close the inspector").clicked();
            ui.label(RichText::new(address).monospace().size(KEY_SIZE).color(ui.visuals().hyperlink_color));
        });
    });
}

/// The note on top of a terminal seat.
fn terminal_note(ui: &mut Ui) {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(NOTE_RADIUS)
        .inner_margin(BOX_MARGIN)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                "A terminal seat runs a shell, not an agent: a dev server, a log tail, a watcher. \
                 Messages to it are typed into the shell as they are.",
            );
        });
    ui.add_space(SECTION_GAP);
}

/// The Basic fields: id, name, agent, profile, runtime, model and effort,
/// folder, role and permissions.
fn basic(ui: &mut Ui, form: &SeatForm, member: &mut Member, requests: &mut Requests) {
    let terminal = member.runtime == TERMINAL_RUNTIME;
    ui.add_enabled_ui(form.editable, |ui| {
        let mut id = member.id.clone();
        let id_issue = form.issue("id");
        let id_changed = labelled_field(ui, "Seat id", "id", true, id_issue, |ui| {
            let response = text_field(ui, &mut id, "build", id_issue.is_some());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                hint(ui, "Address ");
                ui.label(RichText::new(form.address).monospace().small().color(ui.visuals().hyperlink_color));
                hint(ui, ". Letters, digits, - and _; no dots or @.");
            });
            response.changed()
        });
        if id_changed {
            requests.rename_to = Some(id);
        }
        seat_text_field(ui, form, "Name", "label", &mut member.label, "Shown in lists instead of the id", "");
        runtime_field(ui, form, member);
    });
    agent_field(ui, form, member, requests);
    ui.add_enabled_ui(form.editable, |ui| {
        ui.add_enabled_ui(!terminal, |ui| {
            let note = if terminal { "Set for you with terminal. OpenRig needs all three together." } else { "" };
            seat_text_field(ui, form, "Profile", "profile", &mut member.profile, "default", note);
        });
        model_and_effort(ui, form, member);
        folder_field(ui, form, &mut member.cwd);
        ui.add_enabled_ui(!terminal, |ui| {
            let role_hint = if terminal { "Not allowed on terminal seats" } else { "For example builder or reviewer" };
            seat_text_field(ui, form, "Role", "role", &mut member.role, role_hint, "");
            seat_permissions(ui, form, &mut member.permission_policy);
        });
    });
}

/// "Runs on": one pill per runtime. A change clears what the new runtime
/// may not have.
fn runtime_field(ui: &mut Ui, form: &SeatForm, member: &mut Member) {
    let choices = RUNTIMES.map(|runtime| (runtime, runtime));
    let changed = labelled_field(ui, "Runs on", "runtime", true, form.issue("runtime"), |ui| {
        pill_row(ui, &mut member.runtime, &choices)
    });
    if changed {
        apply_runtime_change(member);
    }
}

/// Keep a seat valid after its runtime changed: the Claude Code and Codex
/// fields go with their runtime, a terminal seat gets its fixed agent and
/// profile and loses role and permissions, and a seat leaving terminal
/// loses the fixed agent and profile.
fn apply_runtime_change(member: &mut Member) {
    if member.runtime != CLAUDE_RUNTIME {
        member.claude = ClaudeFields::default();
    }
    if member.runtime != CODEX_RUNTIME {
        member.codex = CodexFields::default();
    }
    if member.runtime == TERMINAL_RUNTIME {
        TERMINAL_AGENT.clone_into(&mut member.agent_ref);
        TERMINAL_PROFILE.clone_into(&mut member.profile);
        member.role.clear();
        member.permission_policy.clear();
        return;
    }
    if member.agent_ref == TERMINAL_AGENT {
        member.agent_ref.clear();
    }
    if member.profile == TERMINAL_PROFILE {
        member.profile.clear();
    }
}

/// The agent link with Choose, and the "Open agent.yaml" link (which works
/// on a read-only team too).
fn agent_field(ui: &mut Ui, form: &SeatForm, member: &mut Member, requests: &mut Requests) {
    let terminal = member.runtime == TERMINAL_RUNTIME;
    let issue = form.issue("agent_ref");
    labelled_field(ui, "Agent", "agent_ref", true, issue, |ui| {
        ui.add_enabled_ui(form.editable && !terminal, |ui| {
            let add = |ui: &mut Ui| {
                text_field(ui, &mut member.agent_ref, "local:agents/implementer", issue.is_some());
            };
            let chosen = if form.can_pick_folders { field_with_button(ui, "Choose", add) } else { add_only(ui, add) };
            if chosen && let Some(folder) = pick_folder("Choose the agent folder", form.spec_folder) {
                member.agent_ref = format!("local:{}", relative_path(form.spec_folder, &folder));
            }
        });
        if let Some(path) = agent_yaml_path(form.spec_folder, &member.agent_ref) {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let link = ui.link(RichText::new("Open agent.yaml").small());
                if link.on_hover_text(path.display().to_string()).clicked() {
                    requests.event = Some(TeamEvent::OpenPath(path));
                }
                hint(ui, ". The agent holds the skills and instructions; this team file only points at it.");
            });
        }
    });
}

/// Model and effort side by side; empty uses the runtime's default.
fn model_and_effort(ui: &mut Ui, form: &SeatForm, member: &mut Member) {
    ui.columns(2, |columns| {
        let hint_text = "runtime default";
        seat_text_field(&mut columns[0], form, "Model", "model", &mut member.model, hint_text, "");
        seat_text_field(&mut columns[1], form, "Effort", "effort", &mut member.effort, hint_text, "");
    });
}

/// The working folder with Browse, relative to the spec's folder.
fn folder_field(ui: &mut Ui, form: &SeatForm, cwd: &mut String) {
    let issue = form.issue("cwd");
    labelled_field(ui, "Folder", "cwd", true, issue, |ui| {
        let add = |ui: &mut Ui| {
            text_field(ui, cwd, ".", issue.is_some());
        };
        let browse = if form.can_pick_folders { field_with_button(ui, "Browse", add) } else { add_only(ui, add) };
        if browse && let Some(folder) = pick_folder("Folder the agent works in", form.spec_folder) {
            *cwd = relative_path(form.spec_folder, &folder);
        }
        hint(ui, "Where the agent works. A dot is the team folder; ~ and $VARS work.");
    });
}

/// Permissions: "Same as team", the built-in policies, or a policy file,
/// with the value as written underneath.
fn seat_permissions(ui: &mut Ui, form: &SeatForm, policy: &mut String) {
    let unset_hint = if form.team_policy.is_empty() {
        "Team sets none, so OpenRig's default floor applies.".to_owned()
    } else {
        format!("Team sets {}.", form.team_policy)
    };
    field_label(ui, "Permissions", "permission_policy", false);
    ui.add_space(LABEL_GAP);
    permissions_field(ui, policy, "Same as team", &unset_hint, form.issue("permission_policy"));
    ui.add_space(FIELD_GAP);
}

/// The closed "Advanced" section. Returns true when "Edit in YAML" was
/// clicked (for the startup actions).
fn advanced(ui: &mut Ui, form: &SeatForm, member: &mut Member) -> bool {
    let mut show_yaml = false;
    egui::CollapsingHeader::new(RichText::new("Advanced").strong())
        .id_salt("team-seat-advanced")
        .default_open(false)
        .show(ui, |ui| {
            ui.add_enabled_ui(form.editable, |ui| {
                labelled_field(ui, "Startup files", "startup.files", false, None, |ui| {
                    let prefix = format!("{}.", form.prefix);
                    show_yaml = startup_file_list(ui, &mut member.startup, form.issues, &prefix, OWNER);
                });
                labelled_field(ui, "Environment", "env", false, None, |ui| {
                    hint(
                        ui,
                        "Laid over the team's. Names in capitals, OPENRIG_ names are reserved. Values are not secret.",
                    );
                    env_list(ui, &mut member.env, form.issues, &format!("{}.env", form.prefix), OWNER);
                });
                restore_and_compaction(ui, form, member);
                if member.runtime == CLAUDE_RUNTIME {
                    claude_block(ui, form, &mut member.claude);
                }
                if member.runtime == CODEX_RUNTIME {
                    codex_block(ui, form, &mut member.codex);
                }
            });
        });
    show_yaml
}

/// After a restart (`restore_policy`) and compaction, with the mechanic for
/// an apprentice handover.
fn restore_and_compaction(ui: &mut Ui, form: &SeatForm, member: &mut Member) {
    let mut restore_choices = vec![("From agent", "")];
    restore_choices.extend(RESTORE_POLICIES.map(|policy| (policy, policy)));
    labelled_field(ui, "After a restart", "restore_policy", false, form.issue("restore_policy"), |ui| {
        pill_row(ui, &mut member.restore_policy, &restore_choices);
        hint(ui, "Can only narrow what the agent allows.");
    });
    let strategy = &mut member.compaction_strategy;
    labelled_field(ui, "Compaction", "compaction_strategy", false, form.issue("compaction_strategy"), |ui| {
        let shown = if strategy.is_empty() { "default" } else { strategy.as_str() }.to_owned();
        egui::ComboBox::from_id_salt("team-seat-compaction")
            .selected_text(RichText::new(&shown).monospace())
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                if ui.selectable_label(strategy.is_empty(), "default").clicked() {
                    strategy.clear();
                }
                for choice in COMPACTION_STRATEGIES {
                    if ui.selectable_label(strategy == choice, choice).clicked() {
                        choice.clone_into(strategy);
                    }
                }
            });
        hint(ui, "managed-compaction and apprentice-handover only act on claude-code seats.");
    });
    if member.compaction_strategy == APPRENTICE_HANDOVER {
        let note = "The seat address an apprentice handover asks.";
        seat_text_field(ui, form, "Mechanic", "mechanic", &mut member.mechanic, "seat@rig", note);
    }
}

/// The "Claude Code only" fields.
fn claude_block(ui: &mut Ui, form: &SeatForm, claude: &mut ClaudeFields) {
    section_heading(ui, "CLAUDE CODE ONLY");
    ui.columns(2, |columns| {
        seat_text_field(&mut columns[0], form, "Advisor model", "advisor", &mut claude.advisor, "from profile", "");
        labelled_field(&mut columns[1], "MCP servers", "claude_mcp", false, form.issue("claude_mcp"), |ui| {
            let mut load = claude.mcp != MCP_OFF;
            if ui.checkbox(&mut load, "Load as usual").changed() {
                claude.mcp = if load { String::new() } else { MCP_OFF.to_owned() };
            }
        });
    });
    labelled_field(ui, "Tool rules", "claude_permissions", false, form.issue("claude_permissions"), |ui| {
        tool_rules(ui, claude);
    });
    let agent_file_hint = "none, a .md file next to rig.yaml";
    seat_text_field(ui, form, "Claude agent file", "agent_file", &mut claude.agent_file, agent_file_hint, "");
    let prompt_file = &mut claude.append_system_prompt_file;
    seat_text_field(ui, form, "Extra system prompt file", "append_system_prompt_file", prompt_file, "none", "");
    labelled_field(ui, "Plugin folders", "plugin_dirs", false, form.issue("plugin_dirs"), |ui| {
        text_list(ui, &mut claude.plugin_dirs, "plugins/review", "+ Add folder");
    });
    labelled_field(ui, "Plugin channels", "plugin_channels", false, form.issue("plugin_channels"), |ui| {
        text_list(ui, &mut claude.plugin_channels, "plugin:name@marketplace", "+ Add channel");
    });
    labelled_field(ui, "Own subscription token", "claude_token", false, form.issue("claude_token"), |ui| {
        ui.columns(2, |columns| {
            text_field(&mut columns[0], &mut claude.token_label, "Label", false);
            text_field(&mut columns[1], &mut claude.token_file, "Token file, full path", false);
        });
    });
}

/// Claude's allow and deny rules, each with remove, and add links.
fn tool_rules(ui: &mut Ui, claude: &mut ClaudeFields) {
    if !claude.allow.is_empty() || !claude.deny.is_empty() {
        list_frame(ui, |ui| {
            for (label, rules) in [("Allow", &mut claude.allow), ("Deny", &mut claude.deny)] {
                let mut removed = None;
                for (index, rule) in rules.iter_mut().enumerate() {
                    egui::Frame::new().inner_margin(BOX_MARGIN).show(ui, |ui| {
                        let remove = row_with_remove(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(label).small());
                                text_field(ui, rule, "Bash(npm test *)", false);
                            });
                        });
                        if remove {
                            removed = Some(index);
                        }
                    });
                }
                if let Some(index) = removed {
                    rules.remove(index);
                }
            }
        });
    }
    ui.horizontal(|ui| {
        if ui.link("+ Allow rule").clicked() {
            claude.allow.push(String::new());
        }
        if ui.link("+ Deny rule").clicked() {
            claude.deny.push(String::new());
        }
    });
}

/// The "Codex only" fields. Network access and writable folders apply only
/// to the `workspace-write` sandbox, and are cleared when it is left.
fn codex_block(ui: &mut Ui, form: &SeatForm, codex: &mut CodexFields) {
    section_heading(ui, "CODEX ONLY");
    let profile = &mut codex.config_profile;
    seat_text_field(ui, form, "Codex profile", "codex_config_profile", profile, "none, passed as -p", "");
    let mut sandboxes = vec![("From Permissions", "")];
    sandboxes.extend(CODEX_SANDBOXES.map(|sandbox| (sandbox, sandbox)));
    let sandbox_issue = form.issue("codex_permissions.sandbox");
    let sandbox_changed = labelled_field(ui, "Sandbox", "codex_permissions.sandbox", false, sandbox_issue, |ui| {
        pill_row(ui, &mut codex.sandbox, &sandboxes)
    });
    if sandbox_changed && codex.sandbox != WORKSPACE_WRITE {
        codex.network_access = None;
        codex.writable_roots.clear();
    }
    let mut approvals = vec![("From Permissions", "")];
    approvals.extend(CODEX_APPROVALS.map(|approval| (approval, approval)));
    let approval_issue = form.issue("codex_permissions.approval");
    labelled_field(ui, "Approval", "codex_permissions.approval", false, approval_issue, |ui| {
        pill_row(ui, &mut codex.approval, &approvals);
    });
    ui.add_enabled_ui(codex.sandbox == WORKSPACE_WRITE, |ui| {
        let network_issue = form.issue("codex_permissions.network_access");
        labelled_field(ui, "Network access", "network_access", false, network_issue, |ui| {
            let mut network = codex.network_access.unwrap_or(false);
            if ui.checkbox(&mut network, "Allow network access").changed() {
                codex.network_access = Some(network);
            }
        });
        let roots_issue = form.issue("codex_permissions.writable_roots");
        labelled_field(ui, "Extra writable folders", "writable_roots", false, roots_issue, |ui| {
            text_list(ui, &mut codex.writable_roots, "Full path", "+ Add folder");
            hint(ui, "Only with workspace-write.");
        });
    });
}

/// A labelled one-line text field for the seat's `key`, warned when it has
/// a problem, with `note` (when not empty) under it. Returns the field's
/// response.
fn seat_text_field(
    ui: &mut Ui,
    form: &SeatForm,
    label: &str,
    key: &str,
    value: &mut String,
    hint_text: &str,
    note: &str,
) -> egui::Response {
    let issue = form.issue(key);
    labelled_field(ui, label, key, is_required(key), issue, |ui| {
        let response = text_field(ui, value, hint_text, issue.is_some());
        if !note.is_empty() {
            hint(ui, note);
        }
        response
    })
}

/// The seat keys every seat must have.
fn is_required(key: &str) -> bool {
    matches!(key, "id" | "agent_ref" | "profile" | "runtime" | "cwd")
}

/// Edit a list of text values: one field per item with a remove button,
/// then an add link that adds an empty item.
fn text_list(ui: &mut Ui, items: &mut Vec<String>, hint_text: &str, add_label: &str) {
    let mut removed = None;
    for (index, item) in items.iter_mut().enumerate() {
        if row_with_remove(ui, |ui| {
            text_field(ui, item, hint_text, false);
        }) {
            removed = Some(index);
        }
    }
    if let Some(index) = removed {
        items.remove(index);
    }
    if ui.link(add_label).clicked() {
        items.push(String::new());
    }
}

/// A row with `add` filling the width and a remove button on the right.
/// Returns true when the remove button was clicked.
fn row_with_remove(ui: &mut Ui, add: impl FnOnce(&mut Ui)) -> bool {
    let mut removed = false;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - REMOVE_SIZE - ui.spacing().item_spacing.x).max(0.0);
        ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), add);
        removed = small_icon_button(ui, Icon::Close, REMOVE_SIZE, "Remove", "Remove").clicked();
    });
    removed
}

/// A row with `add` filling the width and a button labelled `label` on the
/// right. Returns true when the button was clicked.
fn field_with_button(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - SIDE_BUTTON_WIDTH).max(0.0);
        ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), add);
        clicked = ui.button(label).clicked();
    });
    clicked
}

/// The field `add` draws alone, where a picker button would be. Returns
/// false: there is no button to click.
fn add_only(ui: &mut Ui, add: impl FnOnce(&mut Ui)) -> bool {
    add(ui);
    false
}

/// A small heading inside Advanced ("CODEX ONLY"), after a separator.
fn section_heading(ui: &mut Ui, text: &str) {
    ui.add_space(SECTION_GAP);
    ui.separator();
    ui.label(RichText::new(text).small().weak());
    ui.add_space(SECTION_GAP);
}

/// Ask for a folder in the system picker, starting at `start`.
fn pick_folder(title: &str, start: &Path) -> Option<PathBuf> {
    rfd::FileDialog::new().set_title(title).set_directory(start).pick_folder()
}

/// The agent.yaml an agent link points at: `local:` is relative to the
/// spec's folder, `path:` is absolute. `None` for any other link.
fn agent_yaml_path(spec_folder: &Path, agent_ref: &str) -> Option<PathBuf> {
    let folder = if let Some(relative) = agent_ref.strip_prefix("local:") {
        spec_folder.join(relative)
    } else {
        let absolute = Path::new(agent_ref.strip_prefix("path:")?);
        if !absolute.is_absolute() {
            return None;
        }
        absolute.to_path_buf()
    };
    Some(folder.join("agent.yaml"))
}

/// `target` written relative to the folder `base`, with `..` where needed;
/// `.` for the folder itself. Both are resolved first, so a symlinked
/// folder (macOS `/tmp`) still gives a short path.
fn relative_path(base: &Path, target: &Path) -> String {
    let base = std::fs::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
    let target = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let base_parts: Vec<Component> = base.components().collect();
    let target_parts: Vec<Component> = target.components().collect();
    let shared = base_parts.iter().zip(&target_parts).take_while(|(left, right)| left == right).count();
    let mut parts = vec!["..".to_owned(); base_parts.len() - shared];
    parts.extend(target_parts[shared..].iter().map(|part| part.as_os_str().to_string_lossy().into_owned()));
    if parts.is_empty() { ".".to_owned() } else { parts.join("/") }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Context, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, vec2};

    use super::*;
    use crate::document::text_file::{Document, Opened};
    use crate::test_support::TempFolder;

    const STARTER: &str = include_str!("../../openrig/spec/fixtures/starter.yaml");
    /// The starter's Claude Code seat.
    const BUILD: SeatRef = SeatRef { pod: 0, member: 0 };
    /// The starter's Codex seat.
    const REVIEW: SeatRef = SeatRef { pod: 0, member: 1 };
    /// Frames drawn before a click: enough for a section's open animation.
    const SETTLE_FRAMES: usize = 20;

    /// The inspector for one seat of the starter team, drawn headless.
    struct Harness {
        ctx: Context,
        editor: TeamEditor,
        seat: SeatRef,
        _temp: TempFolder,
        /// Each piece of text the last frame drew, with where it was drawn.
        texts: Vec<(String, Rect)>,
        frame: u16,
    }

    impl Harness {
        fn new(seat: SeatRef) -> Self {
            let temp = TempFolder::new();
            let path = temp.write("my-team/rig.yaml", STARTER);
            let Opened::Text(document) = Document::open(&path, None).unwrap() else { panic!("a text file") };
            let mut editor = TeamEditor::new(&document);
            editor.selected = Some(seat);
            let mut harness = Self { ctx: Context::default(), editor, seat, _temp: temp, texts: Vec::new(), frame: 0 };
            harness.run(Vec::new());
            harness
        }

        fn member(&self) -> &Member {
            &self.editor.spec.pods[self.seat.pod].members[self.seat.member]
        }

        fn member_mut(&mut self) -> &mut Member {
            &mut self.editor.spec.pods[self.seat.pod].members[self.seat.member]
        }

        /// Draw one frame with `events`, remembering the text it drew.
        fn run(&mut self, events: Vec<Event>) {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(420.0, 4000.0))),
                time: Some(f64::from(self.frame) * 0.016),
                events,
                ..Default::default()
            };
            self.frame += 1;
            let (editor, seat) = (&mut self.editor, self.seat);
            let mut output = self.ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    if editor.selected.is_some() {
                        show_inspector(ui, editor, seat);
                    }
                });
            });
            output.textures_delta.clear();
            self.texts.clear();
            for clipped in &output.shapes {
                collect_texts(&clipped.shape, &mut self.texts);
            }
        }

        /// Let the layout and the open animations settle, then click the
        /// middle of the text `label`.
        fn click(&mut self, label: &str) {
            for _ in 0..SETTLE_FRAMES {
                self.run(Vec::new());
            }
            let Some((_, rect)) = self.texts.iter().find(|(text, _)| text == label) else {
                panic!("no {label:?} on screen")
            };
            let at = rect.center();
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::default(),
            };
            self.run(vec![Event::PointerMoved(at)]);
            self.run(vec![button(true)]);
            self.run(vec![button(false)]);
        }

        /// Click the field showing `shown`, go to its end and type `typed`.
        fn type_after(&mut self, shown: &str, typed: &str) {
            self.click(shown);
            let end = Event::Key {
                key: Key::End,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::default(),
            };
            self.run(vec![end, Event::Text(typed.to_owned())]);
        }
    }

    /// Every text in `shape`, with its screen rectangle.
    fn collect_texts(shape: &Shape, texts: &mut Vec<(String, Rect)>) {
        match shape {
            Shape::Text(text) => texts.push((text.galley.text().to_owned(), text.visual_bounding_rect())),
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect_texts(shape, texts)),
            _ => {}
        }
    }

    /// Typing in a field changes the seat.
    #[test]
    fn typing_in_a_field_changes_the_seat() {
        let mut harness = Harness::new(BUILD);
        harness.type_after("For example builder or reviewer", "builder");
        assert_eq!(harness.member().role, "builder");
    }

    /// A new seat id carries the links that named the old one.
    #[test]
    fn renaming_a_seat_renames_its_links() {
        let mut harness = Harness::new(BUILD);
        harness.type_after("build", "er");
        assert_eq!(harness.member().id, "builder");
        assert_eq!(harness.editor.spec.pods[0].edges[0].from, "builder");
    }

    /// Choosing terminal fixes agent and profile and drops role and
    /// permissions; leaving terminal clears the fixed values again.
    #[test]
    fn terminal_seats_get_the_fixed_agent_and_profile() {
        let mut harness = Harness::new(BUILD);
        harness.member_mut().role = "builder".into();
        harness.member_mut().permission_policy = "builtin:open".into();
        harness.click("terminal");
        let member = harness.member();
        assert_eq!(member.runtime, TERMINAL_RUNTIME);
        assert_eq!((member.agent_ref.as_str(), member.profile.as_str()), (TERMINAL_AGENT, TERMINAL_PROFILE));
        assert_eq!((member.role.as_str(), member.permission_policy.as_str()), ("", ""));
        assert!(harness.editor.spec.check().iter().all(|issue| !issue.path.starts_with("pods[0].members[0]")));

        harness.click("pi");
        let member = harness.member();
        assert_eq!(member.runtime, "pi");
        assert_eq!((member.agent_ref.as_str(), member.profile.as_str()), ("", ""));
    }

    /// A runtime change drops the other runtime's fields, which the
    /// validator refuses on it.
    #[test]
    fn runtime_change_clears_the_other_runtimes_fields() {
        let mut harness = Harness::new(BUILD);
        harness.member_mut().claude.mcp = MCP_OFF.into();
        harness.member_mut().claude.allow = vec!["Bash(npm test *)".into()];
        harness.click("codex");
        assert_eq!(harness.member().runtime, CODEX_RUNTIME);
        assert_eq!(harness.member().claude, ClaudeFields::default());

        harness.member_mut().codex.sandbox = WORKSPACE_WRITE.into();
        harness.member_mut().codex.writable_roots = vec!["/srv/lease".into()];
        harness.click("claude-code");
        assert_eq!(harness.member().codex, CodexFields::default());
    }

    /// Leaving the workspace-write sandbox clears network access and the
    /// writable folders, which only it allows.
    #[test]
    fn leaving_workspace_write_clears_its_fields() {
        let mut harness = Harness::new(REVIEW);
        harness.click("Advanced");
        harness.click(WORKSPACE_WRITE);
        harness.member_mut().codex.network_access = Some(true);
        harness.member_mut().codex.writable_roots = vec!["/srv/lease".into()];
        harness.click("read-only");
        let codex = &harness.member().codex;
        assert_eq!(codex.sandbox, "read-only");
        assert_eq!((codex.network_access, codex.writable_roots.len()), (None, 0));
    }

    /// A read-only team shows the seat but takes no edits.
    #[test]
    fn read_only_teams_take_no_edits() {
        let mut harness = Harness::new(BUILD);
        harness.editor.read_only = true;
        let before = harness.member().clone();
        harness.click("codex");
        harness.type_after("For example builder or reviewer", "builder");
        harness.click("Remove seat");
        assert_eq!(harness.member(), &before);
    }

    /// The link opens the agent.yaml next to the spec's folder.
    #[test]
    fn opens_the_agents_yaml() {
        let mut harness = Harness::new(BUILD);
        harness.click("Open agent.yaml");
        let expected = harness.editor.spec_folder.join("../../../agents/development/implementer/agent.yaml");
        assert_eq!(harness.editor.events, [TeamEvent::OpenPath(expected)]);
    }

    /// Removing a seat removes its links and closes the inspector.
    #[test]
    fn removing_a_seat_removes_its_links() {
        let mut harness = Harness::new(REVIEW);
        harness.click("Remove seat");
        let pod = &harness.editor.spec.pods[0];
        assert_eq!(pod.members.len(), 1);
        assert_eq!(pod.edges.len(), 0);
        assert_eq!(harness.editor.selected, None);
    }

    /// Agent links resolve against the spec's folder or as absolute paths.
    #[test]
    fn resolves_agent_links() {
        let folder = Path::new("/specs/team");
        assert_eq!(agent_yaml_path(folder, "local:agents/qa"), Some(PathBuf::from("/specs/team/agents/qa/agent.yaml")));
        assert_eq!(agent_yaml_path(folder, "path:/agents/qa"), Some(PathBuf::from("/agents/qa/agent.yaml")));
        assert_eq!(agent_yaml_path(folder, "path:agents/qa"), None);
        assert_eq!(agent_yaml_path(folder, TERMINAL_AGENT), None);
    }

    /// Chosen folders are written relative to the spec's folder.
    #[test]
    fn writes_chosen_folders_relative_to_the_spec() {
        let base = Path::new("/no-such-root/specs/team");
        assert_eq!(relative_path(base, base), ".");
        assert_eq!(relative_path(base, Path::new("/no-such-root/specs/team/work")), "work");
        assert_eq!(relative_path(base, Path::new("/no-such-root/agents/qa")), "../../agents/qa");
    }
}
