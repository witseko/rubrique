//! Text highlight rules written by AI: Settings → Highlights sends a
//! description to the Ask tool, and the pattern it replies with comes back
//! as a draft rule for the user to check before adding it.

use std::fs;

use super::RubriqueApp;
use crate::ai;
use crate::ai::highlight_rule::{name_from_description, pattern_from_reply, pattern_prompt};
use crate::ai::run::{AiRun, RunEnd};
use crate::highlight::text_rules::{HighlightRule, RuleColor};
use crate::settings::highlight_rule_folder;
use crate::settings::text_highlight::TextHighlightSettings;
use crate::settings::view::{HighlightInfo, RuleWriter};

/// The description, the run writing a pattern for it, and its result.
#[derive(Debug, Default)]
pub(super) struct RuleWriterState {
    /// What to highlight, in the user's words.
    pub(super) description: String,
    running: Option<AiRun>,
    /// The rule the AI wrote, waiting in Settings to be added or discarded.
    pub(super) draft: Option<HighlightRule>,
    /// Why the last attempt gave no rule.
    error: Option<String>,
}

impl RuleWriterState {
    /// What the Highlights group shows: the user's `rules`, and this
    /// writer's description, draft and status. `tool_name` is the Ask tool's.
    pub(super) fn highlight_info<'a>(
        &'a mut self,
        rules: &'a mut TextHighlightSettings,
        tool_name: &'static str,
    ) -> HighlightInfo<'a> {
        let RuleWriterState { description, running, draft, error } = self;
        let writer = match (running, error) {
            (Some(run), _) => RuleWriter::Writing(run.elapsed_seconds()),
            (None, Some(error)) => RuleWriter::Failed(error),
            (None, None) => RuleWriter::Idle,
        };
        HighlightInfo { settings: rules, description, draft, writer, tool_name }
    }

    /// Stop a run in progress.
    pub(super) fn stop(&mut self) {
        self.running = None;
    }
}

impl RubriqueApp {
    /// Ask the Ask tool for a pattern for the description; a failure to
    /// start is shown under the field.
    pub(super) fn write_highlight_rule(&mut self) {
        self.rule_writer.error = self.start_rule_writer().err();
    }

    fn start_rule_writer(&mut self) -> Result<(), String> {
        let description = self.rule_writer.description.trim();
        if description.is_empty() {
            return Err("Describe what to highlight first".to_string());
        }
        let tool = self.settings.ai.default_tool;
        let command = self.settings.ai.command(tool).to_string();
        if ai::find_program(&command).is_none() {
            return Err(format!("{} is not installed; install it in Settings → AI", tool.name()));
        }
        // The tool runs in an empty folder of Rubrique's own, so it sees
        // nothing but the description (no notes folder needs allowing).
        let folder = highlight_rule_folder().ok_or("No home folder to run the AI tool in")?;
        fs::create_dir_all(&folder).map_err(|error| format!("Cannot create {}: {error}", folder.display()))?;
        let prompt = pattern_prompt(description);
        let (program, args) = ai::command_args(&command, &prompt).ok_or("The Ask tool's command is empty")?;
        let repaint = self.repaint.clone();
        let run = AiRun::start(&program, &args, &folder, move || repaint.request_repaint())
            .map_err(|error| format!("Cannot start {program}: {error}"))?;
        self.rule_writer.running = Some(run);
        self.rule_writer.draft = None;
        Ok(())
    }

    /// Collect a finished run: a draft rule, or why there is none.
    pub(super) fn poll_rule_writer(&mut self) {
        let writer = &mut self.rule_writer;
        let Some(run) = &mut writer.running else {
            return;
        };
        let Some(end) = run.poll().cloned() else {
            return;
        };
        let reply = run.output_text();
        writer.running = None;
        let pattern = match end {
            RunEnd::Exited { success: true, .. } => pattern_from_reply(&reply),
            RunEnd::Stopped => return,
            RunEnd::Exited { .. } => None,
        };
        let Some(pattern) = pattern else {
            writer.error = Some("No pattern came back; try again or describe it another way".to_string());
            return;
        };
        // A broken pattern is still offered: the draft shows its error and
        // can be fixed by hand.
        writer.error = None;
        let name = name_from_description(&writer.description);
        writer.draft = Some(HighlightRule::new(&name, &pattern, RuleColor::Keyword));
    }
}
