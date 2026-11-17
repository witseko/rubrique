//! reStructuredText tables as rows of cell text: simple tables (columns
//! under `=====  =====` borders), grid tables (`+-----+-----+` borders) and
//! the rows of a `csv-table` directive. The block reader turns the text
//! into inlines.
//!
//! Cells that span columns or rows are read as if they did not span: their
//! text lands in the first column or row they cover.

/// A table's cell text: header rows, then body rows.
#[derive(Debug, Default, PartialEq)]
pub(super) struct RawTable {
    pub(super) header: Vec<Vec<String>>,
    pub(super) rows: Vec<Vec<String>>,
}

/// The column ranges (in characters) of a simple table border such as
/// `=====  ======`, which needs at least two columns: a single run of `=`
/// is a title adornment or a transition.
fn simple_border_columns(line: &str) -> Option<Vec<(usize, usize)>> {
    let line = line.trim_end();
    if line.is_empty() || line.starts_with(' ') || !line.chars().all(|character| character == '=' || character == ' ') {
        return None;
    }
    let mut columns = Vec::new();
    let mut start = None;
    for (at, character) in line.chars().enumerate() {
        match (character, start) {
            ('=', None) => start = Some(at),
            (' ', Some(from)) => {
                columns.push((from, at));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        columns.push((from, line.chars().count()));
    }
    (columns.len() >= 2).then_some(columns)
}

/// The simple table starting at `lines[index]`, and the index after it.
pub(super) fn simple_table(lines: &[&str], index: usize) -> Option<(RawTable, usize)> {
    let columns = simple_border_columns(lines[index])?;
    // Sections of row lines between borders; the table ends at a border
    // followed by a blank line or the end of the file.
    let mut sections: Vec<Vec<&str>> = vec![Vec::new()];
    let mut next = index + 1;
    loop {
        let line = *lines.get(next)?;
        next += 1;
        if simple_border_columns(line).is_none() {
            sections.last_mut()?.push(line);
            continue;
        }
        if lines.get(next).is_none_or(|after| after.trim().is_empty()) {
            break;
        }
        sections.push(Vec::new());
    }
    let mut table = RawTable::default();
    let header_count = usize::from(sections.len() >= 2);
    for (position, section) in sections.iter().enumerate() {
        let rows = simple_rows(section, &columns);
        if position < header_count {
            table.header.extend(rows);
        } else {
            table.rows.extend(rows);
        }
    }
    Some((table, next))
}

/// The rows of one section of a simple table. A line whose first column
/// is empty continues the row above; lines of `-` mark column spans.
fn simple_rows(lines: &[&str], columns: &[(usize, usize)]) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in lines {
        let is_span_marker = line.trim().chars().all(|character| character == '-' || character == ' ');
        if line.trim().is_empty() || is_span_marker {
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let cells: Vec<String> = columns
            .iter()
            .enumerate()
            .map(|(position, &(start, end))| {
                // The last column takes the rest of the line.
                let end = if position + 1 == columns.len() { chars.len() } else { columns[position + 1].0.max(end) };
                slice_text(&chars, start, end)
            })
            .collect();
        match rows.last_mut() {
            Some(row) if cells[0].is_empty() => append_cells(row, &cells),
            _ => rows.push(cells),
        }
    }
    rows
}

/// The trimmed text of `chars[start..end]`, clamped to the line.
fn slice_text(chars: &[char], start: usize, end: usize) -> String {
    let end = end.min(chars.len());
    if start >= end {
        return String::new();
    }
    chars[start..end].iter().collect::<String>().trim().to_string()
}

/// Add each non-empty cell's text to the row's cell, after a space.
fn append_cells(row: &mut [String], cells: &[String]) {
    for (cell, text) in row.iter_mut().zip(cells) {
        if text.is_empty() {
            continue;
        }
        if !cell.is_empty() {
            cell.push(' ');
        }
        cell.push_str(text);
    }
}

/// True for a grid table border: `+`, then `-` or `=` runs and `+`s.
fn is_grid_border(line: &str) -> bool {
    let line = line.trim_end();
    line.len() >= 3
        && line.starts_with('+')
        && line.ends_with('+')
        && line.chars().all(|character| matches!(character, '+' | '-' | '='))
}

/// The grid table starting at `lines[index]`, and the index after it.
pub(super) fn grid_table(lines: &[&str], index: usize) -> Option<(RawTable, usize)> {
    if !is_grid_border(lines[index]) {
        return None;
    }
    let first: Vec<char> = lines[index].trim_end().chars().collect();
    let boundaries: Vec<usize> =
        first.iter().enumerate().filter(|(_, character)| **character == '+').map(|(at, _)| at).collect();
    let mut table = RawTable::default();
    let mut row: Vec<String> = vec![String::new(); boundaries.len() - 1];
    let mut next = index + 1;
    while let Some(line) = lines.get(next).filter(|line| line.starts_with('+') || line.starts_with('|')) {
        next += 1;
        if is_grid_border(line) {
            if row.iter().any(|cell| !cell.is_empty()) {
                table.rows.push(std::mem::replace(&mut row, vec![String::new(); boundaries.len() - 1]));
            }
            if line.contains('=') && table.header.is_empty() {
                table.header = std::mem::take(&mut table.rows);
            }
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let texts: Vec<String> = boundaries.windows(2).map(|pair| slice_text(&chars, pair[0] + 1, pair[1])).collect();
        append_cells(&mut row, &texts);
    }
    // A grid table ends with a border; anything else is not one.
    let closed = next > index + 1 && is_grid_border(lines[next - 1]);
    closed.then_some((table, next))
}

/// The fields of one CSV line: commas separate them, double quotes wrap a
/// field with commas, and `""` inside quotes is one quote.
pub(super) fn csv_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(character) = chars.next() {
        match (character, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => fields.push(std::mem::take(&mut field).trim().to_string()),
            _ => field.push(character),
        }
    }
    fields.push(field.trim().to_string());
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|cell| (*cell).to_string()).collect()
    }

    #[test]
    fn simple_table_with_header_and_continuation_lines() {
        let lines = [
            "=====  ===========",
            "Name   Meaning",
            "=====  ===========",
            "a      first",
            "       and more",
            "b      second",
            "=====  ===========",
            "",
            "after",
        ];
        let (table, next) = simple_table(&lines, 0).unwrap();
        assert_eq!(table.header, [strings(&["Name", "Meaning"])]);
        assert_eq!(table.rows, [strings(&["a", "first and more"]), strings(&["b", "second"])]);
        assert_eq!(next, 7);
    }

    #[test]
    fn simple_table_without_header_and_long_last_column() {
        let lines = ["==  ==", "1   a longer text than the border", "==  =="];
        let (table, next) = simple_table(&lines, 0).unwrap();
        assert_eq!(table.header, Vec::<Vec<String>>::new());
        assert_eq!(table.rows, [strings(&["1", "a longer text than the border"])]);
        assert_eq!(next, 3);
    }

    #[test]
    fn single_column_border_or_unclosed_table_is_not_a_table() {
        assert_eq!(simple_table(&["=====", "x", "====="], 0), None);
        assert_eq!(simple_table(&["==  ==", "a   b"], 0), None);
    }

    #[test]
    fn grid_table_with_header_and_multi_line_cells() {
        let lines = [
            "+------+---------+",
            "| Key  | Value   |",
            "+======+=========+",
            "| a    | one     |",
            "|      | more    |",
            "+------+---------+",
            "| b    | two     |",
            "+------+---------+",
            "not table",
        ];
        let (table, next) = grid_table(&lines, 0).unwrap();
        assert_eq!(table.header, [strings(&["Key", "Value"])]);
        assert_eq!(table.rows, [strings(&["a", "one more"]), strings(&["b", "two"])]);
        assert_eq!(next, 8);
    }

    #[test]
    fn unclosed_grid_table_is_not_a_table() {
        assert_eq!(grid_table(&["+---+", "| a |"], 0), None);
    }

    #[test]
    fn csv_fields_handle_quotes() {
        assert_eq!(csv_fields(r#"a, "b, c", "say ""hi""""#), strings(&["a", "b, c", r#"say "hi""#]));
    }
}
