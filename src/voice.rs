//! Voice memos: audio files linked from Markdown and HTML notes. This module
//! names new memos, writes the link or `<audio>` tag, finds memo links in
//! the preview, builds the transcription command and tidies its output.
//!
//! Pure apart from [`VoiceSettings::find_program`] and
//! [`VoiceSettings::model_exists`], which look at the disk. Recording and
//! playback are in [`audio`].

use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use crate::date::LocalNow;
use crate::explorer::tree::is_markdown_path;
use crate::markdown::html::{escape, is_html_path};
use crate::markdown::inlines::Inline;

pub mod audio;
pub mod view;
pub mod wav;

/// Placeholder in the transcription command for the audio file's path.
pub const AUDIO: &str = "{audio}";

/// Built-in transcription command: whisper.cpp with the small model,
/// language detected, plain text on stdout.
pub const DEFAULT_COMMAND: &str = "whisper-cli -m ~/.local/share/whisper/ggml-small.bin -f {audio} -l auto -nt -np";

/// Where to download the built-in command's model.
pub const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin";

/// File types treated as audio.
const AUDIO_EXTENSIONS: [&str; 9] = ["wav", "m4a", "mp3", "ogg", "oga", "opus", "flac", "webm", "aac"];

/// Shown before a memo's label in Markdown links.
const PLAY_MARK: &str = "▶";

/// A paragraph longer than this is split at the next sentence end.
const PARAGRAPH_CHARS: usize = 400;

/// Has an audio extension.
pub fn is_audio_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| AUDIO_EXTENSIONS.iter().any(|known| extension.eq_ignore_ascii_case(known)))
}

/// A link target that points to an audio file (`?query` and `#part` ignored).
pub fn is_audio_url(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    is_audio_path(Path::new(path))
}

/// A WAV file, which Rubrique can play itself.
pub fn is_wav(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
}

/// The kind of file a memo is added to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKind {
    Markdown,
    Html,
}

impl NoteKind {
    pub fn of(path: &Path) -> Option<NoteKind> {
        if is_html_path(path) {
            Some(NoteKind::Html)
        } else if is_markdown_path(path) {
            Some(NoteKind::Markdown)
        } else {
            None
        }
    }
}

/// An audio link found in the preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoLink {
    /// The link text without the play mark.
    pub label: String,
    /// The link target as written in the file.
    pub url: String,
}

/// The memo in a paragraph that is only an audio link (notes and spaces
/// around it are fine).
pub fn memo_link(content: &[Inline]) -> Option<MemoLink> {
    let mut found = None;
    for inline in content {
        match inline {
            Inline::Link { content, url } if found.is_none() && is_audio_url(url) => {
                let mut label = String::new();
                plain_text(content, &mut label);
                let label = label.trim().trim_start_matches(PLAY_MARK).trim().to_string();
                found = Some(MemoLink { label, url: url.clone() });
            }
            Inline::Text(text) if text.trim().is_empty() => {}
            Inline::Note { .. } => {}
            _ => return None,
        }
    }
    found
}

/// The visible text of inlines.
fn plain_text(inlines: &[Inline], out: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Text(text) | Inline::Code(text) | Inline::Image { alt: text, .. } => out.push_str(text),
            Inline::Emphasis(children) | Inline::Strong(children) | Inline::Strikethrough(children) => {
                plain_text(children, out);
            }
            Inline::Link { content, .. } | Inline::Styled { content, .. } => plain_text(content, out),
            Inline::LineBreak => out.push(' '),
            Inline::Note { .. } => {}
        }
    }
}

/// File name of a memo recorded at `now`: `2026-10-28-1403.wav`.
pub fn memo_file_name(now: &LocalNow) -> String {
    format!("{}-{:02}{:02}.wav", now.date, now.hour, now.minute)
}

/// Link text of a memo recorded at `now`: `Voice memo 2026-10-28 14:03`.
pub fn memo_label(now: &LocalNow) -> String {
    format!("Voice memo {} {:02}:{:02}", now.date, now.hour, now.minute)
}

