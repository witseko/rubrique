//! Send work to an existing `OpenRig` daemon and read its queue through rig,
//! on this computer or on a remote host over SSH.

use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::ai::find_on_path;
use crate::ai::run::{AiRun, RunEnd};
use crate::remote::connection;
use crate::remote::mirror::is_valid_host;

pub mod seat_setup;
pub mod spec;
pub mod store;

const TIMEOUT: Duration = Duration::from_secs(12);
const MAX_HANDOFFS: usize = 8;
const MAX_CLI_OUTPUT: usize = 4 * 1024 * 1024;
pub const MAX_BODY_BYTES: usize = 120_000;
/// What rig writes to stderr when the seat list is incomplete.
const PARTIAL_MARKER: &str = "Warning: failed to fetch nodes";
/// The rig command a remote login shell looks up on its own PATH.
const REMOTE_PROGRAM: &str = "rig";
/// The shell's exit status for a command it cannot find.
const EXIT_COMMAND_NOT_FOUND: i32 = 127;
/// How long `rig up` may take, plan or launch: rig's own wait for the
/// daemon (120 s) and a margin.
const UP_TIMEOUT: Duration = Duration::from_secs(150);
/// What to say when the local rig CLI is missing.
const LOCAL_RIG_MISSING: &str = "OpenRig CLI missing. Install @openrig/cli and put rig on PATH.";

/// Runs rig on a remote host through its login shell (rig is often on PATH
/// only there, through nvm), with the same environment as the local call.
/// `$1` is the daemon URL or empty, `$2` the rig command, the rest rig's
/// arguments; the body arrives on stdin. rig's stderr stays on the host,
/// except for the partial-list check. Prints rig's stdout, then a last line
/// `<exit status> <1 if partial else 0>`, and always exits 0, so a failed rig
/// call never reaches the SSH error text. One line: some login shells (csh)
/// refuse a newline inside quotes.
const REMOTE_RIG: &str = "u=$1; p=$2; shift 2; \
    unset RIGGED_URL OPENRIG_URL OPENRIG_SESSION_NAME RIGGED_SESSION_NAME TMUX_PANE; \
    export OPENRIG_HOST_SELECTED=local NO_PROXY='*' no_proxy='*'; \
    [ -z \"$u\" ] || export OPENRIG_URL=\"$u\"; cd / || exit 1; exec 3>&1; \
    e=$( { o=$(\"${SHELL:-/bin/sh}\" -lc 'command -v \"$0\" >/dev/null || exit 127; exec \"$0\" \"$@\"' \"$p\" \"$@\" 3>&-); s=$?; \
    printf '%s\\n%s ' \"$o\" \"$s\" >&3; } 2>&1 ); \
    case $e in *'Warning: failed to fetch nodes'*) echo 1;; *) echo 0;; esac";

/// The machine an `OpenRig` daemon runs on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RigHost {
    /// This computer.
    #[default]
    Local,
    /// A host reached over SSH, by its SSH host name.
    Ssh(String),
}

impl RigHost {
    /// The name shown in the rig picker and on task cards.
    pub fn label(&self) -> &str {
        match self {
            RigHost::Local => "This computer",
            RigHost::Ssh(host) => host,
        }
    }
}

/// A named agent seat in a particular `OpenRig` team.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seat {
    pub address: String,
    pub rig_id: String,
}

/// Seats offered by one daemon.
pub struct Inventory {
    pub host: RigHost,
    pub endpoint: SocketAddr,
    pub seats: Vec<Seat>,
    pub partial: bool,
}

/// A queue reference saved before sending a document step.
#[derive(Clone, Debug)]
pub struct TaskLink {
    pub id: String,
    pub document: PathBuf,
    pub host: RigHost,
    pub endpoint: SocketAddr,
    pub seat: Seat,
}

/// The current row of a task, after following any unambiguous local handoffs.
#[derive(Debug)]
pub struct TaskStatus {
    pub id: String,
    pub seat: String,
    pub state: String,
    pub claimed: bool,
    pub closure: Option<String>,
    pub resolution: Option<String>,
    pub handoff_note: Option<String>,
}

impl TaskStatus {
    /// A known final state needs no periodic refresh.
    pub fn is_closed(&self) -> bool {
        matches!(self.state.as_str(), "done" | "failed" | "denied" | "canceled" | "handed-off")
    }
}

/// Why a call to rig gave no answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RigCallError {
    /// rig is not installed (or not on PATH) on the machine that should run it.
    NotInstalled(String),
    /// rig ran, or could not be run, without a usable answer.
    Failed(String),
}

impl fmt::Display for RigCallError {
    /// The message to show the user.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RigCallError::NotInstalled(message) | RigCallError::Failed(message) => formatter.write_str(message),
        }
    }
}

impl From<RigCallError> for String {
    fn from(error: RigCallError) -> Self {
        error.to_string()
    }
}

impl From<&str> for RigCallError {
    fn from(message: &str) -> Self {
        RigCallError::Failed(message.to_owned())
    }
}

