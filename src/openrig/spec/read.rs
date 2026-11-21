//! Read a rig spec's YAML text into the typed model.

use std::str::FromStr;

use yaml_edit::{Document, Mapping};

use super::{ClaudeFields, CodexFields, Edge, EnvVar, Member, Pod, Repo, RigSpec, Startup, StartupFile, Workspace};

/// Team keys that are kept as written and edited in YAML only.
const RIG_YAML_ONLY: [&str; 1] = ["services"];
/// Pod keys that are kept as written and edited in YAML only.
const POD_YAML_ONLY: [&str; 2] = ["continuity_policy", "startup"];
/// Seat keys that are kept as written and edited in YAML only.
const MEMBER_YAML_ONLY: [&str; 5] =
    ["claude_auto_mode", "launch_wrapper", "process_limits", "session_source", "starter_ref"];

impl RigSpec {
    /// Read a rig spec from YAML text.
    ///
    /// # Errors
    ///
    /// Fails when the text is not valid YAML, or is not a rig spec: a
    /// mapping with `name`, `version` and a `pods` list of mappings.
    pub fn parse(text: &str) -> Result<Self, String> {
        let document = parse_document(text)?;
        let root = rig_root(&document).ok_or("This YAML is not a rig spec: it needs name, version and pods.")?;
        read_rig(&root)
    }
}

/// Parse YAML text, keeping every byte (comments and layout) in the tree.
pub(super) fn parse_document(text: &str) -> Result<Document, String> {
    Document::from_str(text).map_err(|error| format!("The YAML cannot be read: {error}"))
}

/// The top-level mapping when it has the keys of a rig spec.
pub(super) fn rig_root(document: &Document) -> Option<Mapping> {
    let root = document.as_mapping()?;
    let has_keys = root.contains_key("name") && root.contains_key("version") && root.get_sequence("pods").is_some();
    has_keys.then_some(root)
}

/// The whole spec from its top-level mapping.
pub(super) fn read_rig(root: &Mapping) -> Result<RigSpec, String> {
    let pod_maps = mappings(root, "pods").ok_or("Every pod in the spec must be a mapping.")?;
    let mut pods = Vec::with_capacity(pod_maps.len());
    for (origin, map) in pod_maps.iter().enumerate() {
        pods.push(read_pod(map, origin)?);
    }
    let edges = read_edges(root).ok_or("Every link between pods must be a mapping.")?;
    Ok(RigSpec {
        name: text(root, "name"),
        version: text(root, "version"),
        summary: text(root, "summary"),
        culture_file: text(root, "culture_file"),
        permission_policy: text(root, "permission_policy"),
        non_interruptive: flag(root, "non_interruptive"),
        claude_instruction_file: root
            .get_mapping("managed_blocks")
            .map(|map| text(&map, "claude-code"))
            .unwrap_or_default(),
        env: env(root),
        workspace: root.get_mapping("workspace").map(|map| read_workspace(&map)),
        docs: mappings(root, "docs").unwrap_or_default().iter().map(|map| text(map, "path")).collect(),
        startup: read_startup(root),
        pods,
        edges,
        yaml_only: present(root, &RIG_YAML_ONLY),
    })
}

/// One pod, `origin` being its place in the file's `pods` list.
fn read_pod(map: &Mapping, origin: usize) -> Result<Pod, String> {
    let member_maps = mappings(map, "members").ok_or("Every pod needs a members list of mappings.")?;
    let members = member_maps.iter().enumerate().map(|(index, member)| read_member(member, index)).collect();
    Ok(Pod {
        origin: Some(origin),
        id: text(map, "id"),
        label: text(map, "label"),
        summary: text(map, "summary"),
        members,
        edges: read_edges(map).ok_or("Every link inside a pod must be a mapping.")?,
        yaml_only: present(map, &POD_YAML_ONLY),
    })
}

/// One seat, `origin` being its place in the pod's `members` list.
fn read_member(map: &Mapping, origin: usize) -> Member {
    Member {
        origin: Some(origin),
        id: text(map, "id"),
        label: text(map, "label"),
        agent_ref: text(map, "agent_ref"),
        profile: text(map, "profile"),
        runtime: text(map, "runtime"),
        model: text(map, "model"),
        effort: text(map, "effort"),
        cwd: text(map, "cwd"),
        role: text(map, "role"),
        permission_policy: text(map, "permission_policy"),
        restore_policy: text(map, "restore_policy"),
        compaction_strategy: text(map, "compaction_strategy"),
        mechanic: text(map, "mechanic"),
        env: env(map),
        startup: read_startup(map),
        claude: read_claude(map),
        codex: read_codex(map),
        yaml_only: present(map, &MEMBER_YAML_ONLY),
    }
}