/// `folder/name`, or `name-2`, `name-3`… when `exists` says it is taken.
pub fn free_path(folder: &Path, name: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let first = folder.join(name);
    if !exists(&first) {
        return first;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    (2..10_000)
        .map(|number| folder.join(format!("{stem}-{number}{extension}")))
        .find(|candidate| !exists(candidate))
        .unwrap_or(first)
}

/// A relative URL from the folder `from` to the file `to`, with `../` where
/// needed and unsafe characters percent-encoded. Both must be absolute.
pub fn relative_url(from: &Path, to: &Path) -> String {
    let from: Vec<Component> = from.components().collect();
    let to_parts: Vec<Component> = to.components().collect();
    let shared = from.iter().zip(&to_parts).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_string(); from.len() - shared];
    parts.extend(to_parts[shared..].iter().map(|part| part.as_os_str().to_string_lossy().into_owned()));
    encode_path(&parts.join("/"))
}

/// Percent-encode everything but letters, digits, `-_.~` and `/`.
pub fn encode_path(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~/".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

/// Undo percent-encoding (`%20` becomes a space). Broken escapes stay as written.
pub fn decode_url(url: &str) -> String {
    let bytes = url.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|byte| char::from(*byte).to_digit(16));
        if bytes[index] == b'%'
            && let (Some(high), Some(low)) = (hex(index + 1), hex(index + 2))
        {
            decoded.push(u8::try_from(high * 16 + low).unwrap_or(b'?'));
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// The file a link points to, relative to the note's `folder`. Web links
/// give `None`; `file://` links and absolute paths are read as paths.
pub fn resolve_url(folder: &Path, url: &str) -> Option<PathBuf> {
    let url = url.split(['?', '#']).next().unwrap_or_default();
    if let Some(path) = url.strip_prefix("file://") {
        return Some(PathBuf::from(decode_url(path)));
    }
    if url.contains("://") || url.starts_with("mailto:") || url.is_empty() {
        return None;
    }
    let path = PathBuf::from(decode_url(url));
    Some(if path.is_absolute() { path } else { folder.join(path) })
}

/// The text that links a memo: a Markdown link, or an HTML `<audio>` tag.
/// `url` must already be encoded (see [`relative_url`]).
pub fn reference(kind: NoteKind, url: &str, label: &str) -> String {
    match kind {
        NoteKind::Markdown => {
            let label: String =
                label.chars().map(|character| if matches!(character, '[' | ']') { '-' } else { character }).collect();
            format!("[{PLAY_MARK} {label}]({url})")
        }
        NoteKind::Html => format!("<audio controls src=\"{url}\" title=\"{}\"></audio>", escape(label)),
    }
}

/// Put `block` into `text` at byte `at` as a block of its own (blank lines
/// around it in Markdown, its own line in HTML). Returns the new text and
/// the character index just after the block, for the cursor.
pub fn insert_block(text: &str, at: usize, block: &str, kind: NoteKind) -> (String, usize) {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let (before, after) = text.split_at(at);
    let needed = if kind == NoteKind::Markdown { 2 } else { 1 };
    let missing = |newlines: usize| "\n".repeat(needed - newlines.min(needed));
    let prefix =
        if before.is_empty() { String::new() } else { missing(before.len() - before.trim_end_matches('\n').len()) };
    let suffix =
        if after.is_empty() { "\n".to_string() } else { missing(after.len() - after.trim_start_matches('\n').len()) };
    let inserted = format!("{prefix}{block}");
    let cursor = before.chars().count() + inserted.chars().count();
    (format!("{before}{inserted}{suffix}{after}"), cursor)
}

/// `text` with `transcript` added right after the memo that links `url`, as
/// paragraphs. `None` when the memo is no longer in the text.
pub fn add_transcript(text: &str, kind: NoteKind, url: &str, transcript: &str) -> Option<String> {
    let needle = match kind {
        NoteKind::Markdown => format!("]({url})"),
        NoteKind::Html => format!("src=\"{url}\""),
    };
    let found = text.find(&needle)? + needle.len();
    let line_end = text[found..].find('\n').map_or(text.len(), |offset| found + offset);
    let paragraphs: Vec<&str> = transcript.split("\n\n").filter(|part| !part.trim().is_empty()).collect();
    let added = match kind {
        NoteKind::Markdown => paragraphs.iter().fold(String::new(), |mut out, part| {
            let _ = write!(out, "\n\n{}", part.trim());
            out
        }),
        NoteKind::Html => paragraphs.iter().fold(String::new(), |mut out, part| {
            let _ = write!(out, "\n<p>{}</p>", escape(part.trim()));
            out
        }),
    };
    Some(format!("{}{added}{}", &text[..line_end], &text[line_end..]))
}

/// A transcription tool's output as Markdown paragraphs: timestamps,
/// `[BLANK_AUDIO]`-style markers and extra space removed, lines joined, and
/// long paragraphs split at a sentence end.
pub fn clean_transcript(output: &str) -> String {
    let mut paragraphs = Vec::new();
    let mut current = String::new();
    let mut finish = |current: &mut String| {
        if !current.is_empty() {
            paragraphs.push(std::mem::take(current));
        }
    };
    for line in output.lines() {
        let mut line = line.trim();
        // `[00:00:00.000 --> 00:00:04.000]  text` from tools that print times.
        if line.starts_with('[')
            && let Some(close) = line.find(']')
            && line[..close].contains("-->")
        {
            line = line[close + 1..].trim();
        }
        if line.is_empty() {
            finish(&mut current);
            continue;
        }
        let marker = (line.starts_with('[') && line.ends_with(']')) || (line.starts_with('(') && line.ends_with(')'));
        if marker {
            continue;
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(line);
        if current.chars().count() >= PARAGRAPH_CHARS && line.ends_with(['.', '!', '?', '…']) {
            finish(&mut current);
        }
    }
    finish(&mut current);
    paragraphs.join("\n\n")
}

/// `~` or `~/…` with the home folder put in.
fn expand_home(word: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if word == "~" {
        home
    } else if let Some(rest) = word.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else {
        word.to_string()
    }
}

/// What is wrong with a transcription command, if anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandProblem {
    Empty,
    /// No `{audio}` word, so the tool would not get the file.
    NoAudio,
}

impl CommandProblem {
    pub fn message(self) -> &'static str {
        match self {
            CommandProblem::Empty => "Enter a command.",
            CommandProblem::NoAudio => "Add {audio} where the audio file goes.",
        }
    }
}

/// Voice memo settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceSettings {
    /// Where memos go: relative to the note's folder, or an absolute path.
    /// Empty means the note's own folder.
    pub folder: String,
    /// Copy dropped audio from other folders into the memos folder (else link it where it is).
    pub copy_dropped: bool,
    /// Transcribe right after recording.
    pub auto_transcribe: bool,
    /// Transcription command with `{audio}`.
    pub command: String,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        VoiceSettings {
            folder: "memos".to_string(),
            copy_dropped: true,
            auto_transcribe: true,
            command: DEFAULT_COMMAND.to_string(),
        }
    }
}

