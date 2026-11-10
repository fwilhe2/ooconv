// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Loop W** — a Word-compatible word processor, `wordcraft-cli`, reads every `.docx` we write
//! and must see the document we meant: each block's kind (heading level, list level, plain),
//! alignment and text, every character's formatting and link, and every table's grid and cell
//! text. Nothing about it is tolerant: there are no named divergences yet, so any difference is
//! a bug on one side or the other, to be understood before it is accepted.
//!
//! cargo test -p ooconv --test wordcraft

mod common;

use std::path::PathBuf;
use std::process::Command;

use ooconv::text::{Align, Block, Document, ParaKind, Paragraph, Position, Table};
use ooconv::{Format, Kind, Report};

/// What a character looks like, in the terms both sides can state.
#[derive(Clone, Debug, Default, PartialEq)]
struct Look {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    position: &'static str,
    link: Option<String>,
}

fn ours(p: &Paragraph) -> Vec<Look> {
    let mut out = Vec::new();
    for run in &p.runs {
        let look = Look {
            bold: run.style.bold,
            italic: run.style.italic,
            underline: run.style.underline,
            strike: run.style.strike,
            position: match run.style.position {
                Position::Normal => "",
                Position::Superscript => "superscript",
                Position::Subscript => "subscript",
            },
            link: run.link.clone(),
        };
        // wordcraft counts UTF-16 code units.
        for c in run.text.chars() {
            for _ in 0..c.len_utf16() {
                out.push(look.clone());
            }
        }
    }
    out
}

fn theirs(block: &serde_json::Value, len: usize) -> Vec<Look> {
    let mut out = vec![Look::default(); len];
    for run in block["runs"].as_array().into_iter().flatten() {
        let (start, end) = (
            run["start"].as_u64().unwrap_or(0) as usize,
            run["end"].as_u64().unwrap_or(0) as usize,
        );
        let p = &run["props"];
        for look in out.iter_mut().take(end.min(len)).skip(start) {
            look.bold = p["bold"] == true;
            look.italic = p["italic"] == true;
            look.underline = p["underline"].as_str().is_some_and(|u| u != "none");
            look.strike = p["strike"] == true;
            look.position = match p["vertAlign"].as_str() {
                Some("superscript") => "superscript",
                Some("subscript") => "subscript",
                _ => "",
            };
            look.link = p["link"].as_str().map(str::to_owned);
        }
    }
    out
}

fn compare_paragraph(p: &Paragraph, b: &serde_json::Value, at: &str, out: &mut Vec<String>) {
    if b["type"] != "paragraph" {
        out.push(format!("{at}: a paragraph read as {}", b["type"]));
        return;
    }
    let text = b["text"].as_str().unwrap_or("");
    if text != p.text() {
        out.push(format!("{at}: text {:?} read as {text:?}", p.text()));
        return;
    }
    let style = b["style"].as_str().unwrap_or("");
    match p.kind {
        ParaKind::Heading(n) if style != format!("Heading{n}") => {
            out.push(format!("{at}: heading {n} read with style {style}"))
        }
        ParaKind::ListItem { level, .. } if b["list"]["level"] != level => {
            out.push(format!("{at}: list level {level} read as {}", b["list"]))
        }
        ParaKind::Normal | ParaKind::Heading(_) if !b["list"].is_null() => {
            out.push(format!("{at}: not a list item, read as {}", b["list"]))
        }
        ParaKind::Normal if style.starts_with("Heading") => {
            out.push(format!("{at}: plain paragraph read with style {style}"))
        }
        _ => {}
    }
    let align = match p.align {
        None => None,
        Some(Align::Center) => Some("center"),
        Some(Align::End) => Some("right"),
        Some(Align::Justify) => Some("justify"),
    };
    let theirs_align = b["props"]["align"].as_str().filter(|a| *a != "left");
    if align != theirs_align {
        out.push(format!(
            "{at}: alignment {align:?} read as {theirs_align:?}"
        ));
    }
    let want = ours(p);
    let got = theirs(b, want.len());
    if let Some(i) = want.iter().zip(&got).position(|(a, b)| a != b) {
        out.push(format!(
            "{at}: character {i} of {text:?} is {:?}, read as {:?}",
            want[i], got[i]
        ));
    }
}

