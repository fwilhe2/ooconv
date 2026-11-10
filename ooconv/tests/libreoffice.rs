// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Loop L** — LibreOffice reads what we write as we meant it. Every document we write (the
//! fixtures, and every corpus document in every format of its kind) is opened by `soffice` and
//! saved as flat ODF; our reader reads that, and it must agree with the model we wrote, up to
//! the named [`Divergence`]s and nothing else.
//!
//! LibreOffice's ODF output is not schema-valid (it adds `loext:`), and that is fine: our reader
//! is tolerant, and the claim is about meaning, not markup.
//!
//! cargo test -p ooconv --test libreoffice

mod common;

use std::path::PathBuf;

use ooconv::sheet::formula::Expr;
use ooconv::sheet::{NumFmt, Value, Workbook};
use ooconv::text::{Block, Document, Paragraph};
use ooconv::{Format, Kind, Report};

/// A difference this loop accepts, by what it is: `(name, why)`. Matched by the comparison
/// functions below, never by file name.
const DIVERGENCES: &[(&str, &str)] = &[
    (
        "boolean constant as TRUE()",
        "LibreOffice has no boolean constant: an xlsx `t=\"b\"` cell becomes the formula \
         `TRUE()` or `FALSE()` with that value.",
    ),
    (
        "format inferred for a formula",
        "LibreOffice gives a formula's result the format its operands suggest (`B2%` is a \
         percentage) where the source had none.",
    ),
    (
        "uncached formula calculated",
        "A formula the source stored without a result is calculated on load; there was no value \
         to compare.",
    ),
    (
        "empty string cell",
        "LibreOffice has no empty-string cell; a cell holding \"\" is read as empty.",
    ),
    (
        "1900 leap-year bug",
        "LibreOffice reads an xlsx date serial below 61 against its own epoch, a day before what \
         Excel shows (MEASURED 2026-11-10: serial 1 → 1899-12-31). The converter keeps Excel's \
         meaning.",
    ),
    (
        "bullet list identity",
        "Two bullet lists in a row may come back as one: a bullet carries no counter, so which \
         list an item belongs to has no visible effect.",
    ),
    (
        "undefined name",
        "A formula naming something the model does not define (defined names are a reported \
         loss) is #NAME? to LibreOffice, which also lower-cases the name.",
    ),
];

fn mentions_name(e: &ooconv::sheet::formula::Expr) -> bool {
    use ooconv::sheet::formula::Expr;
    match e {
        Expr::Name(_) => true,
        Expr::Call { args, .. } => args.iter().any(mentions_name),
        Expr::Prefix(_, x) | Expr::Postfix(_, x) | Expr::Paren(x) => mentions_name(x),
        Expr::Binary(_, a, b) => mentions_name(a) || mentions_name(b),
        _ => false,
    }
}

/// Documents that LibreOffice reads with no difference beyond the divergences. **Raise it,
/// never lower it.**
const FLOOR: usize = 214;

struct Case {
    name: String,
    format: Format,
    /// The model we wrote.
    book: Option<Workbook>,
    doc: Option<Document>,
    path: PathBuf,
}

fn cases(dir: &std::path::Path) -> Vec<Case> {
    let mut out = Vec::new();
    let mut add = |name: String, format: Format, book: Option<Workbook>, doc: Option<Document>| {
        let path = dir.join(format!("{name}.{}", format.extension()));
        let mut r = Report::default();
        let bytes = match (&book, &doc) {
            (Some(b), _) => ooconv::sheet::write(b, format, &mut r).unwrap(),
            (_, Some(d)) => ooconv::text::write(d, format, &mut r).unwrap(),
            _ => unreachable!(),
        };
        std::fs::write(&path, bytes).unwrap();
        out.push(Case {
            name,
            format,
            book,
            doc,
            path,
        });
    };
    for format in [Format::Xlsx, Format::Ods] {
        add(
            "fixture".into(),
            format,
            Some(common::fixtures::workbook()),
            None,
        );
    }
    for format in [Format::Docx, Format::Odt] {
        add(
            "fixture".into(),
            format,
            None,
            Some(common::fixtures::document()),
        );
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
        let stem = format!("{i:03}-{}", path.file_stem().unwrap().to_string_lossy());
        let mut r = Report::default();
        match from.kind() {
            Kind::Spreadsheet => {
                let Ok(book) = ooconv::sheet::read(&bytes, from, &mut r) else {
                    continue;
                };
                for to in [Format::Xlsx, Format::Ods] {
                    add(stem.clone(), to, Some(book.clone()), None);
                }
            }
            Kind::Text => {
                let Ok(doc) = ooconv::text::read(&bytes, from, &mut r) else {
                    continue;
                };
                for to in [Format::Docx, Format::Odt] {
                    add(stem.clone(), to, None, Some(doc.clone()));
                }
            }
        }
    }
    out
}

