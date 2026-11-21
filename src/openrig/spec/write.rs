//! Write the form's changes back into a rig spec's YAML text.
//!
//! The text is edited in place through `yaml-edit`'s lossless tree: only
//! keys whose values changed are set or removed, so comments, key order,
//! quoting and every key the form does not know stay as they were.

use yaml_edit::{Mapping, Sequence};

use super::read::{parse_document, read_rig, rig_root};
use super::{ClaudeFields, CodexFields, Edge, EnvVar, Member, Pod, Repo, RigSpec, Startup, StartupFile, Workspace};

/// Writes one list item's fields: `(item's mapping, value before, value now)`.
type WriteItem<'a, T> = &'a dyn Fn(&Mapping, &T, &T) -> Result<(), String>;

impl RigSpec {
    /// The YAML `text` with this model's changes written into it.
    ///
    /// `text` must be the text this model was read from (its items remember
    /// their place in it). Without changes the text comes back unchanged,
    /// byte for byte.
    ///
    /// # Errors
    ///
    /// Fails when `text` is no longer a readable rig spec, or when items
    /// read from the file were put in another order (the form only adds and
    /// removes them).
    pub fn write_into(&self, text: &str) -> Result<String, String> {
        let document = parse_document(text)?;
        let root = rig_root(&document).ok_or("The file is no longer a rig spec.")?;
        let before = read_rig(&root)?;
        if *self == before {
            return Ok(text.to_owned());
        }
        write_rig(&root, &before, self)?;
        Ok(document.to_string())
    }
}

/// The team's own keys, then its pods and the links between pods.
fn write_rig(root: &Mapping, old: &RigSpec, new: &RigSpec) -> Result<(), String> {
    put_required(root, "version", &old.version, &new.version);
    put_required(root, "name", &old.name, &new.name);
    put_text(root, "summary", &old.summary, &new.summary);
    put_text(root, "culture_file", &old.culture_file, &new.culture_file);
    put_text(root, "permission_policy", &old.permission_policy, &new.permission_policy);
    put_flag(root, "non_interruptive", old.non_interruptive, new.non_interruptive);
    if old.claude_instruction_file != new.claude_instruction_file {
        with_child(root, "managed_blocks", |blocks| {
            put_text(blocks, "claude-code", &old.claude_instruction_file, &new.claude_instruction_file);
            Ok(())
        })?;
    }
    put_env(root, &old.env, &new.env);
    write_workspace(root, old.workspace.as_ref(), new.workspace.as_ref())?;
    put_docs(root, &old.docs, &new.docs);
    write_startup(root, &old.startup, &new.startup)?;
    sync_items(root, "pods", &old.pods, &new.pods, |pod| pod.origin, &write_pod)?;
    sync_items(root, "edges", &old.edges, &new.edges, |edge| edge.origin, &write_edge)
}

/// A pod's keys, its seats and the links inside it.
fn write_pod(map: &Mapping, old: &Pod, new: &Pod) -> Result<(), String> {
    put_required(map, "id", &old.id, &new.id);
    put_required(map, "label", &old.label, &new.label);
    put_text(map, "summary", &old.summary, &new.summary);
    sync_items(map, "members", &old.members, &new.members, |member| member.origin, &write_member)?;
    sync_items(map, "edges", &old.edges, &new.edges, |edge| edge.origin, &write_edge)
}

/// A seat's keys. For a new seat the order here is the order in the file.
fn write_member(map: &Mapping, old: &Member, new: &Member) -> Result<(), String> {
    put_required(map, "id", &old.id, &new.id);
    put_text(map, "label", &old.label, &new.label);
    put_required(map, "agent_ref", &old.agent_ref, &new.agent_ref);
    put_required(map, "runtime", &old.runtime, &new.runtime);
    put_text(map, "model", &old.model, &new.model);
    put_text(map, "effort", &old.effort, &new.effort);
    put_required(map, "profile", &old.profile, &new.profile);
    put_required(map, "cwd", &old.cwd, &new.cwd);
    put_text(map, "role", &old.role, &new.role);
    put_text(map, "permission_policy", &old.permission_policy, &new.permission_policy);
    put_text(map, "restore_policy", &old.restore_policy, &new.restore_policy);
    put_text(map, "compaction_strategy", &old.compaction_strategy, &new.compaction_strategy);
    put_text(map, "mechanic", &old.mechanic, &new.mechanic);
    put_env(map, &old.env, &new.env);
    write_startup(map, &old.startup, &new.startup)?;
    write_claude(map, &old.claude, &new.claude)?;
    write_codex(map, &old.codex, &new.codex)
}