/// The answer of `rig spec validate --json`, which needs no daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpecValidation {
    pub valid: bool,
    /// Each error with its path (`pods[0].members[1].id`) when it names one.
    pub errors: Vec<spec::Issue>,
    /// Findings that do not block validation or launch.
    pub advisories: Vec<String>,
}

/// The answer of `rig spec preflight --json`, which asks the daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preflight {
    pub ready: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

/// The answer of `rig up --json`, for a launch or a `--plan` preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpReply {
    /// rig exited cleanly and the daemon reported no failure.
    pub ok: bool,
    /// `rigResult`, such as `launched` or `partially_restored`, when given.
    pub rig_result: Option<String>,
    /// The daemon's own explanation, when it gave one.
    pub message: Option<String>,
    /// Validation or preflight errors that stopped the launch or the plan.
    pub errors: Vec<String>,
    /// Findings that do not stop the launch.
    pub warnings: Vec<String>,
}

/// The rig executable on the machine that runs the daemon.
pub struct Rig {
    program: PathBuf,
    host: RigHost,
}

impl Rig {
    /// The rig CLI for `host`. Locally it is found with the same search as
    /// Rubrique's other CLI tools; a remote host looks it up itself, so no
    /// local rig is needed for it.
    pub fn open(host: &RigHost) -> Result<Self, String> {
        let program = match host {
            RigHost::Local => find_on_path("rig").ok_or(LOCAL_RIG_MISSING)?,
            RigHost::Ssh(name) if is_valid_host(name) => PathBuf::from(REMOTE_PROGRAM),
            RigHost::Ssh(_) => return Err("Use a host name, or user@host.".into()),
        };
        Ok(Self { program, host: host.clone() })
    }

    /// [`Rig::open`] for the team editor, which tells a missing rig apart
    /// from other failures.
    ///
    /// # Errors
    ///
    /// [`RigCallError::NotInstalled`] when rig is not on this computer's
    /// PATH; [`RigCallError::Failed`] for a host name that is not valid.
    pub fn open_checked(host: &RigHost) -> Result<Self, RigCallError> {
        if *host == RigHost::Local && find_on_path("rig").is_none() {
            return Err(RigCallError::NotInstalled(LOCAL_RIG_MISSING.into()));
        }
        Self::open(host).map_err(RigCallError::Failed)
    }

    /// Check a rig spec with `rig spec validate <spec> --json` (no daemon
    /// needed). `spec` is an absolute path on the rig's machine.
    ///
    /// # Errors
    ///
    /// Fails when rig is missing, cannot run or gives no readable answer.
    pub fn spec_validate(&self, spec: &Path) -> Result<SpecValidation, RigCallError> {
        let reply = self.call(None, &["spec", "validate", spec_argument(spec)?, "--json"], None, TIMEOUT)?;
        parse_validation(&reply.stdout)
    }

    /// Ask the daemon whether the spec can launch, with `rig spec preflight
    /// <spec> --json`. `spec` is an absolute path on the rig's machine.
    ///
    /// # Errors
    ///
    /// Fails when rig is missing, the daemon is not running, or rig gives no
    /// readable answer.
    pub fn spec_preflight(&self, spec: &Path) -> Result<Preflight, RigCallError> {
        let reply = self.call(None, &["spec", "preflight", spec_argument(spec)?, "--json"], None, UP_TIMEOUT)?;
        parse_preflight(reply.success, &reply.stdout)
    }

    /// Preview a launch with `rig up <spec> --plan --json`; starts nothing.
    ///
    /// # Errors
    ///
    /// Fails when rig is missing, cannot run or gives no readable answer.
    pub fn up_plan(&self, spec: &Path) -> Result<UpReply, RigCallError> {
        let reply = self.call(None, &["up", spec_argument(spec)?, "--plan", "--json"], None, UP_TIMEOUT)?;
        parse_up(reply.success, &reply.stdout)
    }

    /// Launch the team with `rig up <spec> --json`.
    ///
    /// # Errors
    ///
    /// Fails when rig is missing, cannot run or gives no readable answer.
    pub fn up(&self, spec: &Path) -> Result<UpReply, RigCallError> {
        let reply = self.call(None, &["up", spec_argument(spec)?, "--json"], None, UP_TIMEOUT)?;
        parse_up(reply.success, &reply.stdout)
    }

    /// Read the configured daemon and list its local agent seats.
    pub fn inventory(&self) -> Result<Inventory, String> {
        let endpoint = self.endpoint()?;
        self.inventory_at(endpoint)
    }

