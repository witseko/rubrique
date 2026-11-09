//! Code colors: how syntax is colored in the editor and preview, either from
//! the current theme or with Visual Studio Code's default colors.

use eframe::egui::{Color32, Context, Id, Visuals};

/// What the settings file stores for each choice.
const THEME_ID: &str = "theme";
const VS_CODE_ID: &str = "vscode";

/// Which colors syntax highlighting uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CodeColors {
    /// The theme's own colors: keywords in the accent, strings in the
    /// warning color, numbers in the error color.
    #[default]
    Theme,
    /// VS Code's Dark+ colors on dark themes and Light+ on light ones.
    VsCode,
}

impl CodeColors {
    pub const ALL: [CodeColors; 2] = [CodeColors::Theme, CodeColors::VsCode];

    /// Read a `code_colors=` settings value; anything unknown is `Theme`.
    pub fn parse(value: &str) -> Self {
        match value {
            VS_CODE_ID => CodeColors::VsCode,
            _ => CodeColors::Theme,
        }
    }

    /// The `code_colors=` settings value.
    pub fn id(self) -> &'static str {
        match self {
            CodeColors::Theme => THEME_ID,
            CodeColors::VsCode => VS_CODE_ID,
        }
    }

    /// Shown in the settings panel.
    pub fn name(self) -> &'static str {
        match self {
            CodeColors::Theme => "Theme colors",
            CodeColors::VsCode => "VS Code",
        }
    }
}

/// Where the choice is kept in egui's memory, so every view that colors
/// syntax can read it without passing the settings around.
fn memory_id() -> Id {
    Id::unique("rubrique-code-colors")
}

/// Make `choice` the code colors of every view from the next frame on.
pub fn install_code_colors(ctx: &Context, choice: CodeColors) {
    ctx.data_mut(|data| data.insert_temp(memory_id(), choice));
}

/// The code colors installed with [`install_code_colors`] (`Theme` before that).
pub fn installed_code_colors(ctx: &Context) -> CodeColors {
    ctx.data(|data| data.get_temp(memory_id())).unwrap_or_default()
}

/// The color of each kind of syntax that code colors change. Markdown
/// markers, links, bold and struck text keep the theme's colors in every
/// choice, so prose reads the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SyntaxColors {
    /// Code keywords and XML tag names.
    pub keyword: Color32,
    /// Defined names, decorators and XML attribute names.
    pub name: Color32,
    /// YAML and JSON keys.
    pub key: Color32,
    pub heading: Color32,
    pub string: Color32,
    pub number: Color32,
    pub comment: Color32,
    /// Text in a Markdown block quote.
    pub quote: Color32,
    /// Text of Markdown code spans and fenced code.
    pub code: Color32,
    /// The inside of a Helm `{{ ... }}` action.
    pub template: Color32,
    /// The theme draws comments and quotes in italics; VS Code does not.
    pub italic_comments: bool,
}

impl SyntaxColors {
    /// The colors for `choice` on a theme with these `visuals`.
    pub fn new(choice: CodeColors, visuals: &Visuals) -> Self {
        match (choice, visuals.dark_mode) {
            (CodeColors::Theme, _) => SyntaxColors::from_theme(visuals),
            (CodeColors::VsCode, true) => VS_CODE_DARK,
            (CodeColors::VsCode, false) => VS_CODE_LIGHT,
        }
    }

    /// The theme's colors, as Rubrique has always colored syntax.
    fn from_theme(visuals: &Visuals) -> Self {
        let accent = visuals.hyperlink_color;
        let strong = visuals.strong_text_color();
        let weak = visuals.weak_text_color();
        SyntaxColors {
            keyword: accent,
            name: strong,
            key: accent,
            heading: accent,
            string: visuals.warn_fg_color,
            number: visuals.error_fg_color,
            comment: weak,
            quote: weak,
            code: visuals.text_color(),
            template: strong,
            italic_comments: true,
        }
    }
}

/// VS Code's Dark+ colors. Where Rubrique's scanner joins kinds VS Code
/// tells apart (JSON and YAML keys, class and function names), the more
/// common one wins: JSON keys and function names.
const VS_CODE_DARK: SyntaxColors = SyntaxColors {
    keyword: Color32::from_rgb(0x56, 0x9c, 0xd6),
    name: Color32::from_rgb(0xdc, 0xdc, 0xaa),
    key: Color32::from_rgb(0x9c, 0xdc, 0xfe),
    heading: Color32::from_rgb(0x56, 0x9c, 0xd6),
    string: Color32::from_rgb(0xce, 0x91, 0x78),
    number: Color32::from_rgb(0xb5, 0xce, 0xa8),
    comment: Color32::from_rgb(0x6a, 0x99, 0x55),
    quote: Color32::from_rgb(0x6a, 0x99, 0x55),
    code: Color32::from_rgb(0xce, 0x91, 0x78),
    template: Color32::from_rgb(0xc5, 0x86, 0xc0),
    italic_comments: false,
};

/// VS Code's Light+ colors, chosen the same way as [`VS_CODE_DARK`].
const VS_CODE_LIGHT: SyntaxColors = SyntaxColors {
    keyword: Color32::from_rgb(0x00, 0x00, 0xff),
    name: Color32::from_rgb(0x79, 0x5e, 0x26),
    key: Color32::from_rgb(0x04, 0x51, 0xa5),
    heading: Color32::from_rgb(0x80, 0x00, 0x00),
    string: Color32::from_rgb(0xa3, 0x15, 0x15),
    number: Color32::from_rgb(0x09, 0x86, 0x58),
    comment: Color32::from_rgb(0x00, 0x80, 0x00),
    quote: Color32::from_rgb(0x04, 0x51, 0xa5),
    code: Color32::from_rgb(0x80, 0x00, 0x00),
    template: Color32::from_rgb(0xaf, 0x00, 0xdb),
    italic_comments: false,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_round_trip() {
        for choice in CodeColors::ALL {
            assert_eq!(CodeColors::parse(choice.id()), choice);
        }
        assert_eq!(CodeColors::parse(""), CodeColors::Theme);
        assert_eq!(CodeColors::parse("monokai"), CodeColors::Theme);
    }

    #[test]
    fn theme_colors_follow_the_theme() {
        let visuals = Visuals::dark();
        let colors = SyntaxColors::new(CodeColors::Theme, &visuals);
        assert_eq!(colors.keyword, visuals.hyperlink_color);
        assert_eq!(colors.string, visuals.warn_fg_color);
        assert!(colors.italic_comments);
    }

    #[test]
    fn vs_code_picks_dark_or_light_by_the_theme() {
        assert_eq!(SyntaxColors::new(CodeColors::VsCode, &Visuals::dark()), VS_CODE_DARK);
        assert_eq!(SyntaxColors::new(CodeColors::VsCode, &Visuals::light()), VS_CODE_LIGHT);
    }

    #[test]
    fn the_choice_is_kept_in_egui_memory() {
        let ctx = Context::default();
        assert_eq!(installed_code_colors(&ctx), CodeColors::Theme);
        install_code_colors(&ctx, CodeColors::VsCode);
        assert_eq!(installed_code_colors(&ctx), CodeColors::VsCode);
    }
}
