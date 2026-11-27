//! Front matter drawn as a property sheet at the top of the preview: a
//! line with an Open Knowledge Format concept's type, trust tier and
//! freshness, the title and description, then one labeled row per field
//! with its value in plain words (tags as chips, dates spelled out, who
//! wrote and verified it, sources as links).

use eframe::egui::{self, Color32, Margin, RichText, Ui, vec2};

use super::front_matter::{FrontMatter, TrustTier, actor_parts, list_items, mapping, readable_date};
use super::inlines::is_openable_url;
use super::render::{local_markdown_link, request_open};
use crate::theme::style::bold_family;

/// Text sizes of the card, set from the page's look and zoom.
pub(super) struct CardSizes {
    /// The description.
    pub body: f32,
    /// Row labels and values.
    pub row: f32,
    /// The header line, chips and the gray line under a source.
    pub small: f32,
    pub title: f32,
    /// Width of the label column.
    pub label_width: f32,
}

/// Fields the card shows above its rows, whatever the file.
const SHOWN_ABOVE: [&str; 2] = ["title", "description"];

/// Fields an OKF concept shows in its header line instead of as rows.
const SHOWN_IN_HEADER: [&str; 3] = ["type", "status", "stale_after"];

/// Space between the rows.
const ROW_GAP: f32 = 8.0;

/// Corner radius of the card and its chips.
const CARD_RADIUS: u8 = 8;

/// Draw `front` as a card.
pub(super) fn render_front_matter(ui: &mut Ui, sizes: &CardSizes, front: &FrontMatter) {
    let visuals = ui.visuals();
    let frame = egui::Frame::new()
        .fill(visuals.code_bg_color)
        .stroke(visuals.widgets.noninteractive.bg_stroke)
        .corner_radius(CARD_RADIUS)
        .inner_margin(Margin::symmetric(24, 20));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 6.0;
        let concept = front.concept_type().is_some();
        if concept {
            header_line(ui, sizes, front);
        }
        if let Some(title) = front.get("title") {
            ui.label(
                RichText::new(title).size(sizes.title).family(bold_family()).color(ui.visuals().strong_text_color()),
            );
        }
        if let Some(description) = front.get("description") {
            ui.label(RichText::new(description).size(sizes.body));
        }
        let rows: Vec<&(String, String)> = front
            .fields
            .iter()
            .filter(|(key, _)| !SHOWN_ABOVE.contains(&key.as_str()))
            .filter(|(key, _)| !(concept && SHOWN_IN_HEADER.contains(&key.as_str())))
            .collect();
        if rows.is_empty() {
            return;
        }
        let has_heading = concept || front.get("title").is_some() || front.get("description").is_some();
        if has_heading {
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
        }
        for (key, value) in rows {
            property_row(ui, sizes, front, key, value);
        }
    });
}

/// The OKF line above the title: the type, a draft or deprecated status,
/// the trust tier and the freshness, separated by dots.
fn header_line(ui: &mut Ui, sizes: &CardSizes, front: &FrontMatter) {
    let visuals = ui.visuals().clone();
    let warn = visuals.warn_fg_color;
    let weak = visuals.weak_text_color();
    let mut parts: Vec<(String, Color32, &str)> = Vec::new();
    if let Some(concept_type) = front.concept_type() {
        parts.push((concept_type.to_uppercase(), visuals.hyperlink_color, "Open Knowledge Format concept type"));
    }
    if let Some(status) = front.get("status").filter(|status| *status != "stable") {
        parts.push((capitalized(status), warn, "Lifecycle status"));
    }
    let tier = front.trust_tier();
    let tier_color = match tier {
        TrustTier::HumanReviewed => visuals.strong_text_color(),
        TrustTier::MachineConfirmed => visuals.text_color(),
        TrustTier::Unverified => weak,
    };
    parts.push((tier.label().to_string(), tier_color, "Trust tier, from who verified this concept"));
    let today = crate::date::today_iso();
    if let Some(stale_after) = front.get("stale_after") {
        let date = readable_date(stale_after).unwrap_or_else(|| stale_after.to_string());
        if front.stale_since(&today).is_some() {
            parts.push((format!("Stale since {date}"), warn, "Past its stale_after date"));
        } else {
            parts.push((format!("Fresh until {date}"), weak, "Its stale_after date"));
        }
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (index, (text, color, tip)) in parts.into_iter().enumerate() {
            if index > 0 {
                ui.label(RichText::new("•").size(sizes.small).color(weak));
            }
            let mut rich = RichText::new(text).size(sizes.small).color(color);
            if index == 0 {
                rich = rich.strong();
            }
            ui.label(rich).on_hover_text(tip);
        }
    });
}

/// One field: its label in the left column, its value at the right.
fn property_row(ui: &mut Ui, sizes: &CardSizes, front: &FrontMatter, key: &str, value: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        ui.allocate_ui_with_layout(vec2(sizes.label_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(sizes.label_width);
            ui.label(RichText::new(label_for(key)).size(sizes.row).color(ui.visuals().weak_text_color()));
        });
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            value_view(ui, sizes, front, key, value);
        });
    });
    ui.add_space(ROW_GAP);
}

