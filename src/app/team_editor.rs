//! The `OpenRig` team editor: a rig spec (`rig.yaml`) shown as a form in its
//! document tab, with the header, the Seats / Who works with whom / Team
//! views and the seat inspector.
//!
//! This file lays the tab out and connects it to the app. Each screen has
//! its own file: `seats`, `inspector`, `links`, `team` and `states`; the
//! shared state and widgets are in `common`.

pub(super) mod common;
mod inspector;
mod links;
mod seats;
pub(super) mod states;
mod team;
#[cfg(test)]
mod test_driver;

use std::path::Path;

use eframe::egui::{self, Context, RichText, Ui};

use super::{OpenFile, PageMode, RubriqueApp};
use crate::document::text_file::Document;
use crate::openrig::Rig;
use crate::settings::view::SettingsGroup;
use common::{Job, SubTab, TeamEditor, TeamEvent, plural, rig_target};
use states::NewTeamChoice;

/// Space around the form.
const PAGE_MARGIN: i8 = 24;
/// Size of the team name in the header.
const NAME_SIZE: f32 = 22.0;
/// Starting width of the seat inspector.
const INSPECTOR_WIDTH: f32 = 420.0;
/// The most of the view's width the inspector starts with, so the cards
/// keep room in a narrow window.
const INSPECTOR_MAX_SHARE: f32 = 0.42;
/// Space between the cards and the inspector.
const INSPECTOR_GAP: i8 = 12;
/// Space between the header, the views switch and the view.
const SECTION_GAP: f32 = 12.0;

/// True for a file that may open as a team form: `.yaml` or `.yml`.
pub(super) fn is_yaml_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml"))
}

/// Draw the form for `document` and write its edits into the document.
pub(super) fn show(ui: &mut Ui, editor: &mut TeamEditor, document: &mut Document) {
    editor.sync_from(document);
    poll_jobs(ui.ctx(), editor, &document.path);
    egui::Frame::new().inner_margin(PAGE_MARGIN).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if editor.parse_error.is_some() {
            states::yaml_error_notice(ui, editor);
            return;
        }
        if editor.read_only {
            states::read_only_banner(ui, editor);
        }
        states::rig_missing_notice(ui, editor);
        header(ui, editor, document.is_dirty());
        states::validation_summary(ui, editor);
        states::preflight_result(ui, editor);
        states::launch_result(ui, editor);
        if let Some(error) = &editor.write_error {
            ui.label(RichText::new(error).color(ui.visuals().warn_fg_color));
        }
        ui.add_space(SECTION_GAP);
        view_switch(ui, editor);
        ui.add_space(SECTION_GAP);
        view(ui, editor);
    });
    editor.write_to(document);
}

/// Take finished `rig` answers. A valid spec goes on to preflight, and a
/// ready one on to the launch plan when Launch asked for it. Nothing here
/// launches the team.
fn poll_jobs(ctx: &Context, editor: &mut TeamEditor, path: &Path) {
    let validated = editor.validation.poll();
    let preflighted = editor.preflight.poll();
    editor.plan.poll();
    editor.launch.poll();
    if validated {
        if matches!(&editor.validation.result, Some(Ok(validation)) if validation.valid) {
            let (host, spec) = rig_target(path);
            editor.preflight.start(ctx, move || Rig::open_checked(&host)?.spec_preflight(&spec));
        } else {
            editor.plan_after_preflight = false;
        }
    }
    if preflighted {
        let ready = matches!(&editor.preflight.result, Some(Ok(preflight)) if preflight.ready);
        if ready && editor.plan_after_preflight {
            let (host, spec) = rig_target(path);
            editor.plan.start(ctx, move || Rig::open_checked(&host)?.up_plan(&spec));
        }
        editor.plan_after_preflight = false;
    }
}

