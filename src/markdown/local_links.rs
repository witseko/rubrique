//! Links from a Markdown file to another Markdown file, which the preview
//! opens in a tab.
//!
//! A relative link starts at the open file's folder. A link that starts
//! with `/` starts at the root of the Open Knowledge Format bundle the file
//! is in (OKF writes links between concepts that way, such as
//! `/tables/orders.md`).
//!
//! Links and local images never reach outside the preview root: the
//! explorer folder that holds the open file, else its bundle root, else the
//! file's own folder (see [`preview_root`]).

use std::io::Read as _;
use std::path::{Component, Path, PathBuf};

use super::front_matter::FrontMatter;

/// Bytes of an `index.md` read to look for `okf_version` in its front matter.
const INDEX_HEAD_BYTES: u64 = 4096;

/// The Markdown file `url` points at, or `None` when it is not a link to a
/// local `.md` file (a web address, another kind of file, a `/` link
/// outside a bundle, or a file outside `root`). `folder` is the open file's
/// folder and `root` its [`preview_root`].
pub fn link_target(url: &str, folder: &Path, bundle_root: Option<&Path>, root: &Path) -> Option<PathBuf> {
    let path = url.split(['#', '?']).next().unwrap_or(url);
    // A scheme (`https:`, `file:`) is not a local link.
    if path.is_empty() || path.contains(':') {
        return None;
    }
    let is_markdown = Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown"));
    if !is_markdown {
        return None;
    }
    let joined = match path.strip_prefix('/') {
        Some(inside) => bundle_root?.join(inside),
        None => folder.join(path),
    };
    inside(&joined, root)
}

/// The folder the preview may read links and images from for a file in
/// `folder`: the deepest of `explorer_folders` that holds it, else
/// `bundle_root`, else `folder` itself.
pub fn preview_root(folder: &Path, explorer_folders: &[PathBuf], bundle_root: Option<&Path>) -> PathBuf {
    let explorer_folder =
        explorer_folders.iter().filter(|root| folder.starts_with(root)).max_by_key(|root| root.components().count());
    explorer_folder.map(PathBuf::as_path).or(bundle_root).unwrap_or(folder).to_path_buf()
}

/// `path` with `.` and `..` worked out, when that is inside `root`.
pub fn inside(path: &Path, root: &Path) -> Option<PathBuf> {
    let path = normalized(path);
    path.starts_with(normalized(root)).then_some(path)
}

/// The root folder of the OKF bundle that holds `folder`: the nearest folder
/// up the tree whose `index.md` declares `okf_version`, or else the highest
/// folder of the unbroken run of folders with an `index.md` from `folder`
/// up. `None` when `folder` has no `index.md`.
pub fn bundle_root(folder: &Path) -> Option<PathBuf> {
    let mut root = None;
    for dir in folder.ancestors() {
        let Some(head) = read_head(&dir.join("index.md")) else {
            break;
        };
        let lines: Vec<&str> = head.lines().collect();
        if FrontMatter::parse(&lines).is_some_and(|(front, _)| front.get("okf_version").is_some()) {
            return Some(dir.to_path_buf());
        }
        root = Some(dir.to_path_buf());
    }
    root
}

/// The start of the file at `path`, when it is a readable file.
fn read_head(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(INDEX_HEAD_BYTES).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// `path` with `.` and `..` worked out, so one file never opens in two tabs
/// under two spellings.
fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_links_start_at_the_files_folder() {
        let folder = Path::new("/b/metrics");
        let root = Path::new("/b");
        assert_eq!(link_target("revenue.md", folder, None, root), Some(PathBuf::from("/b/metrics/revenue.md")));
        assert_eq!(
            link_target("../computations/revenue.md#definition", folder, None, root),
            Some(PathBuf::from("/b/computations/revenue.md"))
        );
        assert_eq!(
            link_target("./Notes.MARKDOWN", folder, None, root),
            Some(PathBuf::from("/b/metrics/Notes.MARKDOWN"))
        );
    }

    #[test]
    fn slash_links_start_at_the_bundle_root() {
        let folder = Path::new("/b/metrics");
        let root = Path::new("/b");
        assert_eq!(
            link_target("/tables/orders.md", folder, Some(root), root),
            Some(PathBuf::from("/b/tables/orders.md"))
        );
        assert_eq!(link_target("/tables/orders.md", folder, None, root), None);
    }

    #[test]
    fn links_never_leave_the_preview_root() {
        let folder = Path::new("/notes/work");
        let root = Path::new("/notes");
        assert_eq!(link_target("../home.md", folder, None, root), Some(PathBuf::from("/notes/home.md")));
        assert_eq!(link_target("../../etc/secret.md", folder, None, root), None);
        assert_eq!(link_target("/../../outside.md", folder, Some(root), root), None);
    }

    #[test]
    fn the_preview_root_is_the_explorer_folder_then_the_bundle_then_the_file_folder() {
        let folder = Path::new("/home/a/notes/work/okf");
        let explorer = [PathBuf::from("/home/a/notes"), PathBuf::from("/home/a/notes/work"), PathBuf::from("/srv")];
        let bundle = Some(Path::new("/home/a/notes/work/okf"));
        assert_eq!(preview_root(folder, &explorer, bundle), PathBuf::from("/home/a/notes/work"));
        assert_eq!(preview_root(folder, &[], bundle), PathBuf::from("/home/a/notes/work/okf"));
        assert_eq!(preview_root(folder, &explorer[2..], None), folder);
    }

    #[test]
    fn inside_works_out_dots_and_keeps_to_the_root() {
        let root = Path::new("/notes");
        assert_eq!(inside(Path::new("/notes/a/../img.png"), root), Some(PathBuf::from("/notes/img.png")));
        assert_eq!(inside(Path::new("/notes/../etc/passwd"), root), None);
        assert_eq!(inside(Path::new("/notesbook/img.png"), root), None);
    }

    #[test]
    fn only_local_markdown_files_are_links() {
        let folder = Path::new("/b");
        assert_eq!(link_target("https://a.io/x.md", folder, None, folder), None);
        assert_eq!(link_target("file:///b/x.md", folder, None, folder), None);
        assert_eq!(link_target("script.py", folder, None, folder), None);
        assert_eq!(link_target("subdir/", folder, None, folder), None);
        assert_eq!(link_target("#heading", folder, None, folder), None);
    }

    #[test]
    fn bundle_root_is_the_folder_that_declares_okf_or_the_top_of_the_index_run() {
        let temp = crate::test_support::TempFolder::new();
        let top = temp.path();
        let tables = temp.write("bundle/tables/index.md", "# Tables\n").parent().unwrap().to_path_buf();
        let bundle = temp.write("bundle/index.md", "# Bundle\n").parent().unwrap().to_path_buf();
        assert_eq!(bundle_root(&tables), Some(bundle.clone()));
        assert_eq!(bundle_root(top), None);

        // The declared root wins over an index higher up.
        std::fs::write(top.join("index.md"), "# Repo\n").unwrap();
        assert_eq!(bundle_root(&tables), Some(top.to_path_buf()));
        std::fs::write(bundle.join("index.md"), "---\nokf_version: \"0.2\"\n---\n# Bundle\n").unwrap();
        assert_eq!(bundle_root(&tables), Some(bundle));
    }
}