    /// The configured daemon's loopback address on the rig's machine.
    fn endpoint(&self) -> Result<SocketAddr, String> {
        // The URL override describes this computer's daemon, not a remote one.
        let override_url = (self.host == RigHost::Local)
            .then(|| {
                std::env::var("OPENRIG_URL")
                    .ok()
                    .filter(|url| !url.is_empty())
                    .or_else(|| std::env::var("RIGGED_URL").ok().filter(|url| !url.is_empty()))
            })
            .flatten();
        if let Some(url) = override_url {
            return local_origin(&url).ok_or_else(|| "OpenRig needs a local HTTP daemon on a loopback port.".into());
        }
        let status = self.status(None)?;
        let port = status["daemon"]["port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .filter(|port| *port != 0)
            .ok_or("OpenRig did not report a daemon port.")?;
        Ok(SocketAddr::from(([127, 0, 0, 1], port)))
    }

    /// Read local agent seats and match them to teams on the selected daemon.
    fn inventory_at(&self, endpoint: SocketAddr) -> Result<Inventory, String> {
        let status = self.status(Some(endpoint))?;
        let (nodes, partial) = self.json(
            Some(endpoint),
            &[
                "ps",
                "--nodes",
                "--all-rigs",
                "--no-cleanup",
                "--json",
                "--fields",
                "rigId,canonicalSessionName,nodeKind",
            ],
            None,
        )?;
        let (mut seats, partial) = parse_seats(&nodes, partial)?;
        seats.retain(|seat| has_rig(&status, &seat.rig_id));
        seats.sort_by(|left, right| left.address.cmp(&right.address));
        seats.dedup();
        Ok(Inventory { host: self.host.clone(), endpoint, seats, partial })
    }

    /// Submit one saved ID. If the response is lost, check that ID instead of creating another row.
    pub fn send(&self, task: &TaskLink, body: String) -> Result<TaskStatus, String> {
        if body.trim().is_empty() || body.len() > MAX_BODY_BYTES {
            return Err("Enter a document step of up to 120 KB.".into());
        }
        self.check_instance(task)?;
        let destination = format!("--destination={}", task.seat.address);
        let id = format!("--id={}", task.id);
        let result = self.json(
            Some(task.endpoint),
            &["queue", "create", "--source", "rubrique", &destination, &id, "--body-file", "-", "--full", "--json"],
            Some(body),
        );
        match result.and_then(|(row, _)| parse_task(&row, &task.id)) {
            Ok(status) if status.state == "handed-off" => self.read_chain(task),
            Ok(status) => Ok(status),
            Err(_) => self.read_chain(task).map_err(|_| {
                "OpenRig did not confirm the send. Check this task again before sending another copy.".into()
            }),
        }
    }

    /// Read the saved task from its original daemon and follow local handoffs.
    pub fn read(&self, task: &TaskLink) -> Result<TaskStatus, String> {
        self.check_instance(task)?;
        self.read_chain(task)
    }

    /// Confirm the original team's identity before reading or writing queue data.
    fn check_instance(&self, task: &TaskLink) -> Result<(), String> {
        if task.host != self.host {
            return Err("This task was sent to another computer's OpenRig.".into());
        }
        if !has_rig(&self.status(Some(task.endpoint))?, &task.seat.rig_id) {
            return Err("The original OpenRig team is unavailable at the saved address.".into());
        }
        Ok(())
    }

    /// Read up to a bounded number of successors on the same daemon.
    fn read_chain(&self, task: &TaskLink) -> Result<TaskStatus, String> {
        let mut id = task.id.clone();
        let mut visited = Vec::new();
        for _ in 0..MAX_HANDOFFS {
            let (row, _) = self.json(Some(task.endpoint), &["queue", "show", &id, "--full", "--json"], None)?;
            let mut status = parse_task(&row, &id)?;
            if status.state != "handed-off" {
                return Ok(status);
            }
            let Some(next) = next_task_id(&row).filter(|next| *next != id && !visited.iter().any(|seen| seen == next))
            else {
                status.handoff_note = Some("Open the task in OpenRig to follow this handoff.".into());
                return Ok(status);
            };
            visited.push(std::mem::replace(&mut id, next.to_owned()));
        }
        Err("Open this task in OpenRig to follow more handoffs.".into())
    }

    /// Require a reachable daemon and a readable team roster.
    fn status(&self, endpoint: Option<SocketAddr>) -> Result<Value, String> {
        let (status, _) = self.json(endpoint, &["status", "--json"], None)?;
        let daemon = &status["daemon"];
        match daemon["reachable"].as_bool() {
            Some(false) if daemon["state"].as_str().is_some() => {
                Err("Start the OpenRig daemon, then try again.".into())
            }
            Some(true) if daemon["healthy"] == true && status["rigs"].is_array() => Ok(status),
            _ => Err("Rubrique could not verify the OpenRig daemon and its teams.".into()),
        }
    }

    /// Run a bounded CLI call off the UI thread, on the daemon's machine. Queue data goes through stdin.
    fn json(&self, endpoint: Option<SocketAddr>, args: &[&str], body: Option<String>) -> Result<(Value, bool), String> {
        let reply = self.call(endpoint, args, body, TIMEOUT)?;
        parse_reply(reply.success, &reply.stdout, reply.partial)
    }

    /// Run rig with `args` on the daemon's machine, waiting up to `timeout`.
    fn call(
        &self,
        endpoint: Option<SocketAddr>,
        args: &[&str],
        body: Option<String>,
        timeout: Duration,
    ) -> Result<CliReply, RigCallError> {
        match &self.host {
            RigHost::Local => self.local_call(endpoint, args, body, timeout),
            RigHost::Ssh(host) => self.remote_call(host, endpoint, args, body.as_deref(), timeout),
        }
    }

    /// Run rig on this computer.
    fn local_call(
        &self,
        endpoint: Option<SocketAddr>,
        args: &[&str],
        body: Option<String>,
        timeout: Duration,
    ) -> Result<CliReply, RigCallError> {
        let mut command = Command::new(&self.program);
        command
            .args(args)
            .current_dir("/")
            .env("OPENRIG_HOST_SELECTED", "local")
            .env_remove("RIGGED_URL")
            .env_remove("OPENRIG_SESSION_NAME")
            .env_remove("RIGGED_SESSION_NAME")
            .env_remove("TMUX_PANE")
            .env("NO_PROXY", "*")
            .env("no_proxy", "*");
        if let Some(endpoint) = endpoint {
            command.env("OPENRIG_URL", format!("http://{endpoint}"));
        } else {
            command.env_remove("OPENRIG_URL");
        }
        let mut run =
            AiRun::start_command(&mut command, body, Some(MAX_CLI_OUTPUT), || {}).map_err(|_| "Cannot start rig.")?;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(end) = run.poll().cloned() {
                let success = matches!(end, RunEnd::Exited { success: true, .. });
                let partial = run.error_text().contains(PARTIAL_MARKER);
                return Ok(CliReply { success, stdout: run.output_text(), partial });
            }
            thread::sleep(Duration::from_millis(20));
        }
        Err("OpenRig CLI timed out. Check that the local daemon is running.".into())
    }

