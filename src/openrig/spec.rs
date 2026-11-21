//! The `OpenRig` rig spec (`rig.yaml`) as the team editor sees it: a typed
//! model of the fields the form edits, read from the YAML text and written
//! back into that same text.
//!
//! Reading lives in `read`, writing in `write` and the cheap local checks in
//! `checks`. The model holds only what the form shows. Everything else in
//! the file (comments, key order, unknown keys, the fields edited in YAML
//! only) stays in the text: a save edits the text in place and touches only
//! the keys whose values changed.
//!
//! The source of truth for the format is `docs/reference/rig-spec.md` and
//! `packages/daemon/src/domain/rigspec-schema.ts` in the `OpenRig` repository.

mod checks;
mod read;
mod write;

/// The runtimes the form offers in "Runs on". `stub` is a test harness and
/// is not offered; a spec that uses it keeps it.
pub const RUNTIMES: [&str; 5] = ["claude-code", "codex", "pi", "omp", "terminal"];
/// The runtime of a terminal seat (with [`TERMINAL_AGENT`] and [`TERMINAL_PROFILE`]).
pub const TERMINAL_RUNTIME: &str = "terminal";
/// The only agent a terminal seat may have.
pub const TERMINAL_AGENT: &str = "builtin:terminal";
/// The only profile a terminal seat may have.
pub const TERMINAL_PROFILE: &str = "none";
/// The Claude Code runtime, which has its own block of fields.
pub const CLAUDE_RUNTIME: &str = "claude-code";
/// The Codex runtime, which has its own block of fields.
pub const CODEX_RUNTIME: &str = "codex";

/// Link kinds (`edges[].kind`), in the order the drop-down lists them.
pub const EDGE_KINDS: [&str; 5] = ["delegates_to", "spawned_by", "can_observe", "collaborates_with", ESCALATES_TO];
/// The link kind whose target may be a seat address in another rig.
pub const ESCALATES_TO: &str = "escalates_to";
/// "After a restart" choices (`restore_policy`); empty means "from agent".
pub const RESTORE_POLICIES: [&str; 3] = ["resume_if_possible", "relaunch_fresh", "checkpoint_only"];
/// Compaction choices (`compaction_strategy`); empty means the default.
pub const COMPACTION_STRATEGIES: [&str; 4] =
    ["default-compaction", "managed-compaction", "handover", "apprentice-handover"];
/// Built-in permission policies. The `none` value is not offered.
pub const PERMISSION_POLICIES: [&str; 5] =
    ["builtin:locked", "builtin:standard", "builtin:open", "builtin:yolo", "builtin:auto"];
/// How a startup file is delivered (`delivery_hint`).
pub const DELIVERY_HINTS: [&str; 4] = ["auto", "guidance_merge", "skill_install", "send_text"];
/// Workspace repository kinds (`workspace.repos[].kind`).
pub const REPO_KINDS: [&str; 5] = ["user", "project", "knowledge", "lab", "delivery"];
/// Codex sandbox choices (`codex_permissions.sandbox`).
pub const CODEX_SANDBOXES: [&str; 2] = ["workspace-write", "read-only"];
/// Codex approval choices (`codex_permissions.approval`).
pub const CODEX_APPROVALS: [&str; 3] = ["never", "on-request", "untrusted"];
/// Files that may receive Claude's managed blocks (`managed_blocks.claude-code`).
pub const CLAUDE_INSTRUCTION_FILES: [&str; 2] = ["CLAUDE.md", "CLAUDE.local.md"];

/// A whole rig spec: the team settings, its pods and the links between pods.
///
/// Text fields hold `""` when the key is absent. Lists keep the file's order.
/// Items read from the file remember where they came from, so a save can
/// edit them in place; items the form adds start without an origin.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RigSpec {
    /// `name`: part of every seat address.
    pub name: String,
    /// `version`: `"0.2"` for pod-aware specs.
    pub version: String,
    pub summary: String,
    pub culture_file: String,
    /// `permission_policy` for every seat; empty uses `OpenRig`'s default floor.
    pub permission_policy: String,
    /// `non_interruptive`; `None` when the key is absent.
    pub non_interruptive: Option<bool>,
    /// `managed_blocks.claude-code`: `CLAUDE.md` or `CLAUDE.local.md`.
    pub claude_instruction_file: String,
    pub env: Vec<EnvVar>,
    /// `workspace`; `None` when the key is absent.
    pub workspace: Option<Workspace>,
    /// `docs[].path`.
    pub docs: Vec<String>,
    /// Team-wide `startup`.
    pub startup: Startup,
    pub pods: Vec<Pod>,
    /// Links between pods (`edges`), with `pod.member` ends.
    pub edges: Vec<Edge>,
    /// Keys present in the file that the form shows but does not edit
    /// (`services`). Read-only: changing this list writes nothing.
    pub yaml_only: Vec<String>,
}

