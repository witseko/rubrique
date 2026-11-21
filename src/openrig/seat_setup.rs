//! Add a seat to a running `OpenRig` team and change how a seat runs, through
//! rig. `OpenRig` keeps the seat's setup; Rubrique stores none of it.

use std::net::SocketAddr;
use std::time::Duration;

use serde_json::Value;

use super::{Rig, valid_seat, valid_token};

/// Seat setup steps wait for the agent to start (rig's readiness window is
/// 30 s, and `rig send` waits up to 60 s for the pane), so they get longer
/// than the other rig calls.
const SETUP_TIMEOUT: Duration = Duration::from_secs(90);
/// The audit reason rig records with each change.
const REASON: &str = "--reason=Rubrique seat setup";
/// Longest first instructions, in bytes. They travel as one argument, also
/// inside the SSH command line, which Linux limits to 128 KiB.
pub const MAX_INSTRUCTIONS_BYTES: usize = 16_000;
/// Longest working folder path, in bytes.
const MAX_FOLDER_BYTES: usize = 4096;
/// Most characters of rig's own error text shown in a failure message.
const MAX_RIG_MESSAGE: usize = 200;

/// The agent program a seat runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runtime {
    ClaudeCode,
    Codex,
    Pi,
}

impl Runtime {
    /// Every runtime a new seat can have, in menu order.
    pub const ALL: [Runtime; 3] = [Runtime::ClaudeCode, Runtime::Codex, Runtime::Pi];

    /// rig's name for the runtime.
    pub fn id(self) -> &'static str {
        match self {
            Runtime::ClaudeCode => "claude-code",
            Runtime::Codex => "codex",
            Runtime::Pi => "pi",
        }
    }

    /// The name shown in the dialog.
    pub fn name(self) -> &'static str {
        match self {
            Runtime::ClaudeCode => "Claude Code",
            Runtime::Codex => "Codex",
            Runtime::Pi => "Pi",
        }
    }

    /// The reasoning effort levels the runtime takes, lowest first.
    pub fn effort_levels(self) -> &'static [&'static str] {
        match self {
            Runtime::ClaudeCode | Runtime::Codex => &["low", "medium", "high"],
            Runtime::Pi => &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
        }
    }

    /// The runtime rig calls `id`; `None` for one Rubrique cannot set up.
    pub fn from_id(id: &str) -> Option<Runtime> {
        Runtime::ALL.into_iter().find(|runtime| runtime.id() == id)
    }
}

/// The native permissions a seat starts with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionMode {
    /// Whatever the rig's policy says.
    Inherit,
    /// The runtime's guarded mode.
    Floor,
    /// No permission prompts at all.
    FullBypass,
}

impl PermissionMode {
    /// Every mode, in menu order.
    pub const ALL: [PermissionMode; 3] = [PermissionMode::Inherit, PermissionMode::Floor, PermissionMode::FullBypass];

    /// rig's name for the mode.
    pub fn id(self) -> &'static str {
        match self {
            PermissionMode::Inherit => "inherit",
            PermissionMode::Floor => "floor",
            PermissionMode::FullBypass => "full_bypass",
        }
    }

    /// The name shown in the dialog.
    pub fn name(self) -> &'static str {
        match self {
            PermissionMode::Inherit => "Inherit from the rig",
            PermissionMode::Floor => "Floor (guarded)",
            PermissionMode::FullBypass => "Full bypass",
        }
    }

    /// The mode rig calls `id`.
    fn from_id(id: &str) -> Option<PermissionMode> {
        PermissionMode::ALL.into_iter().find(|mode| mode.id() == id)
    }
}

/// What a seat runs with. `None` leaves a value to `OpenRig`: the runtime's
/// default when adding, unchanged when editing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeatSettings {
    pub model: Option<String>,
    /// For effort only, `None` on edit clears the seat's choice.
    pub effort: Option<String>,
    pub permissions: Option<PermissionMode>,
    /// An absolute folder on the rig's machine.
    pub folder: Option<String>,
}