    /// Run rig on `host` over the shared SSH connection (see [`REMOTE_RIG`]).
    /// On the host, the endpoint is that host's own loopback address.
    fn remote_call(
        &self,
        host: &str,
        endpoint: Option<SocketAddr>,
        args: &[&str],
        body: Option<&str>,
        timeout: Duration,
    ) -> Result<CliReply, RigCallError> {
        let url = endpoint.map(|endpoint| format!("http://{endpoint}")).unwrap_or_default();
        let mut script_args = vec![OsStr::new(&url), self.program.as_os_str()];
        script_args.extend(args.iter().map(OsStr::new));
        let output = connection::run(host, REMOTE_RIG, &script_args, body.map(str::as_bytes), timeout)
            .map_err(|error| RigCallError::Failed(remote_error(host, &error)))?;
        if output.len() > MAX_CLI_OUTPUT {
            return Err("OpenRig CLI returned too much output.".into());
        }
        let output = String::from_utf8_lossy(&output);
        let (status, partial, stdout) =
            parse_remote_output(&output).ok_or("OpenRig CLI returned an unreadable reply.")?;
        if status == EXIT_COMMAND_NOT_FOUND {
            return Err(RigCallError::NotInstalled(format!(
                "OpenRig CLI missing on {host}. Install @openrig/cli there and put rig on PATH."
            )));
        }
        Ok(CliReply { success: status == 0, stdout: stdout.to_owned(), partial })
    }
}

/// How a rig call ended: its exit, its stdout and whether rig warned that
/// the seat list is incomplete. Holds no stderr.
struct CliReply {
    success: bool,
    stdout: String,
    partial: bool,
}

/// Split the remote script's output into rig's exit status, the partial flag and rig's stdout.
fn parse_remote_output(output: &str) -> Option<(i32, bool, &str)> {
    let (stdout, trailer) = output.trim_end().rsplit_once('\n')?;
    let (status, partial) = trailer.split_once(' ')?;
    Some((status.parse().ok()?, partial == "1", stdout))
}

/// An SSH failure as a message. Only ssh's own messages reach it, never rig's
/// stderr: the remote script keeps that on the host.
fn remote_error(host: &str, error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::TimedOut => format!("OpenRig on {host} did not answer in time."),
        _ => error.to_string(),
    }
}

/// Parse the JSON reply without displaying stderr, which can contain private task data.
fn parse_reply(success: bool, stdout: &str, partial: bool) -> Result<(Value, bool), String> {
    let value = serde_json::from_str::<Value>(stdout);
    if !success {
        return Err(if value.as_ref().is_ok_and(|reply| reply["error"] == "qitem_not_found") {
            "OpenRig has no queue item with this ID yet."
        } else {
            "OpenRig CLI could not complete the request."
        }
        .into());
    }
    let value = value.map_err(|_| "OpenRig CLI returned an unreadable JSON reply.")?;
    Ok((value, partial))
}

/// A spec path as rig's argument: absolute (so it never reads as an
/// option) and valid UTF-8.
fn spec_argument(spec: &Path) -> Result<&str, RigCallError> {
    let text = spec.to_str().filter(|_| spec.is_absolute()).ok_or("OpenRig needs the spec's full path.")?;
    Ok(text)
}

/// The JSON object rig printed: the whole output, or else its last line
/// (rig may print a notice before it).
fn reply_object(stdout: &str) -> Option<Value> {
    let parse = |text: &str| serde_json::from_str::<Value>(text).ok().filter(Value::is_object);
    parse(stdout.trim()).or_else(|| stdout.trim().lines().last().and_then(parse))
}

