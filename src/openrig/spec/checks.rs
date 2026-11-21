//! Cheap local checks that mirror `OpenRig`'s structural validator, so the
//! form can mark a field before `rig spec validate` runs. They report the
//! same paths as the validator (`pods[0].members[1].id`).

use super::{
    EDGE_KINDS, ESCALATES_TO, Edge, EnvVar, Issue, Member, Pod, RESTORE_POLICIES, RigSpec, TERMINAL_AGENT,
    TERMINAL_PROFILE, TERMINAL_RUNTIME, is_escalation_target,
};

/// Prefix of environment names `OpenRig` keeps for itself.
const RESERVED_ENV_PREFIX: &str = "OPENRIG_";

impl RigSpec {
    /// Problems the form can find without `OpenRig`, in file order.
    pub fn check(&self) -> Vec<Issue> {
        let mut issues = Vec::new();
        required(&mut issues, "name", &self.name);
        required(&mut issues, "version", &self.version);
        check_env(&mut issues, "env", &self.env);
        if self.pods.is_empty() {
            issues.push(Issue::at("pods", "add at least one pod"));
        }
        for (index, pod) in self.pods.iter().enumerate() {
            let path = format!("pods[{index}]");
            if self.pods[..index].iter().any(|other| other.id == pod.id) && !pod.id.is_empty() {
                issues.push(Issue::at(format!("{path}.id"), format!("another pod is already called \"{}\"", pod.id)));
            }
            check_pod(&mut issues, &path, pod);
        }
        for (index, edge) in self.edges.iter().enumerate() {
            check_cross_pod_edge(&mut issues, &format!("edges[{index}]"), edge, &self.pods);
        }
        issues
    }
}

/// A pod's own fields, its seats and the links inside it.
fn check_pod(issues: &mut Vec<Issue>, path: &str, pod: &Pod) {
    check_id(issues, &format!("{path}.id"), &pod.id);
    required(issues, &format!("{path}.label"), &pod.label);
    for (index, member) in pod.members.iter().enumerate() {
        let member_path = format!("{path}.members[{index}]");
        if pod.members[..index].iter().any(|other| other.id == member.id) && !member.id.is_empty() {
            let message = format!("another seat in this pod is already called \"{}\"", member.id);
            issues.push(Issue::at(format!("{member_path}.id"), message));
        }
        check_member(issues, &member_path, member);
    }
    for (index, edge) in pod.edges.iter().enumerate() {
        let edge_path = format!("{path}.edges[{index}]");
        check_kind(issues, &edge_path, edge);
        for (end, id) in [("from", &edge.from), ("to", &edge.to)] {
            let escalation = end == "to" && names_escalation_target(edge);
            if !escalation && !pod.members.iter().any(|member| member.id == *id) {
                issues.push(Issue::at(format!("{edge_path}.{end}"), format!("no seat \"{id}\" in this pod")));
            }
        }
    }
}

/// An `escalates_to` link whose target is a seat address, possibly in
/// another rig, which `OpenRig` resolves only when it escalates.
fn names_escalation_target(edge: &Edge) -> bool {
    edge.kind == ESCALATES_TO && is_escalation_target(&edge.to)
}

/// A seat's required fields, its agent link and the terminal seat rules.
fn check_member(issues: &mut Vec<Issue>, path: &str, member: &Member) {
    check_id(issues, &format!("{path}.id"), &member.id);
    for (key, value) in [
        ("agent_ref", &member.agent_ref),
        ("profile", &member.profile),
        ("runtime", &member.runtime),
        ("cwd", &member.cwd),
    ] {
        required(issues, &format!("{path}.{key}"), value);
    }
    check_agent_ref(issues, &format!("{path}.agent_ref"), &member.agent_ref);
    check_terminal(issues, path, member);
    if !member.role.is_empty() && !member.role.bytes().all(is_role_byte) {
        issues.push(Issue::at(format!("{path}.role"), "use letters, digits, _, . and - only"));
    }
    if !member.restore_policy.is_empty() && !RESTORE_POLICIES.contains(&member.restore_policy.as_str()) {
        issues.push(Issue::at(format!("{path}.restore_policy"), "choose one of the listed values"));
    }
    check_env(issues, &format!("{path}.env"), &member.env);
}

/// A terminal seat needs the exact triple and has no role or permissions.
fn check_terminal(issues: &mut Vec<Issue>, path: &str, member: &Member) {
    if !member.is_terminal() {
        return;
    }
    let triple = [
        ("runtime", &member.runtime, TERMINAL_RUNTIME),
        ("agent_ref", &member.agent_ref, TERMINAL_AGENT),
        ("profile", &member.profile, TERMINAL_PROFILE),
    ];
    for (key, value, wanted) in triple {
        if value != wanted {
            issues.push(Issue::at(format!("{path}.{key}"), format!("a terminal seat needs {key}: {wanted}")));
        }
    }
    for (key, value) in [("role", &member.role), ("permission_policy", &member.permission_policy)] {
        if !value.is_empty() {
            issues.push(Issue::at(format!("{path}.{key}"), format!("a terminal seat has no {key}")));
        }
    }
}

/// `local:` must be relative, `path:` absolute; `builtin:terminal` is the only other form.
fn check_agent_ref(issues: &mut Vec<Issue>, path: &str, agent_ref: &str) {
    if agent_ref.is_empty() || agent_ref == TERMINAL_AGENT {
        return;
    }
    let message = if let Some(relative) = agent_ref.strip_prefix("local:") {
        relative.starts_with('/').then_some("a local: link must be a relative path")
    } else if let Some(absolute) = agent_ref.strip_prefix("path:") {
        (!absolute.starts_with('/')).then_some("a path: link must be an absolute path")
    } else {
        Some("start the agent link with local: or path:")
    };
    if let Some(message) = message {
        issues.push(Issue::at(path, message));
    }
}