/// A formula's result as LibreOffice calculated it, by case, sheet and position: what the
/// cross-format check compares.
type Calculated = Vec<((String, usize, (u32, u32)), Format, Value)>;

/// Cells LibreOffice must agree on: constants and formulas exactly (numbers to format
/// precision), formats up to the named divergences. A formula's *value* is LibreOffice's own
/// calculation, so it is held to the source's cache only when `trust_cache` (the fixture, whose
/// caches are true); otherwise it is collected for the cross-format check.
fn compare_books(
    want: &Workbook,
    got: &Workbook,
    seen: &mut Vec<&'static str>,
    case: (&str, Format, bool),
    calculated: &mut Calculated,
) -> Vec<String> {
    let (name, format, trust_cache) = case;
    let mut out = Vec::new();
    if want.sheets.len() != got.sheets.len() {
        out.push(format!(
            "{} sheets, got {}",
            want.sheets.len(),
            got.sheets.len()
        ));
        return out;
    }
    for (si, (a, b)) in want.sheets.iter().zip(&got.sheets).enumerate() {
        for (pos, cell) in &a.cells {
            let at = format!("{}!R{}C{}", a.name, pos.0 + 1, pos.1 + 1);
            let default = Default::default();
            let other = b.cells.get(pos).unwrap_or(&default);
            let undefined = cell
                .formula
                .as_ref()
                .is_some_and(|f| mentions_name(&f.expr));
            let uncached = cell.formula.is_some() && cell.value == Value::Empty;
            if cell.formula.is_some() && !undefined {
                calculated.push(((name.to_owned(), si, *pos), format, other.value.clone()));
            }
            let values_agree = match (&cell.value, &other.value) {
                _ if undefined => {
                    seen.push("undefined name");
                    true
                }
                _ if uncached => {
                    seen.push("uncached formula calculated");
                    true
                }
                _ if cell.formula.is_some() && !trust_cache => true,
                (Value::Text(t), Value::Empty) if t.is_empty() => {
                    seen.push("empty string cell");
                    true
                }
                (Value::Number(x), Value::Number(y))
                    if format == Format::Xlsx
                        && matches!(cell.format, NumFmt::Date | NumFmt::DateTime)
                        && *x < 61.0
                        && common::close(*x - 1.0, *y) =>
                {
                    seen.push("1900 leap-year bug");
                    true
                }
                (Value::Number(x), Value::Number(y)) => common::close(*x, *y),
                (x, y) => x == y,
            };
            if !values_agree {
                out.push(format!(
                    "{at}: value {:?} read as {:?}",
                    cell.value, other.value
                ));
            }
            let formulas_agree = match (&cell.formula, &other.formula, &cell.value) {
                (None, Some(f), Value::Bool(b)) if f.expr == Expr::Bool(*b) => {
                    seen.push("boolean constant as TRUE()");
                    true
                }
                (Some(a), Some(b), _) if undefined => {
                    format!("{:?}", a).to_lowercase() == format!("{:?}", b).to_lowercase()
                }
                (a, b, _) => a == b,
            };
            if !formulas_agree {
                out.push(format!(
                    "{at}: formula {:?} read as {:?}",
                    cell.formula, other.formula
                ));
            }
            if cell.format != other.format {
                if cell.format == NumFmt::General && cell.formula.is_some() {
                    // Counted below; any other format change is a disagreement.
                    seen.push("format inferred for a formula");
                } else {
                    out.push(format!(
                        "{at}: format {:?} read as {:?}",
                        cell.format, other.format
                    ));
                }
            }
        }
        let mut am = a.merges.clone();
        let mut bm = b.merges.clone();
        am.sort_by_key(|m| (m.first, m.last));
        bm.sort_by_key(|m| (m.first, m.last));
        if am != bm {
            out.push(format!("{}: merges {am:?} read as {bm:?}", a.name));
        }
        if a.hidden != b.hidden {
            out.push(format!(
                "{}: hidden {} read as {}",
                a.name, a.hidden, b.hidden
            ));
        }
    }
    out
}