/// A new seat: where it goes, what it runs and what it is told first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeatSetup {
    /// The member name; the seat's address becomes `<pod>-<name>@<rig>`.
    pub name: String,
    pub pod: String,
    /// Make `pod` a new pod instead of joining an existing one.
    pub new_pod: bool,
    pub runtime: Runtime,
    pub settings: SeatSettings,
    /// Text sent to the new seat once it runs; blank sends nothing.
    pub instructions: String,
}

/// An existing seat's setup as `OpenRig` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeatState {
    /// rig's runtime name, which may be one Rubrique cannot add (such as `opencode`).
    pub runtime: String,
    pub settings: SeatSettings,
}

/// One rig call of a setup, with the words a failure message uses for it.
#[derive(Debug, PartialEq, Eq)]
struct Step {
    name: &'static str,
    args: Vec<String>,
}

impl Step {
    /// The step `name` running rig with `args`.
    fn new(name: &'static str, args: &[&str]) -> Step {
        Step { name, args: args.iter().map(|arg| (*arg).to_owned()).collect() }
    }
}

impl Rig {
    /// Add `setup` as a seat of the rig named `rig_name`, apply its
    /// settings with a fresh launch, then send its first instructions.
    /// Returns the new seat's address.
    ///
    /// # Errors
    ///
    /// Invalid input, or the step that failed and the steps done before it.
    pub fn add_seat(&self, rig_name: &str, setup: &SeatSetup) -> Result<String, String> {
        validate(setup)?;
        let seat = seat_address(rig_name, setup);
        if !valid_seat(&seat) {
            return Err("This rig's name cannot be part of a seat address.".into());
        }
        let endpoint = self.endpoint()?;
        let status = self.status(Some(endpoint))?;
        let rig_id = rig_id_by_name(&status, rig_name).ok_or("OpenRig does not list this rig any more.")?;
        self.run_steps(endpoint, &add_steps(rig_id, &seat, setup))?;
        Ok(seat)
    }

    /// Read seat `seat`'s runtime, model, effort choice, permissions and
    /// working folder.
    ///
    /// # Errors
    ///
    /// The seat address is invalid, or rig did not answer with the seat.
    pub fn read_setup(&self, seat: &str) -> Result<SeatState, String> {
        if !valid_seat(seat) {
            return Err("This is not an OpenRig seat address.".into());
        }
        let endpoint = Some(self.endpoint()?);
        let session = format!("--session={seat}");
        let node_args = ["ps", "--nodes", "--all-rigs", "--no-cleanup", &session, "--json", "--fields=runtime,model"];
        let (nodes, _) = self.json(endpoint, &node_args, None)?;
        let (status, _) = self.json(endpoint, &["seat", "status", seat, "--json"], None)?;
        // rig refuses the effort question for a runtime without effort levels, such as `terminal`.
        let has_effort = seat_runtime(&nodes, &status).and_then(Runtime::from_id).is_some();
        let effort =
            if has_effort { self.json(endpoint, &["seat", "effort", seat, "--json"], None)?.0 } else { Value::Null };
        parse_state(&nodes, &effort, &status)
    }

    /// Save the settings of seat `seat` that differ between `before` and
    /// `after`, then restart it with a blank conversation when `restart`.
    /// Runs nothing when there is nothing to do. Returns whether it ran
    /// anything.
    ///
    /// # Errors
    ///
    /// Invalid input, or the step that failed and the steps done before it.
    pub fn save_setup(
        &self,
        seat: &str,
        before: &SeatSettings,
        after: &SeatSettings,
        restart: bool,
    ) -> Result<bool, String> {
        validate_settings(after)?;
        if !valid_seat(seat) {
            return Err("This is not an OpenRig seat address.".into());
        }
        let steps = edit_steps(seat, before, after, restart);
        if steps.is_empty() {
            return Ok(false);
        }
        let endpoint = self.endpoint()?;
        self.run_steps(endpoint, &steps)?;
        Ok(true)
    }

