//! Which panels and sections are open: the right panel (Structure or
//! Notes), the Files and Terminals sections of the explorer, and where the
//! tabs are.

/// What the right panel shows when it is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SideView {
    /// The headings of the open Markdown file.
    #[default]
    Structure,
    /// The notes in the open Markdown file.
    Notes,
    /// The `@tmux-` mentions in the open Markdown file, to send to terminals.
    Tasks,
    /// The blocks that differ from the saved file, until Save.
    Changes,
}

impl SideView {
    /// The word stored in the settings file.
    fn id(self) -> &'static str {
        match self {
            SideView::Structure => "structure",
            SideView::Notes => "notes",
            SideView::Tasks => "tasks",
            SideView::Changes => "changes",
        }
    }

    /// Read a stored word; anything unknown is Structure.
    fn parse(value: &str) -> Self {
        [SideView::Notes, SideView::Tasks, SideView::Changes]
            .into_iter()
            .find(|view| view.id() == value)
            .unwrap_or(SideView::Structure)
    }
}

/// What a click does in the Markdown preview.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreviewMode {
    /// A click selects text or opens a link; nothing is edited.
    #[default]
    Read,
    /// A click opens the block for editing in place.
    Edit,
}

impl PreviewMode {
    /// The word stored in the settings file.
    fn id(self) -> &'static str {
        match self {
            PreviewMode::Read => "read",
            PreviewMode::Edit => "edit",
        }
    }

    /// Read a stored word; anything unknown is Read.
    fn parse(value: &str) -> Self {
        if value == PreviewMode::Edit.id() { PreviewMode::Edit } else { PreviewMode::Read }
    }
}

/// Where the tabs are listed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabPosition {
    /// A row above the page.
    #[default]
    Top,
    /// A list in the explorer, behind its Files | Tabs switch, which shows
    /// long names in full. The row above the page then lists no tabs.
    Explorer,
}

impl TabPosition {
    /// Both positions, in the order Settings shows them.
    pub const ALL: [TabPosition; 2] = [TabPosition::Top, TabPosition::Explorer];

    /// The word stored in the settings file.
    fn id(self) -> &'static str {
        match self {
            TabPosition::Top => "top",
            TabPosition::Explorer => "explorer",
        }
    }

    /// Read a stored word; anything unknown is Top.
    fn parse(value: &str) -> Self {
        if value == TabPosition::Explorer.id() { TabPosition::Explorer } else { TabPosition::Top }
    }

    /// The name Settings shows.
    pub fn name(self) -> &'static str {
        match self {
            TabPosition::Top => "Top",
            TabPosition::Explorer => "In the explorer",
        }
    }
}

/// What the explorer shows when the tabs are listed in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExplorerView {
    /// The folders and the Terminals section.
    #[default]
    Files,
    /// The open tabs, one per row.
    Tabs,
}

impl ExplorerView {
    /// The word stored in the settings file.
    fn id(self) -> &'static str {
        match self {
            ExplorerView::Files => "files",
            ExplorerView::Tabs => "tabs",
        }
    }

    /// Read a stored word; anything unknown is Files.
    fn parse(value: &str) -> Self {
        if value == ExplorerView::Tabs.id() { ExplorerView::Tabs } else { ExplorerView::Files }
    }
}

/// Open and closed panels, remembered between runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "each flag is one panel or section, opened and closed on its own")]
pub struct LayoutSettings {
    /// The right panel is shown (for Markdown files). Closed by default, so
    /// the page gets the full width until someone asks for the panel.
    pub side_panel_open: bool,
    /// What the right panel shows, kept while it is closed so the toggle
    /// brings back the same view.
    pub side_view: SideView,
    /// The Files section is folded down to its header.
    pub files_collapsed: bool,
    /// The Terminals section is folded down to its header.
    pub terminals_collapsed: bool,
    /// The `OpenRig` section is folded down to its header.
    pub openrig_collapsed: bool,
    /// Where the tabs are: above the page (the default) or in the explorer.
    pub tab_position: TabPosition,
    /// Files or Tabs, when the tabs are in the explorer.
    pub explorer_view: ExplorerView,
    /// Read or Edit, for the Markdown preview of every file.
    pub preview_mode: PreviewMode,
    /// How the explorer sections were folded before a double click gave one
    /// of them the height, to put back on the next one. Not saved.
    pub before_focus: Option<SectionFolds>,
}

/// Which explorer sections are folded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionFolds {
    pub files: bool,
    pub terminals: bool,
    pub openrig: bool,
}

impl LayoutSettings {
    /// A double click on Terminals' header or empty space: fold Files and
    /// open Terminals so it gets the explorer's height, or, when it already
    /// has it, fold the sections back as they were.
    pub fn toggle_terminals_focus(&mut self) {
        let focused = self.files_collapsed && !self.terminals_collapsed;
        self.toggle_focus(focused, SectionFolds { files: true, terminals: false, openrig: self.openrig_collapsed });
    }