/// Paragraphs as LibreOffice must see them: kind, alignment, and per character its formatting
/// and link — so a run split differently is not a difference.
fn flatten(blocks: &[Block], out: &mut Vec<String>) {
    for b in blocks {
        match b {
            Block::Paragraph(p) => out.push(describe(p)),
            Block::Table(t) => {
                out.push(format!("table {}x{}", t.rows.len(), t.columns()));
                for (r, row) in t.rows.iter().enumerate() {
                    for (c, cell) in row.iter().enumerate() {
                        if cell.covered {
                            continue;
                        }
                        out.push(format!(
                            "cell {r},{c} span {}x{}",
                            cell.row_span, cell.col_span
                        ));
                        flatten(&cell.blocks, out);
                    }
                }
            }
        }
    }
}

/// A paragraph as one line: kind and alignment, then each character with a compact code for its
/// formatting (`b`old, `i`talic, `u`nderline, `s`trike, `^`/`_` position, `@` linked) after it.
fn describe(p: &Paragraph) -> String {
    let mut s = format!("{:?} {:?} |", p.kind, p.align);
    for run in &p.runs {
        let st = run.style;
        let mut code = String::new();
        for (on, c) in [
            (st.bold, 'b'),
            (st.italic, 'i'),
            (st.underline, 'u'),
            (st.strike, 's'),
        ] {
            if on {
                code.push(c);
            }
        }
        match st.position {
            ooconv::text::Position::Superscript => code.push('^'),
            ooconv::text::Position::Subscript => code.push('_'),
            ooconv::text::Position::Normal => {}
        }
        if let Some(l) = &run.link {
            code.push_str(&format!("@{l}"));
        }
        for ch in run.text.chars() {
            s.push(ch);
            if !code.is_empty() {
                s.push_str(&format!("[{code}]"));
            }
        }
    }
    s
}

fn compare_docs(want: &Document, got: &Document, seen: &mut Vec<&'static str>) -> Vec<String> {
    let mut a = Vec::new();
    let mut b = Vec::new();
    flatten(&want.blocks, &mut a);
    flatten(&got.blocks, &mut b);
    // A bullet list's id is not visible; an ordered list's is (it decides the counter).
    let bullet = regex_free_bullet;
    for line in a.iter_mut().chain(b.iter_mut()) {
        if let Some(fixed) = bullet(line) {
            if fixed != *line {
                seen.push("bullet list identity");
            }
            *line = fixed;
        }
    }
    let mut out = Vec::new();
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        if x != y {
            let at = x.chars().zip(y.chars()).take_while(|(a, b)| a == b).count();
            let from = at.saturating_sub(40);
            let tail = |s: &str| s.chars().skip(from).take(120).collect::<String>();
            out.push(format!(
                "block {i}, from char {from}:\n  want {}\n  got  {}",
                short(&tail(x)),
                short(&tail(y))
            ));
            break;
        }
    }
    if a.len() != b.len() {
        out.push(format!("{} blocks, got {}", a.len(), b.len()));
    }
    out
}

/// `ListItem { list: 3, ordered: false, …` → `ListItem { list: _, ordered: false, …`.
fn regex_free_bullet(line: &str) -> Option<String> {
    let rest = line.strip_prefix("ListItem { list: ")?;
    let (_, after) = rest.split_once(',')?;
    after
        .starts_with(" ordered: false")
        .then(|| format!("ListItem {{ list: _,{after}"))
}

fn short(s: &str) -> String {
    s.chars().take(300).collect()
}

