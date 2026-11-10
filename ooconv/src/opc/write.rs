// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! OPC writing (ECMA-376 Part 2): parts, their content types, and the relationships between
//! them. A writer adds parts and relationships; [`PackageWriter::finish`] emits
//! `[Content_Types].xml` and every `_rels/*.rels` from what was added, so a part can never be
//! present and untyped, or a relationship point at a part that is not there.
//!
//! Always Transitional: it is what every consumer reads (grind `doc/xlsx-format.md` §1.1).

use std::collections::BTreeMap;
use std::io::{Cursor, Write as _};

use crate::xml::write::Writer;
use crate::{Error, Result};

pub const REL_BASE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub const CORE_PROPERTIES_REL: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";

struct Part {
    content_type: String,
    bytes: Vec<u8>,
}

struct Relationship {
    id: String,
    kind: String,
    target: String,
    external: bool,
}

#[derive(Default)]
pub struct PackageWriter {
    /// Part name without the leading `/`, in insertion order of the zip.
    parts: Vec<(String, Part)>,
    /// Source part (`""` for the package) to its relationships.
    rels: BTreeMap<String, Vec<Relationship>>,
}

impl PackageWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a part. `name` is package-relative, without a leading `/`: `xl/workbook.xml`.
    pub fn part(&mut self, name: &str, content_type: &str, bytes: impl Into<Vec<u8>>) {
        debug_assert!(!name.starts_with('/'));
        debug_assert!(
            self.parts.iter().all(|(n, _)| n != name),
            "duplicate {name}"
        );
        self.parts.push((
            name.to_owned(),
            Part {
                content_type: content_type.to_owned(),
                bytes: bytes.into(),
            },
        ));
    }

    /// Relate `source` (`""` for the package) to an internal part, by its package-relative name.
    /// Returns the relationship id. The target is written relative to the source's directory.
    pub fn relate(&mut self, source: &str, kind: &str, target_part: &str) -> String {
        let target = relative(source, target_part);
        self.push_rel(source, kind, target, false)
    }

    /// Relate `source` to something outside the package — a hyperlink's URL.
    pub fn relate_external(&mut self, source: &str, kind: &str, url: &str) -> String {
        self.push_rel(source, kind, url.to_owned(), true)
    }

    fn push_rel(&mut self, source: &str, kind: &str, target: String, external: bool) -> String {
        let list = self.rels.entry(source.to_owned()).or_default();
        let id = format!("rId{}", list.len() + 1);
        list.push(Relationship {
            id: id.clone(),
            kind: kind.to_owned(),
            target,
            external,
        });
        id
    }

    pub fn finish(self) -> Result<Vec<u8>> {
        // Every internal relationship must name a part that exists: the package-level invariant
        // a consumer checks first.
        for (source, rels) in &self.rels {
            for rel in rels.iter().filter(|r| !r.external) {
                let resolved = resolve(source, &rel.target);
                assert!(
                    self.parts.iter().any(|(n, _)| *n == resolved),
                    "relationship {} from {source:?} to missing part {resolved}",
                    rel.id
                );
            }
        }

        let zip = |e: zip::result::ZipError| Error::Package(e.to_string());
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        out.start_file("[Content_Types].xml", deflated)
            .map_err(zip)?;
        out.write_all(self.content_types().as_bytes())?;

        for (source, rels) in &self.rels {
            out.start_file(rels_part(source), deflated).map_err(zip)?;
            out.write_all(rels_xml(rels).as_bytes())?;
        }
        for (name, part) in &self.parts {
            out.start_file(name.as_str(), deflated).map_err(zip)?;
            out.write_all(&part.bytes)?;
        }
        Ok(out.finish().map_err(zip)?.into_inner())
    }

    fn content_types(&self) -> String {
        let mut w = Writer::new();
        w.start("Types").attr(
            "xmlns",
            "http://schemas.openxmlformats.org/package/2006/content-types",
        );
        w.start("Default")
            .attr("Extension", "rels")
            .attr(
                "ContentType",
                "application/vnd.openxmlformats-package.relationships+xml",
            )
            .end();
        w.start("Default")
            .attr("Extension", "xml")
            .attr("ContentType", "application/xml")
            .end();
        for (name, part) in &self.parts {
            w.start("Override")
                .attr("PartName", format!("/{name}"))
                .attr("ContentType", &part.content_type)
                .end();
        }
        w.end();
        w.finish()
    }
}

