//! Read the sheet names and one sheet's cells from an `OpenDocument`
//! spreadsheet (`.ods`).
//!
//! All sheets are `table:table` elements in `content.xml`. A row or cell can
//! stand for many identical ones (`number-rows-repeated`,
//! `number-columns-repeated`); a sheet often ends with an empty row repeated
//! a million times, so repeats only move the position.

use super::odt::{OFFICE, TEXT};
use super::package::{read_part, require_root};
use super::table::{MAX_COLUMNS, MAX_ROWS, Table};
use super::xml::{Element, Event, XmlReader};
use super::zip_archive::ZipArchive;

/// The `OpenDocument` table namespace.
const TABLE: &[&str] = &["urn:oasis:names:tc:opendocument:xmlns:table:1.0"];

/// The sheet names and the cells of sheet number `selected`.
pub(super) fn sheet(bytes: &[u8], selected: usize) -> Result<(Vec<String>, Table), String> {
    let archive = ZipArchive::new(bytes)?;
    let xml = read_part(&archive, "content.xml")?;
    let mut reader = XmlReader::new(&xml)?;
    require_root(&mut reader, "document-content", OFFICE)?;
    let mut names = Vec::new();
    let mut table = None;
    loop {
        match reader.next()? {
            Event::Start(element) if element.name.is(TABLE, "table") => {
                names.push(element.attribute_in(TABLE, "name").unwrap_or_default().to_string());
                if names.len() == selected + 1 {
                    table = Some(read_table(&mut reader)?);
                } else {
                    reader.skip_element()?;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let table = table.ok_or("This workbook has no sheet to display.")?;
    Ok((names, table))
}

/// The value of a cell while its element is read.
struct Cell {
    /// Text from the value attributes, when the cell type has one.
    value: Option<String>,
    paragraphs: Vec<String>,
    columns: usize,
}

/// Read the rows of the `table:table` whose start tag was just read.
fn read_table(reader: &mut XmlReader<'_>) -> Result<Table, String> {
    let mut table = Table::default();
    let mut row: usize = 0;
    let mut column: usize = 0;
    let mut row_repeats = 1;
    let mut cell: Option<Cell> = None;
    let mut in_paragraph = false;
    loop {
        match reader.next()? {
            Event::Start(element) if element.name.is(TABLE, "table-row") => {
                row_repeats = repeat(&element, "number-rows-repeated");
                column = 0;
            }
            Event::End(name) if name.is(TABLE, "table-row") => {
                row = row.saturating_add(row_repeats);
                if row >= MAX_ROWS as usize {
                    return Ok(table);
                }
            }
            Event::Start(element)
                if element.name.is(TABLE, "table-cell") || element.name.is(TABLE, "covered-table-cell") =>
            {
                cell = Some(Cell {
                    value: typed_value(&element),
                    paragraphs: Vec::new(),
                    columns: repeat(&element, "number-columns-repeated"),
                });
            }
            Event::Start(element) if element.name.is(OFFICE, "annotation") => reader.skip_element()?,
            Event::Start(element) if element.name.is(TEXT, "p") => {
                in_paragraph = true;
                if let Some(cell) = &mut cell {
                    cell.paragraphs.push(String::new());
                }
            }
            Event::Start(element) if in_paragraph && element.name.is(TEXT, "s") => {
                let count = element.attribute_in(TEXT, "c").and_then(|count| count.parse().ok()).unwrap_or(1_usize);
                push_text(&mut cell, &" ".repeat(count.min(super::table::MAX_CELL_CHARS)));
            }
            Event::Start(element) if in_paragraph && element.name.is(TEXT, "tab") => push_text(&mut cell, "\t"),
            Event::Start(element) if in_paragraph && element.name.is(TEXT, "line-break") => push_text(&mut cell, "\n"),
            Event::End(name) if name.is(TEXT, "p") => in_paragraph = false,
            Event::Text(text) if in_paragraph => push_text(&mut cell, &text),
            Event::End(name) if name.is(TABLE, "table-cell") || name.is(TABLE, "covered-table-cell") => {
                if let Some(cell) = cell.take() {
                    let text = cell.value.unwrap_or_else(|| cell.paragraphs.join("\n"));
                    if !text.is_empty() {
                        fill(&mut table, row, row_repeats, column, cell.columns, &text);
                    }
                    column = column.saturating_add(cell.columns);
                }
            }
            Event::End(name) if name.is(TABLE, "table") => return Ok(table),
            _ => {}
        }
    }
}

/// Add text to the last paragraph of the cell being read.
fn push_text(cell: &mut Option<Cell>, text: &str) {
    if let Some(paragraph) = cell.as_mut().and_then(|cell| cell.paragraphs.last_mut()) {
        paragraph.push_str(text);
    }
}

/// Put `text` in every cell a repeated row and cell cover, within the
/// preview limits.
fn fill(table: &mut Table, row: usize, rows: usize, column: usize, columns: usize, text: &str) {
    let last_row = row.saturating_add(rows).min(MAX_ROWS as usize + 1);
    let last_column = column.saturating_add(columns).min(MAX_COLUMNS as usize + 1);
    for row in row..last_row {
        for column in column..last_column {
            table.set(row, column, text);
        }
    }
}

/// A repeat count attribute, at least one.
fn repeat(element: &Element, attribute: &str) -> usize {
    element.attribute_in(TABLE, attribute).and_then(|count| count.parse().ok()).unwrap_or(1_usize).max(1)
}

/// The text of a cell from its value attributes, by `office:value-type`;
/// `None` for text cells, whose paragraphs hold the text.
fn typed_value(element: &Element) -> Option<String> {
    let attribute = |name| element.attribute_in(OFFICE, name).map(str::to_string);
    match element.attribute_in(OFFICE, "value-type")? {
        "float" | "percentage" | "currency" => attribute("value").map(|value| super::table::number_text(&value)),
        "boolean" => attribute("boolean-value"),
        "date" => attribute("date-value"),
        "time" => attribute("time-value"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_repeats_within_limits_and_reads_typed_values() {
        let xml = br#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
            xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
            xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet>
            <table:table table:name="One"><table:table-row>
                <table:table-cell table:number-columns-repeated="2"><text:p>a<text:s text:c="2"/>b</text:p></table:table-cell>
                <table:table-cell office:value-type="float" office:value="3.0"><text:p>3,00</text:p></table:table-cell>
                <table:table-cell table:number-columns-repeated="16000"/>
            </table:table-row>
            <table:table-row table:number-rows-repeated="1048570"><table:table-cell/></table:table-row>
            </table:table><table:table table:name="Two"/></office:spreadsheet></office:body></office:document-content>"#;
        let archive = super::super::zip_archive::tests::stored_zip(&[("content.xml", xml)]);
        let (names, table) = sheet(&archive, 0).unwrap();
        assert_eq!(names, ["One", "Two"]);
        assert_eq!(table.rows, [vec!["a  b", "a  b", "3"]]);
        assert!(!table.truncated);
        assert!(sheet(&archive, 2).is_err());
    }
}