/// The cell texts wordcraft lists per row: a horizontally covered position is not listed, a
/// vertically covered one is an empty string; a cell's text is its paragraphs', one per line.
fn expected_cells(t: &Table) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for (r, row) in t.rows.iter().enumerate() {
        let mut out = Vec::new();
        let mut c = 0;
        while c < row.len() {
            let cell = &row[c];
            if !cell.covered {
                let text: Vec<String> = cell
                    .blocks
                    .iter()
                    .filter_map(|b| match b {
                        Block::Paragraph(p) => Some(p.text()),
                        Block::Table(_) => None,
                    })
                    .collect();
                out.push(text.join("\n"));
                c += cell.col_span as usize;
                continue;
            }
            // Covered: from above (listed, empty, as wide as its master) or from the left.
            let master = (0..r).rev().find_map(|mr| {
                let m = &t.rows[mr][c];
                (!m.covered && mr + m.row_span as usize > r).then_some(m.col_span)
            });
            match master {
                Some(span) => {
                    out.push(String::new());
                    c += span as usize;
                }
                None => c += 1,
            }
        }
        rows.push(out);
    }
    rows
}

fn compare(doc: &Document, inspect: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut blocks = inspect["blocks"].as_array().cloned().unwrap_or_default();
    // The format's filler after a final table (`Document::drop_table_filler`).
    let n = blocks.len();
    if n >= 2 && blocks[n - 2]["type"] == "table" && blocks[n - 1]["text"] == "" {
        blocks.pop();
    }
    if blocks.len() != doc.blocks.len() {
        out.push(format!(
            "{} blocks, read {}",
            doc.blocks.len(),
            blocks.len()
        ));
    }
    for (i, (ours, theirs)) in doc.blocks.iter().zip(&blocks).enumerate() {
        match ours {
            Block::Paragraph(p) => compare_paragraph(p, theirs, &format!("block {i}"), &mut out),
            Block::Table(t) => {
                if theirs["type"] != "table" {
                    out.push(format!("block {i}: a table read as {}", theirs["type"]));
                    continue;
                }
                let (rows, cols) = (theirs["rows"].as_u64(), theirs["cols"].as_u64());
                if rows != Some(t.rows.len() as u64) || cols != Some(t.columns() as u64) {
                    out.push(format!(
                        "block {i}: a {}x{} table read as {rows:?}x{cols:?}",
                        t.rows.len(),
                        t.columns()
                    ));
                    continue;
                }
                let want = expected_cells(t);
                let got: Vec<Vec<String>> =
                    serde_json::from_value(theirs["cells"].clone()).unwrap_or_default();
                if want != got {
                    out.push(format!("block {i}: cells {want:?} read as {got:?}"));
                }
            }
        }
        if out.len() > 4 {
            break;
        }
    }
    out
}

#[test]
fn wordcraft_reads_what_we_write_as_we_meant_it() {
    let Some(wordcraft) = common::wordcraft() else {
        return;
    };
    let dir = common::scratch("wordcraft");
    let mut cases: Vec<(PathBuf, Document)> = Vec::new();
    let mut add = |name: String, doc: Document| {
        let path = dir.join(format!("{name}.docx"));
        let bytes = ooconv::text::write(&doc, Format::Docx, &mut Report::default()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        cases.push((path, doc));
    };
    add("fixture".into(), common::fixtures::document());
    for (i, src) in common::corpus(&["docx", "odt", "fodt"]).iter().enumerate() {
        let bytes = std::fs::read(src).unwrap();
        let Ok(from) = ooconv::sniff(&bytes) else {
            continue;
        };
        if from.kind() != Kind::Text {
            continue;
        }
        let Ok(doc) = ooconv::text::read(&bytes, from, &mut Report::default()) else {
            continue;
        };
        add(
            format!("{i:03}-{}", src.file_stem().unwrap().to_string_lossy()),
            doc,
        );
    }

    let mut failures = Vec::new();
    for (path, doc) in &cases {
        let out = Command::new(&wordcraft)
            .arg("inspect")
            .arg(path)
            .output()
            .expect("wordcraft-cli runs");
        let name = path.file_name().unwrap().to_string_lossy();
        if !out.status.success() {
            failures.push(format!(
                "{name}: wordcraft could not open it: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
            continue;
        }
        let inspect: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let problems = compare(doc, &inspect);
        if !problems.is_empty() {
            failures.push(format!("{name}:\n  {}", problems.join("\n  ")));
        }
    }
    eprintln!("wordcraft read {} documents", cases.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
