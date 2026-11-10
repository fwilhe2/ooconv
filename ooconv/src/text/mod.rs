// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Word-processing documents: the neutral [`Document`], and a reader and a writer for each of
//! `.docx` and ODF (`.odt`/`.fodt`).

mod docx_read;
mod docx_write;
pub mod model;
mod odt_read;
mod odt_write;

pub use model::*;

use crate::{Format, Report, Result};

pub fn read(bytes: &[u8], from: Format, report: &mut Report) -> Result<Document> {
    let mut doc = match from {
        Format::Docx => docx_read::read(bytes, report)?,
        Format::Odt | Format::Fodt => odt_read::read(bytes, report)?,
        other => unreachable!("{other:?} is not a text document"),
    };
    doc.number_lists();
    doc.drop_table_filler();
    Ok(doc)
}

pub fn write(doc: &Document, to: Format, report: &mut Report) -> Result<Vec<u8>> {
    match to {
        Format::Docx => docx_write::write(doc, report),
        Format::Odt => odt_write::write(doc, false, report),
        Format::Fodt => odt_write::write(doc, true, report),
        other => unreachable!("{other:?} is not a text document"),
    }
}
