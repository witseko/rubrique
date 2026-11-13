//! File tree model: lazy loading, sorting, Markdown filter, symlink loops.
//!
//! No egui here; `view.rs` draws this model.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

use crate::remote::task::Task;

/// Extensions treated as Markdown (compared case-insensitively).
const MARKDOWN_EXTENSIONS: [&str; 5] = ["md", "markdown", "mdown", "mkd", "mdx"];

/// Extension-less file names that are usually Markdown.
const MARKDOWN_BARE_NAMES: [&str; 4] = ["README", "CHANGELOG", "LICENSE", "CONTRIBUTING"];

/// True when a file should get the Markdown preview and pass the
/// "Markdown only" filter.
pub fn is_markdown_path(path: &Path) -> bool {
    if let Some(extension) = path.extension() {
        let extension = extension.to_string_lossy();
        return MARKDOWN_EXTENSIONS.iter().any(|known| extension.eq_ignore_ascii_case(known));
    }
    let name = path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
    MARKDOWN_BARE_NAMES.iter().any(|known| name.eq_ignore_ascii_case(known))
}

/// The name shown for a root folder: the folder's name, and the host for a
/// remote folder ("notes on server").
pub fn root_name(path: &Path) -> String {
    let name = super::paths::name_text(path);
    match crate::remote::from_mirror(path) {
        Some(remote) => format!("{name} on {}", remote.host),
        None => name,
    }
}

/// Children of a folder, once someone expanded it.
#[derive(Debug)]
pub enum Children {
    /// Never expanded; nothing read from disk yet.
    NotLoaded,
    /// A remote folder being listed in the background (see
    /// [`FsNode::start_loading`]).
    Loading(Task<std::io::Result<Vec<FsNode>>>),
    /// Entries read from disk, sorted for display.
    Loaded(Vec<FsNode>),
    /// Reading the folder failed (permissions, deleted, ...).
    Failed(String),
}

/// What kind of entry a node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// A file, or a broken symlink (it has no target to tell otherwise).
    File,
    /// A folder, or a symlink to one.
    Folder,
    /// A symlinked folder pointing at itself or one of its ancestors.
    /// Shown, but never expanded, so the tree cannot recurse forever.
    SymlinkLoop,
}

/// One file or folder in the tree.
#[derive(Debug)]
pub struct FsNode {
    /// Path as listed under the root (symlinks are not resolved).
    pub path: PathBuf,
    /// File name shown in the tree.
    pub name: String,
    /// File, folder, or looping folder link.
    pub kind: NodeKind,
    /// The entry itself is a symlink (shown with a link marker).
    pub is_symlink: bool,
    /// Folder is open in the tree.
    pub expanded: bool,
    /// Entries of a folder, loaded on first expand.
    pub children: Children,
    /// The file starts with Rubrique's encryption header (an encrypted
    /// scratch), read when the folder is listed. Never set for folders, nor
    /// for files on other computers (their bytes are not here).
    pub encrypted: bool,
}

impl FsNode {
    /// Root node for a folder; starts expanded with its children loaded.
    pub fn new_root(path: PathBuf) -> Self {
        let name = root_name(&path);
        let mut root = FsNode {
            path,
            name,
            kind: NodeKind::Folder,
            is_symlink: false,
            expanded: true,
            children: Children::NotLoaded,
            encrypted: false,
        };
        root.load_children();
        root
    }

    /// True for folders, including looping folder links.
    pub fn is_folder(&self) -> bool {
        matches!(self.kind, NodeKind::Folder | NodeKind::SymlinkLoop)
    }

    /// Expand or collapse a folder, reading it on first expand (see
    /// [`FsNode::start_loading`]; `wake` is called when a remote listing
    /// arrives). Folding it again while it loads starts nothing new.
    pub fn toggle_expanded(&mut self, wake: impl FnOnce() + Send + 'static) {
        if self.kind != NodeKind::Folder {
            return;
        }
        self.expanded = !self.expanded;
        if self.expanded && matches!(self.children, Children::NotLoaded) {
            self.start_loading(wake);
        }
    }

