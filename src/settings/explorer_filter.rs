//! The explorer's file filter (the funnel in the Files header): which files
//! the tree shows.

/// What the explorer shows. By default every file is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExplorerFilter {
    /// Hide files that are not Markdown (terminal files stay).
    pub markdown_only: bool,
    /// Show only files with unsaved edits, and the folders holding them.
    pub unsaved_only: bool,
    /// Show `.tmux` files whose session is not running. On by default:
    /// closed terminals stay visible, and the ones nobody needs get deleted.
    pub show_closed_terminals: bool,
}

impl Default for ExplorerFilter {
    fn default() -> Self {
        ExplorerFilter { markdown_only: false, unsaved_only: false, show_closed_terminals: true }
    }
}

impl ExplorerFilter {
    /// True when at least one filter hides files (the funnel shows as active).
    pub fn any(self) -> bool {
        self.markdown_only || self.unsaved_only || !self.show_closed_terminals
    }

    /// Apply one `key=value` line. Returns false when the key is not a filter key.
    pub fn parse_line(&mut self, key: &str, value: &str) -> bool {
        match key {
            // `markdown_only` is the key from before the other filters existed.
            "markdown_only" => self.markdown_only = value == "true",
            "unsaved_only" => self.unsaved_only = value == "true",
            "show_closed_terminals" => self.show_closed_terminals = value == "true",
            // 0.9.4 saved the opposite switch, "Hide closed terminals".
            "hide_closed_terminals" => self.show_closed_terminals = value != "true",
            _ => return false,
        }
        true
    }

    /// The `key=value` lines that [`ExplorerFilter::parse_line`] reads.
    pub fn to_lines(self) -> String {
        format!(
            "markdown_only={}\nunsaved_only={}\nshow_closed_terminals={}\n",
            self.markdown_only, self.unsaved_only, self.show_closed_terminals
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_lines() {
        let filter = ExplorerFilter { markdown_only: true, unsaved_only: false, show_closed_terminals: false };
        let mut read = ExplorerFilter::default();
        for line in filter.to_lines().lines() {
            let (key, value) = line.split_once('=').expect("every line is key=value");
            assert!(read.parse_line(key, value));
        }
        assert_eq!(read, filter);
    }

    #[test]
    fn any_is_false_until_a_filter_hides_files() {
        assert!(!ExplorerFilter::default().any());
        assert!(ExplorerFilter { unsaved_only: true, ..ExplorerFilter::default() }.any());
        assert!(ExplorerFilter { show_closed_terminals: false, ..ExplorerFilter::default() }.any());
    }

    #[test]
    fn closed_terminals_are_shown_by_default() {
        assert!(ExplorerFilter::default().show_closed_terminals);
    }

    #[test]
    fn old_hide_closed_terminals_line_is_read_as_its_opposite() {
        let mut filter = ExplorerFilter::default();
        assert!(filter.parse_line("hide_closed_terminals", "true"));
        assert!(!filter.show_closed_terminals);
        assert!(filter.parse_line("hide_closed_terminals", "false"));
        assert!(filter.show_closed_terminals);
    }
}
