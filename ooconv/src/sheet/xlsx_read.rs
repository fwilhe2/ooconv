// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! SpreadsheetML → [`Workbook`]. Parts are found by relationship, never by path; every element
//! not recognised is skipped with its subtree; every *recognised* thing the model cannot hold is
//! a [`Report`] loss. Facts are grind's (grind `doc/xlsx-format.md`) unless cited otherwise.

use std::collections::HashMap;

use super::dates;
use super::formula::{self, Expr, Refusal};
use super::model::{Area, Cell, CellError, Formula, NumFmt, Sheet, Value, Workbook};
use super::numfmt;
use crate::names::{Ns, RelType, Seen};
use crate::opc::package::Package;
use crate::xml::read::{Attrs, Handled, Name, Reader};
use crate::{Error, Report, Result};

fn is(name: &Name, local: &str) -> bool {
    name.in_ns(Ns::Spreadsheet, local)
}

/// One `<xf>` of `cellXfs`, as far as the model cares.
#[derive(Clone, Copy, Default)]
struct Xf {
    format: NumFmt,
    /// The code said more than the shape (a currency, a colour, sections).
    simplified: bool,
    /// A font, fill or border other than the defaults.
    styled: bool,
}

struct Ctx<'r> {
    report: &'r mut Report,
    strings: Vec<String>,
    xfs: Vec<Xf>,
    date1904: bool,
}

pub fn read(bytes: &[u8], report: &mut Report) -> Result<Workbook> {
    let mut pkg = Package::open(bytes)?;
    let mut seen = Seen::default();
    let main = pkg
        .rel_of("", RelType::OfficeDocument, &mut seen)
        .ok_or(Error::UnknownFormat)?;
    let wb_xml = pkg.part(&main.target).ok_or(Error::UnknownFormat)?;
    let rels = pkg.rels(&main.target, &mut seen);

    let mut ctx = Ctx {
        report,
        strings: Vec::new(),
        xfs: Vec::new(),
        date1904: false,
    };
    if let Some(rel) = rels
        .iter()
        .find(|r| r.kind == RelType::SharedStrings && !r.external)
        && let Some(xml) = pkg.part(&rel.target)
    {
        ctx.strings = shared_strings(&xml, ctx.report)?;
    }
    if let Some(rel) = rels
        .iter()
        .find(|r| r.kind == RelType::Styles && !r.external)
        && let Some(xml) = pkg.part(&rel.target)
    {
        ctx.xfs = styles(&xml)?;
    }

    // `<sheets>` in workbook order: (name, relationship id, hidden).
    let mut entries: Vec<(String, String, bool)> = Vec::new();
    let mut reader = Reader::new(&wb_xml);
    reader.root()?;
    reader.children(|r, name, attrs| {
        if is(name, "workbookPr") {
            ctx.date1904 = attrs.flag("date1904");
            return Ok(Handled::Yes);
        }
        if is(name, "definedNames") {
            r.children(|_, name, attrs| {
                if is(name, "definedName") {
                    let n = attrs.plain("name").unwrap_or("");
                    // Print areas and filters are layout, not content.
                    if !n.starts_with("_xlnm.") {
                        ctx.report.lose("defined name", n);
                    }
                }
                Ok(Handled::Yes)
            })?;
            return Ok(Handled::Yes);
        }
        if !is(name, "sheets") {
            return Ok(Handled::No);
        }
        r.children(|_, name, attrs| {
            if is(name, "sheet") {
                entries.push((
                    attrs.plain("name").unwrap_or("Sheet").to_owned(),
                    attrs.get(Ns::Relationships, "id").unwrap_or("").to_owned(),
                    matches!(attrs.plain("state"), Some("hidden" | "veryHidden")),
                ));
            }
            Ok(Handled::Yes)
        })?;
        Ok(Handled::Yes)
    })?;
    seen.transitional |= reader.seen.transitional;
    seen.strict |= reader.seen.strict;

    let mut book = Workbook::default();
    for (name, rid, hidden) in entries {
        let Some(rel) = rels.iter().find(|r| r.id == rid && !r.external) else {
            ctx.report.lose("sheet without a part", &name);
            continue;
        };
        if rel.kind == RelType::ChartSheet {
            ctx.report.lose("chart sheet", &name);
            continue;
        }
        if rel.kind != RelType::Worksheet {
            ctx.report.lose("sheet of an unknown kind", &name);
            continue;
        }
        let Some(xml) = pkg.part(&rel.target) else {
            ctx.report.lose("sheet without a part", &name);
            continue;
        };
        for side in pkg.rels(&rel.target, &mut seen) {
            match side.kind {
                RelType::Drawing => ctx.report.lose("drawing (charts, pictures)", &name),
                RelType::Comments => ctx.report.lose("comments", &name),
                RelType::PivotTable => ctx.report.lose("pivot table", &name),
                RelType::Table => ctx.report.lose("table definition", &name),
                _ => {}
            }
        }
        let mut sheet = Sheet::new(name);
        sheet.hidden = hidden;
        worksheet(&xml, &mut sheet, &mut ctx)?;
        book.sheets.push(sheet);
    }
    ctx.report.flavour = Some(seen.flavour());
    Ok(book)
}

