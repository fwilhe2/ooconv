// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A converter eats strangers' files. Every hostile fixture has exactly one acceptable outcome,
//! stated here by **class** — a named error, or a conversion — and a conversion of one must
//! itself be valid input to the reader. **Never skips**, except the zip bomb in a debug build,
//! where inflating 256 MiB unoptimised takes minutes rather than seconds.
//!
//! cargo test -p ooconv --test hostile

mod common;

use ooconv::{Error, Format};

#[derive(Debug)]
enum Outcome {
    Converts,
    NotADocument,
    Encrypted,
    Package,
    Xml,
}

const EXPECTED: &[(&str, Outcome, &str)] = &[
    (
        "billion-laughs.xlsx",
        Outcome::Converts,
        "quick-xml expands no entity definitions",
    ),
    (
        "deep-nesting.xlsx",
        Outcome::Xml,
        "nesting past the walker's bound is refused, not recursed",
    ),
    (
        "empty-zip.xlsx",
        Outcome::NotADocument,
        "a zip with no parts names no main part",
    ),
    (
        "encrypted.xlsx",
        Outcome::Encrypted,
        "a CFB container: locked, not broken",
    ),
    (
        "external-entity.xlsx",
        Outcome::Converts,
        "an external entity is never resolved",
    ),
    (
        "html-table.xlsx",
        Outcome::NotADocument,
        "HTML saved as .xlsx is sniffed by its bytes",
    ),
    ("many-parts.xlsx", Outcome::Converts, "under the part cap"),
    (
        "not-a-zip.xlsx",
        Outcome::NotADocument,
        "neither a zip nor flat ODF",
    ),
    (
        "no-workbook-part.xlsx",
        Outcome::NotADocument,
        "the officeDocument relationship is missing",
    ),
    ("truncated.xlsx", Outcome::Package, "no central directory"),
    (
        "zip-bomb.xlsx",
        Outcome::Converts,
        "256 MiB honestly declared, under the 512 MiB part cap",
    ),
    (
        "zip-slip.xlsx",
        Outcome::Converts,
        "the escaping part is unreachable and reported",
    ),
];

#[test]
fn every_hostile_file_has_its_one_outcome() {
    let files = common::corpus(&["xlsx"]);
    let hostile: Vec<_> = files
        .iter()
        .filter(|p| p.parent().is_some_and(|d| d.ends_with("hostile")))
        .collect();
    // Both ways: every file has an expectation and every expectation a file.
    for path in &hostile {
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(
            EXPECTED.iter().any(|(n, ..)| *n == name),
            "{name} has no expectation"
        );
    }
    for (name, ..) in EXPECTED {
        assert!(
            hostile.iter().any(|p| p.ends_with(name)),
            "{name} is missing from the corpus"
        );
    }
    for (name, outcome, why) in EXPECTED {
        if *name == "zip-bomb.xlsx" && cfg!(debug_assertions) {
            eprintln!("skipping {name} in a debug build; cargo test --release runs it");
            continue;
        }
        let path = hostile.iter().find(|p| p.ends_with(name)).unwrap();
        let bytes = std::fs::read(path).unwrap();
        for to in [Format::Ods, Format::Fods, Format::Xlsx] {
            let got = ooconv::convert(&bytes, to);
            let ok = match (outcome, &got) {
                (Outcome::Converts, Ok(c)) => {
                    // What we wrote reads back.
                    ooconv::convert(&c.bytes, Format::Fods).is_ok()
                }
                (Outcome::NotADocument, Err(Error::UnknownFormat)) => true,
                (Outcome::Encrypted, Err(Error::Encrypted)) => true,
                (Outcome::Package, Err(Error::Package(_))) => true,
                (Outcome::Xml, Err(Error::Xml(_))) => true,
                _ => false,
            };
            assert!(
                ok,
                "{name} → {to:?}: expected {outcome:?} ({why}), got {:?}",
                got.map(|c| c.report)
            );
        }
    }
}

/// A flat ODF document is XML from a stranger too.
#[test]
fn flat_odf_with_an_entity_bomb_is_refused_or_harmless() {
    let doc = r#"<?xml version="1.0"?>
<!DOCTYPE office:document [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">]>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  office:mimetype="application/vnd.oasis.opendocument.spreadsheet">
<office:body><office:spreadsheet><table:table table:name="S"><table:table-row>
<table:table-cell office:value-type="string"><text:p>&b;</text:p></table:table-cell>
</table:table-row></table:table></office:spreadsheet></office:body></office:document>"#;
    match ooconv::convert(doc.as_bytes(), Format::Xlsx) {
        Ok(_) | Err(Error::Xml(_)) => {}
        Err(e) => panic!("{e}"),
    }
}

/// `table:number-rows-repeated` is a multiplication a stranger chooses.
#[test]
fn a_repeated_row_with_content_cannot_exhaust_memory() {
    let doc = r#"<?xml version="1.0"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  office:mimetype="application/vnd.oasis.opendocument.spreadsheet">
<office:body><office:spreadsheet><table:table table:name="S">
<table:table-row table:number-rows-repeated="4000000000">
<table:table-cell table:number-columns-repeated="4000000000" office:value-type="float" office:value="1"/>
</table:table-row></table:table></office:spreadsheet></office:body></office:document>"#;
    let started = std::time::Instant::now();
    let mut report = ooconv::Report::default();
    let book = ooconv::sheet::read(doc.as_bytes(), Format::Fods, &mut report).unwrap();
    let cells: usize = book.sheets.iter().map(|s| s.cells.len()).sum();
    assert!(cells <= 5_000_000, "{cells} cells");
    assert!(
        report.count("cell beyond the size cap") > 0,
        "{:?}",
        report.losses
    );
    assert!(
        started.elapsed().as_secs() < 120,
        "took {:?}",
        started.elapsed()
    );
}