    /// Read this folder's entries from disk, replacing any loaded children.
    pub fn load_children(&mut self) {
        self.children = children_from(read_folder(&self.path));
    }

    /// Read this folder's entries: a local folder at once, a remote one in
    /// the background, so a slow host does not freeze the window. `wake` is
    /// called when the listing arrives; [`FsNode::poll_loading`] then puts it in.
    pub fn start_loading(&mut self, wake: impl FnOnce() + Send + 'static) {
        if !crate::remote::is_mirror(&self.path) {
            self.load_children();
            return;
        }
        let path = self.path.clone();
        self.children = Children::Loading(Task::spawn(wake, move || read_folder(&path)));
    }

    /// Put in the entries of a finished background listing.
    pub fn poll_loading(&mut self) {
        let Children::Loading(task) = &self.children else {
            return;
        };
        if let Some(result) = task.take() {
            self.children = children_from(result);
        }
    }

    /// Open every folder from this one down to `target`, so its row shows,
    /// reading folders from disk on the way. Returns true when `target` is
    /// this node or its parent folder was found inside it.
    pub fn reveal(&mut self, target: &Path) -> bool {
        self.path == target || target.parent().is_some_and(|parent| self.open_folder(parent))
    }

    /// Open `folder` and every folder above it from this one, reading them
    /// from disk on the way. Returns true when `folder` was found. When it
    /// is missing, the folders that exist on the way stay open.
    pub fn open_folder(&mut self, folder: &Path) -> bool {
        if self.kind != NodeKind::Folder || !folder.starts_with(&self.path) {
            return false;
        }
        if matches!(self.children, Children::NotLoaded) {
            self.load_children();
        }
        self.expanded = true;
        if self.path == folder {
            return true;
        }
        let Children::Loaded(children) = &mut self.children else {
            return false;
        };
        children.iter_mut().any(|child| child.open_folder(folder))
    }

    /// Re-read every loaded folder, keeping which folders are expanded.
    pub fn refresh(&mut self) {
        // A listing under way is as fresh as a new one would be.
        if matches!(self.children, Children::NotLoaded | Children::Loading(_)) {
            return;
        }
        let previous = std::mem::replace(&mut self.children, Children::NotLoaded);
        self.load_children();
        let (Children::Loaded(mut old_children), Children::Loaded(new_children)) = (previous, &mut self.children)
        else {
            return;
        };
        // Carry over each folder's previous state, then refresh it in turn.
        for new_child in new_children.iter_mut().filter(|child| child.kind == NodeKind::Folder) {
            let Some(index) =
                old_children.iter().position(|old| old.path == new_child.path && old.kind == NodeKind::Folder)
            else {
                continue;
            };
            let old_child = old_children.swap_remove(index);
            new_child.expanded = old_child.expanded;
            new_child.children = old_child.children;
            new_child.refresh();
        }
    }
}

/// A folder's children from the result of reading it.
fn children_from(result: std::io::Result<Vec<FsNode>>) -> Children {
    match result {
        Ok(children) => Children::Loaded(children),
        Err(error) => Children::Failed(error.to_string()),
    }
}

/// `target` spelled the way the tree under `root` spells paths, when it is
/// inside `root`. Programs like tmux report the resolved path
/// (`/private/tmp/x` on macOS), while a root may go through a link
/// (`/tmp/x`), so the resolved root is tried too.
pub fn path_under_root(root: &Path, target: &Path) -> Option<PathBuf> {
    if target.starts_with(root) {
        return Some(target.to_path_buf());
    }
    let resolved_root = fs::canonicalize(root).ok()?;
    let inside = target.strip_prefix(&resolved_root).ok()?;
    Some(root.join(inside))
}