impl VoiceSettings {
    /// Read one settings line; returns false when the key is not a voice key.
    pub fn parse_line(&mut self, key: &str, value: &str) -> bool {
        match key {
            "voice_folder" => value.clone_into(&mut self.folder),
            "voice_copy_dropped" => self.copy_dropped = value != "false",
            "voice_auto_transcribe" => self.auto_transcribe = value != "false",
            "voice_command" if !value.trim().is_empty() => value.trim().clone_into(&mut self.command),
            "voice_command" => {}
            _ => return false,
        }
        true
    }

    /// Settings lines; the command only when changed.
    pub fn to_lines(&self) -> String {
        let mut lines = format!(
            "voice_folder={}\nvoice_copy_dropped={}\nvoice_auto_transcribe={}\n",
            self.folder, self.copy_dropped, self.auto_transcribe
        );
        if self.is_custom() {
            let _ = writeln!(lines, "voice_command={}", self.command.trim());
        }
        lines
    }

    pub fn is_custom(&self) -> bool {
        self.command.trim() != DEFAULT_COMMAND
    }

    pub fn reset_command(&mut self) {
        DEFAULT_COMMAND.clone_into(&mut self.command);
    }

    /// The memos folder for a note in `note_folder`.
    pub fn memos_folder(&self, note_folder: &Path) -> PathBuf {
        let folder = expand_home(self.folder.trim());
        let folder = Path::new(&folder);
        if folder.is_absolute() { folder.to_path_buf() } else { note_folder.join(folder) }
    }