/// One pod: a group of seats and the links inside it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pod {
    origin: Option<usize>,
    pub id: String,
    pub label: String,
    pub summary: String,
    pub members: Vec<Member>,
    /// Links inside the pod, with plain member ids as ends.
    pub edges: Vec<Edge>,
    /// Keys kept as written and edited in YAML only (`continuity_policy`,
    /// `startup`). Read-only.
    pub yaml_only: Vec<String>,
}

/// One seat (a pod member).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Member {
    origin: Option<usize>,
    pub id: String,
    pub label: String,
    /// `local:<relative path>`, `path:<absolute path>` or `builtin:terminal`.
    pub agent_ref: String,
    pub profile: String,
    pub runtime: String,
    pub model: String,
    pub effort: String,
    /// `cwd`, relative to the spec's folder.
    pub cwd: String,
    pub role: String,
    pub permission_policy: String,
    pub restore_policy: String,
    pub compaction_strategy: String,
    /// `mechanic`: the seat address an apprentice handover asks.
    pub mechanic: String,
    pub env: Vec<EnvVar>,
    pub startup: Startup,
    /// Fields only Claude Code seats use.
    pub claude: ClaudeFields,
    /// Fields only Codex seats use.
    pub codex: CodexFields,
    /// Keys kept as written and edited in YAML only (`claude_auto_mode`,
    /// `launch_wrapper`, `process_limits`, `session_source`, `starter_ref`).
    /// Read-only.
    pub yaml_only: Vec<String>,
}

/// The Claude Code only fields of a seat.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClaudeFields {
    /// `advisor` model; empty uses the profile's.
    pub advisor: String,
    /// `claude_mcp`: `"none"` turns MCP servers off; empty loads them as usual.
    pub mcp: String,
    /// `claude_permissions.allow`.
    pub allow: Vec<String>,
    /// `claude_permissions.deny`.
    pub deny: Vec<String>,
    pub agent_file: String,
    pub append_system_prompt_file: String,
    pub plugin_dirs: Vec<String>,
    pub plugin_channels: Vec<String>,
    /// `claude_token.label`.
    pub token_label: String,
    /// `claude_token.file`, an absolute path.
    pub token_file: String,
}

/// The Codex only fields of a seat.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodexFields {
    /// `codex_config_profile`, passed as `-p`.
    pub config_profile: String,
    /// `codex_permissions.sandbox`; empty follows the permission policy.
    pub sandbox: String,
    /// `codex_permissions.approval`; empty follows the permission policy.
    pub approval: String,
    /// `codex_permissions.network_access`.
    pub network_access: Option<bool>,
    /// `codex_permissions.writable_roots`, absolute paths.
    pub writable_roots: Vec<String>,
}

/// A link between two seats: `from` does `kind` to `to`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edge {
    origin: Option<usize>,
    pub kind: String,
    pub from: String,
    pub to: String,
}

/// One launch environment variable (`env` entry).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// A `startup` block: the files the form edits; actions stay as written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Startup {
    pub files: Vec<StartupFile>,
    /// How many startup actions the file has. Read-only: actions are edited in YAML.
    pub action_count: usize,
}

/// One startup file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StartupFile {
    origin: Option<usize>,
    pub path: String,
    /// One of [`DELIVERY_HINTS`]; empty means `auto`.
    pub delivery_hint: String,
    /// `required`; `None` (absent) means true.
    pub required: Option<bool>,
    /// `applies_on`; empty means both `fresh_start` and `restore`.
    pub applies_on: Vec<String>,
}