    /// A double click on `OpenRig`'s header or empty space: fold Files and
    /// Terminals and open `OpenRig` so it gets the explorer's height, or,
    /// when it already has it, fold the sections back as they were.
    pub fn toggle_openrig_focus(&mut self) {
        let focused = self.files_collapsed && self.terminals_collapsed && !self.openrig_collapsed;
        self.toggle_focus(focused, SectionFolds { files: true, terminals: true, openrig: false });
    }

    /// Apply `focus` and remember the folds before it, or, when `focused`,
    /// put those back (Files open when nothing was remembered).
    fn toggle_focus(&mut self, focused: bool, focus: SectionFolds) {
        let folds = if focused {
            let fallback = SectionFolds { files: false, ..self.section_folds() };
            self.before_focus.take().unwrap_or(fallback)
        } else {
            self.before_focus = Some(self.section_folds());
            focus
        };
        self.files_collapsed = folds.files;
        self.terminals_collapsed = folds.terminals;
        self.openrig_collapsed = folds.openrig;
    }

    /// The folds of the explorer sections now.
    fn section_folds(&self) -> SectionFolds {
        SectionFolds {
            files: self.files_collapsed,
            terminals: self.terminals_collapsed,
            openrig: self.openrig_collapsed,
        }
    }

    /// Apply one `key=value` line. Returns false when the key is not a layout key.
    pub fn parse_line(&mut self, key: &str, value: &str) -> bool {
        match key {
            "side_panel_open" => self.side_panel_open = value == "true",
            "side_view" => self.side_view = SideView::parse(value),
            "files_collapsed" => self.files_collapsed = value == "true",
            "terminals_collapsed" => self.terminals_collapsed = value == "true",
            "openrig_collapsed" => self.openrig_collapsed = value == "true",
            "tab_position" => self.tab_position = TabPosition::parse(value),
            "explorer_view" => self.explorer_view = ExplorerView::parse(value),
            "preview" => self.preview_mode = PreviewMode::parse(value),
            _ => return false,
        }
        true
    }

    /// The `key=value` lines that [`LayoutSettings::parse_line`] reads.
    pub fn to_lines(self) -> String {
        format!(
            "side_panel_open={}\nside_view={}\nfiles_collapsed={}\nterminals_collapsed={}\nopenrig_collapsed={}\n\
             tab_position={}\nexplorer_view={}\npreview={}\n",
            self.side_panel_open,
            self.side_view.id(),
            self.files_collapsed,
            self.terminals_collapsed,
            self.openrig_collapsed,
            self.tab_position.id(),
            self.explorer_view.id(),
            self.preview_mode.id()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A double click gives a list the explorer, and a second one folds the
    /// sections back exactly as they were.
    #[test]
    fn focus_toggles_give_a_list_the_height_and_back() {
        let folds =
            |layout: &LayoutSettings| (layout.files_collapsed, layout.terminals_collapsed, layout.openrig_collapsed);
        let mut layout =
            LayoutSettings { terminals_collapsed: true, openrig_collapsed: true, ..LayoutSettings::default() };
        layout.toggle_openrig_focus();
        assert_eq!(folds(&layout), (true, true, false));
        layout.toggle_openrig_focus();
        assert_eq!(folds(&layout), (false, true, true), "Terminals and OpenRig fold again");

        layout.toggle_terminals_focus();
        assert_eq!(folds(&layout), (true, false, true));
        layout.toggle_terminals_focus();
        assert_eq!(folds(&layout), (false, true, true));

        // Focused at startup (nothing remembered): Files opens again.
        let mut layout = LayoutSettings { files_collapsed: true, ..LayoutSettings::default() };
        layout.toggle_terminals_focus();
        assert_eq!(folds(&layout), (false, false, false));
    }

    #[test]
    fn round_trips_through_lines() {
        let layout = LayoutSettings {
            side_panel_open: true,
            side_view: SideView::Notes,
            files_collapsed: true,
            terminals_collapsed: true,
            openrig_collapsed: true,
            tab_position: TabPosition::Explorer,
            explorer_view: ExplorerView::Tabs,
            preview_mode: PreviewMode::Edit,
            before_focus: None,
        };
        let mut parsed = LayoutSettings::default();
        for line in layout.to_lines().lines() {
            let (key, value) = line.split_once('=').expect("every line is key=value");
            assert!(parsed.parse_line(key, value));
        }
        assert_eq!(parsed, layout);
    }

    #[test]
    fn unknown_view_falls_back_to_structure() {
        let mut layout = LayoutSettings::default();
        layout.parse_line("side_view", "outline");
        assert_eq!(layout.side_view, SideView::Structure);
    }

    #[test]
    fn unknown_preview_mode_falls_back_to_read() {
        let mut layout = LayoutSettings { preview_mode: PreviewMode::Edit, ..LayoutSettings::default() };
        layout.parse_line("preview", "write");
        assert_eq!(layout.preview_mode, PreviewMode::Read);
    }

    #[test]
    fn unknown_tab_position_falls_back_to_top() {
        let mut layout = LayoutSettings { tab_position: TabPosition::Explorer, ..LayoutSettings::default() };
        layout.parse_line("tab_position", "right");
        assert_eq!(layout.tab_position, TabPosition::Top);
    }
}
