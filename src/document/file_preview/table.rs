//! Build and draw a bounded preview of a spreadsheet or delimited text file.

use std::io::{Read, Seek};
use std::path::Path;

use calamine::{Data, Range, Xlsx};
use eframe::egui::{self, RichText, Ui};

use crate::document::text_file::Document;

/// Maximum number of rows kept in a table preview.
const MAX_ROWS: u32 = 1_000;
/// Maximum number of columns kept in a table preview.
const MAX_COLUMNS: u32 = 48;
/// Maximum number of characters copied from each cell.
const MAX_CELL_CHARS: usize = 512;
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
    fn delimiter(self) -> u8 {
        match self {
            Self::Csv => b',',
            Self::Tsv => b'\t',
        }
    }
}

/// Parsed table data cached until a text edit changes the document's revision.
#[derive(Debug)]
pub struct DelimitedPreview {
    revision: u64,
    kind: Delimited,
    table: Result<Table, csv::Error>,
}

impl DelimitedPreview {
    /// Draw the tabular view of the current source, including unsaved edits.
    pub fn show(cache: &mut Option<Self>, ui: &mut Ui, kind: Delimited, document: &Document) {
        let revision = document.revision();
        if cache.as_ref().is_none_or(|preview| preview.revision != revision || preview.kind != kind) {
            *cache = Some(Self { revision, kind, table: delimited(kind, &document.text) });
        }
        if let Some(preview) = cache {
            match &preview.table {
                Ok(table) => {
                    ui.push_id(document.id(), |ui| table.show(ui));
                }
                Err(error) => {
                    ui.label(format!("Cannot read this table: {error}"));
                }
            }
        }
    }
}

/// Cell values and whether any data fell outside the preview limits.
#[derive(Debug, Default)]
pub(super) struct Table {
    rows: Vec<Vec<String>>,
    truncated: bool,
}

impl Table {
    /// Set one cell by its zero-based row and column.
    fn set(&mut self, row: usize, column: usize, text: &str) {
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

/// Parse quoted fields and ragged records using the selected separator.
fn delimited(kind: Delimited, text: &str) -> Result<Table, csv::Error> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(kind.delimiter())
        .flexible(true)
        .has_headers(false)
        .from_reader(text.as_bytes());
    let mut table = Table::default();
    for (row, record) in reader.records().take(MAX_ROWS as usize + 1).enumerate() {
        let record = record?;
        if row == MAX_ROWS as usize {
            table.truncated = true;
            break;
        }
        for (column, text) in record.iter().take(MAX_COLUMNS as usize + 1).enumerate() {
            table.set(row, column, text);
        }
    }
    Ok(table)
}

/// Read XLSX cells directly so a sparse sheet never becomes a dense allocation.
pub(super) fn xlsx(workbook: &mut Xlsx<impl Read + Seek>, name: &str) -> Result<Table, String> {
    let mut reader = workbook.worksheet_cells_reader(name).map_err(|error| error.to_string())?;
    let mut table = Table::default();
    while let Some(cell) = reader.next_cell().map_err(|error| error.to_string())? {
        let (row, column) = cell.get_position();
        if row as usize >= MAX_ROWS as usize {
            table.truncated = true;
            break;
        }
        table.set(row as usize, column as usize, &Data::from(cell.get_value().clone()).to_string());
    }
    Ok(table)
}

/// Copy a bounded part of a legacy Excel or `OpenDocument` sheet.
pub(super) fn workbook_range(range: &Range<Data>) -> Table {
    let mut table = Table::default();
    let Some((last_row, last_column)) = range.end() else {
        return table;
    };
    table.truncated = last_row as usize >= MAX_ROWS as usize || last_column as usize >= MAX_COLUMNS as usize;
    for row in 0..MAX_ROWS.min(last_row.saturating_add(1)) {
        for column in 0..MAX_COLUMNS.min(last_column.saturating_add(1)) {
            if let Some(value) = range.get_value((row, column)) {
                table.set(row as usize, column as usize, &value.to_string());
            }
        }
    }
    table
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
        let table =
            delimited(Delimited::Csv, "\u{feff}Name,Description\r\nA,\"one,two\"\r\nB,\"three\nfour\"\r\nC").unwrap();
        assert_eq!(
            table.rows,
            [vec!["Name", "Description"], vec!["A", "one,two"], vec!["B", "three\nfour"], vec!["C"]],
        );
        assert!(!table.truncated);
    }

    #[test]
    fn tsv_preserves_commas_and_reports_a_truncated_cell() {
        let long = "é".repeat(MAX_CELL_CHARS + 1);
        let table = delimited(Delimited::Tsv, &format!("one,two\t{long}")).unwrap();
        assert_eq!(table.rows[0][0], "one,two");
        assert_eq!(table.rows[0][1], format!("{}…", "é".repeat(MAX_CELL_CHARS)));
        assert!(table.truncated);
    }
}