/// The team's `workspace` block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[expect(clippy::struct_field_names, reason = "named after the spec key workspace_root")]
pub struct Workspace {
    pub workspace_root: String,
    pub repos: Vec<Repo>,
    pub default_repo: String,
    pub knowledge_root: String,
}

/// One workspace repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Repo {
    origin: Option<usize>,
    pub name: String,
    pub path: String,
    /// One of [`REPO_KINDS`].
    pub kind: String,
}

/// A problem found in a spec, by the local checks or by `rig spec validate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// Where it is, in the validator's form: `pods[0].members[1].id`. Empty
    /// when the message names no place.
    pub path: String,
    pub message: String,
}

impl Issue {
    /// An issue at `path`.
    pub fn at(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self { path: path.into(), message: message.into() }
    }

    /// Read one line of `rig spec validate` output, `<path>: <message>`.
    /// A line whose first part is not a path keeps it all as the message.
    pub fn parse(line: &str) -> Self {
        match line.split_once(": ") {
            Some((path, message)) if is_issue_path(path) => Self::at(path, message),
            _ => Self::at("", line),
        }
    }
}

/// True for a validator path such as `pods[0].members[1].id` or `name`.
fn is_issue_path(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'[' | b']'))
}

/// True when `text` is a rig spec the team editor can open: YAML with a
/// top-level `pods` list and `name` and `version` keys.
pub fn is_rig_spec(text: &str) -> bool {
    read::parse_document(text).is_ok_and(|document| read::rig_root(&document).is_some())
}

/// The seat address `OpenRig` gives a member: `{pod}-{member}@{rig}`.
pub fn seat_address(pod_id: &str, member_id: &str, rig_name: &str) -> String {
    format!("{pod_id}-{member_id}@{rig_name}")
}

/// True for a seat address an `escalates_to` link may name instead of a seat
/// of this team (`pod-member@rig`, `OpenRig`'s `isEscalationTargetRef`):
/// letters, digits, `_` and `-` on both sides of one `@`, but not `human@...`
/// or `...@external`.
pub fn is_escalation_target(text: &str) -> bool {
    let is_part = |part: &str| {
        !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    };
    let Some((seat, rig)) = text.split_once('@') else { return false };
    is_part(seat) && is_part(rig) && seat != "human" && rig != "external"
}

/// A link end between pods: `pod.member`.
fn qualified(pod_id: &str, member_id: &str) -> String {
    format!("{pod_id}.{member_id}")
}

impl RigSpec {
    /// Seats in all pods.
    pub fn seat_count(&self) -> usize {
        self.pods.iter().map(|pod| pod.members.len()).sum()
    }

    /// Links inside pods and between pods.
    pub fn link_count(&self) -> usize {
        self.edges.len() + self.pods.iter().map(|pod| pod.edges.len()).sum::<usize>()
    }

    /// Remove seat `member` of pod `pod` and the links that name it. The
    /// links stay when another seat in the pod has the same id, since they
    /// may mean that one. Does nothing for a seat that does not exist.
    pub fn remove_seat(&mut self, pod: usize, member: usize) {
        let Some(pod) = self.pods.get_mut(pod) else { return };
        if member >= pod.members.len() {
            return;
        }
        let removed = pod.members.remove(member);
        if pod.members.iter().any(|other| other.id == removed.id) {
            return;
        }
        pod.edges.retain(|edge| edge.from != removed.id && edge.to != removed.id);
        let address = qualified(&pod.id, &removed.id);
        self.edges.retain(|edge| edge.from != address && edge.to != address);
    }

    /// Remove pod `pod` with its seats and the links between pods that name
    /// it. The links stay when another pod has the same id.
    pub fn remove_pod(&mut self, pod: usize) {
        if pod >= self.pods.len() {
            return;
        }
        let removed = self.pods.remove(pod);
        if self.pods.iter().any(|other| other.id == removed.id) {
            return;
        }
        let names_pod = |end: &str| end.split_once('.').is_some_and(|(pod_id, _)| pod_id == removed.id);
        self.edges.retain(|edge| !names_pod(&edge.from) && !names_pod(&edge.to));
    }