    /// Run `steps` in order and stop at the first one that fails.
    fn run_steps(&self, endpoint: SocketAddr, steps: &[Step]) -> Result<(), String> {
        for (index, step) in steps.iter().enumerate() {
            let args: Vec<&str> = step.args.iter().map(String::as_str).collect();
            let detail = match self.run(Some(endpoint), &args, None, SETUP_TIMEOUT) {
                Ok(reply) if reply.success => continue,
                Ok(reply) => rig_message(&reply.stdout),
                Err(error) => Some(error),
            };
            return Err(step_failure(step, &steps[..index], detail.as_deref()));
        }
        Ok(())
    }
}

/// Check a new seat before any rig call.
///
/// # Errors
///
/// A short sentence about the first value that cannot be used.
pub fn validate(setup: &SeatSetup) -> Result<(), String> {
    if !valid_token(&setup.name) {
        return Err("Name the seat with letters, digits, -, _ or . (not starting with -).".into());
    }
    if !valid_token(&setup.pod) {
        return Err("Name the pod with letters, digits, -, _ or . (not starting with -).".into());
    }
    if setup.instructions.len() > MAX_INSTRUCTIONS_BYTES {
        return Err("The first instructions are too long. Use up to 16 KB.".into());
    }
    validate_settings(&setup.settings)
}

/// Check seat settings before any rig call.
///
/// # Errors
///
/// A short sentence about the first value that cannot be used.
pub fn validate_settings(settings: &SeatSettings) -> Result<(), String> {
    if settings.model.as_deref().is_some_and(|model| !valid_model(model)) {
        return Err("Use a model id such as gpt-5 or provider/model.".into());
    }
    if settings.effort.as_deref().is_some_and(|effort| !valid_token(effort)) {
        return Err("Choose a reasoning effort from the list.".into());
    }
    if settings.folder.as_deref().is_some_and(|folder| !valid_folder(folder)) {
        return Err("Use an absolute folder on the rig's computer, such as /home/me/project.".into());
    }
    Ok(())
}

/// A model id: the token characters plus `/` (for `provider/model`) and `:`.
fn valid_model(model: &str) -> bool {
    let bare = model.replace(['/', ':'], "_");
    !model.starts_with(['/', ':']) && valid_token(&bare)
}

/// An absolute path without control characters, of a bounded length.
fn valid_folder(folder: &str) -> bool {
    folder.starts_with('/') && folder.len() <= MAX_FOLDER_BYTES && !folder.chars().any(char::is_control)
}

/// The address rig gives a new seat: `<pod>-<name>@<rig>`.
fn seat_address(rig_name: &str, setup: &SeatSetup) -> String {
    format!("{}-{}@{rig_name}", setup.pod, setup.name)
}

/// The ID of the rig named `name` in a `rig status` reply.
fn rig_id_by_name<'a>(status: &'a Value, name: &str) -> Option<&'a str> {
    let rigs = status["rigs"].as_array()?;
    let rig = rigs.iter().find(|rig| rig["name"] == name)?;
    rig["id"].as_str().filter(|id| valid_token(id))
}