/// The Claude Code only keys of a seat.
fn write_claude(map: &Mapping, old: &ClaudeFields, new: &ClaudeFields) -> Result<(), String> {
    put_text(map, "advisor", &old.advisor, &new.advisor);
    put_text(map, "claude_mcp", &old.mcp, &new.mcp);
    if (&old.allow, &old.deny) != (&new.allow, &new.deny) {
        with_child(map, "claude_permissions", |rules| {
            put_list(rules, "allow", &old.allow, &new.allow);
            put_list(rules, "deny", &old.deny, &new.deny);
            Ok(())
        })?;
    }
    put_text(map, "agent_file", &old.agent_file, &new.agent_file);
    put_text(map, "append_system_prompt_file", &old.append_system_prompt_file, &new.append_system_prompt_file);
    put_list(map, "plugin_dirs", &old.plugin_dirs, &new.plugin_dirs);
    put_list(map, "plugin_channels", &old.plugin_channels, &new.plugin_channels);
    if (&old.token_label, &old.token_file) != (&new.token_label, &new.token_file) {
        with_child(map, "claude_token", |token| {
            put_text(token, "label", &old.token_label, &new.token_label);
            put_text(token, "file", &old.token_file, &new.token_file);
            Ok(())
        })?;
    }
    Ok(())
}

/// The Codex only keys of a seat.
fn write_codex(map: &Mapping, old: &CodexFields, new: &CodexFields) -> Result<(), String> {
    put_text(map, "codex_config_profile", &old.config_profile, &new.config_profile);
    let old_envelope = (&old.sandbox, &old.approval, old.network_access, &old.writable_roots);
    let new_envelope = (&new.sandbox, &new.approval, new.network_access, &new.writable_roots);
    if old_envelope == new_envelope {
        return Ok(());
    }
    with_child(map, "codex_permissions", |codex| {
        put_text(codex, "sandbox", &old.sandbox, &new.sandbox);
        put_text(codex, "approval", &old.approval, &new.approval);
        put_flag(codex, "network_access", old.network_access, new.network_access);
        put_list(codex, "writable_roots", &old.writable_roots, &new.writable_roots);
        Ok(())
    })
}

/// A link's three keys.
#[expect(clippy::unnecessary_wraps, reason = "has the WriteItem shape that sync_items takes")]
fn write_edge(map: &Mapping, old: &Edge, new: &Edge) -> Result<(), String> {
    put_required(map, "kind", &old.kind, &new.kind);
    put_required(map, "from", &old.from, &new.from);
    put_required(map, "to", &old.to, &new.to);
    Ok(())
}

/// The `startup.files` list under `map`; actions are left as written.
fn write_startup(map: &Mapping, old: &Startup, new: &Startup) -> Result<(), String> {
    if old.files == new.files {
        return Ok(());
    }
    with_child(map, "startup", |startup| {
        sync_items(startup, "files", &old.files, &new.files, |file| file.origin, &write_startup_file)
    })
}

/// One startup file's keys.
#[expect(clippy::unnecessary_wraps, reason = "has the WriteItem shape that sync_items takes")]
fn write_startup_file(map: &Mapping, old: &StartupFile, new: &StartupFile) -> Result<(), String> {
    put_required(map, "path", &old.path, &new.path);
    put_text(map, "delivery_hint", &old.delivery_hint, &new.delivery_hint);
    put_flag(map, "required", old.required, new.required);
    put_list(map, "applies_on", &old.applies_on, &new.applies_on);
    Ok(())
}

/// The `workspace` block: removed when the model has none.
fn write_workspace(root: &Mapping, old: Option<&Workspace>, new: Option<&Workspace>) -> Result<(), String> {
    if old == new {
        return Ok(());
    }
    let Some(new) = new else {
        root.remove("workspace");
        return Ok(());
    };
    let blank = Workspace::default();
    let old = old.unwrap_or(&blank);
    with_child(root, "workspace", |workspace| {
        put_required(workspace, "workspace_root", &old.workspace_root, &new.workspace_root);
        sync_items(workspace, "repos", &old.repos, &new.repos, |repo| repo.origin, &write_repo)?;
        put_text(workspace, "default_repo", &old.default_repo, &new.default_repo);
        put_text(workspace, "knowledge_root", &old.knowledge_root, &new.knowledge_root);
        Ok(())
    })
}

