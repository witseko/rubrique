//! Read the sheet names and one sheet's cells from an Excel `.xlsx` workbook.
//!
//! `xl/workbook.xml` lists the sheets with a relationship ID each, the
//! workbook's `.rels` part maps those IDs to sheet parts, and text cells
//! point into the shared string table by index.

use super::package::{DOCUMENT_REL, is_relationship_type, part_path, read_part, relationships, require_root};
use super::table::{MAX_ROWS, Table};
use super::xml::{Event, XmlReader};
use super::zip_archive::ZipArchive;

/// `SpreadsheetML`, transitional and strict.
const SHEET: &[&str] =
    &["http://schemas.openxmlformats.org/spreadsheetml/2006/main", "http://purl.oclc.org/ooxml/spreadsheetml/main"];

/// The sheet names and the cells of sheet number `selected`.
pub(super) fn sheet(bytes: &[u8], selected: usize) -> Result<(Vec<String>, Table), String> {
    let archive = ZipArchive::new(bytes)?;
    let sheets = sheet_list(&read_part(&archive, "xl/workbook.xml")?)?;
    let names = sheets.iter().map(|(name, _)| name.clone()).collect();
    let (_, id) = sheets.get(selected).ok_or("This workbook has no sheet to display.")?;
    let links = relationships(&read_part(&archive, "xl/_rels/workbook.xml.rels")?)?;
    let (_, _, target) = links
        .iter()
        .find(|(link, kind, _)| link == id && is_relationship_type(kind, "worksheet"))
        .ok_or("This sheet has no worksheet part.")?;
    let shared = match links.iter().find(|(_, kind, _)| is_relationship_type(kind, "sharedStrings")) {
        Some((_, _, target)) => shared_strings(&read_part(&archive, &part_path("xl", target)?)?)?,
        None => Vec::new(),
    };
    let table = cells(&read_part(&archive, &part_path("xl", target)?)?, &shared)?;
    Ok((names, table))
}

/// Each sheet's name and relationship ID, in workbook order.
fn sheet_list(xml: &[u8]) -> Result<Vec<(String, String)>, String> {
    let mut reader = XmlReader::new(xml)?;
    require_root(&mut reader, "workbook", SHEET)?;
    let mut sheets = Vec::new();
    loop {
        match reader.next()? {
            Event::Start(element) if element.name.is(SHEET, "sheet") => {
                if let (Some(name), Some(id)) = (element.attribute("name"), element.attribute_in(DOCUMENT_REL, "id")) {
                    sheets.push((name.to_string(), id.to_string()));
                }
            }
            Event::Eof => return Ok(sheets),
            _ => {}
        }
    }
}

/// The shared string table: the text of each `<si>`, without phonetic
/// guides.
fn shared_strings(xml: &[u8]) -> Result<Vec<String>, String> {
    let mut reader = XmlReader::new(xml)?;
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_text = false;
    loop {
        match reader.next()? {
            Event::Start(element) if element.name.is(SHEET, "rPh") => reader.skip_element()?,
            Event::Start(element) if element.name.is(SHEET, "t") => in_text = true,
            Event::End(name) if name.is(SHEET, "t") => in_text = false,
            Event::End(name) if name.is(SHEET, "si") => strings.push(std::mem::take(&mut current)),
            Event::Text(text) if in_text => current.push_str(&text),
            Event::Eof => return Ok(strings),
            _ => {}
        }
    }
}

/// One cell while its `<c>` element is read.
#[derive(Default)]
struct Cell {
    row: usize,
    column: usize,
    kind: String,
    value: String,
}