/// The Claude Code only fields of a seat.
fn read_claude(map: &Mapping) -> ClaudeFields {
    let rules = map.get_mapping("claude_permissions");
    let token = map.get_mapping("claude_token");
    ClaudeFields {
        advisor: text(map, "advisor"),
        mcp: text(map, "claude_mcp"),
        allow: rules.as_ref().map(|rules| list(rules, "allow")).unwrap_or_default(),
        deny: rules.as_ref().map(|rules| list(rules, "deny")).unwrap_or_default(),
        agent_file: text(map, "agent_file"),
        append_system_prompt_file: text(map, "append_system_prompt_file"),
        plugin_dirs: list(map, "plugin_dirs"),
        plugin_channels: list(map, "plugin_channels"),
        token_label: token.as_ref().map(|token| text(token, "label")).unwrap_or_default(),
        token_file: token.as_ref().map(|token| text(token, "file")).unwrap_or_default(),
    }
}

/// The Codex only fields of a seat.
fn read_codex(map: &Mapping) -> CodexFields {
    let permissions = map.get_mapping("codex_permissions");
    let permissions = permissions.as_ref();
    CodexFields {
        config_profile: text(map, "codex_config_profile"),
        sandbox: permissions.map(|codex| text(codex, "sandbox")).unwrap_or_default(),
        approval: permissions.map(|codex| text(codex, "approval")).unwrap_or_default(),
        network_access: permissions.and_then(|codex| flag(codex, "network_access")),
        writable_roots: permissions.map(|codex| list(codex, "writable_roots")).unwrap_or_default(),
    }
}

/// The `edges` list of a pod or of the whole rig; `None` when an item is not a mapping.
fn read_edges(map: &Mapping) -> Option<Vec<Edge>> {
    let Some(sequence) = map.get_sequence("edges") else { return Some(Vec::new()) };
    let items = sequence.values().map(|node| node.as_mapping().cloned()).collect::<Option<Vec<_>>>()?;
    Some(
        items
            .iter()
            .enumerate()
            .map(|(origin, edge)| Edge {
                origin: Some(origin),
                kind: text(edge, "kind"),
                from: text(edge, "from"),
                to: text(edge, "to"),
            })
            .collect(),
    )
}

/// The `startup` block under `map`: its files, and how many actions it has.
fn read_startup(map: &Mapping) -> Startup {
    let Some(startup) = map.get_mapping("startup") else { return Startup::default() };
    let files = mappings(&startup, "files")
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(origin, file)| StartupFile {
            origin: Some(origin),
            path: text(file, "path"),
            delivery_hint: text(file, "delivery_hint"),
            required: flag(file, "required"),
            applies_on: list(file, "applies_on"),
        })
        .collect();
    let action_count = startup.get_sequence("actions").map_or(0, |actions| actions.len());
    Startup { files, action_count }
}

/// The team's `workspace` block.
fn read_workspace(map: &Mapping) -> Workspace {
    let repos = mappings(map, "repos")
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(origin, repo)| Repo {
            origin: Some(origin),
            name: text(repo, "name"),
            path: text(repo, "path"),
            kind: text(repo, "kind"),
        })
        .collect();
    Workspace {
        workspace_root: text(map, "workspace_root"),
        repos,
        default_repo: text(map, "default_repo"),
        knowledge_root: text(map, "knowledge_root"),
    }
}

/// The scalar at `key` as text; `""` when absent, null or not a scalar.
pub(super) fn text(map: &Mapping, key: &str) -> String {
    match map.get(key) {
        Some(node) => node.as_scalar().filter(|scalar| !scalar.is_null()).map(yaml_edit::Scalar::as_string),
        None => None,
    }
    .unwrap_or_default()
}

/// The boolean at `key`; `None` when absent or not a boolean.
pub(super) fn flag(map: &Mapping, key: &str) -> Option<bool> {
    map.get(key)?.to_bool()
}

/// The list of scalars at `key`; items that are not scalars are left out.
pub(super) fn list(map: &Mapping, key: &str) -> Vec<String> {
    let Some(sequence) = map.get_sequence(key) else { return Vec::new() };
    sequence.values().filter_map(|node| node.as_scalar().map(yaml_edit::Scalar::as_string)).collect()
}

/// The `env` mapping at `map`, in file order.
pub(super) fn env(map: &Mapping) -> Vec<EnvVar> {
    let Some(env) = map.get_mapping("env") else { return Vec::new() };
    env.iter()
        .filter_map(|(name, value)| {
            let name = name.as_scalar()?.as_string();
            let value = value.as_scalar().map(yaml_edit::Scalar::as_string).unwrap_or_default();
            Some(EnvVar { name, value })
        })
        .collect()
}