/// Read one folder: every entry, folders first, then case-insensitive by name.
///
/// # Errors
///
/// Fails when the folder cannot be listed (missing, permissions, ...).
pub fn read_folder(folder: &Path) -> std::io::Result<Vec<FsNode>> {
    if let Some(remote) = crate::remote::from_mirror(folder) {
        return read_remote_folder(folder, &remote);
    }
    // Canonical path of the folder being read, used for symlink loop checks.
    let canonical_folder = fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
    let mut nodes = Vec::new();
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        nodes.push(node_from_entry(&entry, &canonical_folder));
    }
    nodes.sort_by(compare_nodes);
    Ok(nodes)
}

/// Read a folder on a host. Entries get mirror paths under `folder`.
fn read_remote_folder(folder: &Path, remote: &crate::remote::RemotePath) -> std::io::Result<Vec<FsNode>> {
    let entries = crate::remote::commands::list(&remote.host, &remote.path)?;
    // The mirror folder is where agents run and images are kept.
    let _ = fs::create_dir_all(folder);
    let mut nodes: Vec<FsNode> = entries
        .into_iter()
        .map(|entry| FsNode {
            path: folder.join(&entry.name),
            kind: if entry.is_loop {
                NodeKind::SymlinkLoop
            } else if entry.is_folder {
                NodeKind::Folder
            } else {
                NodeKind::File
            },
            is_symlink: entry.is_symlink,
            name: entry.name,
            expanded: false,
            children: Children::NotLoaded,
            encrypted: false,
        })
        .collect();
    nodes.sort_by(compare_nodes);
    Ok(nodes)
}

/// Build an unexpanded node for one directory entry.
fn node_from_entry(entry: &fs::DirEntry, canonical_parent: &Path) -> FsNode {
    let path = entry.path();
    let is_symlink = entry.file_type().is_ok_and(|file_type| file_type.is_symlink());
    // `metadata` follows symlinks, so a link to a folder counts as a folder.
    // A broken link has no metadata and is shown as a file.
    let is_folder = fs::metadata(&path).is_ok_and(|metadata| metadata.is_dir());
    let kind = if !is_folder {
        NodeKind::File
    } else if is_symlink && points_to_ancestor(&path, canonical_parent) {
        NodeKind::SymlinkLoop
    } else {
        NodeKind::Folder
    };
    // Only the first bytes are read, once per listing.
    let encrypted = kind == NodeKind::File && crate::crypto::file_is_encrypted(&path);
    FsNode {
        name: entry.file_name().to_string_lossy().into_owned(),
        path,
        kind,
        is_symlink,
        expanded: false,
        children: Children::NotLoaded,
        encrypted,
    }
}

/// True when the symlink's target is the parent folder or one of its
/// ancestors. Expanding such a link would show the same tree again forever.
fn points_to_ancestor(link: &Path, canonical_parent: &Path) -> bool {
    match fs::canonicalize(link) {
        Ok(target) => canonical_parent.starts_with(&target),
        Err(_) => false,
    }
}

