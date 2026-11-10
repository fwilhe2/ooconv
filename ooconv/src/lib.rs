// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Converting between Office Open XML and OpenDocument, in both directions: `.xlsx` ↔ `.ods`
//! (`.fods`) and `.docx` ↔ `.odt` (`.fodt`).
//!
//! Every conversion goes through a neutral model — [`sheet::Workbook`] or [`text::Document`] — so
//! four readers and four writers cover every pair, and each writer is held to its format's
//! validator on its own (`doc/plan.md`, R2). **Strict on the way out, tolerant on the way in**:
//! a reader skips what it does not recognise and says so in the [`Report`]; a writer emits only
//! what it can prove valid.
//!
//! ```no_run
//! let xlsx = std::fs::read("book.xlsx")?;
//! let out = ooconv::convert(&xlsx, ooconv::Format::Ods)?;
//! std::fs::write("book.ods", &out.bytes)?;
//! for loss in &out.report.losses { eprintln!("{loss}"); }
//! # Ok::<(), ooconv::Error>(())
//! ```

use std::fmt;

pub mod names;
pub mod odf;
pub mod opc;
pub mod report;
pub mod sheet;
pub mod text;
pub mod xml;

mod limits;

pub use names::Flavour;
pub use report::{Loss, Report};

pub type Result<T> = std::result::Result<T, Error>;

/// A file that cannot be converted at all. Anything short of this is a [`Loss`] in the
/// [`Report`], since refusing a whole document over one construct helps nobody — unless the
/// caller asked for that with [`Options::strict`].
#[derive(Debug)]
pub enum Error {
    /// The container would not open: not a zip, or one whose central directory is unusable.
    Package(String),
    /// Password-protected: the document is fine, we have no key.
    Encrypted,
    /// The XML would not parse at all. Unrecognised content never reaches here.
    Xml(String),
    /// The bytes are no format this converter reads.
    UnknownFormat,
    /// A spreadsheet asked to become a word-processing document, or the other way round.
    KindMismatch {
        from: Format,
        to: Format,
    },
    /// [`Options::strict`] was set and the conversion would have lost something.
    WouldLose(Report),
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Package(e) => write!(f, "package: {e}"),
            Error::Encrypted => write!(f, "password-protected document"),
            Error::Xml(e) => write!(f, "xml: {e}"),
            Error::UnknownFormat => write!(f, "not a document this converter reads"),
            Error::KindMismatch { from, to } => write!(
                f,
                "{} is a {}, {} is a {}",
                from.extension(),
                from.kind(),
                to.extension(),
                to.kind()
            ),
            Error::WouldLose(report) => {
                write!(f, "the conversion would lose {}", report.losses.len())?;
                write!(f, " kind(s) of content")
            }
            Error::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// What a document is about, independent of its family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Spreadsheet,
    Text,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::Spreadsheet => "spreadsheet",
            Kind::Text => "text document",
        })
    }
}

/// Every format this converter reads and writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Xlsx,
    /// The ODF package form.
    Ods,
    /// The ODF flat form: one XML file.
    Fods,
    Docx,
    Odt,
    Fodt,
}

impl Format {
    pub const ALL: [Format; 6] = [
        Format::Xlsx,
        Format::Ods,
        Format::Fods,
        Format::Docx,
        Format::Odt,
        Format::Fodt,
    ];

    pub fn kind(self) -> Kind {
        match self {
            Format::Xlsx | Format::Ods | Format::Fods => Kind::Spreadsheet,
            Format::Docx | Format::Odt | Format::Fodt => Kind::Text,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Xlsx => "xlsx",
            Format::Ods => "ods",
            Format::Fods => "fods",
            Format::Docx => "docx",
            Format::Odt => "odt",
            Format::Fodt => "fodt",
        }
    }

    /// The format an extension names, case-insensitively. Used for the *output* only: what an
    /// input is gets sniffed from its bytes ([`sniff`]).
    pub fn from_extension(ext: &str) -> Option<Format> {
        Format::ALL
            .into_iter()
            .find(|f| f.extension().eq_ignore_ascii_case(ext))
    }

    /// The other family's format of the same kind: what `convert` is usually asked for.
    pub fn counterpart(self) -> Format {
        match self {
            Format::Xlsx => Format::Ods,
            Format::Ods | Format::Fods => Format::Xlsx,
            Format::Docx => Format::Odt,
            Format::Odt | Format::Fodt => Format::Docx,
        }
    }
}

/// What a file is, from its bytes. Never from its name: a renamed file is still what it is.
pub fn sniff(bytes: &[u8]) -> Result<Format> {
    if opc::package::is_encrypted(bytes) {
        return Err(Error::Encrypted);
    }
    if opc::package::is_zip(bytes) {
        if let Some(format) = odf::package::sniff_package(bytes)? {
            return Ok(format);
        }
        return opc::sniff(bytes);
    }
    odf::package::sniff_flat(bytes).ok_or(Error::UnknownFormat)
}

/// How a conversion behaves.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// Refuse with [`Error::WouldLose`] rather than write a document missing anything the
    /// source had that the reader recognised as content.
    pub strict: bool,
}

/// A converted document and what the conversion could not carry.
#[derive(Debug)]
pub struct Converted {
    pub bytes: Vec<u8>,
    pub from: Format,
    pub report: Report,
}

/// Convert a document to `to`, whatever it is now.
pub fn convert(bytes: &[u8], to: Format) -> Result<Converted> {
    convert_with(bytes, to, Options::default())
}

pub fn convert_with(bytes: &[u8], to: Format, options: Options) -> Result<Converted> {
    let from = sniff(bytes)?;
    if from.kind() != to.kind() {
        return Err(Error::KindMismatch { from, to });
    }
    let mut report = Report::default();
    let bytes = match to.kind() {
        Kind::Spreadsheet => {
            let book = sheet::read(bytes, from, &mut report)?;
            check(&report, options)?;
            sheet::write(&book, to, &mut report)?
        }
        Kind::Text => {
            let doc = text::read(bytes, from, &mut report)?;
            check(&report, options)?;
            text::write(&doc, to, &mut report)?
        }
    };
    check(&report, options)?;
    Ok(Converted {
        bytes,
        from,
        report,
    })
}

fn check(report: &Report, options: Options) -> Result<()> {
    match options.strict && !report.losses.is_empty() {
        true => Err(Error::WouldLose(report.clone())),
        false => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_round_trips_through_its_extension() {
        for format in Format::ALL {
            assert_eq!(Format::from_extension(format.extension()), Some(format));
            assert_eq!(
                Format::from_extension(&format.extension().to_uppercase()),
                Some(format)
            );
            assert_eq!(format.counterpart().kind(), format.kind());
        }
        assert_eq!(Format::from_extension("xls"), None);
    }

    #[test]
    fn junk_is_no_format() {
        assert!(matches!(sniff(b"hello"), Err(Error::UnknownFormat)));
        assert!(matches!(sniff(b""), Err(Error::UnknownFormat)));
    }
}
