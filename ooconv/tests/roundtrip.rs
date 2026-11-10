// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Loop R** — the model survives every writer and its reader: `read(write(m)) == m`, for the
//! fixtures and for every corpus document's model, in every format of its kind. No oracle is
//! needed, so this **never skips**.
//!
//! A corpus document whose write reports a loss is held to a weaker claim — it must still read
//! back — and is counted; [`FLOOR`] is the ratchet on the documents that round-trip exactly.
//!
//! cargo test -p ooconv --test roundtrip

mod common;

use ooconv::{Format, Kind, Report};

/// Corpus documents that round-trip exactly through every format of their kind. **Raise it,
/// never lower it.**
const FLOOR: usize = 104;

fn formats(kind: Kind) -> Vec<Format> {
    Format::ALL
        .into_iter()
        .filter(|f| f.kind() == kind)
        .collect()
}

#[test]
fn the_workbook_fixture_survives_every_format() {
    let book = common::fixtures::workbook();
    for to in formats(Kind::Spreadsheet) {
        let mut report = Report::default();
        let bytes = ooconv::sheet::write(&book, to, &mut report).unwrap();
        assert_eq!(report.losses, vec![], "{to:?} lost something");
        let mut report = Report::default();
        let back = ooconv::sheet::read(&bytes, to, &mut report).unwrap();
        assert_eq!(report.losses, vec![], "reading {to:?} lost something");
        if let Some(d) = common::diff_books(&book, &back) {
            panic!("{to:?}: {d}");
        }
    }
}

#[test]
fn the_document_fixture_survives_every_format() {
    let doc = common::fixtures::document();
    for to in formats(Kind::Text) {
        let mut report = Report::default();
        let bytes = ooconv::text::write(&doc, to, &mut report).unwrap();
        assert_eq!(report.losses, vec![], "{to:?} lost something");
        let mut report = Report::default();
        let back = ooconv::text::read(&bytes, to, &mut report).unwrap();
        assert_eq!(report.losses, vec![], "reading {to:?} lost something");
        if let Some(d) = common::diff_docs(&doc, &back) {
            panic!("{to:?}: {d}");
        }
    }
}

#[test]
fn every_corpus_document_survives_every_format_of_its_kind() {
    let mut exact = 0;
    let mut lossy = Vec::new();
    let mut failures = Vec::new();
    for path in common::corpus(&["xlsx", "ods", "fods", "docx", "odt", "fodt"]) {
        let bytes = std::fs::read(&path).unwrap();
        let Ok(from) = ooconv::sniff(&bytes) else {
            continue;
        };
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // The 256 MiB text node is held by the hostile tests, not here.
        if name == "zip-bomb.xlsx" {
            continue;
        }
        let mut all_exact = true;
        let mut report = Report::default();
        match from.kind() {
            Kind::Spreadsheet => {
                let Ok(book) = ooconv::sheet::read(&bytes, from, &mut report) else {
                    continue;
                };
                for to in formats(Kind::Spreadsheet) {
                    let mut wr = Report::default();
                    let out = ooconv::sheet::write(&book, to, &mut wr).unwrap();
                    let back = match ooconv::sheet::read(&out, to, &mut Report::default()) {
                        Ok(b) => b,
                        Err(e) => {
                            failures.push(format!("{name} → {to:?}: unreadable: {e}"));
                            continue;
                        }
                    };
                    // xlsx has one criteria mode; where no formula depends on the other modes, the
                    // writer reports nothing and the workbook comes back in Excel's.
                    let mut want = book.clone();
                    if to == Format::Xlsx {
                        want.criteria = back.criteria;
                    }
                    match (wr.losses.is_empty(), common::diff_books(&want, &back)) {
                        (_, None) => {}
                        (true, Some(d)) => failures.push(format!("{name} → {to:?}: {d}")),
                        (false, Some(_)) => {
                            all_exact = false;
                            lossy.push(format!("{name} → {to:?}: {:?}", wr.by_class()));
                        }
                    }
                }
            }
            Kind::Text => {
                let Ok(doc) = ooconv::text::read(&bytes, from, &mut report) else {
                    continue;
                };
                for to in formats(Kind::Text) {
                    let mut wr = Report::default();
                    let out = ooconv::text::write(&doc, to, &mut wr).unwrap();
                    let back = match ooconv::text::read(&out, to, &mut Report::default()) {
                        Ok(b) => b,
                        Err(e) => {
                            failures.push(format!("{name} → {to:?}: unreadable: {e}"));
                            continue;
                        }
                    };
                    match (wr.losses.is_empty(), common::diff_docs(&doc, &back)) {
                        (_, None) => {}
                        (true, Some(d)) => failures.push(format!("{name} → {to:?}: {d}")),
                        (false, Some(_)) => {
                            all_exact = false;
                            lossy.push(format!("{name} → {to:?}: {:?}", wr.by_class()));
                        }
                    }
                }
            }
        }
        if all_exact {
            exact += 1;
        }
    }
    for l in &lossy {
        eprintln!("lossy write: {l}");
    }
    eprintln!("{exact} corpus documents round-trip exactly");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    assert!(
        exact >= FLOOR,
        "only {exact} round-trip exactly; the floor is {FLOOR}"
    );
}