/// Read the cells of a worksheet part.
fn cells(xml: &[u8], shared: &[String]) -> Result<Table, String> {
    let mut reader = XmlReader::new(xml)?;
    require_root(&mut reader, "worksheet", SHEET)?;
    let mut table = Table::default();
    let mut row = 0;
    let mut next_column = 0;
    let mut cell: Option<Cell> = None;
    let mut in_value = false;
    loop {
        match reader.next()? {
            Event::Start(element) if element.name.is(SHEET, "row") => {
                row = element
                    .attribute("r")
                    .and_then(|number| number.parse::<usize>().ok())
                    .map_or(row + 1, |number| number.max(1));
                next_column = 0;
                if row > MAX_ROWS as usize {
                    table.truncated = true;
                    return Ok(table);
                }
            }
            Event::Start(element) if element.name.is(SHEET, "c") => {
                let (cell_row, column) =
                    element.attribute("r").and_then(cell_position).unwrap_or((row.saturating_sub(1), next_column));
                next_column = column + 1;
                cell = Some(Cell {
                    row: cell_row,
                    column,
                    kind: element.attribute("t").unwrap_or("n").to_string(),
                    value: String::new(),
                });
            }
            Event::Start(element) if element.name.is(SHEET, "v") || element.name.is(SHEET, "t") => in_value = true,
            Event::Start(element) if element.name.is(SHEET, "rPh") => reader.skip_element()?,
            Event::End(name) if name.is(SHEET, "v") || name.is(SHEET, "t") => in_value = false,
            Event::Text(text) if in_value => {
                if let Some(cell) = &mut cell {
                    cell.value.push_str(&text);
                }
            }
            Event::End(name) if name.is(SHEET, "c") => {
                if let Some(cell) = cell.take() {
                    let text = cell_text(&cell, shared);
                    if !text.is_empty() {
                        table.set(cell.row, cell.column, &text);
                    }
                }
            }
            Event::Eof => return Ok(table),
            _ => {}
        }
    }
}

/// The text shown for a cell by its type (`t` attribute).
fn cell_text(cell: &Cell, shared: &[String]) -> String {
    match cell.kind.as_str() {
        "s" => cell.value.trim().parse::<usize>().ok().and_then(|index| shared.get(index)).cloned().unwrap_or_default(),
        "b" => if cell.value.trim() == "1" { "true" } else { "false" }.to_string(),
        "str" | "inlineStr" | "e" => cell.value.clone(),
        _ => super::table::number_text(&cell.value),
    }
}

/// The zero-based row and column of a reference such as `B3`.
fn cell_position(reference: &str) -> Option<(usize, usize)> {
    let letters = reference.find(|character: char| character.is_ascii_digit())?;
    let (column_letters, row_digits) = reference.split_at(letters);
    if column_letters.is_empty() || column_letters.len() > 3 {
        return None;
    }
    let mut column = 0;
    for letter in column_letters.bytes() {
        if !letter.is_ascii_uppercase() {
            return None;
        }
        column = column * 26 + usize::from(letter - b'A') + 1;
    }
    let row: usize = row_digits.parse().ok()?;
    Some((row.checked_sub(1)?, column - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_cell_references() {
        assert_eq!(cell_position("A1"), Some((0, 0)));
        assert_eq!(cell_position("AB12"), Some((11, 27)));
        assert_eq!(cell_position("1A"), None);
        assert_eq!(cell_position("A0"), None);
    }

    #[test]
    fn reads_shared_inline_boolean_and_number_cells() {
        let xml = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
            <row r="1"><c r="A1" t="s"><v>1</v></c><c r="C1" t="inlineStr"><is><t>in</t></is></c></row>
            <row r="3"><c r="B3" t="b"><v>1</v></c><c><v>2.50</v></c></row>
            </sheetData></worksheet>"#;
        let table = cells(xml, &["zero".into(), "one".into()]).unwrap();
        assert_eq!(table.rows, [vec!["one", "", "in"], vec![], vec!["", "true", "2.5"]]);
    }

    #[test]
    fn skips_phonetic_guides_in_shared_strings() {
        let xml = br#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
            <si><t>plain</t></si><si><r><t>ri</t></r><r><t>ch</t></r><rPh><t>skip</t></rPh></si></sst>"#;
        assert_eq!(shared_strings(xml).unwrap(), ["plain", "rich"]);
    }
}
