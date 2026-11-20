//! Groups of the explorer's Terminals list: a plain session belongs to the
//! explorer folder it runs in, and an `OpenRig` seat (a session named
//! `seat@rig`) to its rig, split into pods, with every rig under one
//! `OpenRig` group. Sessions outside every folder go under "Other".

use std::collections::BTreeMap;
use std::path::Path;

use super::monitor::TerminalRow;

/// Title of the group for sessions outside every explorer folder.
const OTHER_TITLE: &str = "Other";
/// Key and title of the group that holds every rig.
const OPENRIG_KEY: &str = "openrig";
const OPENRIG_TITLE: &str = "OpenRig";

/// One group of the Terminals list, as the explorer draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalGroup {
    /// What folding remembers the group by: `folder:<path>`, `other`,
    /// `openrig`, `rig:<rig>` or `rig:<rig>/<pod>`.
    pub key: String,
    /// The folder's name, "Other", `OpenRig`, the rig's name or the pod's
    /// name.
    pub title: String,
    /// The explorer folder's index, for the group color.
    pub root: Option<usize>,
    /// Indexes into the rows the groups were made from, in their order.
    pub rows: Vec<usize>,
    /// The rigs of the `OpenRig` group or the pods of a rig (their own `rows`
    /// stay empty); empty for other groups.
    pub children: Vec<TerminalGroup>,
}

impl TerminalGroup {
    /// How many sessions the group holds, its sub-groups' included.
    pub fn session_count(&self) -> usize {
        self.rows.len() + self.children.iter().map(TerminalGroup::session_count).sum::<usize>()
    }

    /// True for the group that holds every rig.
    pub fn is_openrig(&self) -> bool {
        self.key == OPENRIG_KEY
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

/// Sort `rows` into groups: one per explorer folder in `roots` (name and
/// path, in explorer order) that has sessions, then "Other", then `OpenRig` with
/// one group per rig by name and its pods by name. A session belongs to the deepest
/// folder it runs in; a seat's session always to its rig. Rows keep their
/// order inside a group, and groups without sessions are left out.
pub fn group_terminals(rows: &[TerminalRow], roots: &[(&str, &Path)]) -> Vec<TerminalGroup> {
    let mut folder_rows = vec![Vec::new(); roots.len()];
    let mut other_rows = Vec::new();
    let mut rigs: BTreeMap<&str, BTreeMap<&str, Vec<usize>>> = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        if let Some((rig, pod)) = seat_of(&row.info.name) {
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
    if !rigs.is_empty() {
        let rigs = rigs.into_iter().map(|(rig, pods)| rig_group(rig, pods)).collect();
        groups.push(TerminalGroup {
            key: OPENRIG_KEY.to_string(),
            title: OPENRIG_TITLE.to_string(),
            root: None,
            rows: Vec::new(),
            children: rigs,
        });
    }
    groups
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

    /// Each group as its path of titles (`OpenRig/rig/pod`) and the names of
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
        let groups = group_terminals(&rows, &roots);
        let expected =
            [("work".to_string(), vec!["outer".to_string()]), ("app".to_string(), vec!["inner".to_string()])];
        assert_eq!(summary(&groups, &rows), expected);
        assert_eq!(groups[1].root, Some(1));
        assert_eq!(groups[1].key, "folder:/work/app");
    }

    #[test]
    fn folders_then_other_then_rigs_with_pods() {
        let rows = [
            row("orch-lead@zeta", "/work"),
            row("stray", "/tmp"),
            row("dev-qa@alpha", "/work"),
            row("build", "/work"),
            row("dev-impl@alpha", "/elsewhere"),
            row("orch-lead@alpha", "/work"),
        ];
        let roots = [("work", Path::new("/work"))];
        let groups = group_terminals(&rows, &roots);
        let titles: Vec<(String, Vec<String>)> = summary(&groups, &rows);
        let expected = [
            ("work", vec!["build"]),
            ("Other", vec!["stray"]),
            ("OpenRig", vec![]),
            ("OpenRig/alpha", vec![]),
            ("OpenRig/alpha/dev", vec!["dev-qa@alpha", "dev-impl@alpha"]),
            ("OpenRig/alpha/orch", vec!["orch-lead@alpha"]),
            ("OpenRig/zeta", vec![]),
            ("OpenRig/zeta/orch", vec!["orch-lead@zeta"]),
        ];
        let expected: Vec<(String, Vec<String>)> = expected
            .iter()
            .map(|(title, names)| ((*title).to_string(), names.iter().map(ToString::to_string).collect()))
            .collect();
        assert_eq!(titles, expected);
        assert!(groups[2].is_openrig());
        assert_eq!(groups[2].session_count(), 4);
        assert_eq!(groups[2].children[0].session_count(), 3);
        assert_eq!(groups[2].children[1].children[0].key, "rig:zeta/orch");
    }

    #[test]
    fn order_inside_a_group_is_kept() {
        let rows = [row("c", "/work"), row("a", "/work"), row("b", "/work")];
        let roots = [("work", Path::new("/work"))];
        let groups = group_terminals(&rows, &roots);
        assert_eq!(groups[0].rows, [0, 1, 2]);
    }

    #[test]
    fn empty_groups_are_left_out() {
        let rows = [row("b", "/two")];
        let roots = [("one", Path::new("/one")), ("two", Path::new("/two"))];
        let groups = group_terminals(&rows, &roots);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].title, "two");
        assert_eq!(group_terminals(&[], &roots), []);
    }
}
