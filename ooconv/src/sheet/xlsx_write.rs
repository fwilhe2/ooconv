// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! [`Workbook`] → SpreadsheetML, Transitional. The parts are the fewest Excel and the Open XML
//! SDK's validator accept (`doc/xlsx-format.md` §1.1): workbook, styles, shared strings, one part
//! per sheet. Element order follows the schema's sequences, which Excel enforces.
//!
//! Excel's limits are the writer's to keep, since the model has ODF's (none): a sheet name is made
//! legal, text over 32,767 characters is cut, a formula over 8,192 characters is dropped to its
//! value — each reported.

use std::collections::{BTreeMap, HashMap};

use super::dates;
use super::formula::{self, Expr};
use super::model::{Cell, Criteria, MAX_COLS, MAX_ROWS, NumFmt, Value, Workbook};
use super::numfmt;
use crate::names::MAIN_T;
use crate::opc::write::{CORE_PROPERTIES_REL, PackageWriter, REL_BASE};
use crate::xml::write::Writer;
use crate::{Report, Result};

const CT_WORKBOOK: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";
const CT_WORKSHEET: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
const CT_STYLES: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml";
const CT_STRINGS: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml";
const CT_CORE: &str = "application/vnd.openxmlformats-package.core-properties+xml";

/// Excel's cap on a cell's text (Excel specifications and limits, `SPEC`).
pub const MAX_TEXT: usize = 32_767;
/// Excel's cap on a formula's length in characters.
pub const MAX_FORMULA: usize = 8_192;
/// Excel's cap on a sheet name.
pub const MAX_SHEET_NAME: usize = 31;

