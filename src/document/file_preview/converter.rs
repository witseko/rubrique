//! Run an external converter on a document snapshot, with bounded output
//! and a time limit.

use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Time limit for one conversion.
const CONVERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// Write `input` to the converter's standard input and collect up to
/// `max_output` bytes of its output; `name` (such as "Word converter") goes
/// in the error messages.
pub(super) fn run(command: &mut Command, input: Arc<Vec<u8>>, max_output: usize, name: &str) -> io::Result<Vec<u8>> {
    let mut child = command.stdin(Stdio::piped()).stderr(Stdio::null()).stdout(Stdio::piped()).spawn()?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other(format!("Cannot read output from the {name}.")));
    };
    let writer = thread::spawn(move || stdin.write_all(&input));
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(max_output as u64 + 1).read_to_end(&mut bytes)?;
        Ok::<_, io::Error>(bytes)
    });
    let deadline = Instant::now() + CONVERSION_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(result.err().unwrap_or_else(|| io::Error::other(format!("The {name} timed out."))));
            }
        }
    };
    let _ = writer.join();
    let output = reader.join().map_err(|_| io::Error::other(format!("Cannot read output from the {name}.")))??;
    if output.len() <= max_output && (!status.success() || output.is_empty()) {
        return Err(io::Error::other(format!("The {name} could not read this document.")));
    }
    Ok(output)
}
