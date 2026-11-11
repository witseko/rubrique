//! Text size (and the zoom gesture that steps it), editor font size,
//! line wrapping, reduced motion and focus outlines.

use std::ops::RangeInclusive;

/// How large everything in the window is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextSize {
    Small,
    #[default]
    Default,
    Large,
    Larger,
    Largest,
}

impl TextSize {
    pub const ALL: [TextSize; 5] = [Self::Small, Self::Default, Self::Large, Self::Larger, Self::Largest];

    pub fn name(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Default => "Default",
            Self::Large => "Large",
            Self::Larger => "Larger",
            Self::Largest => "Largest",
        }
    }

    /// The zoom factor of the whole window.
    pub fn scale(self) -> f32 {
        match self {
            Self::Small => 0.9,
            Self::Default => 1.0,
            Self::Large => 1.15,
            Self::Larger => 1.3,
            Self::Largest => 1.5,
        }
    }

    /// The size whose scale is closest to `scale`.
    pub fn nearest(scale: f32) -> Self {
        let distance = |size: &TextSize| (size.scale() - scale).abs();
        Self::ALL.into_iter().min_by(|a, b| distance(a).total_cmp(&distance(b))).unwrap_or_default()
    }

    /// The next larger size, or the largest.
    pub fn larger(self) -> Self {
        Self::ALL[(self.index() + 1).min(Self::ALL.len() - 1)]
    }

    /// The next smaller size, or the smallest.
    pub fn smaller(self) -> Self {
        Self::ALL[self.index().saturating_sub(1)]
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|size| *size == self).unwrap_or_default()
    }
}

/// How far a Cmd/Ctrl + wheel or pinch gesture has to zoom before the text
/// size moves one step: about one wheel notch, and close to the ratio
/// between neighboring sizes.
const ZOOM_GESTURE_STEP: f32 = 1.15;

/// Gathers the zoom of Cmd/Ctrl + mouse wheel and trackpad pinch, which
/// arrives as a small factor every frame, into whole [`TextSize`] steps.
#[derive(Debug)]
pub struct ZoomGesture {
    /// Product of the frames' zoom factors since the last step.
    gathered: f32,
}

impl Default for ZoomGesture {
    fn default() -> Self {
        ZoomGesture { gathered: 1.0 }
    }
}

impl ZoomGesture {
    /// Add one frame's zoom `factor` (1.0 for none) and return the size to
    /// show: one step larger or smaller than `size` once enough zoom has
    /// gathered, otherwise `size` itself.
    pub fn step(&mut self, size: TextSize, factor: f32) -> TextSize {
        match self.direction(factor) {
            1 => size.larger(),
            -1 => size.smaller(),
            _ => size,
        }
    }

    /// Add one frame's zoom `factor` and say whether a whole step has
    /// gathered: 1 for a step larger, -1 for a step smaller, 0 for none yet.
    pub fn direction(&mut self, factor: f32) -> i8 {
        // A broken factor from the platform must not poison the running product.
        if !factor.is_finite() || factor <= 0.0 {
            return 0;
        }
        self.gathered *= factor;
        if self.gathered >= ZOOM_GESTURE_STEP {
            self.gathered = 1.0;
            1
        } else if self.gathered <= 1.0 / ZOOM_GESTURE_STEP {
            self.gathered = 1.0;
            -1
        } else {
            0
        }
    }
}

/// Zoom of the text area (the editor and the preview) in percent, smallest and largest.
pub const PAGE_ZOOM_PERCENTS: RangeInclusive<u16> = 50..=300;
/// Page zoom when nothing is set, and the percent the reset shortcut returns to.
pub const DEFAULT_PAGE_ZOOM: u16 = 100;
/// How many percent one step of the text area zoom adds or takes away.
pub const PAGE_ZOOM_STEP: u16 = 10;

/// One step more zoom for the text area, up to the largest.
pub fn page_zoom_larger(percent: u16) -> u16 {
    (percent + PAGE_ZOOM_STEP).min(*PAGE_ZOOM_PERCENTS.end())
}

/// One step less zoom for the text area, down to the smallest.
pub fn page_zoom_smaller(percent: u16) -> u16 {
    percent.saturating_sub(PAGE_ZOOM_STEP).max(*PAGE_ZOOM_PERCENTS.start())
}

