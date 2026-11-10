//! The little CSS the HTML preview reads: `property: value` declarations of
//! a `style` attribute, colors, and text alignment.
//!
//! Only values are read here; nothing is fetched or run. A value that is
//! not understood is `None`, and the preview then draws the text plainly.

use super::blocks::ColumnAlign;
use crate::theme::palette::Rgb;

/// Named colors understood, beyond `#hex` and `rgb()`: the 16 basic CSS
/// colors and the extended ones seen most in notes and READMEs.
const NAMED_COLORS: &[(&str, Rgb)] = &[
    ("black", Rgb(0x00, 0x00, 0x00)),
    ("silver", Rgb(0xc0, 0xc0, 0xc0)),
    ("gray", Rgb(0x80, 0x80, 0x80)),
    ("grey", Rgb(0x80, 0x80, 0x80)),
    ("white", Rgb(0xff, 0xff, 0xff)),
    ("maroon", Rgb(0x80, 0x00, 0x00)),
    ("red", Rgb(0xff, 0x00, 0x00)),
    ("purple", Rgb(0x80, 0x00, 0x80)),
    ("fuchsia", Rgb(0xff, 0x00, 0xff)),
    ("magenta", Rgb(0xff, 0x00, 0xff)),
    ("green", Rgb(0x00, 0x80, 0x00)),
    ("lime", Rgb(0x00, 0xff, 0x00)),
    ("olive", Rgb(0x80, 0x80, 0x00)),
    ("yellow", Rgb(0xff, 0xff, 0x00)),
    ("navy", Rgb(0x00, 0x00, 0x80)),
    ("blue", Rgb(0x00, 0x00, 0xff)),
    ("teal", Rgb(0x00, 0x80, 0x80)),
    ("aqua", Rgb(0x00, 0xff, 0xff)),
    ("cyan", Rgb(0x00, 0xff, 0xff)),
    ("orange", Rgb(0xff, 0xa5, 0x00)),
    ("gold", Rgb(0xff, 0xd7, 0x00)),
    ("pink", Rgb(0xff, 0xc0, 0xcb)),
    ("brown", Rgb(0xa5, 0x2a, 0x2a)),
    ("crimson", Rgb(0xdc, 0x14, 0x3c)),
    ("tomato", Rgb(0xff, 0x63, 0x47)),
    ("coral", Rgb(0xff, 0x7f, 0x50)),
    ("indigo", Rgb(0x4b, 0x00, 0x82)),
    ("violet", Rgb(0xee, 0x82, 0xee)),
    ("darkred", Rgb(0x8b, 0x00, 0x00)),
    ("darkgreen", Rgb(0x00, 0x64, 0x00)),
    ("darkblue", Rgb(0x00, 0x00, 0x8b)),
    ("darkorange", Rgb(0xff, 0x8c, 0x00)),
    ("lightgreen", Rgb(0x90, 0xee, 0x90)),
    ("lightblue", Rgb(0xad, 0xd8, 0xe6)),
    ("lightgray", Rgb(0xd3, 0xd3, 0xd3)),
    ("lightgrey", Rgb(0xd3, 0xd3, 0xd3)),
    ("darkgray", Rgb(0xa9, 0xa9, 0xa9)),
    ("darkgrey", Rgb(0xa9, 0xa9, 0xa9)),
    ("steelblue", Rgb(0x46, 0x82, 0xb4)),
    ("royalblue", Rgb(0x41, 0x69, 0xe1)),
    ("dodgerblue", Rgb(0x1e, 0x90, 0xff)),
    ("forestgreen", Rgb(0x22, 0x8b, 0x22)),
    ("seagreen", Rgb(0x2e, 0x8b, 0x57)),
    ("slategray", Rgb(0x70, 0x80, 0x90)),
    ("slategrey", Rgb(0x70, 0x80, 0x90)),
];

/// Longest `style` attribute read. A longer one is cut there, so a hostile
/// page cannot make every element scan megabytes of declarations.
const MAX_STYLE_LENGTH: usize = 1024;

/// The `property: value` pairs of a `style` attribute, property names in
/// lowercase, both trimmed and any `!important` dropped. Pieces without a
/// `:` are skipped.
pub fn declarations(style: &str) -> impl Iterator<Item = (String, &str)> {
    let end = style.char_indices().nth(MAX_STYLE_LENGTH).map_or(style.len(), |(at, _)| at);
    style[..end].split(';').filter_map(|declaration| {
        let (property, value) = declaration.split_once(':')?;
        let value = value.trim();
        let value = value.strip_suffix("!important").map_or(value, str::trim_end);
        Some((property.trim().to_ascii_lowercase(), value))
    })
}

