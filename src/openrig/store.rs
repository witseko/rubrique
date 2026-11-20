//! Persist only queue references, one record per send.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{RigHost, Seat, TaskLink, valid_seat, valid_task_id};
use crate::document::text_file::write_atomically;
use crate::remote::mirror::is_valid_host;

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
    let host = match &task.host {
        RigHost::Local => Value::Null,
        RigHost::Ssh(host) => Value::from(host.as_str()),
    };
    let value = json!({
        "document": document,
        "host": host,
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
/// A record without a host, saved before remote rigs, is local.
fn read_link(path: &Path) -> Option<TaskLink> {
    let id = path.file_name()?.to_str()?.strip_suffix(".json")?.to_owned();
    if !valid_task_id(&id) {
        return None;
    }
    let value: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    let endpoint: std::net::SocketAddr = value["endpoint"].as_str()?.parse().ok()?;
    let address = value["seat"].as_str()?.to_owned();
    let host = match &value["host"] {
        Value::Null => RigHost::Local,
        Value::String(host) if is_valid_host(host) => RigHost::Ssh(host.clone()),
        _ => return None,
    };
    if !endpoint.ip().is_loopback() || endpoint.port() == 0 || !valid_seat(&address) {
        return None;
    }
    Some(TaskLink {
        id,
        document: PathBuf::from(value["document"].as_str()?),
        host,
        endpoint,
        seat: Seat { address, rig_id: value["rigId"].as_str()?.to_owned() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    /// A task on a remote rig comes back with its host after a restart.
    #[test]
    fn saves_and_loads_the_rig_host() {
        let temp = TempFolder::new();
        let folder = temp.make_folder("tasks");
        let task = TaskLink {
            id: "qitem-remote".into(),
            document: temp.path().join("plan.md"),
            host: RigHost::Ssh("user@build-box".into()),
            endpoint: "127.0.0.1:7433".parse().unwrap(),
            seat: Seat { address: "builder@demo".into(), rig_id: "demo-id".into() },
        };
        save(&folder, &task).unwrap();
        let loaded = load(&folder).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].host, task.host);
        assert_eq!(loaded[0].seat, task.seat);
    }

    /// Records saved before remote rigs are local; a bad host makes a record unusable.
    #[test]
    fn reads_old_records_as_local_and_ignores_bad_hosts() {
        let temp = TempFolder::new();
        let folder = temp.make_folder("tasks");
        let record = |host: &str| {
            format!(
                r#"{{"document":"/notes/a.md",{host}"endpoint":"127.0.0.1:7433","rigId":"demo-id","seat":"builder@demo"}}"#
            )
        };
        temp.write("tasks/qitem-old.json", record(""));
        temp.write("tasks/qitem-option.json", record(r#""host":"-oProxyCommand=x","#));
        temp.write("tasks/qitem-number.json", record(r#""host":7,"#));
        let loaded = load(&folder).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "qitem-old");
        assert_eq!(loaded[0].host, RigHost::Local);
    }
}
