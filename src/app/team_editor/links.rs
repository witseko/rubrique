//! The "Who works with whom" view: the links between seats.
//!
//! Each link reads as a sentence, "build delegates to qa": the first seat,
//! the kind and the second seat, then a remove button. Links inside a pod
//! name seats by their plain id; links between pods name them `pod.seat`.

use eframe::egui::{self, Align, FontSelection, Id, RichText, Ui, WidgetText, text::LayoutJob};

use super::common::{CARD_GAP, KEY_SIZE, TeamEditor, issue_line, remove_button, spec_key};
use crate::openrig::spec::{ESCALATES_TO, Edge, Pod};
use crate::settings::view::list_frame;

/// Space around a card's header and around each link row.
const ROW_MARGIN: i8 = 10;
/// Width of a seat drop-down or the escalation target field.
const SEAT_WIDTH: f32 = 170.0;
/// Width of the kind drop-down.
const KIND_WIDTH: f32 = 250.0;
/// The kind a new link starts with.
const NEW_LINK_KIND: &str = "delegates_to";

/// One link kind as the form shows it.
struct LinkKind {
    /// The spec value (`delegates_to`).
    value: &'static str,
    /// The kind in words, as it reads between two seats.
    words: &'static str,
    /// One line on what it means.
    meaning: &'static str,
}

/// The link kinds in the order of `spec::EDGE_KINDS`.
const LINK_KINDS: [LinkKind; 5] = [
    LinkKind {
        value: "delegates_to",
        words: "delegates to",
        meaning: "Hands work to the second seat. Sets launch order.",
    },
    LinkKind {
        value: "spawned_by",
        words: "was spawned by",
        meaning: "The second seat starts first and owns this one. Sets launch order.",
    },
    LinkKind { value: "can_observe", words: "can observe", meaning: "Can read the second seat's output." },
    LinkKind { value: "collaborates_with", words: "collaborates with", meaning: "Peers working together." },
    LinkKind {
        value: ESCALATES_TO,
        words: "escalates to",
        meaning: "Fallback when this seat has no single delegating seat. May name pod-seat@rig in another rig.",
    },
];

/// The link kind in words, as it reads between two seats: `spawned_by`
/// reads "was spawned by". An unknown kind reads with spaces for `_`.
pub(super) fn kind_in_words(kind: &str) -> String {
    LINK_KINDS
        .iter()
        .find(|choice| choice.value == kind)
        .map_or_else(|| kind.replace('_', " "), |choice| choice.words.to_owned())
}

/// Draw the links view (artboard `Edges`): the explanation, one card per
/// pod for the links inside it, and one card for the links between pods.
/// Nothing can change when `editor.read_only`.
pub(super) fn show_links(ui: &mut Ui, editor: &mut TeamEditor) {
    ui.label(
        RichText::new(
            "Each link reads left to right: the first seat does something to the second. \
             A link between pods names each seat as pod.seat.",
        )
        .weak(),
    );
    ui.label(RichText::new("\"delegates to\" and \"was spawned by\" also set the launch order.").weak());
    ui.add_space(CARD_GAP);
    ui.add_enabled_ui(!editor.read_only, |ui| {
        for pod_index in 0..editor.spec.pods.len() {
            pod_card(ui, editor, pod_index);
            ui.add_space(CARD_GAP);
        }
        between_pods_card(ui, editor);
    });
}

/// The card for the links inside pod `pod_index`.
fn pod_card(ui: &mut Ui, editor: &mut TeamEditor, pod_index: usize) {
    let pod = &editor.spec.pods[pod_index];
    let seats: Vec<String> = pod.members.iter().map(|member| member.id.clone()).collect();
    let title = format!("Inside {}", pod.name());
    let key = format!("pods[{pod_index}].edges");
    let issues = edge_issues(editor, &key, pod.edges.len());
    let new_link = seats.first().map(|from| {
        let to = seats.get(1).unwrap_or(from);
        Edge::new(NEW_LINK_KIND, from, to)
    });
    let edges = &mut editor.spec.pods[pod_index].edges;
    link_card(
        ui,
        &CardText { title: &title, key: &key, empty: "No links" },
        edges,
        &seats,
        &issues,
        new_link,
        Some(pod_index),
    );
}

/// The card for the links between pods, whose ends are `pod.seat`.
fn between_pods_card(ui: &mut Ui, editor: &mut TeamEditor) {
    let pods = &editor.spec.pods;
    let seats: Vec<String> =
        pods.iter().flat_map(|pod| pod.members.iter().map(move |member| format!("{}.{}", pod.id, member.id))).collect();
    let first_seat =
        |pod: Option<&Pod>| pod.and_then(|pod| pod.members.first().map(|member| format!("{}.{}", pod.id, member.id)));
    let new_link = match (first_seat(pods.first()), first_seat(pods.get(1))) {
        (Some(from), Some(to)) => Some(Edge::new(NEW_LINK_KIND, &from, &to)),
        _ => None,
    };
    let issues = edge_issues(editor, "edges", editor.spec.edges.len());
    let text = CardText { title: "Between pods", key: "edges", empty: "No links" };
    link_card(ui, &text, &mut editor.spec.edges, &seats, &issues, new_link, None);
}

