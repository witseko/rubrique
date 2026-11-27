//! Turn a theme into egui styles and install it.

use std::sync::Arc;

use eframe::egui::style::ScrollAnimation;
use eframe::egui::{
    self, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Style, TextStyle,
    ThemePreference, Visuals,
};

use super::catalog::{ThemeChoice, Themes};
use super::palette::{Base, Palette, Rgb, Theme};
use crate::settings::accessibility::{AccessibilitySettings, EDITOR_FONT_SIZE};

/// Font size of labels, buttons and fields: the 13 points of macOS and
/// T3 Code interfaces. Inter's tall lowercase reads as large as egui's old
/// font did at 14.
const BODY_SIZE: f32 = 13.0;
/// Font size of hints and secondary details (egui's 9 points are hard to read).
const SMALL_SIZE: f32 = 11.0;
/// Font size of dialog titles.
const HEADING_SIZE: f32 = 18.0;
/// Height of a line in the source editor as a multiple of the editor font
/// size: the breathing room of a writing app rather than a terminal.
pub const EDITOR_LINE_HEIGHT: f32 = 1.5;

/// Corner rounding of buttons, fields and rows.
const WIDGET_RADIUS: u8 = 5;
/// Corner rounding of dialogs and menus.
const WINDOW_RADIUS: u8 = 8;
/// Width of the text cursor.
const CURSOR_WIDTH: f32 = 2.0;
/// Width of the outline around the focused control with strong focus on.
const STRONG_FOCUS_WIDTH: f32 = 2.5;

/// Install the chosen theme. "System" fills egui's light and dark slots with
/// the Light and Dark themes and lets the OS pick; a named theme fills the
/// slot of its base and switches to it. An unknown name falls back to system.
pub fn apply_theme(ctx: &Context, themes: &Themes, choice: &ThemeChoice) {
    for base in [Base::Light, Base::Dark] {
        ctx.set_style_of(egui_theme(base), style(&themes.for_base(base)));
    }
    let chosen = match choice {
        ThemeChoice::System => None,
        ThemeChoice::Named(id) => themes.find(id),
    };
    match chosen {
        Some(theme) => {
            ctx.set_style_of(egui_theme(theme.base), style(theme));
            ctx.set_theme(egui_theme(theme.base));
        }
        None => ctx.set_theme(ThemePreference::System),
    }
}

/// Font of labels, buttons, fields and preview text. Inter is a screen
/// font with real weights; egui's own Ubuntu-Light is thin and has no bold.
const INTERFACE_FONT: &str = "Inter-Regular";
/// Font of headings and bold text (see [`bold_family`]).
const BOLD_FONT: &str = "Inter-SemiBold";

/// The font family of bold text. egui's `strong()` only brightens the color,
/// so text that should look bold also needs this family.
pub fn bold_family() -> FontFamily {
    FontFamily::Name(BOLD_FONT.into())
}

/// Install Inter as the interface font, its semibold weight as the bold
/// family, and Hack as a fallback for both. egui 0.37 no longer falls back
/// to an emoji font, so arrows (the symlink marker `↗`, the `↑ up` button)
/// need a font that has them; Hack is already bundled for the editor.
pub fn install_fonts(ctx: &Context) {
    ctx.set_fonts(font_definitions());
}

/// egui's default fonts with Inter first and Hack right after it.
fn font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        (INTERFACE_FONT, include_bytes!("../../assets/fonts/Inter-Regular.ttf").as_slice()),
        (BOLD_FONT, include_bytes!("../../assets/fonts/Inter-SemiBold.ttf").as_slice()),
    ] {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    // Hack before egui's icon fallbacks, so it wins for arrows but never
    // replaces a letter Inter has.
    proportional.insert(0, "Hack".to_owned());
    let fallbacks = proportional.clone();
    proportional.insert(0, INTERFACE_FONT.to_owned());
    let bold = std::iter::once(BOLD_FONT.to_owned()).chain(fallbacks).collect();
    fonts.families.insert(bold_family(), bold);
    fonts
}

/// Install the text size and adjust the light and dark styles. Call after
/// [`apply_theme`], which replaces the styles.
pub fn apply_accessibility(ctx: &Context, settings: &AccessibilitySettings) {
    ctx.set_zoom_factor(settings.text_scale());
    ctx.all_styles_mut(|style| adjust_style(style, settings));
    ctx.request_repaint();
}