/// The strings in the list at `key`; other items are left out.
fn strings(reply: &Value, key: &str) -> Vec<String> {
    reply[key]
        .as_array()
        .map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

/// Read `rig spec validate --json`: `{valid, errors, advisories?}`, or
/// `{error}` when the validator itself failed.
fn parse_validation(stdout: &str) -> Result<SpecValidation, RigCallError> {
    let reply = reply_object(stdout).ok_or("OpenRig CLI returned an unreadable reply.")?;
    let Some(valid) = reply["valid"].as_bool() else {
        return Err("OpenRig could not finish checking the spec.".into());
    };
    Ok(SpecValidation {
        valid,
        errors: strings(&reply, "errors").iter().map(|line| spec::Issue::parse(line)).collect(),
        advisories: strings(&reply, "advisories"),
    })
}

/// Read `rig spec preflight --json`: `{ready, warnings, errors}`. Without
/// a daemon rig prints no JSON and fails.
fn parse_preflight(success: bool, stdout: &str) -> Result<Preflight, RigCallError> {
    let Some(reply) = reply_object(stdout) else {
        let message = if success {
            "OpenRig CLI returned an unreadable reply."
        } else {
            "Start the OpenRig daemon, then check again."
        };
        return Err(message.into());
    };
    Ok(Preflight {
        ready: reply["ready"].as_bool().unwrap_or(false),
        warnings: strings(&reply, "warnings"),
        errors: strings(&reply, "errors"),
    })
}

/// Read `rig up --json`. A failure explains itself in `error` (text, or
/// `{fact, consequence, action}`), `message` or `errors`.
fn parse_up(success: bool, stdout: &str) -> Result<UpReply, RigCallError> {
    let reply = reply_object(stdout).ok_or("OpenRig CLI returned an unreadable reply.")?;
    let rig_result = reply["rigResult"].as_str().map(str::to_owned);
    let failed_result = matches!(rig_result.as_deref(), Some("partially_restored" | "failed" | "not_attempted"))
        || reply["status"] == "failed";
    let error = &reply["error"];
    let message = error
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            let parts = ["fact", "action"].map(|key| error[key].as_str().unwrap_or_default());
            let text = parts.iter().filter(|part| !part.is_empty()).copied().collect::<Vec<_>>().join(" ");
            (!text.is_empty()).then_some(text)
        })
        .or_else(|| reply["message"].as_str().map(str::to_owned));
    Ok(UpReply {
        ok: success && reply["ok"] != false && !failed_result,
        rig_result,
        message,
        errors: strings(&reply, "errors"),
        warnings: strings(&reply, "warnings"),
    })
}

/// Accept only a local HTTP origin with an explicit port, without resolving a hostname.
fn local_origin(url: &str) -> Option<SocketAddr> {
    let origin = url.strip_prefix("http://")?.trim_end_matches('/');
    if let Some(port) = origin.strip_prefix("localhost:") {
        let port = port.parse::<u16>().ok().filter(|port| *port != 0)?;
        return Some(SocketAddr::from(([127, 0, 0, 1], port)));
    }
    let endpoint: SocketAddr = origin.parse().ok()?;
    (endpoint.ip().is_loopback() && endpoint.port() != 0).then_some(endpoint)
}

/// Match a team by its stored ID, which survives a daemon restart.
fn has_rig(status: &Value, id: &str) -> bool {
    status["rigs"].as_array().is_some_and(|rigs| rigs.iter().any(|rig| rig["id"] == id))
}

/// Read both `OpenRig` node-list reply shapes, including the partial-list flag.
fn parse_seats(reply: &Value, partial: bool) -> Result<(Vec<Seat>, bool), String> {
    let entries = reply.as_array().or_else(|| reply["entries"].as_array()).ok_or("OpenRig returned no seat list.")?;
    let seats = entries
        .iter()
        .filter_map(|entry| {
            let address = entry["canonicalSessionName"].as_str()?;
            let rig_id = entry["rigId"].as_str()?;
            (entry["nodeKind"] == "agent" && valid_seat(address) && !rig_id.is_empty())
                .then(|| Seat { address: address.to_owned(), rig_id: rig_id.to_owned() })
        })
        .collect();
    Ok((seats, partial || reply["truncated"] == true))
}

/// Accept a local seat address with exactly one seat and team component.
fn valid_seat(address: &str) -> bool {
    address.split_once('@').is_some_and(|(seat, team)| valid_token(seat) && valid_token(team))
}

/// A safe identifier for an argument or a cache filename.
fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && !token.starts_with('-')
        && token.len() <= 255
        && token.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// A local queue identifier, including the IDs Rubrique creates.
fn valid_task_id(id: &str) -> bool {
    id.starts_with("qitem-") && valid_token(id)
}