    pub fn check_command(&self) -> Option<CommandProblem> {
        let words = crate::ai::split_command(&self.command);
        if words.is_empty() {
            Some(CommandProblem::Empty)
        } else if !words.iter().any(|word| word.contains(AUDIO)) {
            Some(CommandProblem::NoAudio)
        } else {
            None
        }
    }

    /// The program and its arguments for transcribing `audio`; `~` is the
    /// home folder. No shell is used.
    pub fn command_args(&self, audio: &Path) -> Option<(String, Vec<String>)> {
        let audio = audio.display().to_string();
        let mut words = crate::ai::split_command(&self.command).into_iter().map(|word| expand_home(&word));
        let program = words.next().filter(|program| !program.is_empty())?;
        Some((program, words.map(|word| word.replace(AUDIO, &audio)).collect()))
    }

    /// The program's first word, with `~` expanded.
    fn program(&self) -> Option<String> {
        crate::ai::split_command(&self.command).into_iter().next().map(|word| expand_home(&word))
    }

    /// The model file named after `-m` or `--model`, if any.
    pub fn model_path(&self) -> Option<PathBuf> {
        let words = crate::ai::split_command(&self.command);
        let at = words.iter().position(|word| word == "-m" || word == "--model")?;
        words.get(at + 1).map(|word| PathBuf::from(expand_home(word)))
    }

    /// Where the program is, if installed.
    pub fn find_program(&self) -> Option<PathBuf> {
        crate::ai::find_on_path(&self.program()?)
    }

    /// The command names a model file that is not there.
    pub fn model_missing(&self) -> bool {
        self.model_path().is_some_and(|model| !model.is_file())
    }

