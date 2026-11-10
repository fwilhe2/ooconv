// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **R2 — everything written validates.** Every format the converter writes, from the fixtures
//! and from every corpus document, is handed to its format's validator: `jing` with the ODF 1.4
//! RELAX NG schema, and the Open XML SDK's `OpenXmlValidator` (`tools/ooxml-validate`). Both
//! are what a consumer's strictest reading looks like; neither has an opinion this project
//! wrote.
//!
//! cargo test -p ooconv --test schema

mod common;

use std::path::PathBuf;

use ooconv::{Format, Kind, Report};

/// Write every fixture and corpus document as every format of its kind into `dir`, returning
/// the files by family.
fn write_everything(dir: &std::path::Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut odf = Vec::new();
    let mut ooxml = Vec::new();
    let mut put = |name: &str, to: Format, bytes: Vec<u8>| {
        let path = dir.join(format!("{name}.{}", to.extension()));
        std::fs::write(&path, bytes).unwrap();
        match to {
            Format::Xlsx | Format::Docx => ooxml.push(path),
            _ => odf.push(path),
        }
    };
    let book = common::fixtures::workbook();
    let doc = common::fixtures::document();
    for to in Format::ALL {
        let mut r = Report::default();
        match to.kind() {
            Kind::Spreadsheet => put(
                "fixture",
                to,
                ooconv::sheet::write(&book, to, &mut r).unwrap(),
            ),
            Kind::Text => put(
                "fixture",
                to,
                ooconv::text::write(&doc, to, &mut r).unwrap(),
            ),
        }
    }
    for (i, path) in common::corpus(&["xlsx", "ods", "fods", "docx", "odt", "fodt"])
        .iter()
        .enumerate()
    {
        if path.ends_with("zip-bomb.xlsx") {
            continue;
        }
        let bytes = std::fs::read(path).unwrap();
        let Ok(from) = ooconv::sniff(&bytes) else {
            continue;
        };
        for to in Format::ALL.into_iter().filter(|f| f.kind() == from.kind()) {
            if let Ok(out) = ooconv::convert(&bytes, to) {
                let stem = path.file_stem().unwrap().to_string_lossy();
                put(&format!("{i:03}-{stem}"), to, out.bytes);
            }
        }
    }
    (odf, ooxml)
}

#[test]
fn every_odf_document_written_is_valid() {
    let Some(jing) = common::jing() else { return };
    let dir = common::scratch("schema-odf");
    let (odf, _) = write_everything(&dir);
    assert!(odf.len() > 100, "{} documents", odf.len());
    if let Err(e) = common::validate_odf(&jing, &odf) {
        panic!("{e}");
    }
    eprintln!("validated {} ODF documents", odf.len());
}

#[test]
fn every_ooxml_document_written_is_valid() {
    let Some(validator) = common::ooxml_validator() else {
        return;
    };
    let dir = common::scratch("schema-ooxml");
    let (_, ooxml) = write_everything(&dir);
    assert!(ooxml.len() > 100, "{} documents", ooxml.len());
    if let Err(e) = common::validate_ooxml(&validator, &ooxml) {
        panic!("{e}");
    }
    eprintln!("validated {} OOXML documents", ooxml.len());
}