/// The rig calls that add seat `seat` to rig `rig_id`: grow it with its
/// runtime and folder, save the other settings, relaunch it so they apply
/// (it is seconds old, so no conversation is lost), then send the first
/// instructions.
fn add_steps(rig_id: &str, seat: &str, setup: &SeatSetup) -> Vec<Step> {
    let pod_flag = if setup.new_pod { "--new-pod" } else { "--pod" };
    let pod = format!("{pod_flag}={}", setup.pod);
    let runtime = format!("--runtime={}", setup.runtime.id());
    let folder = setup.settings.folder.as_ref().map(|folder| format!("--cwd={folder}"));
    let mut grow = vec!["grow", rig_id, &setup.name, &pod, &runtime];
    grow.extend(folder.as_deref());
    grow.push("--json");
    let mut steps = vec![Step::new("add the seat", &grow)];
    let later = SeatSettings { folder: None, ..setup.settings.clone() };
    let changes = change_steps(seat, &SeatSettings::default(), &later);
    if !changes.is_empty() {
        steps.extend(changes);
        steps.push(launch_step(seat));
    }
    let instructions = setup.instructions.trim();
    if !instructions.is_empty() {
        steps.push(Step::new("send the first instructions", &["send", "--json", seat, "--", instructions]));
    }
    steps
}

/// The rig calls that save what changed from `before` to `after`, then
/// restart the seat when `restart`.
fn edit_steps(seat: &str, before: &SeatSettings, after: &SeatSettings, restart: bool) -> Vec<Step> {
    let mut steps = change_steps(seat, before, after);
    if restart {
        steps.push(launch_step(seat));
    }
    steps
}

/// One `rig seat set-*` call per setting that differs. A model, permission
/// mode or folder left out stays as it is; an effort left out clears the
/// seat's choice.
fn change_steps(seat: &str, before: &SeatSettings, after: &SeatSettings) -> Vec<Step> {
    let mut steps = Vec::new();
    if let Some(model) = after.model.as_deref().filter(|_| after.model != before.model) {
        let model = format!("--model={model}");
        steps.push(Step::new("set the model", &["seat", "set-model", seat, &model, REASON, "--json"]));
    }
    if after.effort != before.effort {
        let effort = after.effort.as_ref().map_or_else(|| "--clear".to_owned(), |level| format!("--effort={level}"));
        steps.push(Step::new("set the reasoning effort", &["seat", "set-effort", seat, &effort, REASON, "--json"]));
    }
    if let Some(mode) = after.permissions.filter(|mode| before.permissions != Some(*mode)) {
        let mode = format!("--mode={}", mode.id());
        steps.push(Step::new("set the permissions", &["seat", "set-permissions", seat, &mode, REASON, "--json"]));
    }
    if let Some(folder) = after.folder.as_deref().filter(|_| after.folder != before.folder) {
        let folder = format!("--cwd={folder}");
        steps.push(Step::new("set the working folder", &["seat", "set-cwd", seat, &folder, REASON, "--json"]));
    }
    steps
}

/// Stop the seat's agent and start a fresh one with the saved settings.
fn launch_step(seat: &str) -> Step {
    Step::new("restart the seat", &["seat", "launch", seat, "--fresh", "--stop", REASON, "--json"])
}

/// Read a seat's setup from the replies of `rig ps`, `rig seat effort` and
/// `rig seat status`. A value that is missing or unusable reads as `None`,
/// so it changes only when the user sets it.
fn parse_state(nodes: &Value, effort: &Value, status: &Value) -> Result<SeatState, String> {
    let Some(node) = seat_node(nodes) else {
        return Err("OpenRig does not list this seat.".into());
    };
    let runtime = seat_runtime(nodes, status).unwrap_or_default();
    let settings = SeatSettings {
        model: node["model"].as_str().filter(|model| valid_model(model)).map(str::to_owned),
        effort: effort["saved"]["choice"].as_str().filter(|level| valid_token(level)).map(str::to_owned),
        permissions: status["permissions"]["selectionState"].as_str().and_then(PermissionMode::from_id),
        folder: status["launch_cwd"].as_str().filter(|folder| valid_folder(folder)).map(str::to_owned),
    };
    Ok(SeatState { runtime: runtime.to_owned(), settings })
}

/// The one node of a `rig ps` reply for a single seat, in either reply shape.
fn seat_node(nodes: &Value) -> Option<&Value> {
    let entries = nodes.as_array().or_else(|| nodes["entries"].as_array())?;
    let [node] = entries.as_slice() else { return None };
    Some(node)
}