/// The zoomed editor font, no animations, and thicker focus outlines.
fn adjust_style(style: &mut Style, settings: &AccessibilitySettings) {
    // The text area zoom grows the editor's font; the preview scales itself
    // (see `markdown::render::set_page_zoom`).
    let editor_size = (f32::from(EDITOR_FONT_SIZE) * settings.page_scale()).round();
    style.text_styles.insert(TextStyle::Monospace, FontId::monospace(editor_size));
    if settings.reduce_motion {
        style.animation_time = 0.0;
        style.scroll_animation = ScrollAnimation::none();
    }
    if settings.strong_focus {
        let visuals = &mut style.visuals;
        // Focused buttons use the active look; focused text fields the selection stroke.
        visuals.widgets.active.bg_stroke.width = STRONG_FOCUS_WIDTH;
        visuals.selection.stroke.width = STRONG_FOCUS_WIDTH;
    }
}

/// Fill of the page behind the editor and the preview.
///
/// Colors that tell explorer folders (and their tab groups) apart: distinct
/// hues of middle lightness, so they read on light and dark themes alike.
const GROUP_COLORS: [Color32; 6] = [
    Color32::from_rgb(0x4c, 0x8d, 0xf6),
    Color32::from_rgb(0x34, 0xa8, 0x53),
    Color32::from_rgb(0xf2, 0x9d, 0x38),
    Color32::from_rgb(0xa3, 0x71, 0xf7),
    Color32::from_rgb(0x1f, 0xb5, 0xad),
    Color32::from_rgb(0xe5, 0x60, 0x9a),
];

/// The color of explorer folder number `index` (repeating after six).
pub fn group_color(index: usize) -> Color32 {
    GROUP_COLORS[index % GROUP_COLORS.len()]
}

/// The theme's `background` is stored as egui's text field color, so read it
/// from there.
pub fn page_fill(visuals: &Visuals) -> Color32 {
    visuals.text_edit_bg_color()
}

fn egui_theme(base: Base) -> egui::Theme {
    match base {
        Base::Light => egui::Theme::Light,
        Base::Dark => egui::Theme::Dark,
    }
}

fn color(Rgb(red, green, blue): Rgb) -> Color32 {
    Color32::from_rgb(red, green, blue)
}

/// egui's style with the theme's colors and Rubrique's spacing and rounding.
fn style(theme: &Theme) -> Style {
    let mut style = Style { visuals: visuals(theme.base, &theme.palette), ..Style::default() };
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(SMALL_SIZE)),
        (TextStyle::Body, FontId::proportional(BODY_SIZE)),
        (TextStyle::Button, FontId::proportional(BODY_SIZE)),
        (TextStyle::Heading, FontId::new(HEADING_SIZE, bold_family())),
        (TextStyle::Monospace, FontId::monospace(f32::from(EDITOR_FONT_SIZE))),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 4.0);
    style.spacing.button_padding = egui::vec2(8.0, 3.0);
    style.spacing.menu_margin = egui::Margin::same(6);
    // egui lets a trackpad scroll past the end of the content and spring
    // back. In a page of text that reads as the page jumping; stop firmly.
    style.spacing.scroll.kinetic.rubber_band = false;
    style
}

