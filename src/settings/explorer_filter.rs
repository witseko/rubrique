//! The explorer's file filter (the funnel in the Files header): which files
//! the tree shows.

/// What the explorer hides. With nothing set, every file is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExplorerFilter {
    /// Hide files that are not Markdown (terminal files stay).
    pub markdown_only: bool,
    /// Show only files with unsaved edits, and the folders holding them.
    pub unsaved_only: bool,
    /// Hide `.tmux` files whose session is not running.
    pub hide_closed_terminals: bool,
}

impl ExplorerFilter {
    /// True when at least one filter is on (the funnel shows as active).
    pub fn any(self) -> bool {
        self.markdown_only || self.unsaved_only || self.hide_closed_terminals
    }

    /// Apply one `key=value` line. Returns false when the key is not a filter key.
    pub fn parse_line(&mut self, key: &str, value: &str) -> bool {
        match key {
            // `markdown_only` is the key from before the other filters existed.
            "markdown_only" => self.markdown_only = value == "true",
            "unsaved_only" => self.unsaved_only = value == "true",
            "hide_closed_terminals" => self.hide_closed_terminals = value == "true",
            _ => return false,
        }
        true
    }

    /// The `key=value` lines that [`ExplorerFilter::parse_line`] reads.
    pub fn to_lines(self) -> String {
        format!(
            "markdown_only={}\nunsaved_only={}\nhide_closed_terminals={}\n",
            self.markdown_only, self.unsaved_only, self.hide_closed_terminals
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_lines() {
        let filter = ExplorerFilter { markdown_only: true, unsaved_only: false, hide_closed_terminals: true };
        let mut read = ExplorerFilter::default();
        for line in filter.to_lines().lines() {
            let (key, value) = line.split_once('=').expect("every line is key=value");
            assert!(read.parse_line(key, value));
        }
        assert_eq!(read, filter);
    }

    #[test]
    fn any_is_false_until_a_filter_is_on() {
        assert!(!ExplorerFilter::default().any());
        assert!(ExplorerFilter { unsaved_only: true, ..ExplorerFilter::default() }.any());
    }
}