#[test]
fn libreoffice_reads_what_we_write_as_we_meant_it() {
    let Some(soffice) = common::soffice() else {
        return;
    };
    let dir = common::scratch("libreoffice");
    let written = dir.join("written");
    let converted = dir.join("converted");
    std::fs::create_dir_all(&written).unwrap();
    std::fs::create_dir_all(&converted).unwrap();
    let cases = cases(&written);

    for (filter, ext) in [("fods", "fods"), ("fodt", "fodt")] {
        let inputs: Vec<PathBuf> = cases
            .iter()
            .filter(|c| (c.format.kind() == Kind::Spreadsheet) == (ext == "fods"))
            .map(|c| c.path.clone())
            .collect();
        // One output directory per input format, since `a.xlsx` and `a.ods` both become `a.fods`.
        for source in ["xlsx", "ods", "docx", "odt"] {
            let batch: Vec<PathBuf> = inputs
                .iter()
                .filter(|p| p.extension().is_some_and(|e| e == source))
                .cloned()
                .collect();
            if batch.is_empty() {
                continue;
            }
            let out = converted.join(source);
            std::fs::create_dir_all(&out).unwrap();
            common::soffice_convert(&soffice, &batch, filter, &out);
        }
    }

    let mut agreed = 0;
    let mut failures = Vec::new();
    let mut seen = Vec::new();
    let mut calculated: Calculated = Vec::new();
    for case in &cases {
        let source = case.format.extension();
        let ext = if case.format.kind() == Kind::Spreadsheet {
            "fods"
        } else {
            "fodt"
        };
        let path = converted.join(source).join(format!("{}.{ext}", case.name));
        let Ok(bytes) = std::fs::read(&path) else {
            failures.push(format!(
                "{}.{source}: LibreOffice could not open it",
                case.name
            ));
            continue;
        };
        let mut r = Report::default();
        let problems = match (&case.book, &case.doc) {
            (Some(book), _) => match ooconv::sheet::read(&bytes, Format::Fods, &mut r) {
                Ok(got) => compare_books(
                    book,
                    &got,
                    &mut seen,
                    (&case.name, case.format, case.name == "fixture"),
                    &mut calculated,
                ),
                Err(e) => vec![format!("our reader: {e}")],
            },
            (_, Some(doc)) => match ooconv::text::read(&bytes, Format::Fodt, &mut r) {
                Ok(got) => compare_docs(doc, &got, &mut seen),
                Err(e) => vec![format!("our reader: {e}")],
            },
            _ => unreachable!(),
        };
        match problems.is_empty() {
            true => agreed += 1,
            false => failures.push(format!(
                "{}.{source}:\n  {}",
                case.name,
                problems
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n  ")
            )),
        }
    }
    // Cross-format: LibreOffice's result for each formula from our xlsx and from our ods agree,
    // so the two translations of it mean the same.
    calculated.sort_by(|a, b| a.0.cmp(&b.0));
    for pair in calculated.chunk_by(|a, b| a.0 == b.0) {
        if let [(key, f1, v1), (_, f2, v2)] = pair {
            let agree = match (v1, v2) {
                (Value::Number(x), Value::Number(y)) => common::close(*x, *y),
                _ => v1 == v2,
            };
            if !agree {
                failures.push(format!(
                    "{} sheet {} R{}C{}: LibreOffice calculates {v1:?} from our {f1:?} and {v2:?} \
                     from our {f2:?}",
                    key.0,
                    key.1,
                    key.2.0 + 1,
                    key.2.1 + 1
                ));
            }
        }
    }
    seen.sort();
    seen.dedup();
    // Both ways: every divergence seen is named, and every one named is still seen — a
    // divergence that stopped happening is a table entry to delete.
    for name in &seen {
        assert!(
            DIVERGENCES.iter().any(|(n, _)| n == name),
            "unnamed divergence {name}"
        );
    }
    for (name, _) in DIVERGENCES {
        assert!(
            seen.contains(name),
            "divergence {name:?} no longer occurs; delete it"
        );
    }
    eprintln!(
        "{agreed} of {} documents read as written; divergences seen: {seen:?}",
        cases.len()
    );
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(agreed >= FLOOR, "{agreed} below the floor {FLOOR}");
}