pub fn write(book: &Workbook, report: &mut Report) -> Result<Vec<u8>> {
    // A workbook needs at least one sheet in both formats.
    let fallback;
    let book = match book.sheets.is_empty() {
        true => {
            fallback = Workbook {
                sheets: vec![super::model::Sheet::new("Sheet1")],
                ..Workbook::default()
            };
            &fallback
        }
        false => book,
    };
    let names = sheet_names(book, report);
    let renames: HashMap<&str, &str> = book
        .sheets
        .iter()
        .zip(&names)
        .filter(|(s, n)| s.name != **n)
        .map(|(s, n)| (s.name.as_str(), n.as_str()))
        .collect();

    if book.criteria != Criteria::Wildcards
        && let Some(at) = criteria_user(book)
    {
        report.lose(
            match book.criteria {
                Criteria::RegularExpressions => "regular-expression criteria",
                _ => "literal criteria",
            },
            at,
        );
    }

    let mut pkg = PackageWriter::new();
    let mut strings = Strings::default();
    let mut styles = Styles::default();
    let mut dropped = 0;

    let mut sheet_parts = Vec::new();
    for (i, sheet) in book.sheets.iter().enumerate() {
        let mut w = Writer::new();
        w.start("worksheet")
            .attr("xmlns", MAIN_T)
            .attr("xmlns:r", REL_BASE);
        if let Some((r, c)) = sheet.extent() {
            w.start("dimension")
                .attr(
                    "ref",
                    format!(
                        "A1:{}",
                        formula::a1(r.min(MAX_ROWS - 1), c.min(MAX_COLS - 1))
                    ),
                )
                .end();
        }
        if i == 0 {
            w.start("sheetViews");
            w.start("sheetView")
                .attr("tabSelected", "1")
                .attr("workbookViewId", "0")
                .end();
            w.end();
        }
        let widths: Vec<(u32, f64)> = sheet
            .col_widths
            .iter()
            .filter(|(c, _)| **c < MAX_COLS)
            .map(|(c, w)| (*c, *w))
            .collect();
        if !widths.is_empty() {
            w.start("cols");
            for (c, points) in widths {
                let chars = ((points / 0.75 - 5.0) / 7.0).max(0.0);
                w.start("col")
                    .attr("min", (c + 1).to_string())
                    .attr("max", (c + 1).to_string())
                    .attr("width", format!("{:.4}", chars))
                    .attr("customWidth", "1")
                    .end();
            }
            w.end();
        }
        w.start("sheetData");
        let mut rows: BTreeMap<u32, Vec<(u32, &Cell)>> = BTreeMap::new();
        for (&(r, c), cell) in &sheet.cells {
            if r >= MAX_ROWS || c >= MAX_COLS {
                report.lose(
                    "cell beyond Excel's grid",
                    format!("{}!R{}C{}", sheet.name, r + 1, c + 1),
                );
                continue;
            }
            rows.entry(r).or_default().push((c, cell));
        }
        for (r, cells) in rows {
            w.start("row").attr("r", (r + 1).to_string());
            for (c, cell) in cells {
                let at = format!("{}!{}", sheet.name, formula::a1(r, c));
                write_cell(
                    &mut w,
                    r,
                    c,
                    cell,
                    &renames,
                    &mut strings,
                    &mut styles,
                    report,
                    &at,
                );
            }
            w.end();
        }
        w.end(); // sheetData
        let merges: Vec<_> = sheet
            .merges
            .iter()
            .filter(|m| m.last.0 < MAX_ROWS && m.last.1 < MAX_COLS)
            .collect();
        if !merges.is_empty() {
            w.start("mergeCells")
                .attr("count", merges.len().to_string());
            for m in merges {
                w.start("mergeCell")
                    .attr(
                        "ref",
                        format!(
                            "{}:{}",
                            formula::a1(m.first.0, m.first.1),
                            formula::a1(m.last.0, m.last.1)
                        ),
                    )
                    .end();
            }
            w.end();
        }
        w.end(); // worksheet
        dropped += w.dropped;
        let part = format!("xl/worksheets/sheet{}.xml", i + 1);
        pkg.part(&part, CT_WORKSHEET, w.finish());
        sheet_parts.push(part);
    }

    // The workbook, now that the sheets' parts exist to relate to.
    let mut w = Writer::new();
    w.start("workbook")
        .attr("xmlns", MAIN_T)
        .attr("xmlns:r", REL_BASE);
    w.start("workbookPr").end();
    let first_visible = book.sheets.iter().position(|s| !s.hidden).unwrap_or(0);
    w.start("bookViews");
    w.start("workbookView")
        .attr("activeTab", first_visible.to_string())
        .end();
    w.end();
    w.start("sheets");
    for (i, (sheet, part)) in book.sheets.iter().zip(&sheet_parts).enumerate() {
        let id = pkg.relate("xl/workbook.xml", &format!("{REL_BASE}/worksheet"), part);
        w.start("sheet")
            .attr("name", &names[i])
            .attr("sheetId", (i + 1).to_string())
            // Excel refuses a workbook whose every sheet is hidden.
            .attr_opt(
                "state",
                (sheet.hidden && i != first_visible).then_some("hidden"),
            )
            .attr("r:id", id)
            .end();
    }
    w.end();
    // No `calcPr`: the cached values are the source application's results, and they are what an
    // oracle (gridcraft `check`) compares the translated formulas against. `fullCalcOnLoad`
    // would mark them stale.
    w.end();
    dropped += w.dropped;
    pkg.part("xl/workbook.xml", CT_WORKBOOK, w.finish());
    pkg.relate("", &format!("{REL_BASE}/officeDocument"), "xl/workbook.xml");

    let styles_xml = styles.xml();
    pkg.part("xl/styles.xml", CT_STYLES, styles_xml);
    pkg.relate(
        "xl/workbook.xml",
        &format!("{REL_BASE}/styles"),
        "xl/styles.xml",
    );

    let (strings_xml, d) = strings.xml();
    dropped += d;
    pkg.part("xl/sharedStrings.xml", CT_STRINGS, strings_xml);
    pkg.relate(
        "xl/workbook.xml",
        &format!("{REL_BASE}/sharedStrings"),
        "xl/sharedStrings.xml",
    );

    pkg.part(
        "docProps/core.xml",
        CT_CORE,
        crate::opc::write::core_properties(),
    );
    pkg.relate("", CORE_PROPERTIES_REL, "docProps/core.xml");

    if dropped > 0 {
        report.lose("character XML cannot carry", format!("{dropped} dropped"));
    }
    pkg.finish()
}

