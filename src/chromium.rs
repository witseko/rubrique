//! Draw an HTML page with a Chromium browser (Chrome, Chromium, Edge, Brave,
//! or Google's chrome-headless-shell) for the HTML preview.
//!
//! The browser runs headless with a fresh profile and talks the `DevTools`
//! protocol over a pipe (`--remote-debugging-pipe`: commands on its file
//! descriptor 3, answers on 4, each JSON message ended by a NUL byte). The
//! viewport is the preview pane, so `100vh` means the pane's height, and the
//! capture goes past the viewport down to the end of the page. Scripts are
//! turned off before the page loads; images, fonts and style sheets load.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::json::{self, Value};

use crate::ai::find_on_path;

/// The program name of Google's headless-only Chromium build.
pub const HEADLESS_SHELL: &str = "chrome-headless-shell";

/// Browser programs looked for on `PATH`, best first.
const PATH_NAMES: [&str; 7] = [
    HEADLESS_SHELL,
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "microsoft-edge",
    "brave-browser",
];

/// macOS app bundles and their program, looked for in `/Applications` and
/// `~/Applications`, best first.
const MAC_APPS: [&str; 4] = [
    "Google Chrome.app/Contents/MacOS/Google Chrome",
    "Chromium.app/Contents/MacOS/Chromium",
    "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "Brave Browser.app/Contents/MacOS/Brave Browser",
];

/// How deep under the install folder the headless shell is looked for. The
/// installer puts it at `chrome-headless-shell/<platform>-<version>/
/// chrome-headless-shell-<platform>/chrome-headless-shell`.
const INSTALL_SEARCH_DEPTH: usize = 4;

/// Starts the browser with the pipe on file descriptors 3 and 4: the shell
/// moves our stdin and stdout there, then becomes the browser. The browser
/// and its arguments come as `$0` and `$@`, so nothing is parsed by the shell.
const PIPE_LAUNCHER: &str = r#"exec "$0" "$@" 3<&0 4>&1 0</dev/null 1>/dev/null"#;

/// Longest wait for the page's load event. A page whose image or font never
/// arrives is captured as it is then.
const LOAD_LIMIT: Duration = Duration::from_secs(10);

/// Longest a whole snapshot may take, start to finish.
const SNAPSHOT_LIMIT: Duration = Duration::from_secs(30);

/// Numbers the temporary profile folders of this process.
static PROFILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Where Rubrique installs the headless shell (`chromium` in the config folder).
pub fn install_folder() -> Option<PathBuf> {
    crate::settings::config_folder().map(|folder| folder.join("chromium"))
}

/// The browser to draw with: the headless shell Rubrique installed, then a
/// browser app (macOS), then a browser on `PATH`.
pub fn find_browser() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    install_folder()
        .and_then(|folder| find_headless_shell(&folder, INSTALL_SEARCH_DEPTH))
        .or_else(|| cfg!(target_os = "macos").then(|| find_mac_app(home.as_deref(), &|path| path.is_file())).flatten())
        .or_else(|| PATH_NAMES.iter().find_map(|name| find_on_path(name)))
}

/// The first browser app found in `/Applications`, then `~/Applications`.
fn find_mac_app(home: Option<&Path>, exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    let folders = std::iter::once(PathBuf::from("/Applications")).chain(home.map(|home| home.join("Applications")));
    let folders: Vec<PathBuf> = folders.collect();
    MAC_APPS.iter().flat_map(|app| folders.iter().map(move |folder| folder.join(app))).find(|path| exists(path))
}

/// The headless shell program somewhere under `folder`, `depth` levels at
/// most; the newest version when there are several.
fn find_headless_shell(folder: &Path, depth: usize) -> Option<PathBuf> {
    let program = folder.join(HEADLESS_SHELL);
    if program.is_file() {
        return Some(program);
    }
    if depth == 0 {
        return None;
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(folder)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    // Version folders sort by name, so the newest is last.
    entries.sort();
    entries.iter().rev().find_map(|entry| find_headless_shell(entry, depth - 1))
}

/// What to draw and how big.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotRequest {
    pub browser: PathBuf,
    /// The page's `file://` address.
    pub page_url: String,
    /// The viewport in CSS pixels (the preview pane's size in points).
    pub width: u32,
    pub height: u32,
    /// Device pixels per CSS pixel, so text is sharp on a Retina screen.
    pub scale: f32,
    /// Tallest capture in device pixels (the largest texture the GPU takes).
    pub max_pixels: u32,
}

/// A drawn page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub png: Vec<u8>,
    /// The page was taller than [`SnapshotRequest::max_pixels`] and is cut.
    pub cut: bool,
}

