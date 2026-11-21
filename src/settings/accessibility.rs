//! Text size (and the zoom gesture that steps it), text area zoom,
//! line wrapping, reduced motion and focus outlines.

use std::ops::RangeInclusive;

/// Text size (the zoom of the whole window) in percent, smallest and largest.
pub const TEXT_SIZE_PERCENTS: RangeInclusive<u16> = 80..=200;
/// Text size when nothing is set, and the percent the reset shortcut returns to.
pub const DEFAULT_TEXT_SIZE: u16 = 100;
/// How many percent one step of the text size adds or takes away.
pub const TEXT_SIZE_STEP: u16 = 5;

/// One step larger text, up to the largest.
pub fn text_size_larger(percent: u16) -> u16 {
    (percent + TEXT_SIZE_STEP).min(*TEXT_SIZE_PERCENTS.end())
}

/// One step smaller text, down to the smallest.
pub fn text_size_smaller(percent: u16) -> u16 {
    percent.saturating_sub(TEXT_SIZE_STEP).max(*TEXT_SIZE_PERCENTS.start())
}

/// How far a Cmd/Ctrl + wheel or pinch gesture has to zoom before the text
/// size moves one step: about one wheel notch.
pub const ZOOM_GESTURE_STEP: f32 = 1.15;

/// Gathers the zoom of Cmd/Ctrl + mouse wheel and trackpad pinch, which
/// arrives as a small factor every frame, into whole steps.
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
    /// Add one frame's zoom `factor` (1.0 for none) and say whether a whole step has
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

/// Size of the editor's monospace font in points at 100% text area zoom.
pub const EDITOR_FONT_SIZE: u8 = 14;

/// Settings that make Rubrique easier to read and use.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "each flag is an independent on/off setting")]
pub struct AccessibilitySettings {
    /// Zoom of the whole window, in percent.
    pub text_size: u16,
    /// Zoom of the text area only (editor and preview), in percent: it
    /// scales the editor's font from [`EDITOR_FONT_SIZE`]. The
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
            text_size: DEFAULT_TEXT_SIZE,
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
    /// The text size as the zoom factor of the whole window (1.0 for 100%).
    pub fn text_scale(&self) -> f32 {
        f32::from(self.text_size) / 100.0
    }

    /// The text area zoom as a factor (1.0 for 100%).
    pub fn page_scale(&self) -> f32 {
        f32::from(self.page_zoom) / 100.0
    }

    /// Read one `key=value` line. Returns `false` for keys it does not know.
    pub fn parse_line(&mut self, key: &str, value: &str) -> bool {
        match key {
            "text_scale" => {
                self.text_size = parse_text_size(value).unwrap_or(self.text_size);
            }
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
            "text_scale={}\npage_zoom={}\nshow_menu_bar={}\nreduce_motion={}\nstrong_focus={}\nline_numbers={}\nwrap_lines={}\n",
            self.text_scale(),
            self.page_zoom,
            self.show_menu_bar,
            self.reduce_motion,
            self.strong_focus,
            self.line_numbers,
            self.wrap_lines
        )
    }
}

/// A saved zoom factor (1.15) as a percent inside [`TEXT_SIZE_PERCENTS`].
fn parse_text_size(value: &str) -> Option<u16> {
    let scale = value.parse::<f32>().ok().filter(|scale| scale.is_finite())?;
    let (min, max) = (*TEXT_SIZE_PERCENTS.start(), *TEXT_SIZE_PERCENTS.end());
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to the range")]
    Some((scale * 100.0).round().clamp(f32::from(min), f32::from(max)) as u16)
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
        for text_size in [80, 100, 115, 125, 200] {
            let settings = AccessibilitySettings {
                text_size,
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
        let settings = parse("text_scale=1.2\nreduce_motion=yes\n");
        assert_eq!(settings.text_size, 120);
        assert!(!settings.reduce_motion);
        let settings = parse("text_scale=big\n");
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
    fn text_size_steps_stay_in_range() {
        assert_eq!(text_size_larger(100), 105);
        assert_eq!(text_size_larger(198), 200);
        assert_eq!(text_size_smaller(100), 95);
        assert_eq!(text_size_smaller(82), 80);
        assert_eq!(parse("text_scale=5.0\n").text_size, 200);
        assert_eq!(parse("text_scale=0.1\n").text_size, 80);
        assert_eq!(parse("text_scale=-1\n").text_size, 80);
    }

    #[test]
    fn zoom_gesture_steps_once_enough_has_gathered() {
        let mut gesture = ZoomGesture::default();
        // Small frames add up before a step.
        assert_eq!(gesture.direction(1.1), 0);
        assert_eq!(gesture.direction(1.1), 1);
        // The gathered zoom starts over after a step.
        assert_eq!(gesture.direction(1.1), 0);
        assert_eq!(gesture.direction(1.0 / 1.1), 0);
        assert_eq!(gesture.direction(0.8), -1);
        // One big frame moves a single step, not several.
        assert_eq!(gesture.direction(10.0), 1);
    }

    #[test]
    fn zoom_gesture_ignores_bad_factors() {
        let mut gesture = ZoomGesture::default();
        for factor in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            assert_eq!(gesture.direction(factor), 0);
        }
        // Still gathering normally afterwards.
        assert_eq!(gesture.direction(1.2), 1);
    }
}