    /// The command to download the built-in command's model, when that is the one missing.
    pub fn model_download_command(&self) -> Option<String> {
        let model = self.model_path()?;
        let default = VoiceSettings::default().model_path()?;
        (model == default).then(|| format!("curl -L --create-dirs -o {} {MODEL_URL}", model.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> LocalNow {
        LocalNow { date: "2026-10-28".to_string(), hour: 9, minute: 5 }
    }

    #[test]
    fn audio_files_by_extension() {
        assert!(is_audio_path(Path::new("a/memo.WAV")));
        assert!(is_audio_path(Path::new("memo.m4a")));
        assert!(!is_audio_path(Path::new("notes.md")));
        assert!(is_audio_url("memos/a%20b.mp3?t=3#x"));
        assert!(!is_audio_url("https://example.com/"));
        assert!(is_wav(Path::new("x.wav")) && !is_wav(Path::new("x.mp3")));
    }

    #[test]
    fn names_and_labels_from_the_time() {
        assert_eq!(memo_file_name(&now()), "2026-10-28-0905.wav");
        assert_eq!(memo_label(&now()), "Voice memo 2026-10-28 09:05");
    }

    #[test]
    fn free_path_adds_a_number() {
        let folder = Path::new("/n/memos");
        let taken = |path: &Path| path.ends_with("a.wav") || path.ends_with("a-2.wav");
        assert_eq!(free_path(folder, "a.wav", taken), PathBuf::from("/n/memos/a-3.wav"));
        assert_eq!(free_path(folder, "b.wav", taken), PathBuf::from("/n/memos/b.wav"));
    }

    #[test]
    fn relative_urls() {
        let folder = Path::new("/home/a/notes");
        assert_eq!(relative_url(folder, Path::new("/home/a/notes/memos/x.wav")), "memos/x.wav");
        assert_eq!(relative_url(folder, Path::new("/home/a/music/my song.mp3")), "../music/my%20song.mp3");
        assert_eq!(relative_url(folder, Path::new("/home/a/notes/é.wav")), "%C3%A9.wav");
    }

    #[test]
    fn urls_resolve_against_the_note_folder() {
        let folder = Path::new("/n");
        assert_eq!(resolve_url(folder, "memos/my%20memo.wav"), Some(PathBuf::from("/n/memos/my memo.wav")));
        assert_eq!(resolve_url(folder, "/abs/a.wav"), Some(PathBuf::from("/abs/a.wav")));
        assert_eq!(resolve_url(folder, "file:///abs/b%20c.wav"), Some(PathBuf::from("/abs/b c.wav")));
        assert_eq!(resolve_url(folder, "https://x.io/a.mp3"), None);
        assert_eq!(decode_url("100%"), "100%");
    }

    #[test]
    fn references_for_markdown_and_html() {
        assert_eq!(reference(NoteKind::Markdown, "memos/a.wav", "Memo [1]"), "[▶ Memo -1-](memos/a.wav)");
        assert_eq!(
            reference(NoteKind::Html, "memos/a.wav", "Tom's memo"),
            "<audio controls src=\"memos/a.wav\" title=\"Tom&#39;s memo\"></audio>"
        );
        assert_eq!(NoteKind::of(Path::new("a.md")), Some(NoteKind::Markdown));
        assert_eq!(NoteKind::of(Path::new("a.html")), Some(NoteKind::Html));
        assert_eq!(NoteKind::of(Path::new("a.json")), None);
    }

    #[test]
    fn memo_links_are_found_in_paragraphs() {
        let link =
            |url: &str| Inline::Link { content: vec![Inline::Text("▶ Memo".to_string())], url: url.to_string() };
        assert_eq!(
            memo_link(&[link("memos/a.wav")]),
            Some(MemoLink { label: "Memo".to_string(), url: "memos/a.wav".to_string() })
        );
        assert_eq!(memo_link(&[link("a.md")]), None);
        assert_eq!(memo_link(&[Inline::Text("Listen: ".to_string()), link("a.wav")]), None);
        assert_eq!(memo_link(&[link("a.wav"), Inline::Text(" ".to_string())]).map(|m| m.url), Some("a.wav".into()));
    }

    #[test]
    fn blocks_get_blank_lines_in_markdown() {
        let md = NoteKind::Markdown;
        assert_eq!(insert_block("", 0, "X", md), ("X\n".to_string(), 1));
        assert_eq!(insert_block("Hello", 5, "X", md).0, "Hello\n\nX\n");
        assert_eq!(insert_block("Hello\n", 6, "X", md).0, "Hello\n\nX\n");
        assert_eq!(insert_block("A\n\nB", 3, "X", md).0, "A\n\nX\n\nB");
        assert_eq!(insert_block("AB", 1, "X", md), ("A\n\nX\n\nB".to_string(), 4));
        assert_eq!(insert_block("A\n\n\nB", 2, "X", md).0, "A\n\nX\n\nB");
    }

    #[test]
    fn blocks_get_their_own_line_in_html() {
        let html = NoteKind::Html;
        assert_eq!(insert_block("<p>a</p>\n<p>b</p>", 9, "X", html).0, "<p>a</p>\nX\n<p>b</p>");
        assert_eq!(insert_block("<p>a</p>", 3, "X", html).0, "<p>\nX\na</p>");
    }

    #[test]
    fn transcripts_go_under_the_memo() {
        let text = "# Day\n\n[▶ Memo](memos/a.wav)\n\nMore.\n";
        assert_eq!(
            add_transcript(text, NoteKind::Markdown, "memos/a.wav", "One.\n\nTwo."),
            Some("# Day\n\n[▶ Memo](memos/a.wav)\n\nOne.\n\nTwo.\n\nMore.\n".to_string())
        );
        assert_eq!(add_transcript(text, NoteKind::Markdown, "memos/b.wav", "x"), None);
        let html = "<audio controls src=\"a.wav\"></audio>\n</body>";
        assert_eq!(
            add_transcript(html, NoteKind::Html, "a.wav", "Fish & chips"),
            Some("<audio controls src=\"a.wav\"></audio>\n<p>Fish &amp; chips</p>\n</body>".to_string())
        );
    }

    #[test]
    fn whisper_output_is_tidied() {
        let output = "\n [BLANK_AUDIO]\n Hello there.\n  How are you?\n\n(music)\n New thought.\n";
        assert_eq!(clean_transcript(output), "Hello there. How are you?\n\nNew thought.");
        let timed = "[00:00:00.000 --> 00:00:02.000]   Buy milk.\n[00:00:02.000 --> 00:00:03.000]  Call Ann.";
        assert_eq!(clean_transcript(timed), "Buy milk. Call Ann.");
        let long = format!("{}.\nNext.", "word ".repeat(90).trim());
        assert_eq!(clean_transcript(&long).matches("\n\n").count(), 1);
    }

    #[test]
    fn settings_round_trip() {
        let mut settings = VoiceSettings::default();
        let mut parsed = VoiceSettings::default();
        for line in settings.to_lines().lines() {
            let (key, value) = line.split_once('=').unwrap();
            assert!(parsed.parse_line(key, value));
        }
        assert_eq!(parsed, settings);
        settings.folder = String::new();
        settings.copy_dropped = false;
        settings.command = "yap transcribe {audio}".to_string();
        let text = settings.to_lines();
        assert!(text.contains("voice_command=yap transcribe {audio}\n"));
        let mut parsed = VoiceSettings::default();
        for line in text.lines() {
            let (key, value) = line.split_once('=').unwrap();
            parsed.parse_line(key, value);
        }
        assert_eq!(parsed, settings);
        assert!(!parsed.parse_line("theme", "dark"));
    }

    #[test]
    fn commands_get_the_audio_path() {
        let mut settings = VoiceSettings { command: "tool --in={audio} \"a b\"".to_string(), ..Default::default() };
        let (program, args) = settings.command_args(Path::new("/n/my memo.wav")).unwrap();
        assert_eq!(program, "tool");
        assert_eq!(args, ["--in=/n/my memo.wav", "a b"]);
        assert_eq!(settings.check_command(), None);
        settings.command = "tool".to_string();
        assert_eq!(settings.check_command(), Some(CommandProblem::NoAudio));
        settings.command = "  ".to_string();
        assert_eq!(settings.check_command(), Some(CommandProblem::Empty));
    }

    #[test]
    fn the_default_model_path_has_a_download_command() {
        let settings = VoiceSettings::default();
        let home = std::env::var("HOME").unwrap_or_default();
        assert_eq!(settings.model_path(), Some(PathBuf::from(format!("{home}/.local/share/whisper/ggml-small.bin"))));
        assert!(settings.model_download_command().unwrap().ends_with(MODEL_URL));
        let other = VoiceSettings { command: "whisper-cli -m /m.bin -f {audio}".to_string(), ..Default::default() };
        assert_eq!(other.model_download_command(), None);
    }

    #[test]
    fn memos_folder_is_relative_to_the_note() {
        let mut settings = VoiceSettings::default();
        assert_eq!(settings.memos_folder(Path::new("/n")), PathBuf::from("/n/memos"));
        settings.folder = "/all/memos".to_string();
        assert_eq!(settings.memos_folder(Path::new("/n")), PathBuf::from("/all/memos"));
        settings.folder = String::new();
        assert_eq!(settings.memos_folder(Path::new("/n")), PathBuf::from("/n"));
    }
}