/// Draw the page. Blocks for up to [`SNAPSHOT_LIMIT`]; run it on its own thread.
///
/// # Errors
///
/// Fails with a message for people when the browser cannot start, quits,
/// answers with an error, or takes too long.
pub fn take_snapshot(request: &SnapshotRequest) -> Result<Snapshot, String> {
    let profile = std::env::temp_dir().join(format!(
        "rubrique-chromium-{}-{}",
        std::process::id(),
        PROFILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = Browser::start(&request.browser, &profile).and_then(|mut browser| browser.snapshot(request));
    let _ = std::fs::remove_dir_all(&profile);
    result
}

/// The browser's command-line flags: headless, a throwaway profile, no
/// first-run screens, background updates or sync.
fn browser_args(profile: &Path) -> Vec<String> {
    [
        "--headless",
        "--remote-debugging-pipe",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-extensions",
        "--disable-sync",
        "--disable-background-networking",
        "--disable-component-update",
        "--hide-scrollbars",
        "--mute-audio",
    ]
    .into_iter()
    .map(String::from)
    .chain([format!("--user-data-dir={}", profile.display()), "about:blank".to_string()])
    .collect()
}

/// The clip of the page to capture, in CSS pixels: the viewport's width and
/// the page's height, cut so the picture stays under `max_pixels` tall.
/// Returns the height and whether the page was cut.
fn capture_height(content_height: f64, scale: f32, max_pixels: u32) -> (f64, bool) {
    let limit = f64::from(max_pixels) / f64::from(scale.max(0.1));
    let height = content_height.max(1.0);
    if height > limit { (limit.floor(), true) } else { (height.ceil(), false) }
}

/// A running headless browser and its `DevTools` pipe.
struct Browser {
    child: Child,
    commands: ChildStdin,
    messages: Receiver<Value>,
    next_id: u64,
    deadline: Instant,
    /// The page fired its load event.
    loaded: bool,
}

impl Browser {
    fn start(program: &Path, profile: &Path) -> Result<Browser, String> {
        std::fs::create_dir_all(profile).map_err(|error| format!("Cannot make a browser profile: {error}"))?;
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg(PIPE_LAUNCHER)
            .arg(program)
            .args(browser_args(profile))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("Cannot start {}: {error}", program.display()))?;
        let (Some(commands), Some(answers)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            return Err("The browser's pipe did not open".to_string());
        };
        let (sender, messages) = mpsc::channel();
        std::thread::spawn(move || read_messages(answers, &sender));
        Ok(Browser { child, commands, messages, next_id: 0, deadline: Instant::now() + SNAPSHOT_LIMIT, loaded: false })
    }

    fn snapshot(&mut self, request: &SnapshotRequest) -> Result<Snapshot, String> {
        let target = self.call("Target.createTarget", &Value::object([("url", Value::from("about:blank"))]), None)?;
        let target_id = target["targetId"].as_str().ok_or("The browser made no page")?.to_string();
        let attached = self.call(
            "Target.attachToTarget",
            &Value::object([("targetId", Value::from(target_id)), ("flatten", Value::from(true))]),
            None,
        )?;
        let session = attached["sessionId"].as_str().ok_or("The browser did not attach to the page")?.to_string();
        let session = Some(session.as_str());
        self.call("Emulation.setScriptExecutionDisabled", &Value::object([("value", Value::from(true))]), session)?;
        let metrics = Value::object([
            ("width", Value::from(request.width)),
            ("height", Value::from(request.height)),
            ("deviceScaleFactor", Value::from(f64::from(request.scale))),
            ("mobile", Value::from(false)),
        ]);
        self.call("Emulation.setDeviceMetricsOverride", &metrics, session)?;
        self.call("Page.enable", &Value::object([]), session)?;
        self.call("Page.navigate", &Value::object([("url", Value::from(request.page_url.as_str()))]), session)?;
        self.wait_for_load()?;
        let layout = self.call("Page.getLayoutMetrics", &Value::object([]), session)?;
        let content_height = layout["cssContentSize"]["height"].as_f64().unwrap_or(f64::from(request.height));
        let (height, cut) = capture_height(content_height, request.scale, request.max_pixels);
        let clip = Value::object([
            ("x", Value::from(0_u32)),
            ("y", Value::from(0_u32)),
            ("width", Value::from(request.width)),
            ("height", Value::from(height)),
            ("scale", Value::from(1_u32)),
        ]);
        let shot = self.call(
            "Page.captureScreenshot",
            &Value::object([
                ("format", Value::from("png")),
                ("captureBeyondViewport", Value::from(true)),
                ("clip", clip),
            ]),
            session,
        )?;
        let data = shot["data"].as_str().ok_or("The browser sent no picture")?;
        let png = decode_base64(data).map_err(|error| format!("The browser's picture is broken: {error}"))?;
        let _ = self.send("Browser.close", &Value::object([]), None);
        Ok(Snapshot { png, cut })
    }

    /// Send a command and wait for its answer, keeping note of events.
    fn call(&mut self, method: &str, params: &Value, session: Option<&str>) -> Result<Value, String> {
        let id = self.send(method, params, session)?;
        loop {
            let message = self.next_message(self.deadline)?;
            if message["id"].as_u64() == Some(id) {
                if let Some(error) = message.get("error") {
                    let text = error["message"].as_str().unwrap_or("unknown error");
                    return Err(format!("The browser refused {method}: {text}"));
                }
                return Ok(message["result"].clone());
            }
            self.note_event(&message);
        }
    }

    fn send(&mut self, method: &str, params: &Value, session: Option<&str>) -> Result<u64, String> {
        self.next_id += 1;
        let mut message = Value::object([
            ("id", Value::from(self.next_id)),
            ("method", Value::from(method)),
            ("params", params.clone()),
        ]);
        if let (Some(session), Value::Object(entries)) = (session, &mut message) {
            entries.push(("sessionId".to_string(), Value::from(session)));
        }
        let mut bytes = message.to_string().into_bytes();
        bytes.push(0);
        self.commands.write_all(&bytes).map_err(|error| format!("The browser stopped listening: {error}"))?;
        Ok(self.next_id)
    }

    /// Wait for the load event, but no longer than [`LOAD_LIMIT`].
    fn wait_for_load(&mut self) -> Result<(), String> {
        let until = (Instant::now() + LOAD_LIMIT).min(self.deadline);
        while !self.loaded {
            match self.next_message(until) {
                Ok(message) => self.note_event(&message),
                // Draw what is there when an image or font never arrives.
                Err(_) if Instant::now() < self.deadline && self.child_running() => return Ok(()),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn note_event(&mut self, message: &Value) {
        if message["method"] == "Page.loadEventFired" {
            self.loaded = true;
        }
    }

    fn next_message(&self, until: Instant) -> Result<Value, String> {
        let wait = until.saturating_duration_since(Instant::now());
        match self.messages.recv_timeout(wait) {
            Ok(message) => Ok(message),
            Err(RecvTimeoutError::Timeout) => Err("The browser took too long to draw the page".to_string()),
            Err(RecvTimeoutError::Disconnected) => Err("The browser quit before drawing the page".to_string()),
        }
    }

    fn child_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Browser {
    /// The browser never outlives its snapshot.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Read NUL-ended JSON messages from the browser until it closes the pipe.
fn read_messages(mut answers: impl Read, sender: &mpsc::Sender<Value>) {
    let mut pending = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let read = match answers.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        pending.extend_from_slice(&chunk[..read]);
        for message in take_messages(&mut pending) {
            if sender.send(message).is_err() {
                return;
            }
        }
    }
}

/// The complete messages at the start of `pending`, removed from it.
/// Messages that are not JSON are skipped.
fn take_messages(pending: &mut Vec<u8>) -> Vec<Value> {
    let Some(last_end) = pending.iter().rposition(|byte| *byte == 0) else {
        return Vec::new();
    };
    let complete: Vec<u8> = pending.drain(..=last_end).collect();
    complete.split(|byte| *byte == 0).filter_map(|message| json::parse_bytes(message).ok()).collect()
}

/// Decode standard base64 with padding (RFC 4648), as the browser sends its
/// pictures.
fn decode_base64(text: &str) -> Result<Vec<u8>, String> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err("the base64 text has a wrong length".into());
    }
    let groups = bytes.len() / 4;
    let mut decoded = Vec::with_capacity(groups * 3);
    for (index, group) in bytes.as_chunks::<4>().0.iter().enumerate() {
        let padding = group.iter().rev().take_while(|byte| **byte == b'=').count();
        if padding > 2 || (padding > 0 && index + 1 != groups) {
            return Err("the base64 text has padding in the wrong place".into());
        }
        let mut value = 0u32;
        for byte in &group[..4 - padding] {
            value = value << 6 | u32::from(base64_digit(*byte)?);
        }
        for _ in 0..padding {
            value <<= 6;
        }
        decoded.extend_from_slice(&value.to_be_bytes()[1..4 - padding]);
    }
    Ok(decoded)
}

/// The 6-bit value of one base64 character.
fn base64_digit(byte: u8) -> Result<u8, String> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(format!("the base64 text has the character {:?}", char::from(byte))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_base64_as_in_rfc_4648() {
        for (text, expected) in [("", ""), ("Zg==", "f"), ("Zm8=", "fo"), ("Zm9v", "foo"), ("Zm9vYmFy", "foobar")] {
            assert_eq!(decode_base64(text).unwrap(), expected.as_bytes());
        }
        assert_eq!(decode_base64("+/+/").unwrap(), [0xfb, 0xff, 0xbf]);
        for broken in ["Zg=", "Zg==Zg==", "Z===", "Zm9*", "Zm 9"] {
            assert!(decode_base64(broken).is_err(), "{broken}");
        }
    }

    #[test]
    fn splits_messages_on_nul_bytes() {
        let mut pending = b"{\"id\":1}\0{\"id\":2}\0{\"id\"".to_vec();
        let messages = take_messages(&mut pending);
        let id = |number: u32| Value::object([("id", Value::from(number))]);
        assert_eq!(messages, vec![id(1), id(2)]);
        assert_eq!(pending, b"{\"id\"");
        pending.extend_from_slice(b":3}\0");
        assert_eq!(take_messages(&mut pending), vec![id(3)]);
        assert_eq!(pending, b"");
        let mut broken = b"not json\0".to_vec();
        assert_eq!(take_messages(&mut broken), Vec::<Value>::new());
    }

    #[test]
    fn cuts_pages_taller_than_a_texture() {
        assert_eq!(capture_height(1260.2, 2.0, 16384), (1261.0, false));
        assert_eq!(capture_height(20000.0, 2.0, 16384), (8192.0, true));
        assert_eq!(capture_height(0.0, 1.0, 16384), (1.0, false));
    }

    #[test]
    fn finds_mac_apps_in_order() {
        let home = Path::new("/Users/a");
        let only_edge = |path: &Path| path.ends_with("Microsoft Edge");
        assert_eq!(
            find_mac_app(Some(home), &only_edge),
            Some(PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"))
        );
        let in_home = |path: &Path| path.starts_with("/Users/a/Applications/Chromium.app");
        assert_eq!(
            find_mac_app(Some(home), &in_home),
            Some(PathBuf::from("/Users/a/Applications/Chromium.app/Contents/MacOS/Chromium"))
        );
        assert_eq!(find_mac_app(None, &|_| false), None);
    }

    #[test]
    fn finds_the_newest_installed_headless_shell() {
        let temp = crate::test_support::TempFolder::new();
        let folder = temp.path();
        assert_eq!(find_headless_shell(folder, INSTALL_SEARCH_DEPTH), None);
        for version in ["mac_arm-130.0.1", "mac_arm-131.0.2"] {
            let program = folder.join(HEADLESS_SHELL).join(version).join("chrome-headless-shell-mac-arm64");
            std::fs::create_dir_all(&program).unwrap();
            std::fs::write(program.join(HEADLESS_SHELL), "").unwrap();
        }
        let found = find_headless_shell(folder, INSTALL_SEARCH_DEPTH).unwrap();
        assert!(found.to_string_lossy().contains("131.0.2"), "{}", found.display());
        assert_eq!(find_headless_shell(folder, 1), None);
    }

    #[test]
    fn browser_runs_headless_with_its_own_profile() {
        let args = browser_args(Path::new("/tmp/p"));
        assert!(args.contains(&"--headless".to_string()));
        assert!(args.contains(&"--remote-debugging-pipe".to_string()));
        assert!(args.contains(&"--user-data-dir=/tmp/p".to_string()));
    }

    /// Draws a page with a real browser, when one is installed (skipped
    /// otherwise): scripts stay off and the capture covers the whole page.
    #[test]
    fn draws_a_page_with_an_installed_browser() {
        let Some(browser) = find_browser() else {
            return;
        };
        let temp = crate::test_support::TempFolder::new();
        let html = "<body style='margin:0'><div style='height:900px;background:#00f'></div>\
                    <script>document.body.style.background='#f00';document.body.innerHTML+='<div style=\"height:5000px\"></div>'</script>";
        let page = temp.write("page.html", html);
        let request = SnapshotRequest {
            browser,
            page_url: format!("file://{}", page.display()),
            width: 300,
            height: 200,
            scale: 1.0,
            max_pixels: 16384,
        };
        let snapshot = take_snapshot(&request).unwrap();
        let picture = image::load_from_memory(&snapshot.png).unwrap();
        // 900 px tall: past the 200 px viewport, and the script's 5000 px never added.
        assert_eq!((picture.width(), picture.height()), (300, 900));
        assert!(!snapshot.cut);
    }
}