/// One workspace repository's keys.
#[expect(clippy::unnecessary_wraps, reason = "has the WriteItem shape that sync_items takes")]
fn write_repo(map: &Mapping, old: &Repo, new: &Repo) -> Result<(), String> {
    put_required(map, "name", &old.name, &new.name);
    put_required(map, "path", &old.path, &new.path);
    put_required(map, "kind", &old.kind, &new.kind);
    Ok(())
}

/// Bring the list of mappings at `key` from `old` to `new`.
///
/// Items read from the file (with an origin) are edited in place, items
/// gone from `new` are removed, and new items are inserted where `new` has
/// them. Read items must keep their order.
fn sync_items<T: PartialEq + Default>(
    parent: &Mapping,
    key: &str,
    old: &[T],
    new: &[T],
    origin: fn(&T) -> Option<usize>,
    write_item: WriteItem<T>,
) -> Result<(), String> {
    if old == new {
        return Ok(());
    }
    let kept: Vec<usize> = new.iter().filter_map(origin).collect();
    if kept.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(format!("The form cannot reorder the items of {key}; edit their order in YAML."));
    }
    let existing = parent.get_sequence(key).filter(|sequence| !(sequence.is_flow_style() && sequence.is_empty()));
    let Some(sequence) = existing else {
        return replace_list(parent, key, new, write_item);
    };
    for index in (0..old.len()).rev() {
        if !kept.contains(&index) {
            sequence.remove(index);
        }
    }
    for (index, item) in new.iter().enumerate() {
        match origin(item) {
            Some(read_at) => {
                let before = old.get(read_at).ok_or("An item of the form does not match the file.")?;
                let node = sequence.get(index).ok_or("The file changed while the form was writing it.")?;
                let map = node.as_mapping().ok_or("A list item in the file is not a mapping.")?;
                write_item(map, before, item)?;
            }
            None => sequence.insert(index, &new_item(item, write_item)?),
        }
    }
    Ok(())
}

/// Set `key` to a new block list holding all of `items` (all new), or
/// remove it when there are none.
fn replace_list<T: Default>(parent: &Mapping, key: &str, items: &[T], write_item: WriteItem<T>) -> Result<(), String> {
    if items.is_empty() {
        parent.remove(key);
        return Ok(());
    }
    let sequence = Sequence::new_pending_block();
    for item in items {
        sequence.push(&new_item(item, write_item)?);
    }
    parent.set(key, &sequence);
    Ok(())
}

/// A mapping for an item that is not in the file yet: its fields written
/// against an empty item, so only the filled ones appear.
fn new_item<T: Default>(item: &T, write_item: WriteItem<T>) -> Result<Mapping, String> {
    let map = Mapping::new();
    write_item(&map, &T::default(), item)?;
    Ok(map)
}

/// Run `edit` on the mapping at `key`, creating it when absent, and remove
/// it again when it ends up empty.
fn with_child(parent: &Mapping, key: &str, edit: impl FnOnce(&Mapping) -> Result<(), String>) -> Result<(), String> {
    let child = if let Some(child) = parent.get_mapping(key) {
        child
    } else {
        parent.set(key, Mapping::new());
        parent.get_mapping(key).ok_or("A new block could not be added to the file.")?
    };
    edit(&child)?;
    if child.is_empty() {
        parent.remove(key);
    }
    Ok(())
}

/// An optional text key: set when changed, removed when emptied.
fn put_text(map: &Mapping, key: &str, old: &str, new: &str) {
    if old == new {
        return;
    }
    if new.is_empty() {
        map.remove(key);
    } else {
        map.set(key, new);
    }
}

/// A required text key: set when changed, kept (empty) when emptied, so it
/// stays in its place and the checks can point at it.
fn put_required(map: &Mapping, key: &str, old: &str, new: &str) {
    if old != new {
        map.set(key, new);
    }
}

/// An optional boolean key: set when changed, removed when `None`.
fn put_flag(map: &Mapping, key: &str, old: Option<bool>, new: Option<bool>) {
    if old == new {
        return;
    }
    match new {
        Some(value) => map.set(key, value),
        None => {
            map.remove(key);
        }
    }
}

