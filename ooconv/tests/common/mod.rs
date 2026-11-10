// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What every oracle test shares: finding the oracles, the corpus, scratch space, and comparing
//! models with the tolerance their formats need and no more.
//!
//! An oracle that is not installed **skips with a notice**, unless `OOCONV_REQUIRE_ORACLES` names
//! it (`jing,ooxml-validate,soffice`) or is `1`/`all`: CI sets it, and there a missing oracle is
//! a failure, because a test that cannot fail is not a test.

#![allow(dead_code)]

pub mod fixtures;

use std::path::{Path, PathBuf};
use std::process::Command;

use ooconv::sheet::{Cell, Value, Workbook};
use ooconv::text::{Block, Document};

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// An oracle, or `None` after saying why the caller skips.
fn found(name: &str, path: Option<PathBuf>) -> Option<PathBuf> {
    if path.is_none() {
        let msg =
            format!("oracle `{name}` not found; see scripts/fetch-tools.sh and doc/oracles.md");
        // `1` or `all` requires every oracle; otherwise a comma-separated list of names.
        let required = std::env::var("OOCONV_REQUIRE_ORACLES").unwrap_or_default();
        if matches!(required.as_str(), "1" | "all") || required.split(',').any(|r| r.trim() == name)
        {
            panic!("{msg} (OOCONV_REQUIRE_ORACLES={required})");
        }
        eprintln!("skipping: {msg}");
    }
    path
}

fn on_path(exe: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(exe))
            .find(|p| p.is_file())
    })
}

fn env_or(var: &str, default: PathBuf) -> Option<PathBuf> {
    let p = std::env::var_os(var).map(PathBuf::from).unwrap_or(default);
    p.exists().then_some(p)
}

pub fn jing() -> Option<PathBuf> {
    found(
        "jing",
        env_or("OOCONV_JING", root().join(".tools/jing/jing.jar")),
    )
}

pub fn ooxml_validator() -> Option<PathBuf> {
    found(
        "ooxml-validate",
        env_or(
            "OOCONV_OOXML_VALIDATE",
            root().join(".tools/ooxml-validate/ooxml-validate"),
        ),
    )
}

pub fn soffice() -> Option<PathBuf> {
    found("soffice", on_path("soffice"))
}

pub fn gridcraft() -> Option<PathBuf> {
    found("gridcraft-cli", on_path("gridcraft-cli"))
}

pub fn wordcraft() -> Option<PathBuf> {
    found("wordcraft-cli", on_path("wordcraft-cli"))
}

/// A fresh directory below the system temp directory — which is what the containerised oracle
/// (`scripts/soffice-docker/soffice`) can see.
pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ooconv-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every corpus file of these extensions, sorted.
pub fn corpus(exts: &[&str]) -> Vec<PathBuf> {
    fn walk(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, exts, out);
            } else if p
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| exts.contains(&e))
            {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(&root().join("ooconv/tests/data/corpus"), exts, &mut out);
    out.sort();
    out
}

/// `jing -i` over flat documents, and over each XML part of packages.
pub fn validate_odf(jing: &Path, files: &[PathBuf]) -> Result<(), String> {
    let mut xml = Vec::new();
    for f in files {
        let bytes = std::fs::read(f).unwrap();
        if bytes.starts_with(b"PK") {
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
            for part in ["content.xml", "styles.xml"] {
                let mut entry = zip.by_name(part).unwrap();
                let mut out = Vec::new();
                std::io::copy(&mut entry, &mut out).unwrap();
                let path = f.with_extension(format!("{part}.xml"));
                std::fs::write(&path, out).unwrap();
                xml.push(path);
            }
        } else {
            xml.push(f.clone());
        }
    }
    let schema = root().join("doc/OpenDocument-v1.4-schema.rng");
    let mut errors = String::new();
    for chunk in xml.chunks(200) {
        let out = Command::new("java")
            .arg("-jar")
            .arg(jing)
            .arg("-i")
            .arg(&schema)
            .args(chunk)
            .output()
            .expect("java runs");
        if !out.status.success() {
            errors.push_str(&String::from_utf8_lossy(&out.stdout));
            errors.push_str(&String::from_utf8_lossy(&out.stderr));
        }
    }
    match errors.is_empty() {
        true => Ok(()),
        false => Err(errors),
    }
}

/// The Open XML SDK's validator over `.xlsx` and `.docx` files.
pub fn validate_ooxml(validator: &Path, files: &[PathBuf]) -> Result<(), String> {
    let mut errors = String::new();
    for chunk in files.chunks(200) {
        let out = Command::new(validator)
            .args(chunk)
            .output()
            .expect("validator runs");
        if !out.status.success() {
            errors.push_str(&String::from_utf8_lossy(&out.stdout));
        }
    }
    match errors.is_empty() {
        true => Ok(()),
        false => Err(errors),
    }
}

