// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Spreadsheets: the neutral [`Workbook`], and a reader and a writer for each of `.xlsx` and ODF
//! (`.ods`/`.fods`).

pub mod dates;
pub mod formula;
pub mod model;
pub mod numfmt;
mod ods_read;
mod ods_write;
mod xlsx_read;
mod xlsx_write;

pub use model::*;

use crate::{Format, Report, Result};

/// Read a spreadsheet in `from`, which the caller has sniffed.
pub fn read(bytes: &[u8], from: Format, report: &mut Report) -> Result<Workbook> {
    let mut book = match from {
        Format::Xlsx => xlsx_read::read(bytes, report)?,
        Format::Ods | Format::Fods => ods_read::read(bytes, report)?,
        other => unreachable!("{other:?} is not a spreadsheet"),
    };
    // Both formats need a sheet; a file with none reads as one empty sheet, so what a writer
    // must add is already in the model.
    if book.sheets.is_empty() {
        book.sheets.push(Sheet::new("Sheet1"));
    }
    Ok(book)
}

/// Write `book` as `to`.
pub fn write(book: &Workbook, to: Format, report: &mut Report) -> Result<Vec<u8>> {
    match to {
        Format::Xlsx => xlsx_write::write(book, report),
        Format::Ods => ods_write::write(book, false, report),
        Format::Fods => ods_write::write(book, true, report),
        other => unreachable!("{other:?} is not a spreadsheet"),
    }
}