/// The team name, its counts and summary, the unsaved-changes line, and
/// Validate and Launch.
fn header(ui: &mut Ui, editor: &mut TeamEditor, dirty: bool) {
    let spec = &editor.spec;
    let counts = format!(
        "{} {}, {} {}, {} {}",
        spec.pods.len(),
        plural(spec.pods.len(), "pod", "pods"),
        spec.seat_count(),
        plural(spec.seat_count(), "seat", "seats"),
        spec.link_count(),
        plural(spec.link_count(), "link", "links"),
    );
    let mut event = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(&spec.name).monospace().strong().size(NAME_SIZE));
        ui.label(RichText::new(counts).weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let busy = editor.validation.is_running()
                || editor.preflight.is_running()
                || editor.plan.is_running()
                || editor.launch.is_running();
            // The main button, filled with the selection color like Unlock on
            // a locked scratch.
            let selection = ui.visuals().selection;
            let primary =
                egui::Button::new(RichText::new("Launch").color(selection.stroke.color)).fill(selection.bg_fill);
            let launch = ui
                .add_enabled(!busy, primary)
                .on_hover_text("Check the team and show what a launch starts. Nothing starts until you confirm.");
            if launch.clicked() {
                event = Some(TeamEvent::PrepareLaunch);
            }
            if ui.add_enabled(!busy, egui::Button::new(if busy { "Checking" } else { "Validate" })).clicked() {
                event = Some(TeamEvent::Validate);
            }
            if dirty && !editor.read_only && ui.button("Save").clicked() {
                event = Some(TeamEvent::Save);
            }
        });
    });
    if !spec.summary.is_empty() {
        ui.label(RichText::new(spec.summary.trim()).weak());
    }
    if dirty && !editor.read_only {
        let changes = editor.saved.as_ref().map(|saved| states::describe_changes(saved, spec)).unwrap_or_default();
        ui.horizontal(|ui| {
            let line = match changes.len() {
                0 => "Unsaved changes".to_owned(),
                count => format!("{count} unsaved {}", plural(count, "change", "changes")),
            };
            ui.label(RichText::new(line).weak());
            if ui.link("Revert").clicked() {
                event = Some(TeamEvent::Revert);
            }
        });
    }
    if let Some(event) = event {
        editor.request(event);
    }
}

/// The Seats | Who works with whom | Team switch.
fn view_switch(ui: &mut Ui, editor: &mut TeamEditor) {
    ui.horizontal(|ui| {
        ui.selectable_value(&mut editor.tab, SubTab::Seats, "Seats");
        ui.selectable_value(&mut editor.tab, SubTab::Links, "Who works with whom");
        ui.selectable_value(&mut editor.tab, SubTab::Team, "Team");
    });
}

/// The chosen view, with the seat inspector on the right of the Seats view.
fn view(ui: &mut Ui, editor: &mut TeamEditor) {
    let seat = editor
        .selected
        .filter(|_| editor.tab == SubTab::Seats)
        .filter(|seat| editor.spec.pods.get(seat.pod).is_some_and(|pod| seat.member < pod.members.len()));
    if let Some(seat) = seat {
        let width = INSPECTOR_WIDTH.min(ui.available_width() * INSPECTOR_MAX_SHARE);
        // The outer margin keeps the cards off the inspector's edge.
        let frame = egui::Frame::side_top_panel(ui.style())
            .outer_margin(egui::Margin { left: INSPECTOR_GAP, ..Default::default() });
        egui::Panel::right("team-seat-inspector").resizable(true).default_size(width).frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("team-inspector-scroll")
                .show(ui, |ui| inspector::show_inspector(ui, editor, seat));
        });
    }
    egui::ScrollArea::vertical().id_salt("team-view-scroll").auto_shrink([false, false]).show(ui, |ui| {
        match editor.tab {
            SubTab::Seats => seats::show_seats(ui, editor),
            SubTab::Links => links::show_links(ui, editor),
            SubTab::Team => team::show_team(ui, editor),
        }
    });
}

impl RubriqueApp {
    /// Do what the team form asked for in this frame.
    pub(super) fn team_events(&mut self, ctx: &Context) {
        let OpenFile::Text { team: Some(editor), .. } = &mut self.open_file else { return };
        for event in std::mem::take(&mut editor.events) {
            self.team_event(ctx, event);
        }
    }