/// `ST_Xstring`'s `_xHHHH_` escapes decoded — grind's `xlsx/src/strings.rs::unescape`: a UTF-16
/// code unit each, pairs joined, control characters but tab, LF and CR dropped, CR LF one line
/// break, anything malformed left as written.
pub fn unescape(text: &str) -> String {
    if !text.contains("_x") {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let unit = |at: usize| -> Option<u16> {
        let candidate = bytes.get(at..at + 7)?;
        if !(candidate.starts_with(b"_x") && candidate[6] == b'_') {
            return None;
        }
        let hex = std::str::from_utf8(&candidate[2..6]).ok()?;
        hex.bytes()
            .all(|b| b.is_ascii_hexdigit())
            .then(|| u16::from_str_radix(hex, 16).ok())
            .flatten()
    };
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        if let Some(first) = unit(at) {
            let (decoded, used) = match (first, unit(at + 7)) {
                (0xD800..=0xDBFF, Some(second @ 0xDC00..=0xDFFF)) => (
                    char::decode_utf16([first, second])
                        .next()
                        .and_then(|r| r.ok()),
                    14,
                ),
                _ => (char::from_u32(u32::from(first)), 7),
            };
            if let Some(c) = decoded
                && (!c.is_control() || matches!(c, '\t' | '\n' | '\r'))
                && !(c == '\r' && bytes.get(at + used) == Some(&b'\n'))
            {
                out.push(c);
            }
            at += used;
            continue;
        }
        let c = text[at..].chars().next().expect("in bounds");
        out.push(c);
        at += c.len_utf8();
    }
    out
}

/// The text of an `<si>` or `<is>`: its `<t>`, or its runs' `<t>`s concatenated — never phonetic
/// guides (`<rPh>`), which are annotation rather than content.
fn string_item(r: &mut Reader<'_>, report: &mut Report) -> Result<String> {
    let mut text = String::new();
    let mut runs = 0;
    r.children(|r, name, _| {
        if is(name, "t") {
            text.push_str(&r.text()?);
            return Ok(Handled::Yes);
        }
        if is(name, "r") {
            runs += 1;
            r.children(|r, name, _| {
                if is(name, "t") {
                    text.push_str(&r.text()?);
                    return Ok(Handled::Yes);
                }
                Ok(Handled::No)
            })?;
            return Ok(Handled::Yes);
        }
        Ok(Handled::No)
    })?;
    if runs > 1 {
        report.lose(
            "rich text formatting",
            text.chars().take(20).collect::<String>(),
        );
    }
    Ok(unescape(&text))
}

