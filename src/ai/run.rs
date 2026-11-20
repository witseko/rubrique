//! Run one AI tool as a separate program and collect what it prints.
//!
//! The program gets its arguments directly (no shell). Two
//! threads read its output and error streams as they arrive and call `wake`
//! so the window can redraw; the window checks [`AiRun::poll`] each frame.

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// How a run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunEnd {
    /// The program exited; `success` is a zero exit code.
    Exited { success: bool, code: Option<i32> },
    /// The user pressed Stop.
    Stopped,
}

/// A running (or finished) tool.
#[derive(Debug)]
pub struct AiRun {
    child: Child,
    output: Arc<Mutex<Vec<u8>>>,
    errors: Arc<Mutex<Vec<u8>>>,
    /// Reader threads still reading.
    open_readers: Arc<AtomicUsize>,
    started: Instant,
    /// Set once the run is over, with how long it took.
    end: Option<(RunEnd, Duration)>,
    stopped: bool,
}

impl AiRun {
    /// Start `program` with `args` in `folder`.
    ///
    /// # Errors
    ///
    /// Fails when the program cannot be started (for example, not installed).
    pub fn start(
        program: &str,
        args: &[String],
        folder: &Path,
        wake: impl Fn() + Send + Clone + 'static,
    ) -> io::Result<AiRun> {
        let mut command = Command::new(program);
        command.args(args).current_dir(folder);
        Self::start_command(&mut command, None, None, wake)
    }

    /// Run a configured command with optional stdin and a capture limit per stream.
    pub fn start_command(
        command: &mut Command,
        input: Option<String>,
        capture_limit: Option<usize>,
        wake: impl Fn() + Send + Clone + 'static,
    ) -> io::Result<AiRun> {
        let mut child = command
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let output = Arc::new(Mutex::new(Vec::new()));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let open_readers = Arc::new(AtomicUsize::new(0));
        if let (Some(input), Some(mut stream)) = (input, child.stdin.take()) {
            thread::spawn(move || {
                // A child can close its pipe on refusal. Its response is authoritative.
                let _ = stream.write_all(input.as_bytes());
            });
        }
        if let Some(stream) = child.stdout.take() {
            read_in_background(stream, Arc::clone(&output), Arc::clone(&open_readers), capture_limit, wake.clone());
        }
        if let Some(stream) = child.stderr.take() {
            read_in_background(stream, Arc::clone(&errors), Arc::clone(&open_readers), capture_limit, wake);
        }
        Ok(AiRun { child, output, errors, open_readers, started: Instant::now(), end: None, stopped: false })
    }

    /// Check whether the run is over. Returns the end once the program has
    /// exited and its output is fully read.
    pub fn poll(&mut self) -> Option<&RunEnd> {
        if self.end.is_none() {
            let status = self.child.try_wait().ok().flatten();
            let read_all = self.open_readers.load(Ordering::Acquire) == 0;
            if let Some(status) = status
                && (read_all || self.stopped)
            {
                let end = if self.stopped {
                    RunEnd::Stopped
                } else {
                    RunEnd::Exited { success: status.success(), code: status.code() }
                };
                self.end = Some((end, self.started.elapsed()));
            }
        }
        self.end.as_ref().map(|(end, _)| end)
    }

    /// How the run ended, once it has (after [`AiRun::poll`] saw it).
    pub fn end(&self) -> Option<&RunEnd> {
        self.end.as_ref().map(|(end, _)| end)
    }

    /// Stop the program.
    pub fn stop(&mut self) {
        if self.end.is_none() {
            self.stopped = true;
            let _ = self.child.kill();
        }
    }

    /// Seconds since the start, or the run's length once it is over.
    pub fn elapsed_seconds(&self) -> u64 {
        self.end.as_ref().map_or_else(|| self.started.elapsed(), |(_, took)| *took).as_secs()
    }

    pub fn is_running(&self) -> bool {
        self.end.is_none()
    }

    /// Everything printed so far.
    pub fn output_text(&self) -> String {
        text_of(&self.output)
    }

    /// Everything printed to the error stream so far.
    pub fn error_text(&self) -> String {
        text_of(&self.errors)
    }
}

impl Drop for AiRun {
    /// Closing the panel or quitting stops a tool that is still running.
    fn drop(&mut self) {
        if self.end.is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn text_of(buffer: &Mutex<Vec<u8>>) -> String {
    String::from_utf8_lossy(&buffer.lock().unwrap_or_else(PoisonError::into_inner)).into_owned()
}

/// Copy `stream` into `buffer` as it arrives, calling `wake` after each piece.
fn read_in_background(
    mut stream: impl Read + Send + 'static,
    buffer: Arc<Mutex<Vec<u8>>>,
    open_readers: Arc<AtomicUsize>,
    capture_limit: Option<usize>,
    wake: impl Fn() + Send + 'static,
) {
    open_readers.fetch_add(1, Ordering::AcqRel);
    thread::spawn(move || {
        let mut piece = [0_u8; 4096];
        loop {
            match stream.read(&mut piece) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    let mut buffer = buffer.lock().unwrap_or_else(PoisonError::into_inner);
                    let remaining = capture_limit.unwrap_or(usize::MAX).saturating_sub(buffer.len());
                    buffer.extend_from_slice(&piece[..count.min(remaining)]);
                    drop(buffer);
                    wake();
                }
            }
        }
        open_readers.fetch_sub(1, Ordering::AcqRel);
        wake();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finish(run: &mut AiRun) -> RunEnd {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(end) = run.poll() {
                return end.clone();
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the program did not finish");
    }

    #[test]
    fn collects_output_and_exit_code() {
        let args = ["-c".to_string(), "printf 'one\\ntwo'; printf 'oops' >&2; exit 3".to_string()];
        let mut run = AiRun::start("sh", &args, Path::new("/"), || {}).expect("sh starts");
        assert_eq!(finish(&mut run), RunEnd::Exited { success: false, code: Some(3) });
        assert_eq!(run.output_text(), "one\ntwo");
        assert_eq!(run.error_text(), "oops");
        assert!(!run.is_running());
    }

    #[test]
    fn arguments_are_not_run_by_a_shell() {
        let args = ["$(echo hi); echo no".to_string()];
        let mut run = AiRun::start("echo", &args, Path::new("/"), || {}).expect("echo starts");
        assert_eq!(finish(&mut run), RunEnd::Exited { success: true, code: Some(0) });
        assert_eq!(run.output_text(), "$(echo hi); echo no\n");
    }

    #[test]
    fn stop_ends_a_long_run() {
        let args = ["30".to_string()];
        let mut run = AiRun::start("sleep", &args, Path::new("/"), || {}).expect("sleep starts");
        run.stop();
        assert_eq!(finish(&mut run), RunEnd::Stopped);
        assert!(run.elapsed_seconds() < 5);
    }

    #[test]
    fn missing_program_fails_to_start() {
        assert!(AiRun::start("surely-not-a-real-program-xyz", &[], Path::new("/"), || {}).is_err());
    }
}