/// The seat's runtime name, from its node or else from `rig seat status`.
fn seat_runtime<'a>(nodes: &'a Value, status: &'a Value) -> Option<&'a str> {
    seat_node(nodes).and_then(|node| node["runtime"].as_str()).or_else(|| status["runtime"].as_str())
}

/// rig's own `message` or `error` text from its JSON stdout, on one line and
/// shortened. Never stderr, which can hold private task data.
fn rig_message(stdout: &str) -> Option<String> {
    let reply: Value = serde_json::from_str(stdout.trim()).ok()?;
    let text =
        ["message", "error"].iter().find_map(|key| reply[key].as_str().or_else(|| reply[key]["message"].as_str()))?;
    let line: String = text
        .chars()
        .map(|character| if character.is_control() { ' ' } else { character })
        .take(MAX_RIG_MESSAGE)
        .collect();
    Some(line.trim().to_owned()).filter(|line| !line.is_empty())
}

/// Which step failed, which steps finished before it, and rig's reason.
fn step_failure(failed: &Step, done: &[Step], detail: Option<&str>) -> String {
    let mut parts = vec![format!("Could not {}.", failed.name)];
    if !done.is_empty() {
        let names: Vec<&str> = done.iter().map(|step| step.name).collect();
        parts.push(format!("Done before it: {}.", names.join(", ")));
    }
    parts.push(detail.map_or_else(|| "OpenRig gave no reason.".to_owned(), |detail| format!("OpenRig: {detail}")));
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use serde_json::json;

    use super::*;
    use crate::openrig::RigHost;
    use crate::remote::testing::use_fake_ssh;
    use crate::test_support::TempFolder;

    /// A Codex seat in pod `dev` of rig `demo`, with the given settings.
    fn setup(settings: SeatSettings, instructions: &str) -> SeatSetup {
        SeatSetup {
            name: "impl".into(),
            pod: "dev".into(),
            new_pod: false,
            runtime: Runtime::Codex,
            settings,
            instructions: instructions.into(),
        }
    }

    /// A terminal seat has no effort levels, so its effort is not asked for.
    #[test]
    fn a_terminal_seat_has_no_known_runtime() {
        let nodes = json!({"entries": [{"runtime": "terminal", "model": null}]});
        let runtime = seat_runtime(&nodes, &Value::Null);
        assert_eq!(runtime, Some("terminal"));
        assert_eq!(runtime.and_then(Runtime::from_id), None);
        assert_eq!(seat_runtime(&json!({"entries": []}), &json!({"runtime": "codex"})), Some("codex"));
    }

    /// Settings with every field filled.
    fn filled() -> SeatSettings {
        SeatSettings {
            model: Some("zai/glm-5.3".into()),
            effort: Some("medium".into()),
            permissions: Some(PermissionMode::Floor),
            folder: Some("/work/app".into()),
        }
    }

    /// Each step as its arguments joined by spaces.
    fn lines(steps: &[Step]) -> Vec<String> {
        steps.iter().map(|step| step.args.join(" ")).collect()
    }

    /// A filled form grows the seat in its folder, saves each setting,
    /// relaunches it and sends the instructions after `--`.
    #[test]
    fn add_steps_for_a_filled_setup() {
        let steps = add_steps("demo-id", "dev-impl@demo", &setup(filled(), "  --read the plan\n"));
        assert_eq!(
            lines(&steps),
            [
                "grow demo-id impl --pod=dev --runtime=codex --cwd=/work/app --json",
                "seat set-model dev-impl@demo --model=zai/glm-5.3 --reason=Rubrique seat setup --json",
                "seat set-effort dev-impl@demo --effort=medium --reason=Rubrique seat setup --json",
                "seat set-permissions dev-impl@demo --mode=floor --reason=Rubrique seat setup --json",
                "seat launch dev-impl@demo --fresh --stop --reason=Rubrique seat setup --json",
                "send --json dev-impl@demo -- --read the plan",
            ]
        );
    }

    /// An empty form only grows the seat, without a relaunch or a message.
    #[test]
    fn add_steps_for_an_empty_setup() {
        let mut empty = setup(SeatSettings::default(), " \n ");
        empty.new_pod = true;
        let steps = add_steps("demo-id", "dev-impl@demo", &empty);
        assert_eq!(lines(&steps), ["grow demo-id impl --new-pod=dev --runtime=codex --json"]);
    }

    /// Edit saves only what changed, clears a removed effort, keeps a model,
    /// mode or folder left out, and restarts only when asked.
    #[test]
    fn edit_steps_for_changed_and_unchanged_fields() {
        let before = filled();
        assert_eq!(lines(&edit_steps("dev-impl@demo", &before, &before, false)), Vec::<String>::new());
        let left_out = SeatSettings { effort: Some("medium".into()), ..SeatSettings::default() };
        assert_eq!(lines(&edit_steps("dev-impl@demo", &before, &left_out, false)), Vec::<String>::new());
        let after = SeatSettings {
            model: Some("gpt-5".into()),
            effort: None,
            permissions: Some(PermissionMode::FullBypass),
            folder: Some("/work/other".into()),
        };
        assert_eq!(
            lines(&edit_steps("dev-impl@demo", &before, &after, true)),
            [
                "seat set-model dev-impl@demo --model=gpt-5 --reason=Rubrique seat setup --json",
                "seat set-effort dev-impl@demo --clear --reason=Rubrique seat setup --json",
                "seat set-permissions dev-impl@demo --mode=full_bypass --reason=Rubrique seat setup --json",
                "seat set-cwd dev-impl@demo --cwd=/work/other --reason=Rubrique seat setup --json",
                "seat launch dev-impl@demo --fresh --stop --reason=Rubrique seat setup --json",
            ]
        );
        assert_eq!(lines(&edit_steps("dev-impl@demo", &before, &before, true)).len(), 1);
    }

    /// Names, pods and values that start with `-`, relative folders and
    /// characters outside the token rule never reach rig.
    #[test]
    fn validate_refuses_options_relative_folders_and_bad_tokens() {
        assert_eq!(validate(&setup(filled(), "")), Ok(()));
        let bad_setups = [
            SeatSetup { name: "-oProxy".into(), ..setup(filled(), "") },
            SeatSetup { name: "two words".into(), ..setup(filled(), "") },
            SeatSetup { pod: "--json".into(), ..setup(filled(), "") },
            SeatSetup { pod: String::new(), ..setup(filled(), "") },
            setup(filled(), &"x".repeat(MAX_INSTRUCTIONS_BYTES + 1)),
        ];
        for bad in bad_setups {
            assert!(validate(&bad).is_err(), "{bad:?}");
        }
        let bad_settings = [
            SeatSettings { folder: Some("work/app".into()), ..filled() },
            SeatSettings { folder: Some("-work".into()), ..filled() },
            SeatSettings { folder: Some("/work\napp".into()), ..filled() },
            SeatSettings { model: Some("--model=x".into()), ..filled() },
            SeatSettings { model: Some("gpt 5".into()), ..filled() },
            SeatSettings { model: Some("/etc".into()), ..filled() },
            SeatSettings { effort: Some("-high".into()), ..filled() },
        ];
        for bad in bad_settings {
            assert!(validate_settings(&bad).is_err(), "{bad:?}");
        }
        let pi_model = SeatSettings { model: Some("nvidia/nvidia/nemotron-3:latest".into()), ..filled() };
        assert_eq!(validate_settings(&pi_model), Ok(()));
    }

    /// The prefill reads the model, the explicit effort choice, the
    /// permission mode and the launch folder, and skips unusable values.
    #[test]
    fn reads_a_seat_state_from_the_three_replies() {
        let nodes = json!({"entries": [{"runtime": "pi", "model": "zai/glm-5.3"}], "truncated": false});
        let effort = json!({"saved": {"choice": "high", "effective": "high"}});
        let status = json!({"launch_cwd": "/work/app", "permissions": {"selectionState": "full_bypass"}});
        let state = parse_state(&nodes, &effort, &status).unwrap();
        assert_eq!(state.runtime, "pi");
        assert_eq!(
            state.settings,
            SeatSettings {
                model: Some("zai/glm-5.3".into()),
                effort: Some("high".into()),
                permissions: Some(PermissionMode::FullBypass),
                folder: Some("/work/app".into()),
            }
        );
        let nodes = json!([{"runtime": "codex", "model": null}]);
        let effort = json!({"saved": {"choice": null, "effective": "medium"}});
        let status = json!({"launch_cwd": "relative", "permissions": {"selectionState": "custom"}});
        let state = parse_state(&nodes, &effort, &status).unwrap();
        assert_eq!(state.settings, SeatSettings::default());
        assert!(parse_state(&json!({"entries": []}), &effort, &status).is_err());
    }

    /// rig's own reason comes from stdout JSON only, on one shortened line.
    #[test]
    fn failure_messages_name_the_steps_and_rig_reason() {
        assert_eq!(
            rig_message(r#"{"ok":false,"error":"seat_exists","message":"Seat\nexists"}"#).unwrap(),
            "Seat exists"
        );
        assert_eq!(rig_message(r#"{"error":{"message":"No such rig"}}"#).unwrap(), "No such rig");
        assert_eq!(rig_message(&format!(r#"{{"error":"{}"}}"#, "x".repeat(500))).unwrap().len(), MAX_RIG_MESSAGE);
        assert_eq!(rig_message("not json"), None);
        let steps = add_steps("demo-id", "dev-impl@demo", &setup(filled(), ""));
        assert_eq!(
            step_failure(&steps[2], &steps[..2], Some("bad level")),
            "Could not set the reasoning effort. Done before it: add the seat, set the model. OpenRig: bad level"
        );
        assert_eq!(step_failure(&steps[0], &[], None), "Could not add the seat. OpenRig gave no reason.");
    }

    /// Add runs grow, the set commands, the launch and the send in order,
    /// against the rig named in the explorer.
    #[test]
    fn adds_a_seat_with_every_step_in_order() {
        let temp = TempFolder::new();
        let rig = fake_rig(&temp, RigHost::Local);
        let seat = rig.add_seat("demo", &setup(filled(), "Read the plan\nthen wait")).unwrap();
        assert_eq!(seat, "dev-impl@demo");
        assert_eq!(
            calls(&temp),
            [
                "[grow][demo-id][impl][--pod=dev][--runtime=codex][--cwd=/work/app][--json]",
                "[seat][set-model][dev-impl@demo][--model=zai/glm-5.3][--reason=Rubrique seat setup][--json]",
                "[seat][set-effort][dev-impl@demo][--effort=medium][--reason=Rubrique seat setup][--json]",
                "[seat][set-permissions][dev-impl@demo][--mode=floor][--reason=Rubrique seat setup][--json]",
                "[seat][launch][dev-impl@demo][--fresh][--stop][--reason=Rubrique seat setup][--json]",
                "[send][--json][dev-impl@demo][--][Read the plan",
                "then wait]",
            ]
        );
        assert!(rig.add_seat("other", &setup(filled(), "")).unwrap_err().contains("does not list this rig"));
    }

    /// A failing step stops the run and the message says what was done.
    #[test]
    fn a_failing_step_stops_the_run() {
        let temp = TempFolder::new();
        temp.write("fail", "set-effort");
        let error = fake_rig(&temp, RigHost::Local).add_seat("demo", &setup(filled(), "Hello")).unwrap_err();
        assert_eq!(
            error,
            "Could not set the reasoning effort. Done before it: add the seat, set the model. OpenRig: Unknown level"
        );
        let calls = calls(&temp);
        assert_eq!(calls.len(), 3);
        assert!(calls[2].starts_with("[seat][set-effort]"));
    }

    /// Edit with nothing changed and no restart runs no rig call at all.
    #[test]
    fn edit_with_no_change_runs_nothing() {
        let temp = TempFolder::new();
        let rig = fake_rig(&temp, RigHost::Local);
        assert!(!rig.save_setup("dev-impl@demo", &filled(), &filled(), false).unwrap());
        assert!(!temp.path().join("calls").exists());
        let after = SeatSettings { effort: Some("high".into()), ..filled() };
        assert!(rig.save_setup("dev-impl@demo", &filled(), &after, false).unwrap());
        assert_eq!(
            calls(&temp),
            ["[seat][set-effort][dev-impl@demo][--effort=high][--reason=Rubrique seat setup][--json]"]
        );
    }

    /// Over ssh, the seat's setup reads back and an added seat gets every step.
    #[test]
    fn reads_and_adds_a_seat_on_a_remote_rig() {
        use_fake_ssh();
        let temp = TempFolder::new();
        let rig = fake_rig(&temp, RigHost::Ssh("rig-host".into()));
        let state = rig.read_setup("dev-impl@demo").unwrap();
        assert_eq!(state.runtime, "codex");
        assert_eq!(state.settings.model.as_deref(), Some("gpt-5"));
        assert_eq!(state.settings.effort.as_deref(), Some("high"));
        assert_eq!(state.settings.permissions, Some(PermissionMode::Inherit));
        assert_eq!(state.settings.folder.as_deref(), Some("/work/app"));
        fs::remove_file(temp.path().join("calls")).unwrap();
        rig.add_seat("demo", &setup(SeatSettings::default(), "It's $(here) 'quoted'")).unwrap();
        assert_eq!(
            calls(&temp),
            [
                "[grow][demo-id][impl][--pod=dev][--runtime=codex][--json]",
                "[send][--json][dev-impl@demo][--][It's $(here) 'quoted']"
            ]
        );
    }

    /// The rig calls the fake received, one line per call with each
    /// argument in brackets.
    fn calls(temp: &TempFolder) -> Vec<String> {
        fs::read_to_string(temp.path().join("calls")).unwrap().lines().map(str::to_owned).collect()
    }

    /// A rig that logs each call's arguments, answers the reads with fixed
    /// JSON, and fails the call whose second word is in the `fail` file.
    fn fake_rig(temp: &TempFolder, host: RigHost) -> Rig {
        let program = temp.write(
            "rig",
            r#"#!/bin/sh
set -eu
cd "${0%/*}"
test "$OPENRIG_HOST_SELECTED" = local
if [ "$1" = status ]; then
    printf '{"daemon":{"healthy":true,"reachable":true,"port":7433},"rigs":[{"id":"demo-id","name":"demo"}]}\n'
    exit 0
fi
test "$OPENRIG_URL" = http://127.0.0.1:7433
for arg in "$@"; do printf '[%s]' "$arg" >> calls; done
printf '\n' >> calls
if [ -e fail ] && [ "${2:-}" = "$(cat fail)" ]; then
    printf '{"ok":false,"error":"Unknown level"}\n'
    exit 1
fi
case "$1 ${2:-}" in
    "ps --nodes") printf '{"entries":[{"runtime":"codex","model":"gpt-5"}]}\n' ;;
    "seat effort") printf '{"saved":{"choice":"high"}}\n' ;;
    "seat status") printf '{"launch_cwd":"/work/app","permissions":{"selectionState":"inherit"}}\n' ;;
    *) printf '{"ok":true}\n' ;;
esac
"#,
        );
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        Rig { program, host }
    }
}