/// Editor font size when nothing is set.
pub const DEFAULT_EDITOR_FONT_SIZE: u8 = 14;
/// Smallest and largest editor font size.
pub const EDITOR_FONT_SIZES: RangeInclusive<u8> = 9..=28;

/// Settings that make Rubrique easier to read and use.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "each flag is an independent on/off setting")]
pub struct AccessibilitySettings {
    /// Zoom of the whole window.
    pub text_size: TextSize,
    /// Size of the editor's monospace font in points, before the text size.
    pub editor_font_size: u8,
    /// Zoom of the text area only (editor and preview), in percent. The
    /// explorer, tabs and bars keep their size.
    pub page_zoom: u16,
    /// The menu bar along the top of the window; turn it off to give the
    /// page the room (Cmd+, still opens Settings).
    pub show_menu_bar: bool,
    /// No animations.
    pub reduce_motion: bool,
    /// Thicker outlines on the focused control and text field.
    pub strong_focus: bool,
    /// Line numbers to the left of the source editor.
    pub line_numbers: bool,
    /// Long lines in the source editor wrap at the window edge; when off
    /// they run on and the editor scrolls sideways.
    pub wrap_lines: bool,
}

impl Default for AccessibilitySettings {
    fn default() -> Self {
        AccessibilitySettings {
            text_size: TextSize::Default,
            editor_font_size: DEFAULT_EDITOR_FONT_SIZE,
            page_zoom: DEFAULT_PAGE_ZOOM,
            show_menu_bar: true,
            reduce_motion: false,
            strong_focus: false,
            line_numbers: false,
            wrap_lines: true,
        }
    }
}

impl AccessibilitySettings {
    /// The text area zoom as a factor (1.0 for 100%).
    pub fn page_scale(&self) -> f32 {
        f32::from(self.page_zoom) / 100.0
    }

    /// Read one `key=value` line. Returns `false` for keys it does not know.
    pub fn parse_line(&mut self, key: &str, value: &str) -> bool {
        match key {
            "text_scale" => {
                if let Ok(scale) = value.parse::<f32>() {
                    self.text_size = TextSize::nearest(scale);
                }
            }
            "editor_font_size" => self.editor_font_size = parse_font_size(value).unwrap_or(self.editor_font_size),
            "page_zoom" => {
                if let Ok(percent) = value.parse::<u16>() {
                    self.page_zoom = percent.clamp(*PAGE_ZOOM_PERCENTS.start(), *PAGE_ZOOM_PERCENTS.end());
                }
            }
            // Anything but "false" keeps the default, shown.
            "show_menu_bar" => self.show_menu_bar = value != "false",
            "reduce_motion" => self.reduce_motion = value == "true",
            "strong_focus" => self.strong_focus = value == "true",
            "line_numbers" => self.line_numbers = value == "true",
            // Anything but "false" keeps the default, wrapping.
            "wrap_lines" => self.wrap_lines = value != "false",
            _ => return false,
        }
        true
    }

    /// Settings lines for [`AccessibilitySettings::parse_line`].
    pub fn to_lines(&self) -> String {
        format!(
            "text_scale={}\neditor_font_size={}\npage_zoom={}\nshow_menu_bar={}\nreduce_motion={}\nstrong_focus={}\nline_numbers={}\nwrap_lines={}\n",
            self.text_size.scale(),
            self.editor_font_size,
            self.page_zoom,
            self.show_menu_bar,
            self.reduce_motion,
            self.strong_focus,
            self.line_numbers,
            self.wrap_lines
        )
    }
}

