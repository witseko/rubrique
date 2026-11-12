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
    /// Where the tabs are: above the page (the default) or in the explorer.
    pub tab_position: TabPosition,
    /// Files or Tabs, when the tabs are in the explorer.
    pub explorer_view: ExplorerView,
}

impl LayoutSettings {
    /// Apply one `key=value` line. Returns false when the key is not a layout key.
    pub fn parse_line(&mut self, key: &str, value: &str) -> bool {
        match key {
            "side_panel_open" => self.side_panel_open = value == "true",
            "side_view" => self.side_view = SideView::parse(value),
            "files_collapsed" => self.files_collapsed = value == "true",
            "terminals_collapsed" => self.terminals_collapsed = value == "true",
            "tab_position" => self.tab_position = TabPosition::parse(value),
            "explorer_view" => self.explorer_view = ExplorerView::parse(value),
            _ => return false,
        }
        true
    }

    /// The `key=value` lines that [`LayoutSettings::parse_line`] reads.
    pub fn to_lines(self) -> String {
        format!(
            "side_panel_open={}\nside_view={}\nfiles_collapsed={}\nterminals_collapsed={}\ntab_position={}\nexplorer_view={}\n",
            self.side_panel_open,
            self.side_view.id(),
            self.files_collapsed,
            self.terminals_collapsed,
            self.tab_position.id(),
            self.explorer_view.id()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_lines() {
        let layout = LayoutSettings {
            side_panel_open: true,
            side_view: SideView::Notes,
            files_collapsed: true,
            terminals_collapsed: true,
            tab_position: TabPosition::Explorer,
            explorer_view: ExplorerView::Tabs,
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
    fn unknown_tab_position_falls_back_to_top() {
        let mut layout = LayoutSettings { tab_position: TabPosition::Explorer, ..LayoutSettings::default() };
        layout.parse_line("tab_position", "right");
        assert_eq!(layout.tab_position, TabPosition::Top);
    }
}