/// The items of the list at `key` when every one is a mapping; an absent
/// key is an empty list.
pub(super) fn mappings(map: &Mapping, key: &str) -> Option<Vec<Mapping>> {
    let Some(sequence) = map.get_sequence(key) else { return Some(Vec::new()) };
    sequence.values().map(|node| node.as_mapping().cloned()).collect()
}

/// Which of `keys` the mapping has.
fn present(map: &Mapping, keys: &[&str]) -> Vec<String> {
    keys.iter().filter(|key| map.contains_key(**key)).map(|key| (*key).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openrig::spec::is_rig_spec;

    const STARTER: &str = include_str!("fixtures/starter.yaml");
    const FACTORY: &str = include_str!("fixtures/factory.yaml");

    /// The starter spec reads into its pod, seats, link and startup file.
    #[test]
    fn reads_the_starter_spec() {
        let spec = RigSpec::parse(STARTER).unwrap();
        assert_eq!(spec.name, "starter");
        assert_eq!(spec.version, "0.2");
        assert!(spec.summary.starts_with("A Claude Code builder and a Codex reviewer"));
        assert_eq!(spec.culture_file, "CULTURE.md");
        assert_eq!(spec.pods.len(), 1);
        let pod = &spec.pods[0];
        assert_eq!((pod.id.as_str(), pod.label.as_str()), ("dev", "Project"));
        let build = &pod.members[0];
        assert_eq!(build.agent_ref, "local:../../../agents/development/implementer");
        assert_eq!(
            (build.runtime.as_str(), build.profile.as_str(), build.cwd.as_str()),
            ("claude-code", "default", ".")
        );
        assert_eq!(build.startup.files.len(), 1);
        assert_eq!(build.startup.files[0].delivery_hint, "send_text");
        assert_eq!(build.startup.files[0].required, Some(true));
        assert_eq!(pod.members[1].model, "gpt-6-astra");
        assert_eq!(
            pod.edges,
            [Edge { origin: Some(0), kind: "delegates_to".into(), from: "build".into(), to: "review".into() }]
        );
        assert_eq!(spec.edges, []);
        assert_eq!((spec.seat_count(), spec.link_count()), (2, 1));
    }

    /// The factory spec has three pods and nine links between them.
    #[test]
    fn reads_the_factory_spec() {
        let spec = RigSpec::parse(FACTORY).unwrap();
        assert_eq!(spec.pods.iter().map(|pod| pod.id.as_str()).collect::<Vec<_>>(), ["orch", "dev", "review"]);
        assert_eq!((spec.seat_count(), spec.link_count()), (7, 11));
        assert_eq!(spec.edges[5].kind, "can_observe");
        assert_eq!(spec.pods[0].members[1].profile, "advisor");
    }

    /// Only YAML with name, version and a pods list opens in the form.
    #[test]
    fn recognises_rig_specs_only() {
        assert!(is_rig_spec(STARTER));
        assert!(!is_rig_spec("name: x\nversion: '1'\n"));
        assert!(!is_rig_spec("services:\n  web:\n    image: nginx\n"));
        assert!(!is_rig_spec("pods: [\n"));
        assert!(RigSpec::parse("name: x\nversion: '0.2'\npods:\n  - just text\n").is_err());
    }

    /// Fields the form shows but does not edit are listed by name.
    #[test]
    fn lists_yaml_only_fields_and_reads_nested_blocks() {
        let text = "version: '0.2'\nname: t\nservices:\n  kind: compose\nenv:\n  A: '1'\nworkspace:\n  workspace_root: ~/code\n  repos:\n    - name: app\n      path: app\n      kind: project\npods:\n  - id: p\n    label: P\n    startup:\n      files: []\n    members:\n      - id: m\n        agent_ref: local:a\n        profile: default\n        runtime: codex\n        cwd: .\n        launch_wrapper: [nice]\n        codex_permissions:\n          sandbox: read-only\n          network_access: true\n        claude_permissions:\n          allow: [\"Bash(ls)\"]\n        env:\n          B: two\n";
        let spec = RigSpec::parse(text).unwrap();
        assert_eq!(spec.yaml_only, ["services"]);
        assert_eq!(spec.env, [EnvVar { name: "A".into(), value: "1".into() }]);
        let workspace = spec.workspace.as_ref().unwrap();
        assert_eq!(workspace.workspace_root, "~/code");
        assert_eq!(workspace.repos[0].kind, "project");
        assert_eq!(spec.pods[0].yaml_only, ["startup"]);
        let member = &spec.pods[0].members[0];
        assert_eq!(member.yaml_only, ["launch_wrapper"]);
        assert_eq!(member.codex.sandbox, "read-only");
        assert_eq!(member.codex.network_access, Some(true));
        assert_eq!(member.claude.allow, ["Bash(ls)"]);
        assert_eq!(member.env[0].value, "two");
    }
}