/// egui's light or dark visuals, recolored with the palette.
fn visuals(base: Base, palette: &Palette) -> Visuals {
    let mut visuals = match base {
        Base::Light => Visuals::light(),
        Base::Dark => Visuals::dark(),
    };
    let background = color(palette.background);
    let border = Stroke::new(1.0, color(palette.border));
    let text = color(palette.text);
    let text_strong = color(palette.text_strong);
    let accent = color(palette.accent);

    visuals.panel_fill = color(palette.sidebar);
    visuals.window_fill = background;
    visuals.window_stroke = border;
    visuals.extreme_bg_color = background;
    visuals.text_edit_bg_color = Some(background);
    visuals.faint_bg_color = color(palette.surface);
    visuals.code_bg_color = color(palette.code_background);
    visuals.hyperlink_color = accent;
    visuals.warn_fg_color = color(palette.warning);
    visuals.error_fg_color = color(palette.error);
    visuals.weak_text_color = Some(color(palette.text_weak));
    visuals.selection.bg_fill = color(palette.selection);
    visuals.selection.stroke = Stroke::new(1.0, text_strong);
    visuals.text_cursor.stroke = Stroke::new(CURSOR_WIDTH, accent);
    visuals.window_corner_radius = CornerRadius::same(WINDOW_RADIUS);
    visuals.menu_corner_radius = CornerRadius::same(WINDOW_RADIUS);

    let widgets = &mut visuals.widgets;
    // Labels, separators and panel edges.
    widgets.noninteractive.bg_fill = background;
    widgets.noninteractive.weak_bg_fill = background;
    widgets.noninteractive.bg_stroke = border;
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, text);
    // Buttons and checkboxes at rest, under the mouse, while pressed, and open menus.
    let states = [
        (&mut widgets.inactive, palette.surface, Stroke::NONE, text),
        (&mut widgets.hovered, palette.hover, border, text_strong),
        (&mut widgets.active, palette.hover, Stroke::new(1.0, accent), text_strong),
        (&mut widgets.open, palette.surface, border, text),
    ];
    for (state, fill, stroke, foreground) in states {
        state.bg_fill = color(fill);
        state.weak_bg_fill = color(fill);
        state.bg_stroke = stroke;
        state.fg_stroke = Stroke::new(1.0, foreground);
    }
    for state in [
        &mut widgets.noninteractive,
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        state.corner_radius = CornerRadius::same(WIDGET_RADIUS);
    }
    visuals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_font_draws_the_arrows_the_ui_uses() {
        let ctx = Context::default();
        install_fonts(&ctx);
        // Fonts are built on the first frame.
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        let font = FontId::proportional(14.0);
        ctx.fonts_mut(|fonts| {
            assert!(fonts.has_glyphs(&font, "↗↑"));
            // Letters still come from the interface font, not the fallback.
            assert_eq!(fonts.characters(&FontFamily::Proportional)[&'a'][0], INTERFACE_FONT);
            assert!(fonts.has_glyphs(&FontId::new(14.0, bold_family()), "Bold ↗"));
            assert_eq!(fonts.characters(&bold_family())[&'a'][0], BOLD_FONT);
        });
    }

    #[test]
    fn light_and_dark_keep_their_egui_mode() {
        let light = Theme::builtin(Base::Light);
        let dark = Theme::builtin(Base::Dark);
        assert!(!style(&light).visuals.dark_mode);
        assert!(style(&dark).visuals.dark_mode);
    }

    #[test]
    fn scrolling_stops_at_the_edges() {
        assert!(!style(&Theme::builtin(Base::Light)).spacing.scroll.kinetic.rubber_band);
    }

    #[test]
    fn accessibility_adjusts_fonts_motion_and_focus() {
        let mut adjusted = style(&Theme::builtin(Base::Light));
        let settings =
            AccessibilitySettings { page_zoom: 150, reduce_motion: true, strong_focus: true, ..Default::default() };
        adjust_style(&mut adjusted, &settings);
        assert_eq!(adjusted.text_styles[&TextStyle::Monospace], FontId::monospace(21.0));
        assert!(adjusted.animation_time.abs() < f32::EPSILON);
        let unchanged = style(&Theme::builtin(Base::Light)).visuals;
        let focus = Stroke::new(STRONG_FOCUS_WIDTH, unchanged.selection.stroke.color);
        assert_eq!(adjusted.visuals.selection.stroke, focus);

        // The defaults change nothing (the style already has the editor font size).
        let mut plain = style(&Theme::builtin(Base::Light));
        adjust_style(&mut plain, &AccessibilitySettings::default());
        assert_eq!(format!("{plain:?}"), format!("{:?}", style(&Theme::builtin(Base::Light))));
    }

    #[test]
    fn text_size_scales_window_layout_and_text_together() {
        let ctx = Context::default();
        let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0)));
        let input = egui::RawInput { screen_rect, ..Default::default() };
        ctx.run_ui(input.clone(), |_| {
            apply_accessibility(&ctx, &AccessibilitySettings { text_size: 150, ..Default::default() });
        })
        .drop_without_applying_deltas();
        ctx.run_ui(input, |ui| {
            assert_eq!(ctx.content_rect().size(), egui::vec2(600.0, 400.0));
            assert!((ui.pixels_per_point() - 1.5).abs() < f32::EPSILON);
        })
        .drop_without_applying_deltas();
    }

    #[test]
    fn text_area_zoom_schedules_a_frame_for_the_new_style() {
        let ctx = Context::default();
        for _ in 0..3 {
            ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        }
        let settings = AccessibilitySettings { page_zoom: 150, ..Default::default() };
        ctx.run_ui(egui::RawInput::default(), |_| {
            assert!(!ctx.has_requested_repaint());
            apply_accessibility(&ctx, &settings);
            assert!(ctx.has_requested_repaint());
        })
        .drop_without_applying_deltas();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let editor_size = (f32::from(EDITOR_FONT_SIZE) * 1.5).round();
            assert_eq!(TextStyle::Monospace.resolve(ui.style()), FontId::monospace(editor_size));
        })
        .drop_without_applying_deltas();
    }

    #[test]
    fn interface_text_sizes() {
        let text_styles = style(&Theme::builtin(Base::Light)).text_styles;
        assert_eq!(text_styles[&TextStyle::Body], FontId::proportional(13.0));
        assert_eq!(text_styles[&TextStyle::Small], FontId::proportional(11.0));
        assert_eq!(text_styles[&TextStyle::Monospace], FontId::monospace(14.0));
    }

    #[test]
    fn page_fill_is_the_theme_background() {
        let theme = Theme::builtin(Base::Dark);
        assert_eq!(page_fill(&style(&theme).visuals), color(theme.palette.background));
    }
}
