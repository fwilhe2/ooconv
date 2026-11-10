// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! OPC reading — the container (ECMA-376 Part 2), lifted from grind (`ooxml/src/package.rs`). A zip whose `_rels/.rels` points at the main part and
//! whose `[Content_Types].xml` types the parts; parts are found by relationship, never by path. A
//! converter eats strangers' files, so the caps here are tested with hostile fixtures.

use std::collections::BTreeMap;
use std::io::Cursor;

use crate::limits::{MAX_PART_BYTES, MAX_PARTS, MAX_TOTAL_BYTES, read_entry};
use crate::names::{RelType, Seen};
use crate::{Error, Result};

/// The zip local file header. Every OPC package starts with it.
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// The CFB/OLE compound-file signature. A password-protected `.xlsx` or `.docx` is not a zip:
/// agile encryption wraps the package in one of these, and recognising it is what lets a locked
/// file be [`Error::Encrypted`] rather than "not a document".
const CFB_MAGIC: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// One `<Relationship>`.
#[derive(Clone, Debug)]
pub struct Rel {
    pub id: String,
    pub kind: RelType,
    /// Already resolved against the source part's directory, so it names a part.
    pub target: String,
    /// `TargetMode="External"` — a link to another workbook or a URL. **Never fetched.**
    pub external: bool,
}

pub struct Package<'a> {
    archive: zip::ZipArchive<Cursor<&'a [u8]>>,
    /// Lower-cased part name to the entry that holds it: part names compare case-insensitively
    /// (Part 2 §9.1.1.3) and producers disagree about case.
    index: BTreeMap<String, usize>,
}

/// Are these bytes a zip at all?
pub fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(ZIP_MAGIC)
}

/// Are these bytes an encrypted OOXML package rather than a plain one?
pub fn is_encrypted(bytes: &[u8]) -> bool {
    bytes.starts_with(CFB_MAGIC)
}

/// Normalise an OPC part name into the form the zip index is keyed by. A leading `/`, backslashes,
/// `.` and `..` segments and percent encoding all name the plain form; a target climbing out of
/// the package is a zip-slip and refused.
pub fn normalise(name: &str) -> Option<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in name.replace('\\', "/").split('/') {
        let segment = percent_decode(raw);
        match segment.as_str() {
            "" | "." => {}
            ".." => {
                // Popping past the root would name something outside the package.
                out.pop()?;
            }
            _ => out.push(segment),
        }
    }
    (!out.is_empty()).then(|| out.join("/").to_lowercase())
}

/// Percent-decoding, byte-wise. Invalid escapes are left alone rather than rejected: this is
/// the read path, where tolerance is the rule.
fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_owned();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let decoded = (bytes[i] == b'%' && i + 2 < bytes.len())
            .then(|| {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
                u8::from_str_radix(hex, 16).ok()
            })
            .flatten();
        match decoded {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolve a relationship target against the part that declared it.
///
/// `xl/workbook.xml` + `worksheets/sheet1.xml` → `xl/worksheets/sheet1.xml`; an absolute
/// target ignores the base.
fn resolve_target(base: &str, target: &str) -> Option<String> {
    if target.starts_with('/') || target.starts_with('\\') {
        return normalise(target);
    }
    let dir = base.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
    normalise(&format!("{dir}/{target}"))
}

impl<'a> Package<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Self> {
        if is_encrypted(bytes) {
            return Err(Error::Encrypted);
        }
        if !is_zip(bytes) {
            return Err(Error::Package("not a zip archive".into()));
        }
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
            .map_err(|e| Error::Package(format!("not a readable package: {e}")))?;

        if archive.len() > MAX_PARTS {
            return Err(Error::Package(format!(
                "{} parts, more than the {MAX_PARTS} any document has",
                archive.len()
            )));
        }
        let mut total: u64 = 0;
        let mut index = BTreeMap::new();
        for i in 0..archive.len() {
            let entry = archive
                .by_index_raw(i)
                .map_err(|e| Error::Package(format!("unreadable entry {i}: {e}")))?;
            let size = entry.size();
            if size > MAX_PART_BYTES {
                return Err(Error::Package(format!(
                    "part {} claims {size} bytes",
                    String::from_utf8_lossy(entry.name_raw())
                )));
            }
            total = total.saturating_add(size);
            if total > MAX_TOTAL_BYTES {
                return Err(Error::Package(
                    "package decompresses to more than the cap".into(),
                ));
            }
            // A part whose name will not normalise cannot be referred to either, so dropping
            // it from the index is the same as not having it.
            if let Some(name) = normalise(&String::from_utf8_lossy(entry.name_raw())) {
                index.insert(name, i);
            }
        }
        Ok(Self { archive, index })
    }

    /// One part's bytes, by name. `None` when the package has no such part — which is not an
    /// error: half the parts a workbook may declare are optional.
    pub fn part(&mut self, name: &str) -> Option<Vec<u8>> {
        let index = *self.index.get(&normalise(name)?)?;
        let mut file = self.archive.by_index(index).ok()?;
        // Bounded whatever the header claimed, and never allocated from a claim. Not `?`: a
        // corrupt deflate stream is a damaged part, and losing one optional part is no reason to
        // refuse the file.
        read_entry(&mut file).ok()
    }

    /// The relationships declared *by* a part, already resolved to part names.
    ///
    /// For `xl/workbook.xml` that is `xl/_rels/workbook.xml.rels`; for the package itself
    /// (`""`) it is `_rels/.rels`.
    pub fn rels(&mut self, part: &str, seen: &mut Seen) -> Vec<Rel> {
        let (dir, file) = part.rsplit_once('/').unwrap_or(("", part));
        let rels_part = if dir.is_empty() {
            format!("_rels/{file}.rels")
        } else {
            format!("{dir}/_rels/{file}.rels")
        };
        let Some(bytes) = self.part(&rels_part) else {
            return Vec::new();
        };
        parse_rels(&bytes, part, seen)
    }

    /// The first relationship of a kind, from a part.
    pub fn rel_of(&mut self, part: &str, kind: RelType, seen: &mut Seen) -> Option<Rel> {
        self.rels(part, seen)
            .into_iter()
            .find(|rel| rel.kind == kind && !rel.external)
    }
}