    /// Do one thing the team form asked for.
    fn team_event(&mut self, ctx: &Context, event: TeamEvent) {
        match event {
            TeamEvent::Save => {
                self.save();
            }
            TeamEvent::Revert => {
                if let OpenFile::Text { document, preview, .. } = &mut self.open_file {
                    document.discard_changes();
                    *preview = None;
                }
            }
            TeamEvent::Validate => self.check_team(ctx, false),
            TeamEvent::PrepareLaunch => self.check_team(ctx, true),
            TeamEvent::Launch => self.launch_team(ctx),
            TeamEvent::OpenPath(path) => self.open_in_tab(path),
            TeamEvent::Duplicate => self.duplicate_team(),
            TeamEvent::ShowYaml => self.set_page_mode(PageMode::Source),
            TeamEvent::OpenSettings => {
                self.settings_group = SettingsGroup::OpenRig;
                self.settings_open = true;
            }
        }
    }

    /// Save, then validate the team (and preflight it when valid). With
    /// `plan`, a ready team then gets its launch plan.
    fn check_team(&mut self, ctx: &Context, plan: bool) {
        if !self.save_before_rig() {
            return;
        }
        self.start_team_job(ctx, |editor, ctx, host, spec| {
            editor.preflight = Job::default();
            editor.plan = Job::default();
            editor.plan_after_preflight = plan;
            editor.validation.start(ctx, move || Rig::open_checked(&host)?.spec_validate(&spec));
        });
    }

    /// Launch the team with `rig up`, only when the file is saved and its
    /// launch plan found nothing in the way (any edit clears the plan).
    fn launch_team(&mut self, ctx: &Context) {
        if self.open_file.is_dirty() {
            return;
        }
        self.start_team_job(ctx, |editor, ctx, host, spec| {
            let planned = matches!(&editor.plan.result, Some(Ok(plan)) if plan.ok);
            if planned && !editor.launch.is_running() {
                editor.launch.start(ctx, move || Rig::open_checked(&host)?.up(&spec));
                // The checks led to this launch; its result replaces them, and
                // another launch needs a new plan.
                editor.preflight = Job::default();
                editor.plan = Job::default();
            }
        });
    }

    /// Validate and Launch check what is on disk, so unsaved edits are saved
    /// first. Returns false when the save failed.
    fn save_before_rig(&mut self) -> bool {
        !self.open_file.is_dirty() || self.save_now()
    }

    /// Run `start` on the open team form with the place `rig` runs and the
    /// spec's path there.
    fn start_team_job(
        &mut self,
        ctx: &Context,
        start: impl FnOnce(&mut TeamEditor, &Context, crate::openrig::RigHost, std::path::PathBuf),
    ) {
        let OpenFile::Text { document, team: Some(editor), .. } = &mut self.open_file else { return };
        let (host, spec) = rig_target(&document.path);
        start(editor, ctx, host, spec);
    }

    /// Copy the open read-only team into the user's spec folder and open the copy.
    fn duplicate_team(&mut self) {
        let OpenFile::Text { document, .. } = &self.open_file else { return };
        match states::duplicate_team(&document.path, &document.text) {
            Ok(path) => self.open_in_tab(path),
            Err(message) => self.show_status(message),
        }
    }