    /// Give seat `member` of pod `pod` the id `new_id`, and point the links
    /// that named the old id at the new one. The links stay as they are
    /// while another seat in the pod still has the old id, since they may
    /// mean that one.
    pub fn rename_seat(&mut self, pod: usize, member: usize, new_id: &str) {
        let Some(pod) = self.pods.get_mut(pod) else { return };
        let Some(seat) = pod.members.get_mut(member) else { return };
        let old_id = std::mem::replace(&mut seat.id, new_id.to_owned());
        if old_id == new_id || pod.members.iter().any(|other| other.id == old_id) {
            return;
        }
        for edge in &mut pod.edges {
            for end in [&mut edge.from, &mut edge.to] {
                if *end == old_id {
                    new_id.clone_into(end);
                }
            }
        }
        let (old_address, new_address) = (qualified(&pod.id, &old_id), qualified(&pod.id, new_id));
        for edge in &mut self.edges {
            for end in [&mut edge.from, &mut edge.to] {
                if *end == old_address {
                    end.clone_from(&new_address);
                }
            }
        }
    }

    /// Give pod `pod` the id `new_id`, and point the links between pods
    /// that named the old id at the new one (unless another pod still has
    /// the old id, which makes them ambiguous).
    pub fn rename_pod(&mut self, pod: usize, new_id: &str) {
        let Some(renamed) = self.pods.get_mut(pod) else { return };
        let old_id = std::mem::replace(&mut renamed.id, new_id.to_owned());
        if old_id == new_id || self.pods.iter().any(|other| other.id == old_id) {
            return;
        }
        for edge in &mut self.edges {
            for end in [&mut edge.from, &mut edge.to] {
                if let Some((pod_id, member)) = end.split_once('.')
                    && pod_id == old_id
                {
                    *end = qualified(new_id, member);
                }
            }
        }
    }
}

impl Pod {
    /// A new pod for the form to add, not yet in the file.
    pub fn new(id: &str, label: &str) -> Self {
        Self { id: id.to_owned(), label: label.to_owned(), ..Self::default() }
    }

    /// The pod as people read it: its label, else its id.
    pub fn name(&self) -> &str {
        if self.label.is_empty() { &self.id } else { &self.label }
    }
}

impl Member {
    /// A new seat for the form to add, not yet in the file.
    pub fn new(id: &str, runtime: &str) -> Self {
        Self { id: id.to_owned(), runtime: runtime.to_owned(), cwd: ".".to_owned(), ..Self::default() }
    }

    /// A copy to add as another seat: it carries the same values but is a
    /// new item, so a save writes it next to the original instead of over it.
    #[must_use]
    pub fn duplicate(&self) -> Self {
        let mut copy = self.clone();
        copy.origin = None;
        for file in &mut copy.startup.files {
            file.origin = None;
        }
        copy
    }

    /// A terminal seat: runtime `terminal` or the `builtin:terminal` agent.
    pub fn is_terminal(&self) -> bool {
        self.runtime == TERMINAL_RUNTIME || self.agent_ref == TERMINAL_AGENT
    }
}

impl Edge {
    /// A new link for the form to add, not yet in the file.
    pub fn new(kind: &str, from: &str, to: &str) -> Self {
        Self { origin: None, kind: kind.to_owned(), from: from.to_owned(), to: to.to_owned() }
    }
}

impl StartupFile {
    /// A new startup file for the form to add, not yet in the file.
    pub fn new(path: &str) -> Self {
        Self { path: path.to_owned(), ..Self::default() }
    }
}