fn shared_strings(xml: &[u8], report: &mut Report) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut reader = Reader::new(xml);
    reader.root()?;
    reader.children(|r, name, _| {
        if !is(name, "si") {
            return Ok(Handled::No);
        }
        out.push(string_item(r, report)?);
        Ok(Handled::Yes)
    })?;
    Ok(out)
}

fn styles(xml: &[u8]) -> Result<Vec<Xf>> {
    let mut codes: HashMap<u32, String> = HashMap::new();
    let mut xfs = Vec::new();
    let mut reader = Reader::new(xml);
    reader.root()?;
    reader.children(|r, name, _| {
        if is(name, "numFmts") {
            r.children(|_, name, attrs| {
                if is(name, "numFmt")
                    && let (Some(id), Some(code)) = (
                        attrs.plain("numFmtId").and_then(|v| v.parse().ok()),
                        attrs.plain("formatCode"),
                    )
                {
                    codes.insert(id, code.to_owned());
                }
                Ok(Handled::Yes)
            })?;
            return Ok(Handled::Yes);
        }
        if is(name, "cellXfs") {
            r.children(|_, name, attrs| {
                if !is(name, "xf") {
                    return Ok(Handled::No);
                }
                let id: u32 = attrs
                    .plain("numFmtId")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let (format, simplified) = match codes.get(&id) {
                    Some(code) => numfmt::classify_excel(code),
                    None => match numfmt::from_builtin(id) {
                        Some(f) => (f, false),
                        None => (NumFmt::General, true),
                    },
                };
                let nonzero = |a: &str| attrs.plain(a).is_some_and(|v| v != "0");
                xfs.push(Xf {
                    format,
                    simplified,
                    styled: nonzero("fontId") || nonzero("fillId") || nonzero("borderId"),
                });
                Ok(Handled::Yes)
            })?;
            return Ok(Handled::Yes);
        }
        Ok(Handled::No)
    })?;
    Ok(xfs)
}

/// `B12` → `(11, 1)`.
pub fn parse_a1(s: &str) -> Option<(u32, u32)> {
    let split = s.find(|c: char| c.is_ascii_digit())?;
    let col = formula::column_index(&s[..split])?;
    let row = formula::row_index(&s[split..])?;
    Some((row, col))
}

fn parse_area(s: &str) -> Option<Area> {
    let (a, b) = s.split_once(':').unwrap_or((s, s));
    Some(Area {
        first: parse_a1(a)?,
        last: parse_a1(b)?,
    })
}

struct Shared {
    expr: Expr,
    origin: (u32, u32),
}

