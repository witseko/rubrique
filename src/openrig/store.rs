//! Persist only queue references, one record per send.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Seat, TaskLink, valid_seat, valid_task_id};
use crate::document::text_file::write_atomically;

/// Allocate a fresh task ID so a lost CLI response can be reconciled.
pub fn new_id() -> Result<String, String> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| "Cannot create an OpenRig task ID.")?;
    let mut id = String::from("qitem-rubrique-");
    for byte in random {
        let _ = write!(id, "{byte:02x}");
    }
    Ok(id)
}

/// Save the reference before sending, so it survives an interrupted submission.
pub fn save(folder: &Path, task: &TaskLink) -> io::Result<()> {
    let document = task.document.to_str().ok_or(io::Error::new(io::ErrorKind::InvalidInput, "invalid file path"))?;
    let value = json!({
        "document": document,
        "endpoint": task.endpoint.to_string(),
        "rigId": task.seat.rig_id,
        "seat": task.seat.address,
    });
    fs::create_dir_all(folder)?;
    write_atomically(&folder.join(format!("{}.json", task.id)), value.to_string().as_bytes())
}

/// Load saved links without reading document contents.
pub fn load(folder: &Path) -> io::Result<Vec<TaskLink>> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut tasks: Vec<_> = entries.filter_map(Result::ok).filter_map(|entry| read_link(&entry.path())).collect();
    tasks.sort_by_cached_key(|task| {
        std::cmp::Reverse(
            fs::metadata(folder.join(format!("{}.json", task.id))).and_then(|metadata| metadata.modified()).ok(),
        )
    });
    Ok(tasks)
}

/// Parse one saved reference. Ignore temporary files and unsupported records.
fn read_link(path: &Path) -> Option<TaskLink> {
    let id = path.file_name()?.to_str()?.strip_suffix(".json")?.to_owned();
    if !valid_task_id(&id) {
        return None;
    }
    let value: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    let endpoint: std::net::SocketAddr = value["endpoint"].as_str()?.parse().ok()?;
    let address = value["seat"].as_str()?.to_owned();
    if !endpoint.ip().is_loopback() || endpoint.port() == 0 || !valid_seat(&address) {
        return None;
    }
    Some(TaskLink {
        id,
        document: PathBuf::from(value["document"].as_str()?),
        endpoint,
        seat: Seat { address, rig_id: value["rigId"].as_str()?.to_owned() },
    })
}
