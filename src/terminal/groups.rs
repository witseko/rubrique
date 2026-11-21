//! Groups of the explorer's Terminals and `OpenRig` lists: a plain session
//! belongs to the explorer folder it runs in (Terminals), and an `OpenRig`
//! seat (a session named `seat@rig`) to its rig, split into pods (`OpenRig`).
//! Sessions outside every folder go under "Other".

use std::collections::BTreeMap;
use std::path::Path;

use super::monitor::TerminalRow;

/// Title of the group for sessions outside every explorer folder.
const OTHER_TITLE: &str = "Other";

/// The groups of both lists, by indexes into the same rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionGroups {
    /// The Terminals list: one group per explorer folder, then "Other".
    pub terminals: Vec<TerminalGroup>,
    /// The `OpenRig` list: one group per rig, with its pods.
    pub rigs: Vec<TerminalGroup>,
}

/// One group of the Terminals list, as the explorer draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalGroup {
    /// What folding remembers the group by: `folder:<path>`, `other`,
    /// `rig:<rig>` or `rig:<rig>/<pod>`.
    pub key: String,
    /// The folder's name, "Other", the rig's name or the pod's name.
    pub title: String,
    /// The explorer folder's index, for the group color.
    pub root: Option<usize>,
    /// Indexes into the rows the groups were made from, in their order.
    pub rows: Vec<usize>,
    /// The pods of a rig (its own `rows` stay empty); empty for other groups.
    pub children: Vec<TerminalGroup>,
}

impl TerminalGroup {
    /// How many sessions the group holds, its sub-groups' included.
    pub fn session_count(&self) -> usize {
        self.rows.len() + self.children.iter().map(TerminalGroup::session_count).sum::<usize>()
    }
}

/// The rig and pod of an `OpenRig` seat's session (`dev-impl@my-rig` is pod
/// `dev` of rig `my-rig`): the pod is the seat up to its first `-`, or the
/// whole seat. `None` for a plain session name.
pub fn seat_of(session_name: &str) -> Option<(&str, &str)> {
    let (seat, rig) = session_name.split_once('@')?;
    if seat.is_empty() || rig.is_empty() {
        return None;
    }
    let pod = seat.split_once('-').map_or(seat, |(pod, _)| pod);
    Some((rig, pod))
}

/// Sort `rows` into groups: for Terminals one per explorer folder in `roots`
/// (name and path, in explorer order) that has sessions, then "Other"; for
/// `OpenRig` one group per rig by name with its pods by name. A session
/// belongs to the deepest folder it runs in; a seat's session always to its
/// rig. Rows keep their order inside a group, and groups without sessions are
/// left out. Without `openrig` every session is a plain terminal.
pub fn group_terminals(rows: &[TerminalRow], roots: &[(&str, &Path)], openrig: bool) -> SessionGroups {
    let mut folder_rows = vec![Vec::new(); roots.len()];
    let mut other_rows = Vec::new();
    let mut rigs: BTreeMap<&str, BTreeMap<&str, Vec<usize>>> = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        if let Some((rig, pod)) = seat_of(&row.info.name).filter(|_| openrig) {
            rigs.entry(rig).or_default().entry(pod).or_default().push(index);
        } else if let Some(root) = root_of(&row.info.folder, roots) {
            folder_rows[root].push(index);
        } else {
            other_rows.push(index);
        }
    }
    let mut groups: Vec<TerminalGroup> = roots
        .iter()
        .zip(folder_rows)
        .enumerate()
        .filter(|(_, (_, rows))| !rows.is_empty())
        .map(|(index, ((name, path), rows))| TerminalGroup {
            key: format!("folder:{}", path.display()),
            title: (*name).to_string(),
            root: Some(index),
            rows,
            children: Vec::new(),
        })
        .collect();
    if !other_rows.is_empty() {
        groups.push(plain_group("other".to_string(), OTHER_TITLE, other_rows));
    }
    let rigs = rigs.into_iter().map(|(rig, pods)| rig_group(rig, pods)).collect();
    SessionGroups { terminals: groups, rigs }
}

/// The index of the deepest folder in `roots` that holds `folder`.
fn root_of(folder: &Path, roots: &[(&str, &Path)]) -> Option<usize> {
    roots
        .iter()
        .enumerate()
        .filter(|(_, (_, root))| folder.starts_with(root))
        .max_by_key(|(_, (_, root))| root.components().count())
        .map(|(index, _)| index)
}

/// A rig's group, with one sub-group per pod.
fn rig_group(rig: &str, pods: BTreeMap<&str, Vec<usize>>) -> TerminalGroup {
    let key = format!("rig:{rig}");
    let children = pods.into_iter().map(|(pod, rows)| plain_group(format!("{key}/{pod}"), pod, rows)).collect();
    TerminalGroup { key, title: rig.to_string(), root: None, rows: Vec::new(), children }
}

