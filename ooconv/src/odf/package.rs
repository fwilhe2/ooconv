// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The two physical forms of an ODF document, read and written. The package's `mimetype` comes
//! first, stored, with no trailing newline (Part 2 §3.3); the flat form is one `office:document`.

use std::io::{Cursor, Write as _};

use super::{MIME_ODS, MIME_ODT, NAMESPACES, Parts, VERSION};
use crate::limits::{MAX_PART_BYTES, MAX_PARTS, MAX_TOTAL_BYTES, read_entry};
use crate::names::Ns;
use crate::xml::read::Reader;
use crate::{Error, Format, Result};

/// A package's format from its `mimetype` entry, or `None` for a zip that is not ODF.
pub fn sniff_package(bytes: &[u8]) -> Result<Option<Format>> {
    let mut archive = archive(bytes)?;
    let Ok(mut entry) = archive.by_name("mimetype") else {
        return Ok(None);
    };
    let mime = read_entry(&mut entry)?;
    Ok(match String::from_utf8_lossy(&mime).trim() {
        MIME_ODS => Some(Format::Ods),
        MIME_ODT => Some(Format::Odt),
        _ => None,
    })
}

/// A flat document's format from its root's `office:mimetype`.
pub fn sniff_flat(bytes: &[u8]) -> Option<Format> {
    let mut reader = Reader::new(bytes);
    let (name, attrs) = reader.root().ok()??;
    if !name.in_ns(Ns::Office, "document") {
        return None;
    }
    match attrs.get(Ns::Office, "mimetype")? {
        MIME_ODS => Some(Format::Fods),
        MIME_ODT => Some(Format::Fodt),
        _ => None,
    }
}

fn archive(bytes: &[u8]) -> Result<zip::ZipArchive<Cursor<&[u8]>>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| Error::Package(format!("not a readable package: {e}")))?;
    if archive.len() > MAX_PARTS {
        return Err(Error::Package(format!("{} parts", archive.len())));
    }
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .map_err(|e| Error::Package(format!("unreadable entry {i}: {e}")))?;
        if entry.size() > MAX_PART_BYTES {
            return Err(Error::Package("a part larger than any document's".into()));
        }
        total = total.saturating_add(entry.size());
        if total > MAX_TOTAL_BYTES {
            return Err(Error::Package("decompresses past the cap".into()));
        }
    }
    Ok(archive)
}

/// The XML documents a reader walks, in the order it should walk them: `styles.xml` before
/// `content.xml` for a package, so common styles are known when the body refers to them; the one
/// document for the flat form.
pub fn documents(bytes: &[u8]) -> Result<Vec<Vec<u8>>> {
    if !crate::opc::package::is_zip(bytes) {
        return Ok(vec![bytes.to_vec()]);
    }
    let mut archive = archive(bytes)?;
    if let Ok(mut manifest) = archive.by_name("META-INF/manifest.xml") {
        let manifest = read_entry(&mut manifest)?;
        if manifest
            .windows(b"encryption-data".len())
            .any(|w| w == b"encryption-data")
        {
            return Err(Error::Encrypted);
        }
    }
    let mut out = Vec::new();
    for name in ["styles.xml", "content.xml"] {
        if let Ok(mut entry) = archive.by_name(name) {
            out.push(read_entry(&mut entry)?);
        } else if name == "content.xml" {
            return Err(Error::Package("no content.xml".into()));
        }
    }
    Ok(out)
}

fn root(w: &mut String, element: &str, mimetype: Option<&str>) {
    w.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<");
    w.push_str(element);
    for (k, v) in NAMESPACES {
        w.push_str(&format!(" {k}=\"{v}\""));
    }
    w.push_str(&format!(" office:version=\"{VERSION}\""));
    if let Some(m) = mimetype {
        w.push_str(&format!(" office:mimetype=\"{m}\""));
    }
    w.push('>');
}

fn section(w: &mut String, element: &str, inner: &str) {
    if !inner.is_empty() {
        w.push_str(&format!("<{element}>{inner}</{element}>"));
    }
}

/// Assemble the parts as the form `flat` asks for.
pub fn assemble(parts: &Parts, flat: bool) -> Result<Vec<u8>> {
    let auto_all = format!("{}{}", parts.automatic_styles, parts.page_layouts);
    if flat {
        let mut w = String::new();
        root(&mut w, "office:document", Some(parts.mimetype));
        section(&mut w, "office:font-face-decls", &parts.font_faces);
        section(&mut w, "office:styles", &parts.styles);
        section(&mut w, "office:automatic-styles", &auto_all);
        section(&mut w, "office:master-styles", &parts.master_styles);
        w.push_str(&format!("<office:body>{}</office:body>", parts.body));
        w.push_str("</office:document>\n");
        return Ok(w.into_bytes());
    }

    let mut content = String::new();
    root(&mut content, "office:document-content", None);
    section(&mut content, "office:font-face-decls", &parts.font_faces);
    section(
        &mut content,
        "office:automatic-styles",
        &parts.automatic_styles,
    );
    content.push_str(&format!("<office:body>{}</office:body>", parts.body));
    content.push_str("</office:document-content>\n");

    let mut styles = String::new();
    root(&mut styles, "office:document-styles", None);
    section(&mut styles, "office:font-face-decls", &parts.font_faces);
    section(&mut styles, "office:styles", &parts.styles);
    section(&mut styles, "office:automatic-styles", &parts.page_layouts);
    section(&mut styles, "office:master-styles", &parts.master_styles);
    styles.push_str("</office:document-styles>\n");

    let manifest = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <manifest:manifest xmlns:manifest=\"{}\" manifest:version=\"{VERSION}\">\
         <manifest:file-entry manifest:full-path=\"/\" manifest:version=\"{VERSION}\" \
         manifest:media-type=\"{}\"/>\
         <manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>\
         <manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>\
         </manifest:manifest>\n",
        crate::names::ODF_MANIFEST,
        parts.mimetype
    );

    let zip = |e: zip::result::ZipError| Error::Package(e.to_string());
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    w.start_file("mimetype", stored).map_err(zip)?;
    w.write_all(parts.mimetype.as_bytes())?;
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, body) in [
        ("content.xml", content.as_str()),
        ("styles.xml", styles.as_str()),
        ("META-INF/manifest.xml", manifest.as_str()),
    ] {
        w.start_file(name, deflated).map_err(zip)?;
        w.write_all(body.as_bytes())?;
    }
    Ok(w.finish().map_err(zip)?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts() -> Parts {
        Parts {
            mimetype: MIME_ODT,
            body: "<office:text><text:p>hi</text:p></office:text>".into(),
            ..Parts::default()
        }
    }

    #[test]
    fn both_forms_sniff_as_what_they_are() {
        let pkg = assemble(&parts(), false).unwrap();
        assert_eq!(
            &pkg[30..38],
            b"mimetype",
            "the first entry, at its fixed offset"
        );
        assert_eq!(crate::sniff(&pkg).unwrap(), Format::Odt);
        let flat = assemble(&parts(), true).unwrap();
        assert_eq!(crate::sniff(&flat).unwrap(), Format::Fodt);
    }

    #[test]
    fn the_package_is_walked_styles_first() {
        let pkg = assemble(&parts(), false).unwrap();
        let docs = documents(&pkg).unwrap();
        assert_eq!(docs.len(), 2);
        assert!(String::from_utf8_lossy(&docs[0]).contains("document-styles"));
        assert!(String::from_utf8_lossy(&docs[1]).contains("<text:p>hi</text:p>"));
    }
}
