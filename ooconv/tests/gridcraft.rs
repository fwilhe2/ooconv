// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Loop G** — an Excel-compatible spreadsheet, `gridcraft-cli`, reads every `.xlsx` we write
//! and calculates it. Three claims:
//!
//! 1. every file opens, and gridcraft's own xlsx round trip of it changes nothing;
//! 2. the fixture's formulas, whose cached values are true results, all calculate to them;
//! 3. where a corpus document's cache disagrees with gridcraft — corpus caches are often
//!    synthetic — gridcraft's result equals **LibreOffice's**: two independent engines agree on
//!    what our translated formula means, so the cache was the odd one out.
//!
//! cargo test -p ooconv --test gridcraft

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use ooconv::sheet::Value;
use ooconv::{Format, Kind, Report};

/// Mismatches LibreOffice and gridcraft *also* disagree on, by what they are. Matched on the
/// formula text, never on a file name.
const ENGINES_DISAGREE: &[(&str, &str)] = &[
    // Empty: every corpus cache gridcraft disputes, LibreOffice disputes the same way. Kept, and
    // checked both ways, because the corpus grows.
];

fn write_all(dir: &std::path::Path) -> Vec<(PathBuf, bool)> {
    let mut out = Vec::new();
    let mut r = Report::default();
    let path = dir.join("fixture.xlsx");
    let book = common::fixtures::workbook();
    std::fs::write(
        &path,
        ooconv::sheet::write(&book, Format::Xlsx, &mut r).unwrap(),
    )
    .unwrap();
    out.push((path, true));
    for (i, src) in common::corpus(&["xlsx", "ods", "fods"]).iter().enumerate() {
        if src.ends_with("zip-bomb.xlsx") {
            continue;
        }
        let bytes = std::fs::read(src).unwrap();
        if !ooconv::sniff(&bytes).is_ok_and(|f| f.kind() == Kind::Spreadsheet) {
            continue;
        }
        let Ok(converted) = ooconv::convert(&bytes, Format::Xlsx) else {
            continue;
        };
        let stem = src.file_stem().unwrap().to_string_lossy();
        let path = dir.join(format!("{i:03}-{stem}.xlsx"));
        std::fs::write(&path, converted.bytes).unwrap();
        out.push((path, false));
    }
    out
}

/// `Sheet!A1` → (sheet name, row, col).
fn split_cell(cell: &str) -> Option<(String, u32, u32)> {
    let (sheet, a1) = cell.rsplit_once('!')?;
    let split = a1.find(|c: char| c.is_ascii_digit())?;
    let col = ooconv::sheet::formula::column_index(&a1[..split])?;
    let row = ooconv::sheet::formula::row_index(&a1[split..])?;
    Some((sheet.trim_matches('\'').to_owned(), row, col))
}

fn same(v: &Value, json: &serde_json::Value) -> bool {
    match (v, json) {
        (Value::Number(x), serde_json::Value::Number(n)) => {
            n.as_f64().is_some_and(|y| common::close(*x, y))
        }
        (Value::Text(t), serde_json::Value::String(s)) => t == s,
        (Value::Bool(b), serde_json::Value::Bool(c)) => b == c,
        (Value::Error(e), serde_json::Value::String(s)) => e.as_str() == s,
        (Value::Error(e), serde_json::Value::Object(o)) => {
            o.get("error").and_then(|x| x.as_str()) == Some(e.as_str())
        }
        _ => false,
    }
}

#[test]
fn gridcraft_opens_and_calculates_what_we_write() {
    let Some(gridcraft) = common::gridcraft() else {
        return;
    };
    let dir = common::scratch("gridcraft");
    let files = write_all(&dir);
    let out = Command::new(&gridcraft)
        .arg("check")
        .args(files.iter().map(|(p, _)| p))
        .arg("--json")
        .arg("--max-report")
        .arg("1000")
        .output()
        .expect("gridcraft-cli runs");
    let report: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stderr)));

    let mut failures = Vec::new();
    let mut to_ask_libreoffice: Vec<(PathBuf, String, serde_json::Value, String)> = Vec::new();
    let trusted: HashMap<String, bool> = files
        .iter()
        .map(|(p, t)| (p.file_name().unwrap().to_string_lossy().into_owned(), *t))
        .collect();
    let mut checked = 0;
    for file in report["files"].as_array().expect("files") {
        let path = PathBuf::from(file["path"].as_str().unwrap());
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if file["opened"] != true || !file["error"].is_null() {
            failures.push(format!(
                "{name}: gridcraft could not open it: {}",
                file["error"]
            ));
            continue;
        }
        let rt = &file["roundTrip"];
        if !rt["error"].is_null() || rt["differing"].as_array().is_some_and(|d| !d.is_empty()) {
            failures.push(format!("{name}: gridcraft's round trip differs: {rt}"));
        }
        checked += file["calc"]["checked"].as_u64().unwrap_or(0);
        for m in file["calc"]["mismatches"].as_array().into_iter().flatten() {
            let cell = m["cell"].as_str().unwrap_or("").to_owned();
            let formula = m["formula"].as_str().unwrap_or("").to_owned();
            if trusted.get(&name).copied().unwrap_or(false) {
                failures.push(format!(
                    "{name} {cell} {formula}: cache {} but gridcraft {}",
                    m["expected"], m["actual"]
                ));
            } else {
                to_ask_libreoffice.push((path.clone(), cell, m["actual"].clone(), formula));
            }
        }
    }

    // Claim 3: ask LibreOffice about every corpus mismatch.
    if !to_ask_libreoffice.is_empty()
        && let Some(soffice) = common::soffice()
    {
        let mut inputs: Vec<PathBuf> = to_ask_libreoffice.iter().map(|m| m.0.clone()).collect();
        inputs.sort();
        inputs.dedup();
        let lo = dir.join("libreoffice");
        std::fs::create_dir_all(&lo).unwrap();
        common::soffice_convert(&soffice, &inputs, "fods", &lo);
        let mut books = HashMap::new();
        for input in &inputs {
            let fods = lo.join(input.with_extension("fods").file_name().unwrap());
            let bytes = std::fs::read(&fods).unwrap();
            let book = ooconv::sheet::read(&bytes, Format::Fods, &mut Report::default()).unwrap();
            books.insert(input.clone(), book);
        }
        let mut used = Vec::new();
        for (path, cell, actual, formula) in &to_ask_libreoffice {
            let name = path.file_name().unwrap().to_string_lossy();
            let Some((sheet, row, col)) = split_cell(cell) else {
                failures.push(format!("{name}: unreadable cell {cell}"));
                continue;
            };
            let lo_value = books[path]
                .sheets
                .iter()
                .find(|s| s.name == sheet)
                .and_then(|s| s.cells.get(&(row, col)))
                .map(|c| c.value.clone())
                .unwrap_or_default();
            if same(&lo_value, actual) {
                continue;
            }
            if let Some((needle, _)) = ENGINES_DISAGREE.iter().find(|(n, _)| formula.contains(n)) {
                used.push(*needle);
                continue;
            }
            failures.push(format!(
                "{name} {cell} {formula}: gridcraft {actual}, LibreOffice {lo_value:?}"
            ));
        }
        // Both ways: an entry that no longer explains anything is to be deleted.
        for (needle, _) in ENGINES_DISAGREE {
            if !used.contains(needle) {
                failures.push(format!(
                    "ENGINES_DISAGREE entry {needle:?} no longer occurs"
                ));
            }
        }
    }
    eprintln!(
        "gridcraft checked {checked} formulas in {} workbooks",
        files.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
