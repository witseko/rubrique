//! Images dropped on the page: copied into an `images` folder next to the
//! note and linked where the cursor is.

use std::path::{Path, PathBuf};

use eframe::egui::Context;

use super::RubriqueApp;
use crate::explorer::paths::name_text;
use crate::markdown::html::escape;
use crate::voice::{NoteKind, free_path};

/// The folder next to the note that dropped images go into.
const IMAGES_FOLDER: &str = "images";

impl RubriqueApp {
    /// Add an image file to the open note: copied into the `images` folder
    /// next to it (unless it is already there) and linked at the cursor.
    pub(super) fn add_image_file(&mut self, ctx: &Context, image: &Path) {
        if self.refuse_remote_note("images") || self.refuse_encrypted_note("images") {
            return;
        }
        let Some((note, _)) = self.memo_note() else {
            self.show_status("Open a Markdown or HTML file to add images".to_string());
            return;
        };
        let folder = note.parent().unwrap_or(Path::new("/")).join(IMAGES_FOLDER);
        let target = match copy_into(&folder, image) {
            Ok(target) => target,
            Err(error) => {
                self.show_status(format!("Cannot copy {}: {error}", name_text(image)));
                return;
            }
        };
        let name = image.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        if self.insert_link(ctx, &target, |kind, url| image_reference(kind, url, &name)).is_some() {
            self.refresh_explorer();
            self.show_status(format!("Added {} (save to keep the link)", name_text(&target)));
        }
    }
}

/// Markdown or HTML that shows the image at `url`, with `name` as its alt text.
fn image_reference(kind: NoteKind, url: &str, name: &str) -> String {
    match kind {
        NoteKind::Markdown => {
            let name: String =
                name.chars().map(|character| if matches!(character, '[' | ']') { '-' } else { character }).collect();
            format!("![{name}]({url})")
        }
        NoteKind::Html => format!("<img src=\"{url}\" alt=\"{}\">", escape(name)),
    }
}

/// Copy `file` into `folder` (created when missing) and return where it is
/// now. A file already in `folder`, or the same content under the same name,
/// is reused; a different file with that name gets a free one (`photo-2.png`).
pub(super) fn copy_into(folder: &Path, file: &Path) -> std::io::Result<PathBuf> {
    if file.parent() == Some(folder) {
        return Ok(file.to_path_buf());
    }
    std::fs::create_dir_all(folder)?;
    let name = name_text(file);
    let same = folder.join(&name);
    if same.is_file() && std::fs::read(&same)? == std::fs::read(file)? {
        return Ok(same);
    }
    let target = free_path(folder, &name, Path::exists);
    std::fs::copy(file, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    #[test]
    fn references_for_markdown_and_html() {
        assert_eq!(image_reference(NoteKind::Markdown, "images/a.png", "a"), "![a](images/a.png)");
        assert_eq!(image_reference(NoteKind::Markdown, "images/b%20c.png", "b [c]"), "![b -c-](images/b%20c.png)");
        assert_eq!(
            image_reference(NoteKind::Html, "images/tom.png", "Tom's \"cat\""),
            "<img src=\"images/tom.png\" alt=\"Tom&#39;s &quot;cat&quot;\">"
        );
    }

    #[test]
    fn copies_create_the_folder() {
        let temp = TempFolder::new();
        let photo = temp.write("downloads/photo.png", "one");
        let folder = temp.path().join("notes/images");
        let copied = copy_into(&folder, &photo).unwrap();
        assert_eq!(copied, folder.join("photo.png"));
        assert_eq!(std::fs::read(&copied).unwrap(), b"one");
    }

    #[test]
    fn same_content_is_reused_and_other_content_gets_a_free_name() {
        let temp = TempFolder::new();
        let folder = temp.path().join("images");
        temp.write("images/photo.png", "one");
        let same = temp.write("a/photo.png", "one");
        let other = temp.write("b/photo.png", "two");
        assert_eq!(copy_into(&folder, &same).unwrap(), folder.join("photo.png"));
        assert_eq!(copy_into(&folder, &other).unwrap(), folder.join("photo-2.png"));
        assert_eq!(std::fs::read(folder.join("photo-2.png")).unwrap(), b"two");
    }

    #[test]
    fn encrypted_scratches_take_no_images() {
        use crate::document::text_file::Document;
        let temp = TempFolder::new();
        let photo = temp.write("downloads/photo.png", "one");
        let ctx = Context::default();
        let mut app = RubriqueApp::new(&ctx, crate::settings::Settings::default(), vec![temp.path().into()], None);
        let vault = std::sync::Arc::new(crate::crypto::Vault::create("pw").0);
        let document = Document::missing(&temp.path().join("secret.md"), String::new(), String::new(), Some(vault));
        app.open_file =
            super::super::OpenFile::Text { document, preview: None, disk_notice: None, editor: None, source: false };
        app.add_image_file(&ctx, &photo);
        // No plain copy of the image lands next to the scratch.
        assert!(!temp.path().join(IMAGES_FOLDER).exists());
        let status = app.status_message.as_ref().map(|(message, _)| message.as_str());
        assert!(status.is_some_and(|message| message.contains("encrypted scratches")), "{status:?}");
    }

    #[test]
    fn files_already_in_the_folder_are_linked_in_place() {
        let temp = TempFolder::new();
        let inside = temp.write("images/photo.png", "one");
        assert_eq!(copy_into(&temp.path().join("images"), &inside).unwrap(), inside);
        assert_eq!(std::fs::read_dir(temp.path().join("images")).unwrap().count(), 1);
    }
}