/// The functions whose text criteria `table:calculation-settings` governs (Part 4 §4.11.8).
const CRITERIA_FUNCTIONS: &[&str] = &[
    "COUNTIF",
    "COUNTIFS",
    "SUMIF",
    "SUMIFS",
    "AVERAGEIF",
    "AVERAGEIFS",
    "COM.MICROSOFT.MAXIFS",
    "COM.MICROSOFT.MINIFS",
    "MATCH",
    "HLOOKUP",
    "VLOOKUP",
    "LOOKUP",
    "SEARCH",
    "DAVERAGE",
    "DCOUNT",
    "DCOUNTA",
    "DGET",
    "DMAX",
    "DMIN",
    "DPRODUCT",
    "DSTDEV",
    "DSTDEVP",
    "DSUM",
    "DVAR",
    "DVARP",
];

/// The first cell whose formula calls a criteria function — where Excel's wildcards would read
/// a criterion differently from the document's own mode.
fn criteria_user(book: &Workbook) -> Option<String> {
    fn calls(e: &Expr) -> bool {
        match e {
            Expr::Call { name, args } => {
                CRITERIA_FUNCTIONS.contains(&name.as_str()) || args.iter().any(calls)
            }
            Expr::Prefix(_, x) | Expr::Postfix(_, x) | Expr::Paren(x) => calls(x),
            Expr::Binary(_, a, b) => calls(a) || calls(b),
            _ => false,
        }
    }
    book.sheets.iter().find_map(|s| {
        s.cells.iter().find_map(|((r, c), cell)| {
            cell.formula
                .as_ref()
                .filter(|f| calls(&f.expr))
                .map(|_| format!("{}!{}", s.name, formula::a1(*r, *c)))
        })
    })
}

/// Every sheet's name, made legal for Excel and unique case-insensitively.
fn sheet_names(book: &Workbook, report: &mut Report) -> Vec<String> {
    let mut taken: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for (i, sheet) in book.sheets.iter().enumerate() {
        let mut name: String = sheet
            .name
            .chars()
            .map(|c| match c {
                '[' | ']' | ':' | '*' | '?' | '/' | '\\' => '_',
                c if c.is_control() => '_',
                c => c,
            })
            .collect();
        name = name.trim_matches('\'').to_owned();
        if name.chars().count() > MAX_SHEET_NAME {
            name = name.chars().take(MAX_SHEET_NAME).collect();
        }
        if name.is_empty() || name.eq_ignore_ascii_case("history") {
            name = format!("Sheet{}", i + 1);
        }
        let base = name.clone();
        let mut n = 2;
        while taken
            .iter()
            .any(|t| t.to_lowercase() == name.to_lowercase())
        {
            let suffix = format!(" ({n})");
            let keep = MAX_SHEET_NAME - suffix.chars().count();
            name = format!("{}{suffix}", base.chars().take(keep).collect::<String>());
            n += 1;
        }
        if name != sheet.name {
            report.lose("sheet renamed for Excel", &sheet.name);
        }
        taken.push(name.clone());
        out.push(name);
    }
    out
}

fn rename(expr: &Expr, renames: &HashMap<&str, &str>) -> Expr {
    if renames.is_empty() {
        return expr.clone();
    }
    let sheet = |s: &Option<String>| {
        s.as_ref().map(|n| {
            renames
                .get(n.as_str())
                .map_or(n.clone(), |r| (*r).to_owned())
        })
    };
    match expr {
        Expr::Ref(r) => {
            let mut r = r.clone();
            r.start.sheet = sheet(&r.start.sheet);
            if let Some(e) = &mut r.end {
                e.sheet = sheet(&e.sheet);
            }
            Expr::Ref(r)
        }
        Expr::Call { name, args } => Expr::Call {
            name: name.clone(),
            args: args.iter().map(|a| rename(a, renames)).collect(),
        },
        Expr::Prefix(op, e) => Expr::Prefix(*op, Box::new(rename(e, renames))),
        Expr::Postfix(op, e) => Expr::Postfix(*op, Box::new(rename(e, renames))),
        Expr::Binary(op, a, b) => Expr::Binary(
            *op,
            Box::new(rename(a, renames)),
            Box::new(rename(b, renames)),
        ),
        Expr::Paren(e) => Expr::Paren(Box::new(rename(e, renames))),
        other => other.clone(),
    }
}