fn worksheet(xml: &[u8], sheet: &mut Sheet, ctx: &mut Ctx<'_>) -> Result<()> {
    let mut shared: HashMap<String, std::result::Result<Shared, Refusal>> = HashMap::new();
    let mut reader = Reader::new(xml);
    reader.root()?;
    let name = sheet.name.clone();
    reader.children(|r, el, _| {
        if is(el, "sheetData") {
            let mut next_row = 0u32;
            r.children(|r, el, attrs| {
                if !is(el, "row") {
                    return Ok(Handled::No);
                }
                let row = attrs
                    .plain("r")
                    .and_then(|v| v.parse::<u32>().ok())
                    .and_then(|v| v.checked_sub(1))
                    .unwrap_or(next_row);
                next_row = row + 1;
                let mut next_col = 0u32;
                r.children(|r, el, attrs| {
                    if !is(el, "c") {
                        return Ok(Handled::No);
                    }
                    let (row, col) = attrs
                        .plain("r")
                        .and_then(parse_a1)
                        .unwrap_or((row, next_col));
                    next_col = col + 1;
                    let cell = cell(r, attrs, (row, col), &mut shared, &name, ctx)?;
                    sheet.set(row, col, cell);
                    Ok(Handled::Yes)
                })?;
                Ok(Handled::Yes)
            })?;
            return Ok(Handled::Yes);
        }
        if is(el, "mergeCells") {
            r.children(|_, el, attrs| {
                if is(el, "mergeCell")
                    && let Some(area) = attrs.plain("ref").and_then(parse_area)
                    && area.first != area.last
                {
                    sheet.merges.push(area);
                }
                Ok(Handled::Yes)
            })?;
            return Ok(Handled::Yes);
        }
        if is(el, "cols") {
            r.children(|_, el, attrs| {
                if is(el, "col")
                    && attrs.flag("customWidth")
                    && let (Some(min), Some(max), Some(width)) = (
                        attrs.plain("min").and_then(|v| v.parse::<u32>().ok()),
                        attrs.plain("max").and_then(|v| v.parse::<u32>().ok()),
                        attrs.plain("width").and_then(|v| v.parse::<f64>().ok()),
                    )
                    && min >= 1
                    && max >= min
                    && width.is_finite()
                    && width > 0.0
                {
                    // ponytail: a character width is converted at Calibri 11's 7 px per
                    // character (§18.3.1.13's formula), which is what most files assume.
                    let points = (width * 7.0 + 5.0) * 0.75;
                    for c in (min - 1)..max.min(super::model::MAX_COLS) {
                        sheet.col_widths.insert(c, points);
                    }
                }
                Ok(Handled::Yes)
            })?;
            return Ok(Handled::Yes);
        }
        let lost = match el.local.as_str() {
            "conditionalFormatting" => "conditional formatting",
            "dataValidations" => "data validation",
            "hyperlinks" => "hyperlink",
            "autoFilter" => "auto filter",
            "sheetProtection" => "sheet protection",
            _ => return Ok(Handled::No),
        };
        if el.ns == Ns::Spreadsheet {
            ctx.report.lose(lost, &name);
        }
        Ok(Handled::No)
    })?;
    Ok(())
}

fn cell(
    r: &mut Reader<'_>,
    attrs: &Attrs,
    at: (u32, u32),
    shared: &mut HashMap<String, std::result::Result<Shared, Refusal>>,
    sheet: &str,
    ctx: &mut Ctx<'_>,
) -> Result<Cell> {
    let where_ = || format!("{sheet}!{}", formula::a1(at.0, at.1));
    let kind = attrs.plain("t").unwrap_or("n").to_owned();
    let xf = attrs
        .plain("s")
        .and_then(|v| v.parse::<usize>().ok())
        .and_then(|i| ctx.xfs.get(i).copied())
        .unwrap_or_default();
    let mut raw: Option<String> = None;
    let mut inline: Option<String> = None;
    let mut f: Option<(String, Attrs)> = None;
    r.children(|r, el, a| {
        if is(el, "v") {
            raw = Some(r.text()?);
            return Ok(Handled::Yes);
        }
        if is(el, "is") {
            inline = Some(string_item(r, ctx.report)?);
            return Ok(Handled::Yes);
        }
        if is(el, "f") {
            let attrs = a.clone();
            f = Some((r.text()?, attrs));
            return Ok(Handled::Yes);
        }
        Ok(Handled::No)
    })?;

    let value = match (kind.as_str(), raw.as_deref()) {
        ("s", Some(v)) => match v
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|i| ctx.strings.get(i))
        {
            Some(s) => Value::Text(s.clone()),
            None => {
                ctx.report.lose("dangling shared string", where_());
                Value::Empty
            }
        },
        ("inlineStr", _) => Value::Text(inline.unwrap_or_default()),
        ("str", Some(v)) => Value::Text(unescape(v)),
        ("b", Some(v)) => Value::Bool(matches!(v.trim(), "1" | "true")),
        ("e", Some(v)) => Value::Error(CellError::from_name(v).unwrap_or(CellError::Name)),
        ("d", Some(v)) => match dates::from_iso(v) {
            Some(serial) => Value::Number(serial),
            None => Value::Text(v.to_owned()),
        },
        (_, Some(v)) => match v.trim().parse::<f64>() {
            Ok(n) if n.is_finite() => Value::Number(match xf.format {
                NumFmt::Date | NumFmt::DateTime if kind != "d" => match ctx.date1904 {
                    true => n + dates::DAYS_1904,
                    false => dates::from_excel_1900(n),
                },
                _ => n,
            }),
            _ if v.trim().is_empty() => Value::Empty,
            _ => {
                ctx.report.lose("unreadable number", where_());
                Value::Empty
            }
        },
        (_, None) => Value::Empty,
    };
    if xf.simplified {
        ctx.report.lose("number format simplified", where_());
    }
    if xf.styled {
        ctx.report
            .lose("cell styling (font, fill, border)", where_());
    }

    let formula = match f {
        None => None,
        Some((text, fa)) => formula_of(&text, &fa, at, shared, ctx, &where_()),
    };
    Ok(Cell {
        value,
        formula,
        format: xf.format,
    })
}