/// The value of `property` in a `style` attribute; the last one wins, as
/// in CSS.
pub fn property<'a>(style: &'a str, property: &str) -> Option<&'a str> {
    declarations(style).filter(|(name, _)| name == property).last().map(|(_, value)| value)
}

/// A CSS color: `#rgb`, `#rrggbb` (with or without alpha, which is
/// ignored), `rgb()`/`rgba()` with numbers or percentages, or one of the
/// common names. `None` for anything else, `transparent` included.
pub fn parse_color(value: &str) -> Option<Rgb> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex(hex);
    }
    if let Some(arguments) = value.strip_prefix("rgba(").or_else(|| value.strip_prefix("rgb(")) {
        return parse_rgb_function(arguments.strip_suffix(')')?);
    }
    NAMED_COLORS.iter().find(|(name, _)| *name == value).map(|(_, color)| *color)
}

/// `rgb`, `rgba`, `rrggbb` or `rrggbbaa` hex digits.
fn parse_hex(hex: &str) -> Option<Rgb> {
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        3 | 4 => {
            // Each digit stands for itself twice: `f80` is `ff8800`.
            let digit = |at: usize| u8::from_str_radix(&hex[at..=at], 16).ok().map(|value| value * 0x11);
            Some(Rgb(digit(0)?, digit(1)?, digit(2)?))
        }
        6 | 8 => Rgb::parse(&hex[..6]),
        _ => None,
    }
}

/// The inside of `rgb(...)`: three channels split by commas or spaces,
/// then an optional alpha (after a comma or a `/`), which is ignored.
fn parse_rgb_function(arguments: &str) -> Option<Rgb> {
    let mut channels = arguments.split([',', ' ', '/']).filter(|part| !part.is_empty()).map(parse_channel);
    Some(Rgb(channels.next()??, channels.next()??, channels.next()??))
}

/// One `rgb()` channel: 0 to 255, or a percentage; out of range values are
/// clamped, as browsers do.
fn parse_channel(text: &str) -> Option<u8> {
    let value: f32 = match text.strip_suffix('%') {
        Some(percent) => percent.parse::<f32>().ok()? * 2.55,
        None => text.parse().ok()?,
    };
    if !value.is_finite() {
        return None;
    }
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to 0..=255 first")]
    Some(value.round().clamp(0.0, 255.0) as u8)
}

/// `left`, `center` or `right` (as in `align="center"` or `text-align`).
/// `justify`, `start` and `end` read as left, the way most notes are set.
pub fn parse_align(value: &str) -> Option<ColumnAlign> {
    match value.trim().to_ascii_lowercase().as_str() {
        "center" => Some(ColumnAlign::Center),
        "right" | "end" => Some(ColumnAlign::Right),
        "left" | "start" | "justify" => Some(ColumnAlign::Left),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations_are_split_and_trimmed() {
        let found: Vec<_> = declarations(" Color : red ;font-weight:bold;; junk ").collect();
        assert_eq!(found, [("color".to_string(), "red"), ("font-weight".to_string(), "bold")]);
        assert_eq!(property("color: red; COLOR: blue !important", "color"), Some("blue"));
        assert_eq!(property("color: red", "background-color"), None);
    }

    #[test]
    fn colors_in_every_common_form() {
        assert_eq!(parse_color("#F80"), Some(Rgb(0xff, 0x88, 0x00)));
        assert_eq!(parse_color("#ff8800cc"), Some(Rgb(0xff, 0x88, 0x00)));
        assert_eq!(parse_color(" rgb(255, 0, 10) "), Some(Rgb(255, 0, 10)));
        assert_eq!(parse_color("rgba(0 128 255 / 50%)"), Some(Rgb(0, 128, 255)));
        assert_eq!(parse_color("rgb(100%, 0%, 300)"), Some(Rgb(255, 0, 255)));
        assert_eq!(parse_color("DarkGreen"), Some(Rgb(0x00, 0x64, 0x00)));
    }

    #[test]
    fn unknown_colors_are_none() {
        for value in
            ["", "transparent", "inherit", "#12", "#ggg", "rgb(1, 2)", "rgb(a, b, c)", "var(--x)", "rgb(nan, 0, 0)"]
        {
            assert_eq!(parse_color(value), None, "{value}");
        }
    }

    #[test]
    fn alignment_names() {
        assert_eq!(parse_align(" Center"), Some(ColumnAlign::Center));
        assert_eq!(parse_align("right"), Some(ColumnAlign::Right));
        assert_eq!(parse_align("justify"), Some(ColumnAlign::Left));
        assert_eq!(parse_align("top"), None);
    }

    #[test]
    fn very_long_styles_are_cut() {
        let style = format!("{}; color: red", "x".repeat(MAX_STYLE_LENGTH * 4));
        assert_eq!(property(&style, "color"), None);
    }
}