/// An optional list of text: replaced when changed, removed when emptied.
fn put_list(map: &Mapping, key: &str, old: &[String], new: &[String]) {
    if old == new {
        return;
    }
    if new.is_empty() {
        map.remove(key);
        return;
    }
    let sequence = Sequence::new_pending_block();
    for item in new {
        sequence.push(item.as_str());
    }
    map.set(key, &sequence);
}

/// The `env` mapping, by variable name: removed names go, changed and new
/// ones are set. A variable without a name yet is not written.
fn put_env(map: &Mapping, old: &[EnvVar], new: &[EnvVar]) {
    if old == new {
        return;
    }
    let named: Vec<&EnvVar> = new.iter().filter(|var| !var.name.is_empty()).collect();
    if named.is_empty() {
        map.remove("env");
        return;
    }
    // Always Ok: the edit below cannot fail.
    let _ = with_child(map, "env", |env| {
        for var in old.iter().filter(|var| !named.iter().any(|kept| kept.name == var.name)) {
            env.remove(var.name.as_str());
        }
        for var in &named {
            let before = old.iter().find(|old_var| old_var.name == var.name);
            if before.is_none_or(|before| before.value != var.value) {
                env.set(var.name.as_str(), var.value.as_str());
            }
        }
        Ok(())
    });
}