/// A link between pods: `pod.seat` ends in two different pods that exist.
fn check_cross_pod_edge(issues: &mut Vec<Issue>, path: &str, edge: &Edge, pods: &[Pod]) {
    check_kind(issues, path, edge);
    let mut pod_ids = Vec::new();
    for (end, address) in [("from", &edge.from), ("to", &edge.to)] {
        if end == "to" && names_escalation_target(edge) {
            continue;
        }
        let Some((pod_id, member_id)) = address.split_once('.') else {
            issues.push(Issue::at(format!("{path}.{end}"), "name the seat as pod.seat"));
            continue;
        };
        let found = pods
            .iter()
            .find(|pod| pod.id == pod_id)
            .is_some_and(|pod| pod.members.iter().any(|member| member.id == member_id));
        if !found {
            issues.push(Issue::at(format!("{path}.{end}"), format!("no seat \"{address}\" in this team")));
        }
        pod_ids.push(pod_id);
    }
    if let [from_pod, to_pod] = pod_ids[..]
        && from_pod == to_pod
    {
        issues.push(Issue::at(path, "both seats are in one pod; add this link inside the pod"));
    }
}

/// The link kind is one of [`EDGE_KINDS`].
fn check_kind(issues: &mut Vec<Issue>, path: &str, edge: &Edge) {
    if !EDGE_KINDS.contains(&edge.kind.as_str()) {
        issues.push(Issue::at(format!("{path}.kind"), "choose one of the listed kinds"));
    }
}

/// A pod or seat id: required, letters, digits, `-` and `_`, no `.` or `@`.
fn check_id(issues: &mut Vec<Issue>, path: &str, id: &str) {
    if id.is_empty() {
        issues.push(Issue::at(path, "required"));
    } else if id.contains(['.', '@']) {
        issues.push(Issue::at(path, "no dots or @: they separate parts of the seat address"));
    } else if !id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')) {
        issues.push(Issue::at(path, "use letters, digits, - and _ only"));
    }
}

/// Environment names in capitals, not reserved, with a value.
fn check_env(issues: &mut Vec<Issue>, path: &str, env: &[EnvVar]) {
    for var in env {
        let place = format!("{path}.{}", var.name);
        if !is_env_name(&var.name) {
            issues.push(Issue::at(&place, "use capitals, digits and _, not starting with a digit"));
        } else if var.name.starts_with(RESERVED_ENV_PREFIX) {
            issues.push(Issue::at(&place, "OPENRIG_ names are reserved"));
        } else if var.value.is_empty() {
            issues.push(Issue::at(&place, "give it a value"));
        }
    }
}

/// `^[A-Z_][A-Z0-9_]*$`.
fn is_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|first| first.is_ascii_uppercase() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Characters allowed in a role.
fn is_role_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')
}

/// A required text field.
fn required(issues: &mut Vec<Issue>, path: &str, value: &str) {
    if value.trim().is_empty() {
        issues.push(Issue::at(path, "required"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FACTORY: &str = include_str!("fixtures/factory.yaml");

    /// The shipped specs pass.
    #[test]
    fn shipped_specs_pass() {
        assert_eq!(RigSpec::parse(FACTORY).unwrap().check(), []);
        assert_eq!(RigSpec::parse(include_str!("fixtures/starter.yaml")).unwrap().check(), []);
    }

    /// Each rule reports the validator's path for the field.
    #[test]
    fn reports_problems_at_their_fields() {
        let mut spec = RigSpec::parse(FACTORY).unwrap();
        spec.pods[1].members[1].id = "q.a".into();
        spec.pods[1].members[2].id = "build".into();
        spec.pods[0].members[0].agent_ref = "local:/abs".into();
        let watch = &mut spec.pods[2].members[0];
        watch.runtime = TERMINAL_RUNTIME.into();
        watch.role = "watcher".into();
        spec.pods[0].edges[0].to = "nobody".into();
        spec.edges[0].to = "orch.advisor".into();
        spec.env.push(EnvVar { name: "OPENRIG_X".into(), value: "1".into() });
        let paths: Vec<String> = spec.check().into_iter().map(|issue| issue.path).collect();
        for expected in [
            "env.OPENRIG_X",
            "pods[0].members[0].agent_ref",
            "pods[0].edges[0].to",
            "pods[1].members[1].id",
            "pods[1].members[2].id",
            "pods[2].members[0].agent_ref",
            "pods[2].members[0].profile",
            "pods[2].members[0].role",
            "edges[0]",
        ] {
            assert!(paths.iter().any(|path| path == expected), "{expected} missing from {paths:?}");
        }
    }

    /// An escalation may name a seat address in another rig, inside a pod or
    /// between pods, as the validator allows; other kinds may not.
    #[test]
    fn escalation_targets_may_be_in_another_rig() {
        let mut spec = RigSpec::parse(FACTORY).unwrap();
        spec.pods[0].edges.push(Edge::new(ESCALATES_TO, "lead", "ops-lead@company"));
        spec.edges.push(Edge::new(ESCALATES_TO, "dev.build", "ops-lead@company"));
        assert_eq!(spec.check(), []);

        spec.pods[0].edges.push(Edge::new("delegates_to", "lead", "ops-lead@company"));
        spec.edges.push(Edge::new(ESCALATES_TO, "dev.build", "human@company"));
        let paths: Vec<String> = spec.check().into_iter().map(|issue| issue.path).collect();
        let last_pod_edge = spec.pods[0].edges.len() - 1;
        let last_edge = spec.edges.len() - 1;
        assert_eq!(paths, [format!("pods[0].edges[{last_pod_edge}].to"), format!("edges[{last_edge}].to")]);
    }
}
