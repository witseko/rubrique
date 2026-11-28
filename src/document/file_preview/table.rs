//! Build and draw a bounded preview of a spreadsheet or delimited text file.

use std::iter::Peekable;
use std::path::Path;
use std::str::Chars;

use eframe::egui::{self, RichText, Ui};

use crate::document::text_file::Document;

/// Maximum number of rows kept in a table preview.
pub(super) const MAX_ROWS: u32 = 1_000;
/// Maximum number of columns kept in a table preview.
pub(super) const MAX_COLUMNS: u32 = 48;
/// Maximum number of characters copied from each cell.
pub(super) const MAX_CELL_CHARS: usize = 512;
/// Width of a cell in logical pixels.
const CELL_WIDTH: f32 = 136.0;
/// Width of the row number in logical pixels.
const ROW_NUMBER_WIDTH: f32 = 54.0;

/// A comma- or tab-separated text file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delimited {
    Csv,
    Tsv,
}

impl Delimited {
    /// Identify CSV and TSV extensions without regard to case.
    pub fn of_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        if extension.eq_ignore_ascii_case("csv") {
            Some(Self::Csv)
        } else if extension.eq_ignore_ascii_case("tsv") {
            Some(Self::Tsv)
        } else {
            None
        }
    }

    /// The separator between fields.
    fn delimiter(self) -> char {
        match self {
            Self::Csv => ',',
            Self::Tsv => '\t',
        }
    }
}

/// Parsed table data cached until a text edit changes the document's revision.
#[derive(Debug)]
pub struct DelimitedPreview {
    revision: u64,
    kind: Delimited,
    table: Table,
}

impl DelimitedPreview {
    /// Draw the tabular view of the current source, including unsaved edits.
    pub fn show(cache: &mut Option<Self>, ui: &mut Ui, kind: Delimited, document: &Document) {
        let revision = document.revision();
        if cache.as_ref().is_none_or(|preview| preview.revision != revision || preview.kind != kind) {
            *cache = Some(Self { revision, kind, table: delimited(kind, &document.text) });
        }
        if let Some(preview) = cache {
            ui.push_id(document.id(), |ui| preview.table.show(ui));
        }
    }
}

/// Cell values and whether any data fell outside the preview limits.
#[derive(Debug, Default)]
pub(super) struct Table {
    pub(super) rows: Vec<Vec<String>>,
    pub(super) truncated: bool,
}

impl Table {
    /// Set one cell by its zero-based row and column.
    pub(super) fn set(&mut self, row: usize, column: usize, text: &str) {
        if row >= MAX_ROWS as usize || column >= MAX_COLUMNS as usize {
            self.truncated = true;
            return;
        }
        if self.rows.len() <= row {
            self.rows.resize_with(row + 1, Vec::new);
        }
        let cells = &mut self.rows[row];
        if cells.len() <= column {
            cells.resize_with(column + 1, String::new);
        }
        cells[column] = if let Some((end, _)) = text.char_indices().nth(MAX_CELL_CHARS) {
            self.truncated = true;
            format!("{}…", &text[..end])
        } else {
            text.to_string()
        };
    }

    /// Draw a scrollable grid, laying out only the rows currently on screen.
    pub(super) fn show(&self, ui: &mut Ui) {
        let columns = self.rows.iter().map(Vec::len).max().unwrap_or(0);
        if self.truncated {
            ui.label(format!(
                "Preview limited to {MAX_ROWS} rows, {MAX_COLUMNS} columns and {MAX_CELL_CHARS} characters per cell."
            ));
        }
        if columns == 0 {
            ui.weak("No cells to display.");
            return;
        }
        let row_height = ui.spacing().interact_size.y;
        egui::ScrollArea::both().auto_shrink([false, false]).show_rows(ui, row_height, self.rows.len(), |ui, range| {
            for row in range {
                self.show_row(ui, row, columns, row_height);
            }
        });
    }

    /// Draw the row number and fixed-width cells, with the full value on hover.
    fn show_row(&self, ui: &mut Ui, row: usize, columns: usize, height: f32) {
        let background = ui.painter().add(egui::Shape::Noop);
        let response = ui
            .horizontal(|ui| {
                ui.add_sized([ROW_NUMBER_WIDTH, height], egui::Label::new(RichText::new((row + 1).to_string()).weak()));
                let cells = &self.rows[row];
                for column in 0..columns {
                    let text = cells.get(column).map_or("", String::as_str);
                    ui.add_sized([CELL_WIDTH, height], egui::Label::new(text).truncate()).on_hover_text(text);
                }
            })
            .response;
        if row.is_multiple_of(2) {
            ui.painter().set(background, egui::Shape::rect_filled(response.rect, 0.0, ui.visuals().faint_bg_color));
        }
    }
}