/// The `docs` list of `{path}` entries: replaced when changed.
fn put_docs(map: &Mapping, old: &[String], new: &[String]) {
    if old == new {
        return;
    }
    if new.is_empty() {
        map.remove("docs");
        return;
    }
    let sequence = Sequence::new_pending_block();
    for path in new {
        let doc = Mapping::new();
        doc.set("path", path.as_str());
        sequence.push(&doc);
    }
    map.set("docs", &sequence);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openrig::spec::{CODEX_RUNTIME, EDGE_KINDS};

    const STARTER: &str = include_str!("fixtures/starter.yaml");
    const FACTORY: &str = include_str!("fixtures/factory.yaml");

    /// Reading and writing without an edit gives the same bytes.
    #[test]
    fn writes_unchanged_specs_back_byte_for_byte() {
        for text in [STARTER, FACTORY] {
            let spec = RigSpec::parse(text).unwrap();
            assert_eq!(spec.write_into(text).unwrap(), text);
        }
    }

    /// Changing one seat's model changes only that line.
    #[test]
    fn an_edit_leaves_unrelated_text_untouched() {
        let mut spec = RigSpec::parse(FACTORY).unwrap();
        spec.pods[1].members[1].model = "gpt-7".into();
        spec.pods[0].members[0].cwd = "app".into();
        let written = spec.write_into(FACTORY).unwrap();
        let expected = FACTORY
            .replacen("        runtime: codex\n        profile: default\n        cwd: \".\"\n      - id: design", "        runtime: codex\n        profile: default\n        cwd: \".\"\n        model: gpt-7\n      - id: design", 1)
            .replacen("        profile: default\n        cwd: \".\"\n        startup:\n          files:\n            - path: guidance/lead-first-move.md", "        profile: default\n        cwd: app\n        startup:\n          files:\n            - path: guidance/lead-first-move.md", 1);
        assert_ne!(expected, FACTORY, "the expected text must differ");
        assert_eq!(written, expected);
    }

    /// Comments and keys the form does not know survive edits around them.
    #[test]
    fn keeps_comments_and_unknown_keys() {
        let text = STARTER.replace("    members:\n", "    members: # the two seats\n").replace(
            "        runtime: codex\n",
            "        runtime: codex\n        launch_wrapper: [nice] # lower priority\n",
        );
        let mut spec = RigSpec::parse(&text).unwrap();
        spec.pods[0].members[1].effort = "high".into();
        spec.summary = "A short summary.".into();
        let written = spec.write_into(&text).unwrap();
        assert!(written.contains("    members: # the two seats\n"), "{written}");
        assert!(written.contains("        launch_wrapper: [nice] # lower priority\n"), "{written}");
        assert!(written.contains("summary: A short summary.\n"), "{written}");
        assert_eq!(RigSpec::parse(&written).unwrap(), spec);
    }

    /// Seats and links can be added and removed; a `[]` list becomes a block list.
    #[test]
    fn adds_and_removes_items() {
        let mut spec = RigSpec::parse(FACTORY).unwrap();
        spec.pods[1].members.remove(2);
        let mut docs = Member::new("docs", CODEX_RUNTIME);
        docs.agent_ref = "path:/agents/docs".into();
        docs.profile = "default".into();
        docs.env.push(EnvVar { name: "MODE".into(), value: "1".into() });
        spec.pods[1].members.insert(1, docs);
        spec.pods[2].edges.push(Edge::new(EDGE_KINDS[3], "r1", "r2"));
        spec.edges.remove(0);
        let mut ops = Pod::new("ops", "Operations");
        ops.members.push(Member::new("watch", "terminal"));
        spec.pods.push(ops);
        let written = spec.write_into(FACTORY).unwrap();
        let reread = RigSpec::parse(&written).unwrap();
        let ids = |pod: &Pod| pod.members.iter().map(|member| member.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&reread.pods[1]), ["build", "docs", "qa"]);
        assert_eq!(reread.pods[1].members[1].env[0].value, "1");
        assert_eq!(reread.pods[2].edges[0].kind, "collaborates_with");
        assert_eq!(reread.edges.len(), 8);
        assert_eq!(reread.pods[3].members[0].runtime, "terminal");
        assert!(
            written.contains("    edges:\n      - kind: collaborates_with\n        from: r1\n        to: r2\n"),
            "{written}"
        );
        assert!(written.contains("        cwd: \".\"\n      - id: docs\n"), "{written}");
        assert!(!written.contains("product-designer"));
    }

    /// Nested blocks are created when needed and removed when emptied.
    #[test]
    fn writes_runtime_blocks_and_environment() {
        let mut spec = RigSpec::parse(STARTER).unwrap();
        let review = &mut spec.pods[0].members[1];
        review.codex.sandbox = "workspace-write".into();
        review.codex.writable_roots = vec!["/srv/lease".into()];
        review.codex.network_access = Some(false);
        let build = &mut spec.pods[0].members[0];
        build.claude.allow = vec!["Bash(npm test *)".into()];
        build.claude.mcp = "none".into();
        build.startup.files[0].required = Some(false);
        build.startup.files.push(StartupFile::new("guidance/extra.md"));
        spec.env.push(EnvVar { name: "FEATURE_FLAGS".into(), value: "beta".into() });
        spec.env.push(EnvVar::default());
        let written = spec.write_into(STARTER).unwrap();
        let reread = RigSpec::parse(&written).unwrap();
        assert_eq!(reread.pods[0].members[1].codex, spec.pods[0].members[1].codex);
        assert_eq!(reread.pods[0].members[0].claude.allow, ["Bash(npm test *)"]);
        assert_eq!(reread.pods[0].members[0].startup.files.len(), 2);
        assert_eq!(reread.env, [EnvVar { name: "FEATURE_FLAGS".into(), value: "beta".into() }]);

        let mut cleared = reread.clone();
        cleared.pods[0].members[1].codex = CodexFields::default();
        cleared.env.clear();
        let written_again = cleared.write_into(&written).unwrap();
        assert!(!written_again.contains("codex_permissions"), "{written_again}");
        assert!(!written_again.contains("env:"), "{written_again}");
    }

    /// Read items that changed order are refused rather than rewritten.
    #[test]
    fn refuses_reordered_items() {
        let mut spec = RigSpec::parse(FACTORY).unwrap();
        spec.pods.swap(0, 1);
        assert!(spec.write_into(FACTORY).unwrap_err().contains("reorder"));
    }

    /// The workspace block is written in full and removed again.
    #[test]
    fn writes_and_removes_the_workspace() {
        let mut spec = RigSpec::parse(STARTER).unwrap();
        spec.workspace = Some(Workspace {
            workspace_root: "~/code/shop".into(),
            repos: vec![Repo::new("app", "app", "project")],
            default_repo: "app".into(),
            knowledge_root: String::new(),
        });
        let written = spec.write_into(STARTER).unwrap();
        let reread = RigSpec::parse(&written).unwrap();
        let workspace = reread.workspace.as_ref().unwrap();
        assert_eq!((workspace.workspace_root.as_str(), workspace.default_repo.as_str()), ("~/code/shop", "app"));
        assert_eq!((workspace.repos[0].name.as_str(), workspace.repos[0].kind.as_str()), ("app", "project"));
        let mut cleared = reread;
        cleared.workspace = None;
        assert!(!cleared.write_into(&written).unwrap().contains("workspace"));
    }
}