/// A whole number of points inside [`EDITOR_FONT_SIZES`].
fn parse_font_size(value: &str) -> Option<u8> {
    let size = value.parse::<f32>().ok().filter(|size| size.is_finite())?;
    let (min, max) = (*EDITOR_FONT_SIZES.start(), *EDITOR_FONT_SIZES.end());
    // Clamped to 9..=28 first, so the cast cannot truncate.
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to the range")]
    Some(size.round().clamp(f32::from(min), f32::from(max)) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> AccessibilitySettings {
        let mut settings = AccessibilitySettings::default();
        for line in text.lines() {
            let (key, value) = line.split_once('=').unwrap();
            settings.parse_line(key, value);
        }
        settings
    }

    #[test]
    fn round_trip() {
        for text_size in TextSize::ALL {
            let settings = AccessibilitySettings {
                text_size,
                editor_font_size: 16,
                page_zoom: 130,
                show_menu_bar: false,
                reduce_motion: true,
                strong_focus: true,
                line_numbers: true,
                wrap_lines: false,
            };
            assert_eq!(parse(&settings.to_lines()), settings);
        }
        assert_eq!(parse(&AccessibilitySettings::default().to_lines()), AccessibilitySettings::default());
        assert!(parse("text_scale=1.15\n").to_lines().contains("text_scale=1.15\n"));
        assert!(!AccessibilitySettings::default().parse_line("theme", "dark"));
    }

    #[test]
    fn lines_wrap_unless_turned_off() {
        assert!(AccessibilitySettings::default().wrap_lines);
        // Settings saved before the option existed keep wrapping.
        assert!(parse("line_numbers=true\n").wrap_lines);
        assert!(!parse("wrap_lines=false\n").wrap_lines);
        assert!(parse("wrap_lines=true\n").wrap_lines);
    }

    #[test]
    fn odd_values_snap_or_are_ignored() {
        let settings = parse("text_scale=1.2\neditor_font_size=99\nreduce_motion=yes\n");
        assert_eq!(settings.text_size, TextSize::Large);
        assert_eq!(settings.editor_font_size, 28);
        assert!(!settings.reduce_motion);
        assert_eq!(parse("editor_font_size=14.4\n").editor_font_size, 14);
        let settings = parse("text_scale=big\neditor_font_size=NaN\n");
        assert_eq!(settings, AccessibilitySettings::default());
    }

    #[test]
    fn page_zoom_steps_stay_in_range() {
        assert_eq!(page_zoom_larger(100), 110);
        assert_eq!(page_zoom_larger(295), 300);
        assert_eq!(page_zoom_smaller(100), 90);
        assert_eq!(page_zoom_smaller(55), 50);
        assert_eq!(parse("page_zoom=900\n").page_zoom, 300);
        assert_eq!(parse("page_zoom=1\n").page_zoom, 50);
        assert_eq!(parse("page_zoom=big\n").page_zoom, DEFAULT_PAGE_ZOOM);
    }

    #[test]
    fn steps_stop_at_the_ends() {
        assert_eq!(TextSize::Default.larger(), TextSize::Large);
        assert_eq!(TextSize::Largest.larger(), TextSize::Largest);
        assert_eq!(TextSize::Default.smaller(), TextSize::Small);
        assert_eq!(TextSize::Small.smaller(), TextSize::Small);
        assert_eq!(TextSize::nearest(5.0), TextSize::Largest);
        assert_eq!(TextSize::nearest(0.1), TextSize::Small);
    }

    #[test]
    fn zoom_gesture_steps_once_enough_has_gathered() {
        let mut gesture = ZoomGesture::default();
        // Small frames add up before the size moves.
        assert_eq!(gesture.step(TextSize::Default, 1.1), TextSize::Default);
        assert_eq!(gesture.step(TextSize::Default, 1.1), TextSize::Large);
        // The gathered zoom starts over after a step.
        assert_eq!(gesture.step(TextSize::Large, 1.1), TextSize::Large);
        assert_eq!(gesture.step(TextSize::Large, 1.0 / 1.1), TextSize::Large);
        assert_eq!(gesture.step(TextSize::Large, 0.8), TextSize::Default);
        // One big frame moves a single step, not several.
        assert_eq!(gesture.step(TextSize::Default, 10.0), TextSize::Large);
    }

    #[test]
    fn zoom_gesture_stops_at_the_ends_and_ignores_bad_factors() {
        let mut gesture = ZoomGesture::default();
        assert_eq!(gesture.step(TextSize::Largest, 2.0), TextSize::Largest);
        assert_eq!(gesture.step(TextSize::Small, 0.5), TextSize::Small);
        for factor in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            assert_eq!(gesture.step(TextSize::Default, factor), TextSize::Default);
        }
        // Still gathering normally afterwards.
        assert_eq!(gesture.step(TextSize::Default, 1.2), TextSize::Large);
    }
}