impl Repo {
    /// A new workspace repository for the form to add, not yet in the file.
    pub fn new(name: &str, path: &str, kind: &str) -> Self {
        Self { origin: None, name: name.to_owned(), path: path.to_owned(), kind: kind.to_owned() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Validator lines split into the path and the message; other lines stay whole.
    #[test]
    fn parses_validator_lines() {
        let issue = Issue::parse("pods[0].members[1].id: must not contain dots");
        assert_eq!(issue, Issue::at("pods[0].members[1].id", "must not contain dots"));
        let issue = Issue::parse("rig spec must be an object");
        assert_eq!(issue, Issue::at("", "rig spec must be an object"));
        let issue = Issue::parse("YAML parse error: bad indent");
        assert_eq!(issue, Issue::at("", "YAML parse error: bad indent"));
    }

    /// A duplicated seat is a new item, so a save adds it instead of replacing the original.
    #[test]
    fn duplicate_is_a_new_item() {
        let spec = RigSpec::parse(include_str!("spec/fixtures/starter.yaml")).unwrap();
        let build = &spec.pods[0].members[0];
        let copy = build.duplicate();
        assert_eq!(copy.origin, None);
        assert!(copy.startup.files.iter().all(|file| file.origin.is_none()));
        assert_eq!(copy.agent_ref, build.agent_ref);
        assert_eq!(seat_address(&spec.pods[0].id, &build.id, &spec.name), "dev-build@starter");
    }

    const FACTORY: &str = include_str!("spec/fixtures/factory.yaml");

    /// The ends of every link between pods.
    fn cross_pod_ends(spec: &RigSpec) -> Vec<&str> {
        spec.edges.iter().flat_map(|edge| [edge.from.as_str(), edge.to.as_str()]).collect()
    }

    /// A removed seat takes its links inside the pod and between pods with it.
    #[test]
    fn removing_a_seat_removes_its_links() {
        let mut spec = RigSpec::parse(FACTORY).unwrap();
        spec.remove_seat(1, 0);
        let development = &spec.pods[1];
        assert_eq!(development.members.iter().map(|member| member.id.as_str()).collect::<Vec<_>>(), ["qa", "design"]);
        assert_eq!(development.edges, []);
        assert!(!cross_pod_ends(&spec).contains(&"dev.build"));
        assert!(cross_pod_ends(&spec).contains(&"dev.design"));
        spec.remove_seat(1, 9);
        assert_eq!(spec.pods[1].members.len(), 2);
    }

    /// Links stay while another seat still has the removed seat's id.
    #[test]
    fn removing_one_of_two_seats_with_an_id_keeps_the_links() {
        let mut spec = RigSpec::parse(FACTORY).unwrap();
        spec.pods[1].members[2].id = "build".into();
        spec.remove_seat(1, 2);
        assert_eq!(spec.pods[1].edges.len(), 1);
        assert!(cross_pod_ends(&spec).contains(&"dev.build"));
    }

    /// A removed pod takes the links between pods that name it.
    #[test]
    fn removing_a_pod_removes_its_links_between_pods() {
        let mut spec = RigSpec::parse(FACTORY).unwrap();
        spec.remove_pod(1);
        assert_eq!(spec.pods.iter().map(|pod| pod.id.as_str()).collect::<Vec<_>>(), ["orch", "review"]);
        assert!(cross_pod_ends(&spec).iter().all(|end| !end.starts_with("dev.")));
        assert!(cross_pod_ends(&spec).contains(&"review.r1"));
    }

    /// A renamed seat keeps its links, inside the pod and between pods, even
    /// when typed through an empty id.
    #[test]
    fn renaming_a_seat_renames_its_links() {
        let mut spec = RigSpec::parse(FACTORY).unwrap();
        spec.rename_seat(1, 0, "");
        spec.rename_seat(1, 0, "maker");
        assert_eq!(spec.pods[1].members[0].id, "maker");
        assert_eq!(spec.pods[1].edges[0].from, "maker");
        assert!(cross_pod_ends(&spec).contains(&"dev.maker"));
        assert!(!cross_pod_ends(&spec).contains(&"dev.build"));

        spec.rename_seat(1, 1, "maker");
        spec.rename_seat(1, 1, "qa");
        assert_eq!(spec.pods[1].edges[0].from, "maker", "a shared id leaves the links alone");
    }

    /// A renamed pod keeps its links between pods.
    #[test]
    fn renaming_a_pod_renames_its_links_between_pods() {
        let mut spec = RigSpec::parse(FACTORY).unwrap();
        spec.rename_pod(1, "build");
        assert_eq!(spec.pods[1].id, "build");
        assert!(cross_pod_ends(&spec).contains(&"build.design"));
        assert!(cross_pod_ends(&spec).iter().all(|end| !end.starts_with("dev.")));
    }

    /// Escalation targets follow `OpenRig`'s address pattern.
    #[test]
    fn recognises_escalation_targets() {
        for target in ["ops-lead@company", "a_b@c-d"] {
            assert!(is_escalation_target(target), "{target}");
        }
        for text in ["lead", "dev.lead", "human@company", "lead@external", "a@b@c", "@rig", "lead@", "a.b@rig"] {
            assert!(!is_escalation_target(text), "{text}");
        }
    }
}