/// The problems of each link in the list at `key`, one list per link, each
/// message prefixed with the part it is about.
fn edge_issues(editor: &TeamEditor, key: &str, count: usize) -> Vec<Vec<String>> {
    (0..count)
        .map(|index| {
            let path = format!("{key}[{index}]");
            [
                ("", path.clone()),
                ("Kind: ", format!("{path}.kind")),
                ("From: ", format!("{path}.from")),
                ("To: ", format!("{path}.to")),
            ]
            .into_iter()
            .filter_map(|(prefix, path)| editor.issue(&path).map(|issue| format!("{prefix}{}", issue.message)))
            .collect()
        })
        .collect()
}

/// The words in a card's header.
struct CardText<'a> {
    title: &'a str,
    /// The spec key of the list (`pods[0].edges`).
    key: &'a str,
    /// What the header says when the list is empty.
    empty: &'a str,
}

/// One card: the header with "+ Add link", then a row per link. `new_link`
/// is what "+ Add link" adds; `None` disables it (not enough seats).
/// `pod` names the pod for links inside one, `None` for links between pods.
fn link_card(
    ui: &mut Ui,
    text: &CardText,
    edges: &mut Vec<Edge>,
    seats: &[String],
    issues: &[Vec<String>],
    new_link: Option<Edge>,
    pod: Option<usize>,
) {
    let mut remove = None;
    list_frame(ui, |ui| {
        egui::Frame::new().inner_margin(ROW_MARGIN).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(text.title).strong());
                spec_key(ui, text.key);
                let count = match edges.len() {
                    0 => text.empty.to_owned(),
                    1 => "1 link".to_owned(),
                    count => format!("{count} links"),
                };
                ui.label(RichText::new(count).weak().size(KEY_SIZE));
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    let add = ui.add_enabled(new_link.is_some(), egui::Button::new("+ Add link").frame(false));
                    if add.clicked()
                        && let Some(edge) = new_link
                    {
                        edges.push(edge);
                    }
                });
            });
        });
        for (index, edge) in edges.iter_mut().enumerate() {
            ui.separator();
            let row_issues = issues.get(index).map_or(&[][..], Vec::as_slice);
            if link_row(ui, edge, seats, row_issues, remove_link_id(pod, index)) {
                remove = Some(index);
            }
        }
    });
    if let Some(index) = remove {
        edges.remove(index);
    }
}

/// The id of the remove button of link `index` in pod `pod` (`None`: between pods).
fn remove_link_id(pod: Option<usize>, index: usize) -> Id {
    Id::unique(("team-link-remove", pod, index))
}

/// One link as a sentence: first seat, kind, second seat, remove button,
/// and the link's problems under it. Returns true when remove was clicked.
fn link_row(ui: &mut Ui, edge: &mut Edge, seats: &[String], issues: &[String], remove_id: Id) -> bool {
    let mut removed = false;
    egui::Frame::new().inner_margin(ROW_MARGIN).show(ui, |ui| {
        ui.horizontal(|ui| {
            seat_choice(ui, remove_id.with("from"), &mut edge.from, seats);
            kind_choice(ui, remove_id.with("kind"), &mut edge.kind);
            if edge.kind == ESCALATES_TO {
                let field = egui::TextEdit::singleline(&mut edge.to)
                    .hint_text("seat or pod-seat@rig")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(SEAT_WIDTH);
                ui.add(field);
            } else {
                seat_choice(ui, remove_id.with("to"), &mut edge.to, seats);
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                removed = remove_button(ui, remove_id, "Remove link", "Remove this link");
            });
        });
        for message in issues {
            issue_line(ui, message);
        }
    });
    removed
}

/// A drop-down of `seats` that sets `value`. A value not in the list (a
/// seat that is gone) still shows, so the problem under the row makes sense.
fn seat_choice(ui: &mut Ui, id: Id, value: &mut String, seats: &[String]) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(RichText::new(value.as_str()).monospace())
        .width(SEAT_WIDTH)
        .show_ui(ui, |ui| {
            for seat in seats {
                if ui.selectable_label(value == seat, RichText::new(seat).monospace()).clicked() {
                    value.clone_from(seat);
                }
            }
        });
}