/// Display order: folders (including looping links) first, then by name
/// ignoring case, with an exact-case tie-break so the order is stable.
fn compare_nodes(left: &FsNode, right: &FsNode) -> Ordering {
    right
        .is_folder()
        .cmp(&left.is_folder())
        .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        .then_with(|| left.name.cmp(&right.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    fn names(nodes: &[FsNode]) -> Vec<&str> {
        nodes.iter().map(|node| node.name.as_str()).collect()
    }

    fn loaded(node: &FsNode) -> &[FsNode] {
        match &node.children {
            Children::Loaded(children) => children,
            other => panic!("expected loaded children, got {other:?}"),
        }
    }

    #[test]
    fn reveal_opens_the_folders_down_to_a_file() {
        let temp = TempFolder::new();
        let target = temp.write("a/b/c.tmux", "");
        temp.write("a/other.md", "");
        let mut root = FsNode::new_root(temp.path().to_path_buf());
        assert!(root.reveal(&target));
        let a = &loaded(&root)[0];
        assert!(a.expanded);
        let b = &loaded(a)[0];
        assert!(b.expanded);
        assert_eq!(names(loaded(b)), ["c.tmux"]);
    }

    #[test]
    fn reveal_reports_a_path_outside_the_root() {
        let temp = TempFolder::new();
        let other = TempFolder::new();
        let mut root = FsNode::new_root(temp.path().to_path_buf());
        assert!(!root.reveal(&other.path().join("x.md")));
    }

    #[test]
    fn resolved_paths_are_mapped_under_a_linked_root() {
        let temp = TempFolder::new();
        let real = temp.make_folder("real");
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let inside = real.join("docs");
        assert_eq!(path_under_root(&link, &inside), Some(link.join("docs")));
        assert_eq!(path_under_root(&real, &inside), Some(inside.clone()));
        assert_eq!(path_under_root(&link, temp.path()), None);
    }

    #[test]
    fn markdown_detection() {
        assert!(is_markdown_path(Path::new("a/notes.md")));
        assert!(is_markdown_path(Path::new("a/NOTES.MarkDown")));
        assert!(is_markdown_path(Path::new("README")));
        assert!(is_markdown_path(Path::new("x/license")));
        assert!(!is_markdown_path(Path::new("main.rs")));
        assert!(!is_markdown_path(Path::new("Makefile")));
        assert!(!is_markdown_path(Path::new(".gitignore")));
    }

    #[test]
    fn lists_all_files_folders_first_case_insensitive() {
        let temp = TempFolder::new();
        temp.write("b.md", "");
        temp.write("A.txt", "");
        temp.write(".hidden", "");
        temp.write("Makefile", "");
        temp.make_folder("zeta");
        temp.make_folder("Alpha");

        let nodes = read_folder(temp.path()).unwrap();
        assert_eq!(names(&nodes), ["Alpha", "zeta", ".hidden", "A.txt", "b.md", "Makefile"]);
    }

    #[test]
    fn folders_load_lazily() {
        let temp = TempFolder::new();
        temp.write("docs/inner.md", "");
        let mut root = FsNode::new_root(temp.path().to_path_buf());
        let docs = &mut match &mut root.children {
            Children::Loaded(children) => children,
            _ => unreachable!(),
        }[0];
        assert!(matches!(docs.children, Children::NotLoaded));
        docs.toggle_expanded(|| {});
        assert_eq!(names(loaded(docs)), ["inner.md"]);
    }

    #[test]
    fn marks_encrypted_files() {
        let temp = TempFolder::new();
        temp.write("plain.md", "words");
        let vault = crate::crypto::Vault::create("password").0;
        fs::write(temp.path().join("secret.md"), vault.encrypt(b"words")).unwrap();
        let root = FsNode::new_root(temp.path().to_path_buf());
        let marks: Vec<(&str, bool)> = loaded(&root).iter().map(|node| (node.name.as_str(), node.encrypted)).collect();
        assert_eq!(marks, vec![("plain.md", false), ("secret.md", true)]);
    }

    #[test]
    fn refresh_picks_up_new_files_and_keeps_expansion() {
        let temp = TempFolder::new();
        temp.write("docs/one.md", "");
        let mut root = FsNode::new_root(temp.path().to_path_buf());
        if let Children::Loaded(children) = &mut root.children {
            children[0].toggle_expanded(|| {});
        }
        temp.write("docs/two.md", "");
        temp.write("top.md", "");
        root.refresh();

        assert_eq!(names(loaded(&root)), ["docs", "top.md"]);
        let docs = &loaded(&root)[0];
        assert!(docs.expanded);
        assert_eq!(names(loaded(docs)), ["one.md", "two.md"]);
    }

    #[test]
    fn unreadable_folder_reports_error() {
        let mut node = FsNode::new_root(PathBuf::from("/definitely/not/here"));
        assert!(matches!(node.children, Children::Failed(_)));
        node.refresh();
        assert!(matches!(node.children, Children::Failed(_)));
    }

    #[test]
    fn remote_folders_list_like_local_ones() {
        let temp = TempFolder::new();
        temp.write("b.md", "");
        temp.write("A.txt", "");
        temp.make_folder("zeta");
        temp.write("docs/inner.md", "");
        let mirror = crate::remote::testing::mirror_of("tree-host", temp.path());
        let root = FsNode::new_root(mirror.clone());
        assert!(root.name.ends_with(" on tree-host"), "{}", root.name);
        assert_eq!(names(loaded(&root)), ["docs", "zeta", "A.txt", "b.md"]);
        assert_eq!(loaded(&root)[3].path, mirror.join("b.md"));
        assert!(mirror.is_dir());
    }

    /// Put in a background listing of `node`, waiting for it (failing after
    /// ten seconds).
    fn finish_loading(node: &mut FsNode) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while matches!(node.children, Children::Loading(_)) {
            assert!(std::time::Instant::now() < deadline, "the listing did not arrive");
            std::thread::yield_now();
            node.poll_loading();
        }
    }

    #[test]
    fn remote_folders_expand_in_the_background_once() {
        let temp = TempFolder::new();
        temp.write("docs/inner.md", "");
        temp.make_folder("gone");
        let mirror = crate::remote::testing::mirror_of("tree-host", temp.path());
        let mut root = FsNode::new_root(mirror);
        let Children::Loaded(children) = &mut root.children else {
            panic!("the root is listed");
        };
        // Every listing started calls back once on this channel.
        let (woken, wakes) = std::sync::mpsc::channel();
        let wake = || {
            let woken = woken.clone();
            move || {
                let _ = woken.send(());
            }
        };
        children[0].toggle_expanded(wake());
        assert!(matches!(children[0].children, Children::Loading(_)), "the window does not wait");
        // Folding and unfolding while it loads starts no second listing.
        children[0].toggle_expanded(wake());
        children[0].toggle_expanded(wake());
        assert!(matches!(children[0].children, Children::Loading(_)));
        // Nor does a refresh; the listing under way is kept.
        temp.write("docs/new.md", "");
        root.refresh();
        let docs = &mut match &mut root.children {
            Children::Loaded(children) => children,
            _ => unreachable!(),
        }[0];
        assert!(matches!(docs.children, Children::Loading(_)));
        finish_loading(docs);
        wakes.recv().unwrap();
        assert_eq!(names(loaded(docs)), ["inner.md", "new.md"]);

        // A folder gone from the host shows why, as before.
        let gone = &mut match &mut root.children {
            Children::Loaded(children) => children,
            _ => unreachable!(),
        }[1];
        std::fs::remove_dir(temp.path().join("gone")).unwrap();
        gone.toggle_expanded(|| {});
        finish_loading(gone);
        assert!(matches!(gone.children, Children::Failed(_)));
        assert_eq!(wakes.try_iter().count(), 0, "only one listing of docs ran");
    }

    #[test]
    fn symlinked_folder_is_followed_and_loop_detected() {
        let temp = TempFolder::new();
        temp.write("real/file.md", "");
        temp.make_folder("inner");
        std::os::unix::fs::symlink(temp.path().join("real"), temp.path().join("link")).unwrap();
        std::os::unix::fs::symlink(temp.path(), temp.path().join("inner/back_to_root")).unwrap();

        let nodes = read_folder(temp.path()).unwrap();
        let link = nodes.iter().find(|node| node.name == "link").unwrap();
        assert!(link.kind == NodeKind::Folder && link.is_symlink);

        let inner = read_folder(&temp.path().join("inner")).unwrap();
        assert_eq!(inner[0].kind, NodeKind::SymlinkLoop);
        let mut looping = FsNode { expanded: false, ..inner.into_iter().next().unwrap() };
        looping.toggle_expanded(|| {});
        assert!(!looping.expanded);
    }
}