/// `soffice --convert-to <filter>` over `inputs`, into `out`, with a private profile so parallel
/// test binaries do not fight over one.
pub fn soffice_convert(soffice: &Path, inputs: &[PathBuf], filter: &str, out: &Path) {
    let profile = out.join("profile");
    // Always recalculate on load, both families: LibreOffice is the calculation oracle, and a
    // cached value is the producer's claim, not a result.
    std::fs::create_dir_all(profile.join("user")).unwrap();
    std::fs::write(
        profile.join("user/registrymodifications.xcu"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<oor:items xmlns:oor="http://openoffice.org/2001/registry" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<item oor:path="/org.openoffice.Office.Calc/Formula/Load"><prop oor:name="OOXMLRecalcMode" oor:op="fuse"><value>0</value></prop></item>
<item oor:path="/org.openoffice.Office.Calc/Formula/Load"><prop oor:name="ODFRecalcMode" oor:op="fuse"><value>0</value></prop></item>
</oor:items>
"#,
    )
    .unwrap();
    for chunk in inputs.chunks(50) {
        let status = Command::new(soffice)
            .arg(format!(
                "-env:UserInstallation=file://{}",
                profile.display()
            ))
            .args(["--headless", "--convert-to", filter, "--outdir"])
            .arg(out)
            .args(chunk)
            .output()
            .expect("soffice runs");
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
    }
}

// ---- comparing models ----

/// Numbers agree to the precision the formats carry: ODF dates are written to the millisecond.
pub fn close(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= 1e-8 * a.abs().max(b.abs()).max(1.0)
}

fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => close(*x, *y),
        _ => a == b,
    }
}

fn same_cell(a: &Cell, b: &Cell) -> bool {
    same_value(&a.value, &b.value) && a.formula == b.formula && a.format == b.format
}

/// The first difference between two workbooks, described for a person.
pub fn diff_books(want: &Workbook, got: &Workbook) -> Option<String> {
    if want.sheets.len() != got.sheets.len() {
        return Some(format!(
            "{} sheets, got {}",
            want.sheets.len(),
            got.sheets.len()
        ));
    }
    if want.criteria != got.criteria {
        return Some(format!(
            "criteria {:?} came back as {:?}",
            want.criteria, got.criteria
        ));
    }
    for (a, b) in want.sheets.iter().zip(&got.sheets) {
        if a.name != b.name {
            return Some(format!("sheet {:?} came back as {:?}", a.name, b.name));
        }
        if a.hidden != b.hidden {
            return Some(format!(
                "sheet {:?}: hidden {} → {}",
                a.name, a.hidden, b.hidden
            ));
        }
        for (pos, cell) in &a.cells {
            match b.cells.get(pos) {
                Some(other) if same_cell(cell, other) => {}
                other => {
                    return Some(format!(
                        "{}!R{}C{}: {cell:?} came back as {other:?}",
                        a.name,
                        pos.0 + 1,
                        pos.1 + 1
                    ));
                }
            }
        }
        if let Some(extra) = b.cells.keys().find(|k| !a.cells.contains_key(k)) {
            return Some(format!("{}: an extra cell at {extra:?}", a.name));
        }
        let mut am = a.merges.clone();
        let mut bm = b.merges.clone();
        am.sort_by_key(|m| (m.first, m.last));
        bm.sort_by_key(|m| (m.first, m.last));
        if am != bm {
            return Some(format!("{}: merges {am:?} came back as {bm:?}", a.name));
        }
        for (c, w) in &a.col_widths {
            match b.col_widths.get(c) {
                Some(v) if (v - w).abs() < 0.1 => {}
                other => return Some(format!("{}: column {c} width {w} → {other:?}", a.name)),
            }
        }
    }
    None
}

pub fn diff_docs(want: &Document, got: &Document) -> Option<String> {
    diff_blocks(&want.blocks, &got.blocks, "body")
}

fn diff_blocks(want: &[Block], got: &[Block], at: &str) -> Option<String> {
    if want.len() != got.len() {
        return Some(format!(
            "{at}: {} blocks, got {}\nwant {want:#?}\ngot {got:#?}",
            want.len(),
            got.len()
        ));
    }
    for (i, (a, b)) in want.iter().zip(got).enumerate() {
        match (a, b) {
            (Block::Paragraph(p), Block::Paragraph(q)) => {
                if p != q {
                    return Some(format!("{at} block {i}: {p:#?}\ncame back as {q:#?}"));
                }
            }
            (Block::Table(t), Block::Table(u)) => {
                if t.rows.len() != u.rows.len() || t.columns() != u.columns() {
                    return Some(format!("{at} table {i}: shape differs"));
                }
                for (r, (ra, rb)) in t.rows.iter().zip(&u.rows).enumerate() {
                    for (c, (ca, cb)) in ra.iter().zip(rb).enumerate() {
                        if (ca.col_span, ca.row_span, ca.covered)
                            != (cb.col_span, cb.row_span, cb.covered)
                        {
                            return Some(format!(
                                "{at} table {i} R{r}C{c}: spans {ca:?} vs {cb:?}"
                            ));
                        }
                        if !ca.covered
                            && let Some(d) = diff_blocks(
                                &ca.blocks,
                                &cb.blocks,
                                &format!("{at} table {i} R{r}C{c}"),
                            )
                        {
                            return Some(d);
                        }
                    }
                }
            }
            _ => return Some(format!("{at} block {i}: {a:?} came back as {b:?}")),
        }
    }
    None
}