/// Text as `ST_Xstring`: a character XML cannot carry becomes `_xHHHH_`, and a literal that
/// *looks* like an escape has its underscore escaped, so the reader's [`unescape`] gives back
/// what was written.
///
/// [`unescape`]: super::xlsx_read::unescape
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_'
            && chars.get(i + 1) == Some(&'x')
            && chars.len() >= i + 7
            && chars[i + 2..i + 6].iter().all(|c| c.is_ascii_hexdigit())
            && chars[i + 6] == '_'
        {
            out.push_str("_x005F_");
        } else if (c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
            || c == '\u{FFFE}'
            || c == '\u{FFFF}'
        {
            out.push_str(&format!("_x{:04X}_", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn write_cell(
    w: &mut Writer,
    r: u32,
    c: u32,
    cell: &Cell,
    renames: &HashMap<&str, &str>,
    strings: &mut Strings,
    styles: &mut Styles,
    report: &mut Report,
    at: &str,
) {
    let style = styles.index(cell.format);
    let f = cell.formula.as_ref().and_then(|f| {
        let printed = formula::excel::print(&rename(&f.expr, renames));
        match printed {
            Ok(text) if text.chars().count() <= MAX_FORMULA => Some((text, f.array)),
            Ok(_) => {
                report.lose("formula longer than Excel allows", at);
                None
            }
            Err(why) => {
                report.lose(why.label(), at);
                None
            }
        }
    });
    w.start("c").attr("r", formula::a1(r, c));
    if style != 0 {
        w.attr("s", style.to_string());
    }
    let number = |n: f64| match cell.format {
        NumFmt::Date | NumFmt::DateTime => dates::to_excel_1900(n),
        _ => n,
    };
    let (t, v): (Option<&str>, Option<String>) = match &cell.value {
        Value::Empty => (None, None),
        Value::Number(n) => {
            if cell.format.is_temporal() && *n < 1.0 && cell.format != NumFmt::Time {
                report.lose("date before 1900", at);
            }
            (None, Some(formula::number(number(*n))))
        }
        Value::Bool(b) => (Some("b"), Some(if *b { "1" } else { "0" }.into())),
        Value::Error(e) => (Some("e"), Some(e.as_str().into())),
        Value::Text(s) => {
            let s = match s.chars().count() > MAX_TEXT {
                true => {
                    report.lose("text longer than Excel allows", at);
                    s.chars().take(MAX_TEXT).collect()
                }
                false => s.clone(),
            };
            match f.is_some() {
                // A formula's text result is inline (`t="str"`), never a shared string.
                true => (Some("str"), Some(escape(&s))),
                false => (Some("s"), Some(strings.index(&s).to_string())),
            }
        }
    };
    w.attr_opt("t", t);
    if let Some((text, array)) = &f {
        w.start("f");
        if let Some((rows, cols)) = array {
            let last = (r + rows.saturating_sub(1), c + cols.saturating_sub(1));
            w.attr("t", "array").attr(
                "ref",
                format!("{}:{}", formula::a1(r, c), formula::a1(last.0, last.1)),
            );
        }
        w.text(text).end();
    }
    if let Some(v) = v {
        w.leaf("v", &v);
    }
    w.end();
}

#[derive(Default)]
struct Strings {
    list: Vec<String>,
    index: HashMap<String, usize>,
    count: usize,
}

impl Strings {
    fn index(&mut self, s: &str) -> usize {
        self.count += 1;
        if let Some(&i) = self.index.get(s) {
            return i;
        }
        self.list.push(s.to_owned());
        self.index.insert(s.to_owned(), self.list.len() - 1);
        self.list.len() - 1
    }

    fn xml(&self) -> (String, usize) {
        let mut w = Writer::new();
        w.start("sst")
            .attr("xmlns", MAIN_T)
            .attr("count", self.count.to_string())
            .attr("uniqueCount", self.list.len().to_string());
        for s in &self.list {
            w.start("si").start("t");
            let escaped = escape(s);
            if escaped.starts_with(char::is_whitespace) || escaped.ends_with(char::is_whitespace) {
                w.attr("xml:space", "preserve");
            }
            w.text(&escaped).end().end();
        }
        w.end();
        let d = w.dropped;
        (w.finish(), d)
    }
}

/// One `cellXfs` entry per number format used; index 0 is General.
#[derive(Default)]
struct Styles {
    formats: Vec<NumFmt>,
}

impl Styles {
    fn index(&mut self, f: NumFmt) -> usize {
        if f == NumFmt::General {
            return 0;
        }
        match self.formats.iter().position(|g| *g == f) {
            Some(i) => i + 1,
            None => {
                self.formats.push(f);
                self.formats.len()
            }
        }
    }

    fn xml(&self) -> String {
        let mut ids = Vec::new();
        let mut custom: Vec<(u32, String)> = Vec::new();
        for &f in &self.formats {
            match numfmt::builtin_id(f) {
                Some(id) => ids.push(id),
                None => {
                    let id = 164 + custom.len() as u32;
                    custom.push((id, numfmt::excel_code(f)));
                    ids.push(id);
                }
            }
        }
        let mut w = Writer::new();
        w.start("styleSheet").attr("xmlns", MAIN_T);
        if !custom.is_empty() {
            w.start("numFmts").attr("count", custom.len().to_string());
            for (id, code) in &custom {
                w.start("numFmt")
                    .attr("numFmtId", id.to_string())
                    .attr("formatCode", code)
                    .end();
            }
            w.end();
        }
        w.start("fonts").attr("count", "1");
        w.start("font");
        w.start("sz").attr("val", "11").end();
        w.start("name").attr("val", "Calibri").end();
        w.start("family").attr("val", "2").end();
        w.end().end();
        w.start("fills").attr("count", "2");
        for pattern in ["none", "gray125"] {
            w.start("fill")
                .start("patternFill")
                .attr("patternType", pattern)
                .end()
                .end();
        }
        w.end();
        w.start("borders").attr("count", "1");
        w.start("border");
        for side in ["left", "right", "top", "bottom", "diagonal"] {
            w.start(side).end();
        }
        w.end().end();
        w.start("cellStyleXfs").attr("count", "1");
        xf(&mut w, 0, false);
        w.end();
        w.start("cellXfs")
            .attr("count", (ids.len() + 1).to_string());
        xf(&mut w, 0, true);
        for id in ids {
            xf(&mut w, id, true);
        }
        w.end();
        w.start("cellStyles").attr("count", "1");
        w.start("cellStyle")
            .attr("name", "Normal")
            .attr("xfId", "0")
            .attr("builtinId", "0")
            .end();
        w.end();
        w.end();
        w.finish()
    }
}

fn xf(w: &mut Writer, id: u32, cell: bool) {
    w.start("xf")
        .attr("numFmtId", id.to_string())
        .attr("fontId", "0")
        .attr("fillId", "0")
        .attr("borderId", "0");
    if cell {
        w.attr("xfId", "0");
        if id != 0 {
            w.attr("applyNumberFormat", "1");
        }
    }
    w.end();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sheet::xlsx_read::unescape;

    #[test]
    fn escaping_and_unescaping_are_inverse() {
        for s in ["plain", "_x0041_ literal", "tab\tnl\n", "_x", "a_x00_b"] {
            assert_eq!(unescape(&escape(s)), s, "{s:?}");
        }
        // A control character is escaped so the part stays well-formed, and dropped on reading,
        // as the oracle does (grind `doc/xlsx-format.md` §2.6).
        assert_eq!(escape("bell\u{7}"), "bell_x0007_");
        assert_eq!(unescape(&escape("bell\u{7}")), "bell");
    }

    #[test]
    fn sheet_names_become_legal_and_stay_unique() {
        let mut book = Workbook::default();
        for n in ["a/b", "A_B", "x".repeat(40).as_str(), "", "History", "'q'"] {
            book.sheets.push(super::super::model::Sheet::new(n));
        }
        let mut report = Report::default();
        let names = sheet_names(&book, &mut report);
        assert_eq!(names[0], "a_b");
        assert_eq!(names[1], "A_B (2)");
        assert_eq!(names[2].chars().count(), 31);
        assert_eq!(names[3], "Sheet4");
        assert_eq!(names[4], "Sheet5");
        assert_eq!(names[5], "q");
        assert_eq!(report.count("sheet renamed for Excel"), 6);
    }
}