fn formula_of(
    text: &str,
    fa: &Attrs,
    at: (u32, u32),
    shared: &mut HashMap<String, std::result::Result<Shared, Refusal>>,
    ctx: &mut Ctx<'_>,
    where_: &str,
) -> Option<Formula> {
    let refuse = |ctx: &mut Ctx<'_>, why: Refusal| {
        ctx.report.lose(why.label(), where_);
        None
    };
    match fa.plain("t").unwrap_or("normal") {
        "dataTable" => {
            ctx.report.lose("what-if data table", where_);
            None
        }
        "array" => {
            let area = fa.plain("ref").and_then(parse_area);
            match formula::excel::parse(text) {
                Ok(expr) => Some(Formula {
                    expr,
                    array: Some(match area {
                        Some(a) => (a.last.0 - a.first.0 + 1, a.last.1 - a.first.1 + 1),
                        None => (1, 1),
                    }),
                }),
                Err(why) => refuse(ctx, why),
            }
        }
        "shared" => {
            let si = fa.plain("si").unwrap_or("").to_owned();
            if !text.trim().is_empty() {
                // The master: its text is the formula, and dependents shift from here.
                let parsed = formula::excel::parse(text).map(|expr| Shared { expr, origin: at });
                let out = match &parsed {
                    Ok(s) => Some(Formula {
                        expr: s.expr.clone(),
                        array: None,
                    }),
                    Err(why) => refuse(ctx, *why),
                };
                shared.insert(si, parsed);
                return out;
            }
            match shared.get(&si) {
                Some(Ok(s)) => Some(Formula {
                    expr: s.expr.shifted(
                        i64::from(at.0) - i64::from(s.origin.0),
                        i64::from(at.1) - i64::from(s.origin.1),
                    ),
                    array: None,
                }),
                Some(Err(why)) => {
                    let why = *why;
                    refuse(ctx, why)
                }
                None => refuse(ctx, Refusal::Syntax),
            }
        }
        _ if text.trim().is_empty() => None,
        _ => match formula::excel::parse(text) {
            Ok(expr) => Some(Formula { expr, array: None }),
            Err(why) => refuse(ctx, why),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_decode_the_way_the_oracle_reads_them() {
        for (written, read) in [
            ("a_x0041_b", "aAb"),
            ("s_xD83D__xDE00_s", "s😀s"),
            ("_x005F_x0041_", "_x0041_"),
            ("x_x000D_\ny", "x\ny"),
            ("nul_x0000_end", "nulend"),
            ("short_x41_", "short_x41_"),
            ("lone_xD83D_", "lone"),
        ] {
            assert_eq!(unescape(written), read, "{written}");
        }
    }

    #[test]
    fn addresses() {
        assert_eq!(parse_a1("A1"), Some((0, 0)));
        assert_eq!(parse_a1("XFD1048576"), Some((1_048_575, 16_383)));
        assert_eq!(parse_a1("A0"), None);
        assert_eq!(parse_a1("1A"), None);
    }
}