/// Show a stored number as the shortest decimal that reads back the same
/// (`42`, not `42.0`); text that is not a number stays as written.
pub(super) fn number_text(raw: &str) -> String {
    raw.trim().parse::<f64>().map_or_else(|_| raw.to_string(), |number| number.to_string())
}

/// Parse quoted fields and ragged records using the selected separator.
fn delimited(kind: Delimited, text: &str) -> Table {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut characters = text.chars().peekable();
    let mut table = Table::default();
    let mut row = 0;
    while let Some(record) = next_record(&mut characters, kind.delimiter()) {
        if row == MAX_ROWS as usize {
            table.truncated = true;
            break;
        }
        for (column, text) in record.iter().take(MAX_COLUMNS as usize + 1).enumerate() {
            table.set(row, column, text);
        }
        row += 1;
    }
    table
}

/// Read one record of delimited text (RFC 4180, but lenient like
/// spreadsheets: a quote inside an unquoted field is kept, and an unclosed
/// quote runs to the end of the text). Empty lines are skipped; `None` at
/// the end of the text.
fn next_record(characters: &mut Peekable<Chars<'_>>, delimiter: char) -> Option<Vec<String>> {
    while characters.next_if(|character| matches!(character, '\r' | '\n')).is_some() {}
    characters.peek()?;
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut field_start = true;
    while let Some(character) = characters.next() {
        if quoted {
            if character != '"' {
                field.push(character);
            } else if characters.next_if_eq(&'"').is_some() {
                field.push('"');
            } else {
                quoted = false;
            }
            continue;
        }
        if character == delimiter {
            record.push(std::mem::take(&mut field));
            field_start = true;
            continue;
        }
        match character {
            '"' if field_start => quoted = true,
            '\r' => {
                characters.next_if_eq(&'\n');
                break;
            }
            '\n' => break,
            _ => field.push(character),
        }
        field_start = false;
    }
    record.push(field);
    Some(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_selected_sheet_in_excel_and_open_document_workbooks() {
        use super::super::{Content, Kind, workbook};

        for (kind, bytes) in [
            (Kind::Xls, include_bytes!("fixtures/two-sheets.xls").as_slice()),
            (Kind::Xlsx, include_bytes!("fixtures/two-sheets.xlsx").as_slice()),
            (Kind::Ods, include_bytes!("fixtures/two-sheets.ods").as_slice()),
        ] {
            for (index, expected) in ["Name", "42"].into_iter().enumerate() {
                let Content::Sheet { names, table } = workbook(kind, bytes, index).unwrap() else {
                    panic!("expected a sheet preview");
                };
                assert_eq!(names, ["Overview", "Totals"]);
                assert_eq!(table.rows, [vec![expected]]);
            }
        }
    }

    #[test]
    fn csv_keeps_first_row_and_reads_quoted_fields() {
        let table = delimited(Delimited::Csv, "\u{feff}Name,Description\r\nA,\"one,two\"\r\nB,\"three\nfour\"\r\nC");
        assert_eq!(
            table.rows,
            [vec!["Name", "Description"], vec!["A", "one,two"], vec!["B", "three\nfour"], vec!["C"]],
        );
        assert!(!table.truncated);
    }

    #[test]
    fn tsv_preserves_commas_and_reports_a_truncated_cell() {
        let long = "é".repeat(MAX_CELL_CHARS + 1);
        let table = delimited(Delimited::Tsv, &format!("one,two\t{long}"));
        assert_eq!(table.rows[0][0], "one,two");
        assert_eq!(table.rows[0][1], format!("{}…", "é".repeat(MAX_CELL_CHARS)));
        assert!(table.truncated);
    }

    #[test]
    fn csv_handles_escaped_quotes_empty_fields_and_blank_lines() {
        let table = delimited(Delimited::Csv, "a,\"say \"\"hi\"\"\",\n\n\r\nx\"y,,\"open");
        assert_eq!(table.rows, [vec!["a", "say \"hi\"", ""], vec!["x\"y", "", "open"]]);
    }
}