/// Validate a queue row against the ID that was requested.
fn parse_task(row: &Value, id: &str) -> Result<TaskStatus, String> {
    let Some(state) = row["state"].as_str().filter(|_| row["qitemId"] == id) else {
        return Err("OpenRig did not return the requested queue item.".into());
    };
    let closure = row["closureReason"].as_str().or_else(|| row["closingTransition"]["closureReason"].as_str());
    let state = if closure == Some("handed_off_to") { "handed-off" } else { state };
    Ok(TaskStatus {
        id: id.to_owned(),
        seat: row["destinationSession"].as_str().unwrap_or_default().to_owned(),
        state: state.to_owned(),
        claimed: row["claimedAt"].as_str().is_some(),
        closure: closure.map(str::to_owned),
        resolution: row["resolution"].as_str().map(str::to_owned),
        handoff_note: None,
    })
}

/// Follow a successor only if `OpenRig` names exactly one local queue item.
fn next_task_id(row: &Value) -> Option<&str> {
    let [successor] = row["successors"].as_array()?.as_slice() else { return None };
    successor["qitemId"].as_str().filter(|id| row["successorsTruncated"] != true && valid_task_id(id))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use serde_json::json;

    use super::*;
    use crate::remote::testing::use_fake_ssh;
    use crate::test_support::TempFolder;

    /// Exercise the pinned CLI exchange with a child process that loses the create reply.
    #[test]
    fn recovers_a_lost_send_and_tracks_the_same_task_after_reopen() {
        let temp = TempFolder::new();
        let rig = fake_rig(&temp);
        let folder = temp.make_folder("saved");
        let body = "Review café\nKeep $(printf literal) as text\n";
        let task = TaskLink {
            id: store::new_id().unwrap(),
            document: temp.path().join("A step #1.md"),
            host: RigHost::Local,
            endpoint: "127.0.0.1:7433".parse().unwrap(),
            seat: Seat { address: "builder@demo".into(), rig_id: "demo-id".into() },
        };

        store::save(&folder, &task).unwrap();
        let sent = rig.send(&task, body.into()).unwrap();
        assert_eq!(sent.state, "pending");
        assert_eq!(fs::read_to_string(temp.path().join("received")).unwrap(), body);

        temp.write("handoff", "");
        let saved = store::load(&folder).unwrap().pop().unwrap();
        assert_eq!(saved.document, task.document);
        assert_eq!(saved.id, task.id);
        assert_eq!(saved.seat, task.seat);
        let completed = fake_rig(&temp).read(&saved).unwrap();
        assert_eq!(completed.id, "qitem-next");
        assert_eq!(completed.state, "done");
        assert_eq!(completed.resolution.as_deref(), Some("Reviewed"));
        assert_eq!(fs::read_to_string(temp.path().join("creates")).unwrap(), "create\n");
        let saved_json = fs::read_to_string(folder.join(format!("{}.json", task.id))).unwrap();
        assert!(!saved_json.contains("Review café"));
    }

    /// Changing the daemon's team roster must prevent write and read requests.
    #[test]
    fn refuses_a_different_team_at_the_saved_address() {
        let temp = TempFolder::new();
        let rig = fake_rig(&temp);
        temp.write("other-team", "");
        let task = TaskLink {
            id: "qitem-old".into(),
            document: temp.path().join("note.md"),
            host: RigHost::Local,
            endpoint: "127.0.0.1:7433".parse().unwrap(),
            seat: Seat { address: "builder@demo".into(), rig_id: "demo-id".into() },
        };
        assert!(rig.send(&task, "Review this".into()).unwrap_err().contains("original OpenRig team"));
        assert!(rig.read(&task).unwrap_err().contains("original OpenRig team"));
        assert!(!temp.path().join("creates").exists());
    }

    /// Include only local agent seats from an `OpenRig` node-list reply.
    #[test]
    fn filters_non_agent_and_remote_inventory_entries() {
        let reply = json!({"entries": [
            {"nodeKind": "agent", "rigId": "demo-id", "canonicalSessionName": "dev@demo"},
            {"nodeKind": "agent", "rigId": "remote-id", "canonicalSessionName": "dev@demo@host"},
            {"nodeKind": "infrastructure", "rigId": "demo-id", "canonicalSessionName": "tracer@demo"},
        ], "truncated": true});
        let (seats, partial) = parse_seats(&reply, false).unwrap();
        assert_eq!(seats, [Seat { address: "dev@demo".into(), rig_id: "demo-id".into() }]);
        assert!(partial);
        assert!(local_origin("http://127.0.0.1:7433").is_some());
        assert!(local_origin("http://192.0.2.1:7433").is_none());
        assert!(local_origin("http://127.0.0.1:7433/path").is_none());
    }

    /// Discover a seat through the CLI using its projected stable address.
    #[test]
    fn discovers_seats_on_the_selected_daemon() {
        let temp = TempFolder::new();
        let endpoint = "127.0.0.1:7433".parse().unwrap();
        let inventory = fake_rig(&temp).inventory_at(endpoint).unwrap();
        assert_eq!(inventory.endpoint, endpoint);
        assert_eq!(inventory.seats, [Seat { address: "builder@demo".into(), rig_id: "demo-id".into() }]);
        assert!(!inventory.partial);
    }

    /// Do not follow a remote, ambiguous or truncated handoff.
    #[test]
    fn follows_only_one_local_successor() {
        let local = json!({"successors": [{"qitemId": "qitem-local"}]});
        let remote = json!({"successors": [{"qitemId": "qitem-local@elsewhere"}]});
        let several = json!({"successors": [{"qitemId": "qitem-local"}, {"qitemId": "qitem-other"}]});
        let truncated = json!({"successors": [{"qitemId": "qitem-local"}], "successorsTruncated": true});
        assert_eq!(next_task_id(&local), Some("qitem-local"));
        for reply in [remote, several, truncated] {
            assert_eq!(next_task_id(&reply), None);
        }
    }

    /// Find seats on a remote host through ssh, with the daemon port the host reports.
    #[test]
    fn discovers_seats_on_a_remote_rig_over_ssh() {
        use_fake_ssh();
        let temp = TempFolder::new();
        temp.write("partial", "");
        let host = RigHost::Ssh("rig-host".into());
        let inventory = fake_rig_at(&temp, host.clone()).inventory().unwrap();
        assert_eq!(inventory.host, host);
        assert_eq!(inventory.endpoint, "127.0.0.1:7433".parse().unwrap());
        assert_eq!(inventory.seats, [Seat { address: "builder@demo".into(), rig_id: "demo-id".into() }]);
        assert!(inventory.partial, "the stderr warning marks the list partial");
    }

    /// Send through ssh with the body on stdin, recover the lost reply, then read
    /// the task only from the computer it was sent to.
    #[test]
    fn sends_and_reads_a_task_on_a_remote_rig() {
        use_fake_ssh();
        let temp = TempFolder::new();
        let host = RigHost::Ssh("user@rig-host".into());
        let rig = fake_rig_at(&temp, host.clone());
        let body = "Review café\nKeep $(printf literal) and 'quotes' as text\n";
        let task = TaskLink {
            id: store::new_id().unwrap(),
            document: temp.path().join("plan.md"),
            host,
            endpoint: "127.0.0.1:7433".parse().unwrap(),
            seat: Seat { address: "builder@demo".into(), rig_id: "demo-id".into() },
        };
        let sent = rig.send(&task, body.into()).unwrap();
        assert_eq!(sent.state, "pending");
        assert_eq!(fs::read_to_string(temp.path().join("received")).unwrap(), body);

        temp.write("handoff", "");
        let completed = rig.read(&task).unwrap();
        assert_eq!(completed.id, "qitem-next");
        assert_eq!(completed.resolution.as_deref(), Some("Reviewed"));
        assert!(fake_rig(&temp).read(&task).unwrap_err().contains("another computer"));
        assert_eq!(fs::read_to_string(temp.path().join("creates")).unwrap(), "create\n");
    }

    /// A host without rig says so; a host name that looks like an option never reaches ssh.
    #[test]
    fn reports_a_missing_remote_rig_and_rejects_bad_hosts() {
        use_fake_ssh();
        let temp = TempFolder::new();
        let rig = Rig { program: temp.path().join("no-rig-here"), host: RigHost::Ssh("rig-host".into()) };
        let error = rig.inventory().err().unwrap();
        assert!(error.contains("missing on rig-host"), "{error}");
        assert!(Rig::open(&RigHost::Ssh("-oProxyCommand=touch x".into())).is_err());
        assert!(Rig::open(&RigHost::Ssh("rig-host".into())).is_ok());
    }

    /// Validation answers keep the error paths, with or without a notice line first.
    #[test]
    fn parses_spec_validation_replies() {
        let reply = "notice: update available\n{\"valid\":false,\"errors\":[\"pods[0].members[1].id: must not contain dots\",\"pods: required non-empty array\"],\"advisories\":[\"a\"]}";
        let validation = parse_validation(reply).unwrap();
        assert!(!validation.valid);
        assert_eq!(validation.errors[0], spec::Issue::at("pods[0].members[1].id", "must not contain dots"));
        assert_eq!(validation.errors[1].path, "pods");
        assert_eq!(validation.advisories, ["a"]);
        assert!(parse_validation("{\"valid\":true,\"errors\":[]}").unwrap().valid);
        assert!(parse_validation("{\"error\":\"Internal validator error\"}").is_err());
        assert!(parse_validation("not json").is_err());
    }

    /// Preflight without a daemon prints no JSON; with one it lists warnings and errors.
    #[test]
    fn parses_preflight_replies() {
        let preflight =
            parse_preflight(false, "{\"ready\":false,\"warnings\":[\"w\"],\"errors\":[\"agent.yaml missing\"]}")
                .unwrap();
        assert_eq!(
            preflight,
            Preflight { ready: false, warnings: vec!["w".into()], errors: vec!["agent.yaml missing".into()] }
        );
        let error = parse_preflight(false, "").unwrap_err();
        assert_eq!(error, RigCallError::Failed("Start the OpenRig daemon, then check again.".into()));
    }

    /// A launch reply is ok only when rig and the daemon both say so, and a failure explains itself.
    #[test]
    fn parses_up_replies() {
        let launched = parse_up(true, "{\"ok\":true,\"rigResult\":\"launched\"}").unwrap();
        assert!(launched.ok);
        assert_eq!(launched.rig_result.as_deref(), Some("launched"));
        let refused = parse_up(false, "{\"code\":\"preflight_failed\",\"errors\":[\"cwd missing\"]}").unwrap();
        assert!(!refused.ok);
        assert_eq!(refused.errors, ["cwd missing"]);
        let three_part = "{\"ok\":false,\"error\":{\"fact\":\"A rig named starter is running.\",\"consequence\":\"x\",\"action\":\"Stop it first.\"}}";
        let refused = parse_up(false, three_part).unwrap();
        assert_eq!(refused.message.as_deref(), Some("A rig named starter is running. Stop it first."));
        let partial = parse_up(false, "{\"rigResult\":\"partially_restored\"}").unwrap();
        assert!(!partial.ok);
        let blocked_plan =
            parse_up(true, "{\"status\":\"failed\",\"errors\":[\"no agent\"],\"warnings\":[\"w\"]}").unwrap();
        assert!(!blocked_plan.ok);
        assert_eq!((blocked_plan.errors, blocked_plan.warnings), (vec!["no agent".to_owned()], vec!["w".to_owned()]));
        assert!(parse_up(true, "{\"status\":\"planned\",\"warnings\":[]}").unwrap().ok);
    }

    /// The spec path reaches rig as one argument, even with spaces and quotes,
    /// and a failed validation still returns its answer.
    #[test]
    fn validates_a_spec_through_the_cli() {
        let temp = TempFolder::new();
        let spec = temp.write("my team/rig 'x'.yaml", "name: x\n");
        let program = temp.write(
            "rig",
            r#"#!/bin/sh
test "$1" = spec && test "$2" = validate && test "$4" = --json && test -f "$3" || exit 3
printf '{"valid":false,"errors":["name: required non-empty string"]}\n'
exit 1
"#,
        );
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let rig = Rig { program, host: RigHost::Local };
        let validation = rig.spec_validate(&spec).unwrap();
        assert!(!validation.valid);
        assert_eq!(validation.errors[0].path, "name");
        assert!(rig.spec_validate(Path::new("relative.yaml")).is_err());
    }

    /// A host without rig is a distinct answer, so the editor can say how to install it.
    #[test]
    fn reports_rig_missing_on_a_remote_host() {
        use_fake_ssh();
        let temp = TempFolder::new();
        let rig = Rig { program: temp.path().join("no-rig-here"), host: RigHost::Ssh("rig-host".into()) };
        let error = rig.spec_validate(Path::new("/specs/rig.yaml")).unwrap_err();
        assert!(matches!(error, RigCallError::NotInstalled(message) if message.contains("missing on rig-host")));
    }

    /// The local fake rig.
    fn fake_rig(temp: &TempFolder) -> Rig {
        fake_rig_at(temp, RigHost::Local)
    }

    /// Simulate `OpenRig`'s JSON, argument contract and queue state on disk.
    fn fake_rig_at(temp: &TempFolder, host: RigHost) -> Rig {
        let program = temp.write(
            "rig",
            r#"#!/bin/sh
set -eu
cd "${0%/*}"
test -z "${RIGGED_URL:-}"
test -z "${TMUX_PANE:-}"
if [ -z "${OPENRIG_URL:-}" ]; then
    test "$1" = status
    printf '{"daemon":{"healthy":true,"reachable":true,"port":7433},"rigs":[]}\n'
    exit 0
fi
test "$OPENRIG_URL" = http://127.0.0.1:7433
test "$OPENRIG_HOST_SELECTED" = local
if [ "$1" = status ]; then
    team=demo-id
    if [ -e other-team ]; then team=other-id; fi
    printf '{"daemon":{"healthy":true,"reachable":true},"rigs":[{"id":"%s"}]}\n' "$team"
elif [ "$1" = ps ]; then
    test "$2" = --nodes
    test "$3" = --all-rigs
    test "$4" = --no-cleanup
    if [ -e partial ]; then echo 'Warning: failed to fetch nodes' >&2; fi
    printf '[{"nodeKind":"agent","rigId":"demo-id","canonicalSessionName":"builder@demo"}]\n'
elif [ "$1" = queue ] && [ "$2" = create ]; then
    test "$3" = --source
    test "$4" = rubrique
    test "$5" = --destination=builder@demo
    test "$7" = --body-file
    test "$8" = -
    printf 'create\n' >> creates
    printf '%s' "${6#--id=}" > saved-id
    cat > received
    exit 1
elif [ "$1" = queue ] && [ "$2" = show ]; then
    if [ "$3" = qitem-next ]; then
        printf '{"qitemId":"qitem-next","state":"done","destinationSession":"reviewer@demo","resolution":"Reviewed"}\n'
    else
        id=$(cat saved-id)
        test "$id" = "$3"
        if [ -e handoff ]; then
            printf '{"qitemId":"%s","state":"done","closureReason":"handed_off_to","successors":[{"qitemId":"qitem-next"}]}\n' "$id"
        else
            printf '{"qitemId":"%s","state":"pending","destinationSession":"builder@demo"}\n' "$id"
        fi
    fi
else
    exit 3
fi
"#,
        );
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        Rig { program, host }
    }
}