    /// The new team dialog, while it is open: create the team and open it.
    pub(super) fn new_team_dialog(&mut self, ctx: &Context) {
        let Some(dialog) = &mut self.new_team else { return };
        match states::new_team_dialog(ctx, dialog) {
            None => {}
            Some(NewTeamChoice::Cancel) => self.new_team = None,
            Some(NewTeamChoice::Create) => match states::create_team(dialog) {
                Ok(path) => {
                    self.new_team = None;
                    self.open_in_tab(path);
                }
                Err(message) => self.show_status(message),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::text_file::Opened;
    use crate::test_support::TempFolder;

    const STARTER: &str = include_str!("../openrig/spec/fixtures/starter.yaml");

    /// A form edit reaches the document text (dirty, ready for the normal
    /// save), and a YAML edit reaches the form.
    #[test]
    fn form_and_text_stay_in_step() {
        let temp = TempFolder::new();
        let path = temp.write("my-team/rig.yaml", STARTER);
        let Opened::Text(mut document) = Document::open(&path, None).unwrap() else { panic!("a text file") };
        let mut editor = TeamEditor::new(&document);
        assert!(!editor.read_only);
        assert_eq!(editor.parse_error, None);

        let ctx = Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| show(ui, &mut editor, &mut document));
        output.textures_delta.clear();
        assert!(!document.is_dirty(), "drawing the form changes nothing");

        editor.spec.pods[0].members[1].model = "gpt-7".into();
        editor.write_to(&mut document);
        assert!(document.is_dirty());
        assert_eq!(document.text, STARTER.replace("model: gpt-6-astra", "model: gpt-7"));

        document.text = document.text.replace("name: starter", "name: crew");
        document.on_text_changed();
        editor.sync_from(&document);
        assert_eq!(editor.spec.name, "crew");
        assert_eq!(editor.spec.pods[0].members[1].model, "gpt-7");
    }

    /// Every view, the inspector and its Advanced section draw without
    /// changing the file: opening a team never marks it unsaved.
    #[test]
    fn drawing_every_view_changes_nothing() {
        let temp = TempFolder::new();
        let factory = include_str!("../openrig/spec/fixtures/factory.yaml");
        let path = temp.write("my-team/rig.yaml", factory);
        let Opened::Text(mut document) = Document::open(&path, None).unwrap() else { panic!("a text file") };
        let mut editor = TeamEditor::new(&document);
        editor.selected = Some(common::SeatRef { pod: 1, member: 0 });
        let ctx = Context::default();
        for tab in [SubTab::Seats, SubTab::Links, SubTab::Team] {
            editor.tab = tab;
            for _ in 0..3 {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| show(ui, &mut editor, &mut document));
                output.textures_delta.clear();
            }
        }
        assert!(!document.is_dirty());
        assert_eq!(document.text, factory);
    }

    /// Draw the form in a window `width` wide for a few frames. Returns the
    /// outlined cards (pod cards and the like) of the last frame, and where
    /// the inspector's "Seat id" label began in each frame.
    fn card_layout(width: f32, editor: &mut TeamEditor, document: &mut Document) -> (Vec<egui::Rect>, Vec<f32>) {
        let ctx = Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 800.0));
        let mut cards = Vec::new();
        let mut inspector_lefts = Vec::new();
        for _ in 0..4 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| show(ui, editor, document));
            output.textures_delta.clear();
            cards.clear();
            for clipped in output.shapes {
                match clipped.shape {
                    egui::Shape::Rect(rect) if rect.stroke.width > 0.0 && rect.rect.height() > 40.0 => {
                        cards.push(rect.rect);
                    }
                    egui::Shape::Text(text) if text.galley.text() == "Seat id" => inspector_lefts.push(text.pos.x),
                    _ => {}
                }
            }
        }
        (cards, inspector_lefts)
    }

    /// Every card stays inside the page; in the Seats and Who works with
    /// whom views they share one width. With a seat selected, the pod cards
    /// keep that width left of the inspector, which keeps its own width.
    #[test]
    fn cards_keep_one_width_beside_the_inspector() {
        let temp = TempFolder::new();
        let factory = include_str!("../openrig/spec/fixtures/factory.yaml");
        let path = temp.write("my-team/rig.yaml", factory);
        let Opened::Text(mut document) = Document::open(&path, None).unwrap() else { panic!("a text file") };
        let mut editor = TeamEditor::new(&document);
        let same_width = |cards: &[egui::Rect]| cards.iter().all(|card| (card.width() - cards[0].width()).abs() < 0.5);
        for width in [1100.0, 1400.0] {
            let page_right = width - f32::from(PAGE_MARGIN);
            editor.selected = None;
            for tab in [SubTab::Seats, SubTab::Links, SubTab::Team] {
                editor.tab = tab;
                let (cards, _) = card_layout(width, &mut editor, &mut document);
                assert!(cards.len() >= 3, "{tab:?} cards: {cards:?}");
                assert!(cards.iter().all(|card| card.max.x <= page_right + 0.5), "{tab:?} inside the page: {cards:?}");
                assert!(tab == SubTab::Team || same_width(&cards), "{tab:?} equal widths: {cards:?}");
            }

            editor.tab = SubTab::Seats;
            editor.selected = Some(common::SeatRef { pod: 1, member: 0 });
            let (cards, inspector_lefts) = card_layout(width, &mut editor, &mut document);
            let inspector_left = *inspector_lefts.last().expect("the inspector is open");
            assert!(
                inspector_lefts.iter().all(|left| (left - inspector_left).abs() < 0.5),
                "the inspector moved: {inspector_lefts:?}"
            );
            let pod_cards: Vec<egui::Rect> = cards.into_iter().filter(|card| card.min.x < inspector_left).collect();
            assert!(pod_cards.len() >= 3, "pod cards: {pod_cards:?}");
            assert!(same_width(&pod_cards), "equal widths: {pod_cards:?}");
            assert!(pod_cards.iter().all(|card| card.max.x < inspector_left), "under the inspector: {pod_cards:?}");
        }
    }

    /// The app for `text` saved as a team in the user's folder, opened as a form.
    fn app_with_team(temp: &TempFolder, text: &str) -> (Context, RubriqueApp) {
        let path = temp.write("my-team/rig.yaml", text);
        let ctx = Context::default();
        let settings = crate::settings::Settings { openrig_enabled: true, ..Default::default() };
        let mut app = RubriqueApp::new(&ctx, settings, vec![temp.path().into()], None);
        app.open_in_tab(path);
        assert!(app.showing_team_file());
        (ctx, app)
    }

    /// The open form.
    fn open_editor(app: &mut RubriqueApp) -> &mut TeamEditor {
        let OpenFile::Text { team: Some(editor), .. } = &mut app.open_file else { panic!("a team form") };
        editor
    }

    /// Launch does nothing without a launch plan that found nothing in the
    /// way: the header's Launch only asks for checks and a plan.
    #[test]
    fn launch_needs_a_plan_first() {
        let temp = TempFolder::new();
        let (ctx, mut app) = app_with_team(&temp, STARTER);
        app.team_event(&ctx, TeamEvent::Launch);
        assert!(!open_editor(&mut app).launch.is_running());

        let blocked = crate::openrig::UpReply {
            ok: false,
            rig_result: None,
            message: None,
            errors: vec!["no agent".into()],
            warnings: Vec::new(),
        };
        open_editor(&mut app).plan.result = Some(Ok(blocked));
        app.team_event(&ctx, TeamEvent::Launch);
        assert!(!open_editor(&mut app).launch.is_running());
    }

    /// "Edit in YAML" shows the file as text, and the rig-missing notice
    /// opens Settings at `OpenRig`.
    #[test]
    fn opens_the_yaml_view_and_settings() {
        let temp = TempFolder::new();
        let (ctx, mut app) = app_with_team(&temp, STARTER);
        assert_eq!(app.page_mode(), PageMode::Form);
        app.team_event(&ctx, TeamEvent::ShowYaml);
        assert_eq!(app.page_mode(), PageMode::Source);
        app.team_event(&ctx, TeamEvent::OpenSettings);
        assert!(app.settings_open);
        assert_eq!(app.settings_group, SettingsGroup::OpenRig);
    }

    /// A read-only team takes back edits instead of writing them.
    #[test]
    fn read_only_teams_are_not_written() {
        let temp = TempFolder::new();
        let path = temp.write("openrig/packages/daemon/specs/rigs/launch/starter/rig.yaml", STARTER);
        let Opened::Text(mut document) = Document::open(&path, None).unwrap() else { panic!("a text file") };
        let mut editor = TeamEditor::new(&document);
        assert!(editor.read_only);
        editor.spec.name = "changed".into();
        editor.write_to(&mut document);
        assert_eq!(document.text, STARTER);
        assert_eq!(editor.spec.name, "starter");
    }

    /// Only YAML files are candidates for the form.
    #[test]
    fn recognises_yaml_paths() {
        assert!(is_yaml_path(Path::new("/specs/rig.yaml")));
        assert!(is_yaml_path(Path::new("/specs/rig.YML")));
        assert!(!is_yaml_path(Path::new("/specs/rig.json")));
        assert!(!is_yaml_path(Path::new("/specs/yaml")));
    }
}