/// The value of field `key`, drawn for what it holds.
fn value_view(ui: &mut Ui, sizes: &CardSizes, front: &FrontMatter, key: &str, value: &str) {
    match key {
        "tags" | "keywords" => chips(ui, sizes, &list_items(value)),
        "generated" | "verified" => {
            for event in front.events(key) {
                let (name, kind) = actor_parts(&event.actor);
                let date = event.at.as_deref().map(|at| readable_date(at).unwrap_or_else(|| at.to_string()));
                let details: Vec<&str> = kind.into_iter().chain(date.as_deref()).collect();
                main_and_details(ui, sizes, name, &details.join(" · "));
            }
        }
        "sources" => sources(ui, sizes, front),
        "usage_window" => {
            let fields = mapping(value);
            let date = |name: &str| {
                let value = fields.iter().find(|(key, _)| key == name).map_or("", |(_, value)| value.as_str());
                readable_date(value).unwrap_or_else(|| value.to_string())
            };
            ui.label(RichText::new(format!("{} to {}", date("from"), date("to"))).size(sizes.row));
        }
        _ => plain_value(ui, sizes, value),
    }
}

/// A value with no special meaning: a link, a date, one line of text, or
/// YAML shown as written.
fn plain_value(ui: &mut Ui, sizes: &CardSizes, value: &str) {
    let value = value.trim();
    let value = value.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')).unwrap_or(value);
    if value.contains('\n') || value.starts_with(['{', '[', '-']) {
        ui.label(RichText::new(value).monospace().size(sizes.small));
    } else if let Some(date) = readable_date(value) {
        ui.label(RichText::new(date).size(sizes.row)).on_hover_text(value);
    } else if is_openable_url(value) || local_markdown_link(ui.ctx(), value, true).is_some() {
        link_or_text(ui, RichText::new(value).size(sizes.row), value);
    } else {
        ui.label(RichText::new(value).size(sizes.row));
    }
}

/// Each source: its title as a link to its resource, and under it who
/// wrote it, when it last changed and how often it was used.
fn sources(ui: &mut Ui, sizes: &CardSizes, front: &FrontMatter) {
    for source in front.sources() {
        let Some(name) = source.title.as_deref().or(source.resource.as_deref()) else {
            ui.label(RichText::new(&source.text).monospace().size(sizes.small));
            continue;
        };
        let rich = RichText::new(name).size(sizes.row);
        match source.resource.as_deref() {
            Some(resource) => link_or_text(ui, rich, resource),
            None => {
                ui.label(rich);
            }
        }
        let author = source.author.as_deref().map(|author| actor_parts(author).0.to_string());
        let changed = source
            .last_modified
            .as_deref()
            .map(|date| format!("changed {}", readable_date(date).unwrap_or_else(|| date.to_string())));
        let uses = source.usage_count.as_deref().map(|count| format!("{count} uses"));
        let details: Vec<String> = [author, changed, uses].into_iter().flatten().collect();
        if !details.is_empty() {
            ui.label(RichText::new(details.join(" · ")).size(sizes.small).color(ui.visuals().weak_text_color()));
        }
        ui.add_space(2.0);
    }
}

/// `main` in the text color, then ` · details` in gray, wrapping together.
fn main_and_details(ui: &mut Ui, sizes: &CardSizes, main: &str, details: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(RichText::new(main).size(sizes.row));
        if !details.is_empty() {
            let weak = ui.visuals().weak_text_color();
            ui.label(RichText::new(format!(" · {details}")).size(sizes.row).color(weak));
        }
    });
}

/// Short words in rounded boxes, wrapping.
fn chips(ui: &mut Ui, sizes: &CardSizes, items: &[String]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        let fill = ui.visuals().widgets.inactive.weak_bg_fill;
        for item in items {
            egui::Frame::new()
                .fill(fill)
                .corner_radius(CARD_RADIUS)
                .inner_margin(Margin::symmetric(9, 1))
                .show(ui, |ui| ui.label(RichText::new(item).size(sizes.small)));
        }
    });
}

/// `rich` as a link to `target`: a web or mail address opens in the browser,
/// a Markdown file in a tab; anything else is plain text with the target on hover.
fn link_or_text(ui: &mut Ui, rich: RichText, target: &str) {
    if is_openable_url(target) {
        ui.hyperlink_to(rich, target).on_hover_text(target);
    } else if let Some(path) = local_markdown_link(ui.ctx(), target, true) {
        if ui.link(rich).on_hover_text(path.display().to_string()).clicked() {
            request_open(ui.ctx(), path);
        }
    } else {
        ui.label(rich).on_hover_text(target);
    }
}

/// The label shown for field `key`.
fn label_for(key: &str) -> String {
    match key {
        "resource" => "Resource".to_string(),
        "generated" => "Written by".to_string(),
        "verified" => "Verified by".to_string(),
        "usage_window" => "Usage counted".to_string(),
        "timestamp" => "Updated".to_string(),
        _ => capitalized(&key.replace(['_', '-'], " ")),
    }
}

/// `text` with its first letter upper case.
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_read_as_words() {
        assert_eq!(label_for("generated"), "Written by");
        assert_eq!(label_for("stale_after"), "Stale after");
        assert_eq!(label_for("okf-version"), "Okf version");
        assert_eq!(label_for("tags"), "Tags");
    }
}