/// A group of sessions without a folder color or sub-groups.
fn plain_group(key: String, title: &str, rows: Vec<usize>) -> TerminalGroup {
    TerminalGroup { key, title: title.to_string(), root: None, rows, children: Vec::new() }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::terminal::status::TerminalStatus;
    use crate::terminal::tmux::SessionInfo;

    fn row(name: &str, folder: &str) -> TerminalRow {
        let info = SessionInfo {
            name: name.to_string(),
            created: 0,
            last_output: 0,
            command: "zsh".to_string(),
            folder: PathBuf::from(folder),
        };
        TerminalRow { info, status: TerminalStatus::Shell }
    }

    /// Each group as its path of titles (`rig/pod`) and the names of
    /// its rows, sub-groups after their parent.
    fn summary(groups: &[TerminalGroup], rows: &[TerminalRow]) -> Vec<(String, Vec<String>)> {
        let mut lines = Vec::new();
        for group in groups {
            add_lines(&mut lines, "", group, rows);
        }
        lines
    }

    fn add_lines(lines: &mut Vec<(String, Vec<String>)>, parent: &str, group: &TerminalGroup, rows: &[TerminalRow]) {
        let path = format!("{parent}{}", group.title);
        let names = group.rows.iter().map(|&index| rows[index].info.name.clone()).collect();
        lines.push((path.clone(), names));
        for child in &group.children {
            add_lines(lines, &format!("{path}/"), child, rows);
        }
    }

    #[test]
    fn plain_names_are_not_seats() {
        assert_eq!(seat_of("build"), None);
        assert_eq!(seat_of("notes agent"), None);
    }

    #[test]
    fn a_seat_names_its_rig_and_pod() {
        assert_eq!(seat_of("dev-impl@my-rig"), Some(("my-rig", "dev")));
        assert_eq!(seat_of("dev-impl-2@rig"), Some(("rig", "dev")));
    }

    /// A remote seat (`seat@rig@host`) goes under its rig and host, apart
    /// from the same rig on this computer.
    #[test]
    fn a_remote_seat_groups_by_rig_and_host() {
        assert_eq!(seat_of("dev-impl@my-rig@gpu-box"), Some(("my-rig@gpu-box", "dev")));
        assert_eq!(seat_of("lead@rig@me@gpu-box"), Some(("rig@me@gpu-box", "lead")));
    }

    #[test]
    fn a_seat_without_a_dash_is_its_own_pod() {
        assert_eq!(seat_of("lead@rig"), Some(("rig", "lead")));
    }

    #[test]
    fn empty_seat_or_rig_is_not_a_seat() {
        assert_eq!(seat_of("@rig"), None);
        assert_eq!(seat_of("dev-impl@"), None);
        assert_eq!(seat_of("@"), None);
    }

    #[test]
    fn the_deepest_folder_wins() {
        let rows = [row("outer", "/work"), row("inner", "/work/app/src")];
        let roots = [("work", Path::new("/work")), ("app", Path::new("/work/app"))];
        let groups = group_terminals(&rows, &roots, true).terminals;
        let expected =
            [("work".to_string(), vec!["outer".to_string()]), ("app".to_string(), vec!["inner".to_string()])];
        assert_eq!(summary(&groups, &rows), expected);
        assert_eq!(groups[1].root, Some(1));
        assert_eq!(groups[1].key, "folder:/work/app");
    }

    #[test]
    fn folders_then_other_and_rigs_with_pods_apart() {
        let rows = [
            row("orch-lead@zeta", "/work"),
            row("stray", "/tmp"),
            row("dev-qa@alpha", "/work"),
            row("build", "/work"),
            row("dev-impl@alpha", "/elsewhere"),
            row("orch-lead@alpha", "/work"),
        ];
        let roots = [("work", Path::new("/work"))];
        let groups = group_terminals(&rows, &roots, true);
        let lines = |expected: &[(&str, Vec<&str>)]| -> Vec<(String, Vec<String>)> {
            expected
                .iter()
                .map(|(title, names)| ((*title).to_string(), names.iter().map(ToString::to_string).collect()))
                .collect()
        };
        assert_eq!(summary(&groups.terminals, &rows), lines(&[("work", vec!["build"]), ("Other", vec!["stray"])]));
        assert_eq!(
            summary(&groups.rigs, &rows),
            lines(&[
                ("alpha", vec![]),
                ("alpha/dev", vec!["dev-qa@alpha", "dev-impl@alpha"]),
                ("alpha/orch", vec!["orch-lead@alpha"]),
                ("zeta", vec![]),
                ("zeta/orch", vec!["orch-lead@zeta"]),
            ])
        );
        assert_eq!(groups.rigs[0].session_count(), 3);
        assert_eq!(groups.rigs[1].children[0].key, "rig:zeta/orch");
    }

    #[test]
    fn order_inside_a_group_is_kept() {
        let rows = [row("c", "/work"), row("a", "/work"), row("b", "/work")];
        let roots = [("work", Path::new("/work"))];
        let groups = group_terminals(&rows, &roots, true).terminals;
        assert_eq!(groups[0].rows, [0, 1, 2]);
    }

    #[test]
    fn empty_groups_are_left_out() {
        let rows = [row("b", "/two")];
        let roots = [("one", Path::new("/one")), ("two", Path::new("/two"))];
        let groups = group_terminals(&rows, &roots, true).terminals;
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].title, "two");
        assert_eq!(group_terminals(&[], &roots, true), SessionGroups::default());
    }
}
