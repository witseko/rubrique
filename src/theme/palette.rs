//! Theme colors and the `.theme` file format (`key=value` lines, like settings).

/// A color as red, green, blue.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Parse `#rrggbb`; the `#` is optional.
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#').unwrap_or(text);
        if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |start: usize| u8::from_str_radix(&hex[start..start + 2], 16).ok();
        Some(Rgb(channel(0)?, channel(2)?, channel(4)?))
    }

    /// True for colors closer to black than to white (perceived brightness).
    fn is_dark(self) -> bool {
        let Rgb(red, green, blue) = self;
        let brightness = 299 * u32::from(red) + 587 * u32::from(green) + 114 * u32::from(blue);
        brightness < 128 * 1000
    }
}

/// The built-in theme a theme starts from. It also tells egui whether this
/// is a light or a dark look (for things a theme does not color).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    Light,
    Dark,
}

impl Base {
    /// The `base=` value and the id of the matching built-in theme.
    pub fn id(self) -> &'static str {
        match self {
            Base::Light => "light",
            Base::Dark => "dark",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "light" => Some(Base::Light),
            "dark" => Some(Base::Dark),
            _ => None,
        }
    }

    /// Text of the built-in theme file, shipped inside the binary.
    fn builtin_text(self) -> &'static str {
        match self {
            Base::Light => include_str!("../../themes/light.theme"),
            Base::Dark => include_str!("../../themes/dark.theme"),
        }
    }
}

/// Every color a theme sets. See `themes/light.theme` for what each one paints.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Palette {
    pub background: Rgb,
    pub sidebar: Rgb,
    pub surface: Rgb,
    pub hover: Rgb,
    pub border: Rgb,
    pub text: Rgb,
    pub text_strong: Rgb,
    pub text_weak: Rgb,
    pub accent: Rgb,
    pub selection: Rgb,
    pub code_background: Rgb,
    pub warning: Rgb,
    pub error: Rgb,
}

impl Palette {
    /// The color stored under a theme file key.
    fn color_mut(&mut self, key: &str) -> Option<&mut Rgb> {
        let color = match key {
            "background" => &mut self.background,
            "sidebar" => &mut self.sidebar,
            "surface" => &mut self.surface,
            "hover" => &mut self.hover,
            "border" => &mut self.border,
            "text" => &mut self.text,
            "text_strong" => &mut self.text_strong,
            "text_weak" => &mut self.text_weak,
            "accent" => &mut self.accent,
            "selection" => &mut self.selection,
            "code_background" => &mut self.code_background,
            "warning" => &mut self.warning,
            "error" => &mut self.error,
            _ => return None,
        };
        Some(color)
    }

    /// Set every valid `color=#rrggbb` line; other lines are left alone.
    fn apply(&mut self, text: &str) {
        for (key, value) in key_values(text) {
            if let (Some(color), Some(rgb)) = (self.color_mut(key), Rgb::parse(value)) {
                *color = rgb;
            }
        }
    }
}

/// A named set of colors that the app can switch to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    /// File name without `.theme`; what the settings file stores.
    pub id: String,
    /// Shown in the theme picker.
    pub name: String,
    pub base: Base,
    pub palette: Palette,
}

impl Theme {
    /// One of the two themes shipped with Rubrique.
    pub fn builtin(base: Base) -> Self {
        let text = base.builtin_text();
        let mut palette = Palette::default();
        palette.apply(text);
        let name = value_of(text, "name").unwrap_or(base.id()).to_string();
        Theme { id: base.id().to_string(), name, base, palette }
    }

    /// Read a theme file. It starts from its `base` theme, so a file only
    /// needs the colors it changes. Without `base`, a dark `background` picks
    /// the dark theme and anything else the light one. Bad lines are ignored.
    pub fn parse(id: &str, text: &str) -> Self {
        let background = value_of(text, "background").and_then(Rgb::parse);
        let base = value_of(text, "base").and_then(Base::parse).unwrap_or(match background {
            Some(color) if !color.is_dark() => Base::Light,
            Some(_) => Base::Dark,
            None => Base::Light,
        });
        let mut palette = Theme::builtin(base).palette;
        palette.apply(text);
        let name = value_of(text, "name").filter(|name| !name.is_empty()).unwrap_or(id).to_string();
        Theme { id: id.to_string(), name, base, palette }
    }
}

/// Trimmed `key=value` pairs; comments (`#`) and lines without `=` are skipped.
fn key_values(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
}

/// The last value given for `key`.
fn value_of<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    key_values(text).filter(|(found, _)| *found == key).map(|(_, value)| value).last()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colors() {
        assert_eq!(Rgb::parse("#1d1F23"), Some(Rgb(0x1d, 0x1f, 0x23)));
        assert_eq!(Rgb::parse("ffffff"), Some(Rgb(255, 255, 255)));
        for bad in ["", "#fff", "#12345", "#1234567", "#gg0000", "#+f0000", "#ééé"] {
            assert_eq!(Rgb::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn builtin_files_set_every_color() {
        // Each key must appear in the file: a missing one would stay black.
        for base in [Base::Light, Base::Dark] {
            let text = base.builtin_text();
            let mut probe = Palette::default();
            let keys: Vec<&str> =
                key_values(text).map(|(key, _)| key).filter(|key| probe.color_mut(key).is_some()).collect();
            assert_eq!(keys.len(), 13, "{base:?} has {keys:?}");
            for (key, value) in key_values(text) {
                if probe.color_mut(key).is_some() {
                    assert!(Rgb::parse(value).is_some(), "{base:?}: bad color {key}={value}");
                }
            }
        }
    }

    #[test]
    fn builtin_themes_have_names_and_ids() {
        let light = Theme::builtin(Base::Light);
        let dark = Theme::builtin(Base::Dark);
        assert_eq!((light.id.as_str(), light.name.as_str()), ("light", "Light"));
        assert_eq!((dark.id.as_str(), dark.name.as_str()), ("dark", "Indigo"));
        assert_ne!(light.palette.background, dark.palette.background);
        assert!(!light.palette.background.is_dark());
        assert!(dark.palette.background.is_dark());
    }

    #[test]
    fn a_theme_only_needs_the_colors_it_changes() {
        let theme = Theme::parse("ocean", "name=Ocean\nbase=dark\naccent=#00aaff\n");
        let dark = Theme::builtin(Base::Dark);
        assert_eq!(theme.name, "Ocean");
        assert_eq!(theme.base, Base::Dark);
        assert_eq!(theme.palette.accent, Rgb(0, 0xaa, 0xff));
        assert_eq!(theme.palette.background, dark.palette.background);
    }

    #[test]
    fn base_follows_the_background_when_missing() {
        assert_eq!(Theme::parse("a", "background=#101010").base, Base::Dark);
        assert_eq!(Theme::parse("b", "background=#f0f0f0").base, Base::Light);
        assert_eq!(Theme::parse("c", "").base, Base::Light);
    }

    #[test]
    fn bad_lines_are_ignored_and_name_defaults_to_id() {
        let theme = Theme::parse("mine", "nonsense\nbase=purple\ntext=blue\nsize=3\n# accent=#000000\n");
        assert_eq!(theme.name, "mine");
        assert_eq!(theme.palette, Theme::builtin(Base::Light).palette);
    }
}