/// `docProps/core.xml`: the one metadata part both writers add, naming the producer.
pub fn core_properties() -> String {
    let mut w = Writer::new();
    w.start("cp:coreProperties")
        .attr(
            "xmlns:cp",
            "http://schemas.openxmlformats.org/package/2006/metadata/core-properties",
        )
        .attr("xmlns:dc", "http://purl.org/dc/elements/1.1/");
    w.leaf("dc:creator", "ooconv");
    w.end();
    w.finish()
}

fn rels_xml(rels: &[Relationship]) -> String {
    let mut w = Writer::new();
    w.start("Relationships").attr(
        "xmlns",
        "http://schemas.openxmlformats.org/package/2006/relationships",
    );
    for rel in rels {
        w.start("Relationship")
            .attr("Id", &rel.id)
            .attr("Type", &rel.kind)
            .attr("Target", &rel.target);
        if rel.external {
            w.attr("TargetMode", "External");
        }
        w.end();
    }
    w.end();
    w.finish()
}

/// `xl/workbook.xml` → `xl/_rels/workbook.xml.rels`; the package → `_rels/.rels`.
fn rels_part(source: &str) -> String {
    match source.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None if source.is_empty() => "_rels/.rels".to_owned(),
        None => format!("_rels/{source}.rels"),
    }
}

fn dir_of(part: &str) -> &str {
    part.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// `target` as a path relative to `source`'s directory. Only ever called with targets in or
/// below that directory, or for the package root; anything else is written absolute.
fn relative(source: &str, target: &str) -> String {
    let dir = dir_of(source);
    if dir.is_empty() {
        return target.to_owned();
    }
    match target.strip_prefix(dir).and_then(|t| t.strip_prefix('/')) {
        Some(rest) => rest.to_owned(),
        None => format!("/{target}"),
    }
}

fn resolve(source: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_owned();
    }
    let dir = dir_of(source);
    match dir.is_empty() {
        true => target.to_owned(),
        false => format!("{dir}/{target}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names::{RelType, Seen};
    use crate::opc::package::Package;

    #[test]
    fn what_is_written_is_what_the_reader_finds() {
        let mut p = PackageWriter::new();
        p.part("xl/workbook.xml", "application/test", "<workbook/>");
        p.part(
            "xl/worksheets/sheet1.xml",
            "application/test",
            "<worksheet/>",
        );
        p.part("docProps/core.xml", "application/test", "<core/>");
        p.relate("", &format!("{REL_BASE}/officeDocument"), "xl/workbook.xml");
        p.relate(
            "xl/workbook.xml",
            &format!("{REL_BASE}/worksheet"),
            "xl/worksheets/sheet1.xml",
        );
        p.relate("", CORE_PROPERTIES_REL, "docProps/core.xml");
        let bytes = p.finish().unwrap();

        let mut pkg = Package::open(&bytes).unwrap();
        let mut seen = Seen::default();
        let main = pkg.rel_of("", RelType::OfficeDocument, &mut seen).unwrap();
        assert_eq!(main.target, "xl/workbook.xml");
        let sheet = pkg
            .rel_of(&main.target, RelType::Worksheet, &mut seen)
            .unwrap();
        assert_eq!(sheet.target, "xl/worksheets/sheet1.xml");
        assert_eq!(pkg.part(&sheet.target).unwrap(), b"<worksheet/>");
    }

    #[test]
    #[should_panic(expected = "missing part")]
    fn a_relationship_to_nothing_is_a_bug() {
        let mut p = PackageWriter::new();
        p.relate("", "x", "nowhere.xml");
        let _ = p.finish();
    }

    #[test]
    fn targets_are_relative_to_their_source() {
        assert_eq!(relative("xl/workbook.xml", "xl/styles.xml"), "styles.xml");
        assert_eq!(relative("", "xl/workbook.xml"), "xl/workbook.xml");
        assert_eq!(
            relative("word/document.xml", "docProps/x.xml"),
            "/docProps/x.xml"
        );
        assert_eq!(
            rels_part("word/document.xml"),
            "word/_rels/document.xml.rels"
        );
        assert_eq!(rels_part(""), "_rels/.rels");
    }
}