/// `_rels/*.rels` — Part 2's own tiny vocabulary, in its own namespace.
fn parse_rels(bytes: &[u8], base: &str, seen: &mut Seen) -> Vec<Rel> {
    use crate::xml::read::{Handled, Reader};

    let mut reader = Reader::new(bytes);
    let mut out = Vec::new();
    if reader.root().unwrap_or(None).is_none() {
        return out;
    }
    let _ = reader.children(|_, name, attrs| {
        if name.local != "Relationship" {
            return Ok(Handled::No);
        }
        let (Some(id), Some(kind), Some(target)) = (
            attrs.plain("Id"),
            attrs.plain("Type"),
            attrs.plain("Target"),
        ) else {
            return Ok(Handled::Yes);
        };
        // A relationship type is Part 1 vocabulary and votes on the flavour like a namespace URI,
        // so a Strict file is recognised before its parts are opened.
        crate::xml::read::note_flavour(seen, kind);
        let external = matches!(attrs.plain("TargetMode"), Some("External"));
        // An external target is a URL or another workbook. It is recorded so the report can
        // count it, and its "target" is deliberately left unresolved: nothing here fetches.
        let resolved = if external {
            target.to_owned()
        } else {
            match resolve_target(base, target) {
                Some(resolved) => resolved,
                None => return Ok(Handled::Yes),
            }
        };
        out.push(Rel {
            id: id.to_owned(),
            kind: RelType::from_uri(kind),
            target: resolved,
            external,
        });
        Ok(Handled::Yes)
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_name_is_normalised_before_it_is_looked_up() {
        assert_eq!(
            normalise("xl/workbook.xml").as_deref(),
            Some("xl/workbook.xml")
        );
        // The four spellings real producers use for the same part.
        assert_eq!(
            normalise("/xl/workbook.xml").as_deref(),
            Some("xl/workbook.xml")
        );
        assert_eq!(
            normalise("xl\\workbook.xml").as_deref(),
            Some("xl/workbook.xml")
        );
        assert_eq!(
            normalise("xl/./workbook.xml").as_deref(),
            Some("xl/workbook.xml")
        );
        assert_eq!(
            normalise("XL/Workbook.XML").as_deref(),
            Some("xl/workbook.xml")
        );
    }

    #[test]
    fn percent_escapes_name_the_same_part() {
        assert_eq!(
            normalise("xl/worksheets/my%20sheet.xml").as_deref(),
            Some("xl/worksheets/my sheet.xml")
        );
        // An invalid escape is text, not a refusal.
        assert_eq!(normalise("xl/a%zz.xml").as_deref(), Some("xl/a%zz.xml"));
    }

    /// Zip-slip: a target that climbs out of the package names a file on the machine running
    /// the converter, and there is no reading of it that is a part.
    #[test]
    fn a_target_that_escapes_the_package_is_refused() {
        assert_eq!(normalise("../../etc/passwd"), None);
        assert_eq!(
            resolve_target("xl/workbook.xml", "../../../etc/passwd"),
            None
        );
    }

    #[test]
    fn a_target_resolves_against_the_part_that_declared_it() {
        assert_eq!(
            resolve_target("xl/workbook.xml", "worksheets/sheet1.xml").as_deref(),
            Some("xl/worksheets/sheet1.xml")
        );
        assert_eq!(
            resolve_target("xl/workbook.xml", "/xl/styles.xml").as_deref(),
            Some("xl/styles.xml")
        );
        assert_eq!(
            resolve_target("xl/workbook.xml", "../docProps/app.xml").as_deref(),
            Some("docprops/app.xml")
        );
    }

    #[test]
    fn a_cfb_container_is_locked_rather_than_unreadable() {
        let mut bytes = CFB_MAGIC.to_vec();
        bytes.extend_from_slice(&[0u8; 64]);
        assert!(matches!(Package::open(&bytes), Err(Error::Encrypted)));
    }

    #[test]
    fn something_that_is_not_a_zip_says_so() {
        assert!(matches!(
            Package::open(b"not a document at all"),
            Err(Error::Package(_))
        ));
    }

    #[test]
    fn relationships_resolve_and_carry_their_type() {
        let xml = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
            <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
        </Relationships>"#;
        let mut seen = Seen::default();
        let rels = parse_rels(xml, "", &mut seen);
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].kind, RelType::OfficeDocument);
        assert_eq!(rels[0].target, "xl/workbook.xml");
        assert!(
            seen.transitional,
            "a relationship type votes on the flavour"
        );
    }

    #[test]
    fn an_external_target_is_recorded_and_never_resolved() {
        let xml = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
            <Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLink" Target="https://example.invalid/book.xlsx" TargetMode="External"/>
        </Relationships>"#;
        let rels = parse_rels(xml, "xl/workbook.xml", &mut Seen::default());
        assert!(rels[0].external);
        assert_eq!(rels[0].target, "https://example.invalid/book.xlsx");
    }
}