/// The kind drop-down: each kind in words with its spec value small, and
/// its meaning in the list. An unknown value shows as written.
fn kind_choice(ui: &mut Ui, id: Id, kind: &mut String) {
    let style = ui.style().clone();
    let current = LINK_KINDS.iter().find(|choice| choice.value == kind.as_str());
    let selected = match current {
        Some(choice) => kind_text(&style, choice, false),
        None => WidgetText::from(RichText::new(kind.as_str()).monospace()),
    };
    egui::ComboBox::from_id_salt(id).selected_text(selected).width(KIND_WIDTH).show_ui(ui, |ui| {
        ui.set_min_width(KIND_WIDTH);
        for choice in &LINK_KINDS {
            if ui.selectable_label(kind == choice.value, kind_text(&style, choice, true)).clicked() {
                choice.value.clone_into(kind);
            }
        }
    });
}

/// "delegates to `delegates_to`", with the meaning on a second line when
/// `with_meaning`.
fn kind_text(style: &egui::Style, choice: &LinkKind, with_meaning: bool) -> WidgetText {
    let mut job = LayoutJob::default();
    let mut append = |text: RichText| text.append_to(&mut job, style, FontSelection::Default, Align::Center);
    append(RichText::new(format!("{} ", choice.words)));
    append(RichText::new(choice.value).monospace().size(KEY_SIZE).weak());
    if with_meaning {
        append(RichText::new(format!("\n{}", choice.meaning)).small().weak());
    }
    WidgetText::from(job)
}

#[cfg(test)]
mod tests {
    use super::super::test_driver::Driver;
    use super::*;
    use crate::openrig::spec::EDGE_KINDS;

    const FACTORY: &str = include_str!("../../openrig/spec/fixtures/factory.yaml");

    /// Click the remove button of link `index` in `pod` (`None`: between pods).
    fn click_remove(driver: &mut Driver, pod: Option<usize>, index: usize) {
        let response = driver.ctx.read_response(remove_link_id(pod, index)).expect("the remove button is drawn");
        driver.click_at(response.rect.center());
    }

    /// The form's kinds are the spec's kinds, in the same order.
    #[test]
    fn offers_every_link_kind() {
        let values: Vec<&str> = LINK_KINDS.iter().map(|kind| kind.value).collect();
        assert_eq!(values, EDGE_KINDS);
    }

    /// Inside a pod a link is added, its seat and kind changed, and removed.
    #[test]
    fn edits_links_inside_a_pod() {
        let mut driver = Driver::new(FACTORY, false, show_links);
        // The third "+ Add link" is the Review pod's, which has no links.
        driver.click_nth_text("+ Add link", 2);
        assert_eq!(driver.editor.spec.pods[2].edges, [Edge::new("delegates_to", "r1", "r2")]);

        driver.click_text("advisor");
        driver.click_text("lead");
        let edge = &driver.editor.spec.pods[0].edges[0];
        assert_eq!((edge.from.as_str(), edge.to.as_str()), ("lead", "lead"));

        driver.click_text("collaborates with");
        driver.click_text("can observe");
        assert_eq!(driver.editor.spec.pods[0].edges[0].kind, "can_observe");

        click_remove(&mut driver, Some(1), 0);
        assert_eq!(driver.editor.spec.pods[1].edges, []);
        assert_eq!(driver.editor.spec.link_count(), 11);
    }

    /// Between pods the seats are `pod.seat`, and an escalation target is free text.
    #[test]
    fn edits_links_between_pods() {
        let mut driver = Driver::new(FACTORY, false, show_links);
        driver.click_text("+ Add link");
        assert_eq!(driver.editor.spec.edges[9], Edge::new("delegates_to", "orch.lead", "dev.build"));

        // The third "review.r2" is the first seat of the last link read from the file.
        driver.click_nth_text("review.r2", 2);
        driver.click_text("orch.advisor");
        assert_eq!(driver.editor.spec.edges[8].from, "orch.advisor");

        driver.click_nth_text("delegates to", 6);
        driver.click_text("escalates to");
        assert_eq!(driver.editor.spec.edges[9].kind, "escalates_to");
        driver.click_nth_text("dev.build", 3);
        driver.replace_text("lead-boss@company");
        assert_eq!(driver.editor.spec.edges[9].to, "lead-boss@company");

        click_remove(&mut driver, None, 0);
        assert_eq!(driver.editor.spec.edges.len(), 9);
        assert_eq!(driver.editor.spec.edges[0].to, "dev.design");
    }

    /// A built-in team's links cannot be added, changed or removed.
    #[test]
    fn read_only_blocks_link_edits() {
        let mut driver = Driver::new(FACTORY, true, show_links);
        let before = driver.editor.spec.clone();
        driver.click_text("+ Add link");
        driver.click_text("advisor");
        driver.click_text("lead");
        click_remove(&mut driver, None, 0);
        assert_eq!(driver.editor.spec, before);
    }
}
