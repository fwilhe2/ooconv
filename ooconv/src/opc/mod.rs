// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The OOXML family's container layer: **OPC** reading ([`package`], lifted from grind) and
//! writing ([`write`](mod@write)), and **markup compatibility** ([`mce`]).

pub mod mce;
pub mod package;
pub mod write;

use crate::names::{Ns, RelType, Seen};
use crate::xml::read::Reader;
use crate::{Error, Format, Result};

/// Which OOXML document a zip is: the main part's *root element* decides, since a workbook and a
/// Word document share the `officeDocument` relationship type (grind `doc/docx-format.md` §1.1).
pub fn sniff(bytes: &[u8]) -> Result<Format> {
    let mut pkg = package::Package::open(bytes)?;
    let mut seen = Seen::default();
    let main = pkg
        .rel_of("", RelType::OfficeDocument, &mut seen)
        .ok_or(Error::UnknownFormat)?;
    let part = pkg.part(&main.target).ok_or(Error::UnknownFormat)?;
    let mut reader = Reader::new(&part);
    match reader.root()? {
        Some((name, _)) if name.in_ns(Ns::Spreadsheet, "workbook") => Ok(Format::Xlsx),
        Some((name, _)) if name.in_ns(Ns::Word, "document") => Ok(Format::Docx),
        _ => Err(Error::UnknownFormat),
    }
}
