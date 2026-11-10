// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! OpenDocument spreadsheet → [`Workbook`], either form. Tolerant: LibreOffice's extension
//! namespaces are skipped except where they alias ODF (`calcext:value-type`), and repeated rows
//! and columns are expanded only where they hold something.

use std::collections::HashMap;

use super::dates;
use super::formula;
use super::model::{
    Area, Cell, CellError, Criteria, Formula, MAX_COLS, MAX_ROWS, NumFmt, Sheet, Value, Workbook,
};
use crate::names::Ns;
use crate::odf::{self, Collapser};
use crate::xml::read::{Attrs, Handled, Name, Reader};
use crate::{Report, Result};

/// More cells than this converter holds in memory at once; past it a file is a repetition bomb
/// or bigger than the model is built for, and the rest of its cells are a reported loss.
///
/// ponytail: the model is a `BTreeMap` of ~130 bytes per cell, so this is ~650 MB. A columnar
/// store would raise it by an order of magnitude; nothing real in the corpus comes near.
pub const MAX_CELLS: usize = 5_000_000;

#[derive(Default)]
struct Styles {
    /// Data style name → shape, and whether the style said more than the shape.
    data: HashMap<String, (NumFmt, bool)>,
    /// Cell style name → (data style name, parent style name).
    cells: HashMap<String, (Option<String>, Option<String>)>,
    /// Column style name → width in points.
    columns: HashMap<String, f64>,
    /// Table style names whose table is hidden.
    hidden_tables: Vec<String>,
}

impl Styles {
    fn format(&self, cell_style: &str) -> (NumFmt, bool) {
        let mut name = cell_style;
        for _ in 0..16 {
            let Some((data, parent)) = self.cells.get(name) else {
                break;
            };
            if let Some(d) = data {
                return self.data.get(d).copied().unwrap_or((NumFmt::General, true));
            }
            match parent {
                Some(p) => name = p,
                None => break,
            }
        }
        (NumFmt::General, false)
    }
}

pub fn read(bytes: &[u8], report: &mut Report) -> Result<Workbook> {
    let mut styles = Styles::default();
    let mut book = Workbook::default();
    let mut cells = 0usize;
    for doc in odf::package::documents(bytes)? {
        let mut r = Reader::new(&doc);
        r.root()?;
        r.children(|r, name, _| {
            if name.ns != Ns::Office {
                return Ok(Handled::No);
            }
            match name.local.as_str() {
                "styles" | "automatic-styles" => {
                    read_styles(r, &mut styles)?;
                    Ok(Handled::Yes)
                }
                "body" => {
                    r.children(|r, name, _| {
                        if !name.in_ns(Ns::Office, "spreadsheet") {
                            return Ok(Handled::No);
                        }
                        // No settings at all is the schema's defaults: regular expressions.
                        book.criteria = Criteria::RegularExpressions;
                        r.children(|r, name, attrs| {
                            if name.in_ns(Ns::Table, "calculation-settings") {
                                let flag = |a: &str, default: bool| {
                                    attrs.get(Ns::Table, a).map_or(default, |v| v == "true")
                                };
                                book.criteria = if flag("use-wildcards", false) {
                                    Criteria::Wildcards
                                } else if flag("use-regular-expressions", true) {
                                    Criteria::RegularExpressions
                                } else {
                                    Criteria::Literal
                                };
                                return Ok(Handled::No);
                            }
                            if name.in_ns(Ns::Table, "table") {
                                let sheet = table(r, attrs, &styles, &mut cells, report)?;
                                book.sheets.push(sheet);
                                return Ok(Handled::Yes);
                            }
                            if name.in_ns(Ns::Table, "named-expressions") {
                                r.children(|_, name, attrs| {
                                    if name.ns == Ns::Table {
                                        report.lose(
                                            "defined name",
                                            attrs.get(Ns::Table, "name").unwrap_or(""),
                                        );
                                    }
                                    Ok(Handled::Yes)
                                })?;
                                return Ok(Handled::Yes);
                            }
                            if name.ns == Ns::Table {
                                let lost = match name.local.as_str() {
                                    "data-pilot-tables" => "pivot table",
                                    "content-validations" => "data validation",
                                    "database-ranges" => "database range",
                                    _ => return Ok(Handled::No),
                                };
                                report.lose(lost, "");
                            }
                            Ok(Handled::No)
                        })?;
                        Ok(Handled::Yes)
                    })?;
                    Ok(Handled::Yes)
                }
                _ => Ok(Handled::No),
            }
        })?;
    }
    Ok(book)
}

/// A length (`2.258cm`, `64pt`, `0.889in`) in points; `None` for units that are not absolute.
pub fn points(len: &str) -> Option<f64> {
    let len = len.trim();
    let split = len.find(|c: char| c.is_ascii_alphabetic())?;
    let n: f64 = len[..split].parse().ok()?;
    let factor = match &len[split..] {
        "pt" => 1.0,
        "pc" => 12.0,
        "in" => 72.0,
        "cm" => 72.0 / 2.54,
        "mm" => 72.0 / 25.4,
        "px" => 0.75,
        _ => return None,
    };
    let p = n * factor;
    (p.is_finite() && p >= 0.0).then_some(p)
}

fn read_styles(r: &mut Reader<'_>, styles: &mut Styles) -> Result<()> {
    r.children(|r, name, attrs| {
        let style_name = || attrs.get(Ns::Style, "name").unwrap_or("").to_owned();
        if name.ns == Ns::Number {
            let shape = data_style(r, &name.local)?;
            if let Some(shape) = shape {
                styles.data.insert(style_name(), shape);
            }
            return Ok(Handled::Yes);
        }
        if !name.in_ns(Ns::Style, "style") {
            return Ok(Handled::No);
        }
        let n = style_name();
        match attrs.get(Ns::Style, "family") {
            Some("table-cell") => {
                styles.cells.insert(
                    n,
                    (
                        attrs.get(Ns::Style, "data-style-name").map(str::to_owned),
                        attrs.get(Ns::Style, "parent-style-name").map(str::to_owned),
                    ),
                );
                Ok(Handled::No)
            }
            Some("table-column") => {
                r.children(|_, name, attrs| {
                    if name.in_ns(Ns::Style, "table-column-properties")
                        && let Some(p) = attrs.get(Ns::Style, "column-width").and_then(points)
                    {
                        styles.columns.insert(n.clone(), p);
                    }
                    Ok(Handled::Yes)
                })?;
                Ok(Handled::Yes)
            }
            Some("table") => {
                r.children(|_, name, attrs| {
                    if name.in_ns(Ns::Style, "table-properties")
                        && attrs.get(Ns::Table, "display") == Some("false")
                    {
                        styles.hidden_tables.push(n.clone());
                    }
                    Ok(Handled::Yes)
                })?;
                Ok(Handled::Yes)
            }
            _ => Ok(Handled::No),
        }
    })
}

/// A `number:*-style`'s shape.
fn data_style(r: &mut Reader<'_>, kind: &str) -> Result<Option<(NumFmt, bool)>> {
    let mut decimals = 0u8;
    let mut grouping = false;
    let mut scientific = false;
    let mut has_date = false;
    let mut has_time = false;
    let mut extra = false;
    let mut maps = false;
    r.children(|r, name, attrs| {
        if name.ns == Ns::Style && name.local == "map" {
            maps = true;
            return Ok(Handled::Yes);
        }
        if name.ns != Ns::Number {
            extra |= name.ns == Ns::Style;
            return Ok(Handled::No);
        }
        match name.local.as_str() {
            "number" | "scientific-number" | "fraction" => {
                decimals = attrs
                    .get(Ns::Number, "decimal-places")
                    .and_then(|d| d.parse().ok())
                    .unwrap_or(0u8)
                    .min(30);
                grouping = attrs.get(Ns::Number, "grouping") == Some("true");
                scientific = name.local == "scientific-number";
                extra |= name.local == "fraction";
            }
            "year" | "month" | "day" | "day-of-week" | "quarter" | "week-of-year" | "era" => {
                has_date = true
            }
            "hours" | "minutes" | "seconds" | "am-pm" => has_time = true,
            "text" => {
                let t = r.text()?;
                extra |= !t
                    .chars()
                    .all(|c| matches!(c, '-' | ':' | ' ' | '%' | '/' | '.'));
            }
            "currency-symbol" => extra = true,
            _ => {}
        }
        Ok(Handled::Yes)
    })?;
    let shape = match kind {
        "number-style" if scientific => NumFmt::Scientific { decimals },
        "number-style" => NumFmt::Fixed { decimals, grouping },
        "currency-style" => {
            extra = true;
            NumFmt::Fixed { decimals, grouping }
        }
        "percentage-style" => NumFmt::Percent { decimals },
        "date-style" if has_time => NumFmt::DateTime,
        "date-style" => NumFmt::Date,
        "time-style" => NumFmt::Time,
        "text-style" => NumFmt::Text,
        "boolean-style" => NumFmt::General,
        _ => return Ok(None),
    };
    let _ = has_date;
    Ok(Some((shape, extra || maps)))
}

type Placed = (u32, Cell, Option<(u32, u32)>);

struct TableCtx<'a> {
    sheet: Sheet,
    styles: &'a Styles,
    row: u32,
    /// Default cell style per column, from `table:table-column`.
    column_styles: Vec<Option<String>>,
    next_col_def: u32,
}

fn table(
    r: &mut Reader<'_>,
    attrs: &Attrs,
    styles: &Styles,
    cells: &mut usize,
    report: &mut Report,
) -> Result<Sheet> {
    let mut sheet = Sheet::new(attrs.get(Ns::Table, "name").unwrap_or("Sheet"));
    if let Some(style) = attrs.get(Ns::Table, "style-name") {
        sheet.hidden = styles.hidden_tables.iter().any(|h| h == style);
    }
    let mut t = TableCtx {
        sheet,
        styles,
        row: 0,
        column_styles: Vec::new(),
        next_col_def: 0,
    };
    rows_and_columns(r, &mut t, cells, report)?;
    Ok(t.sheet)
}

/// The children of a `table:table` or of any row or column grouping inside it.
fn rows_and_columns(
    r: &mut Reader<'_>,
    t: &mut TableCtx<'_>,
    cells: &mut usize,
    report: &mut Report,
) -> Result<()> {
    r.children(|r, name, attrs| {
        if name.ns != Ns::Table {
            if name.in_ns(Ns::Office, "forms") {
                return Ok(Handled::No);
            }
            return Ok(Handled::No);
        }
        match name.local.as_str() {
            "table-column-group"
            | "table-header-columns"
            | "table-columns"
            | "table-row-group"
            | "table-header-rows"
            | "table-rows" => {
                rows_and_columns(r, t, cells, report)?;
                Ok(Handled::Yes)
            }
            "table-column" => {
                let n: u32 = repeat(attrs, "number-columns-repeated");
                let width = attrs
                    .get(Ns::Table, "style-name")
                    .and_then(|s| t.styles.columns.get(s))
                    .copied();
                let default = attrs
                    .get(Ns::Table, "default-cell-style-name")
                    .map(str::to_owned);
                for _ in 0..n {
                    if t.next_col_def >= MAX_COLS {
                        break;
                    }
                    if let Some(w) = width {
                        t.sheet.col_widths.insert(t.next_col_def, w);
                    }
                    t.column_styles.push(default.clone());
                    t.next_col_def += 1;
                }
                Ok(Handled::Yes)
            }
            "table-row" => {
                let n = repeat(attrs, "number-rows-repeated");
                row(r, t, n, cells, report)?;
                Ok(Handled::Yes)
            }
            "shapes" => {
                report.lose("drawing (charts, pictures)", &t.sheet.name);
                Ok(Handled::No)
            }
            "named-expressions" => {
                report.lose("defined name", &t.sheet.name);
                Ok(Handled::No)
            }
            _ => Ok(Handled::No),
        }
    })
}

fn repeat(attrs: &Attrs, name: &str) -> u32 {
    attrs
        .get(Ns::Table, name)
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1)
        .max(1)
}

/// One `table:table-row`, repeated `n` times. Its cells are read once and placed on every row of
/// the run: a repeated row is identical by definition.
fn row(
    r: &mut Reader<'_>,
    t: &mut TableCtx<'_>,
    n: u32,
    cells: &mut usize,
    report: &mut Report,
) -> Result<()> {
    let first = t.row;
    // (column, cell, merge span) for each cell that holds something.
    let mut read: Vec<Placed> = Vec::new();
    let mut col = 0u32;
    let name = t.sheet.name.clone();
    r.children(|r, el, attrs| {
        let covered = el.in_ns(Ns::Table, "covered-table-cell");
        if !(el.in_ns(Ns::Table, "table-cell") || covered) {
            return Ok(Handled::No);
        }
        let repeat_n = repeat(attrs, "number-columns-repeated");
        let style = attrs
            .get(Ns::Table, "style-name")
            .map(str::to_owned)
            .or_else(|| t.column_styles.get(col as usize).cloned().flatten());
        let at = format!(
            "{name}!{}",
            formula::a1(first.min(MAX_ROWS - 1), col.min(MAX_COLS - 1))
        );
        let cell = cell(r, el, attrs, style.as_deref(), t.styles, report, &at)?;
        let span = (!covered)
            .then(|| {
                let cols = repeat(attrs, "number-columns-spanned");
                let rows = repeat(attrs, "number-rows-spanned");
                (rows > 1 || cols > 1).then_some((rows, cols))
            })
            .flatten();
        if cell != Cell::default() || span.is_some() {
            // Bounded by the grid before anything is allocated: the repeat count is a
            // stranger's number.
            let fits = repeat_n.min(MAX_COLS.saturating_sub(col));
            for k in 0..fits {
                read.push((col + k, cell.clone(), span));
            }
            if fits < repeat_n {
                report.lose("cell beyond the grid", &name);
            }
        }
        col = col.saturating_add(repeat_n);
        Ok(Handled::Yes)
    })?;
    let rows = match read.is_empty() {
        true => 0,
        false => n.min(MAX_ROWS.saturating_sub(first)),
    };
    if rows < n && !read.is_empty() {
        report.lose("cell beyond the grid", &name);
    }
    'rows: for k in 0..rows {
        let row = first + k;
        for (c, cell, span) in &read {
            *cells += 1;
            if *cells > MAX_CELLS {
                report.lose("cell beyond the size cap", &name);
                break 'rows;
            }
            if let Some((rows, cols)) = span {
                t.sheet.merges.push(Area {
                    first: (row, *c),
                    last: (
                        (row + rows - 1).min(MAX_ROWS - 1),
                        (c + cols - 1).min(MAX_COLS - 1),
                    ),
                });
            }
            t.sheet.set(row, *c, cell.clone());
        }
    }
    t.row = first.saturating_add(n);
    Ok(())
}

fn cell(
    r: &mut Reader<'_>,
    _el: &Name,
    attrs: &Attrs,
    style: Option<&str>,
    styles: &Styles,
    report: &mut Report,
    at: &str,
) -> Result<Cell> {
    let (mut format, simplified) = style.map(|s| styles.format(s)).unwrap_or_default();
    let value_type = attrs
        .get(Ns::Office, "value-type")
        .or_else(|| attrs.get(Ns::Calcext, "value-type"));
    let calc_type = attrs.get(Ns::Calcext, "value-type");

    // The paragraphs: the display text, and a string cell's value.
    let mut lines: Vec<String> = Vec::new();
    r.children(|r, name, _| {
        if name.in_ns(Ns::Text, "p") || name.in_ns(Ns::Text, "h") {
            let mut c = Collapser::default();
            odf::read_text(r, &mut c)?;
            lines.push(c.finish());
            return Ok(Handled::Yes);
        }
        if name.in_ns(Ns::Office, "annotation") {
            report.lose("comments", at);
        } else if name.ns == Ns::Draw {
            report.lose("drawing (charts, pictures)", at);
        }
        Ok(Handled::No)
    })?;
    let text = lines.join("\n");

    let number = |key: &str| {
        attrs
            .get(Ns::Office, key)
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|n| n.is_finite())
    };
    let mut value = match value_type {
        Some("float" | "percentage" | "currency") => match number("value") {
            Some(n) => Value::Number(n),
            None => Value::Empty,
        },
        Some("date") => match attrs
            .get(Ns::Office, "date-value")
            .and_then(dates::from_iso)
        {
            Some(n) => {
                if !format.is_temporal() {
                    let has_time = attrs
                        .get(Ns::Office, "date-value")
                        .is_some_and(|v| v.contains('T'));
                    format = if has_time {
                        NumFmt::DateTime
                    } else {
                        NumFmt::Date
                    };
                }
                Value::Number(n)
            }
            None => Value::Empty,
        },
        Some("time") => match attrs
            .get(Ns::Office, "time-value")
            .and_then(dates::from_duration)
        {
            Some(n) => {
                if !format.is_temporal() {
                    format = NumFmt::Time;
                }
                Value::Number(n)
            }
            None => Value::Empty,
        },
        Some("boolean") => Value::Bool(matches!(
            attrs.get(Ns::Office, "boolean-value"),
            Some("true" | "1")
        )),
        Some("string") => Value::Text(
            attrs
                .get(Ns::Office, "string-value")
                .map(str::to_owned)
                .unwrap_or(text.clone()),
        ),
        _ => Value::Empty,
    };
    if calc_type == Some("error") {
        value = Value::Error(lo_error(&text));
    }

    let formula = match attrs.get(Ns::Table, "formula") {
        None => None,
        Some(f) => match formula::odf::parse_attribute(f) {
            Ok(expr) => {
                let cols = repeat(attrs, "number-matrix-columns-spanned");
                let rows = repeat(attrs, "number-matrix-rows-spanned");
                let array = (attrs
                    .get(Ns::Table, "number-matrix-columns-spanned")
                    .is_some()
                    || attrs.get(Ns::Table, "number-matrix-rows-spanned").is_some())
                .then_some((rows, cols));
                // A cell whose formula is a bare error constant is how an error *value* is
                // written; read it back as one.
                match (&expr, &value, array) {
                    (formula::Expr::Error(e), Value::Empty | Value::Error(_), None) => {
                        value = Value::Error(*e);
                        None
                    }
                    _ => Some(Formula { expr, array }),
                }
            }
            Err(why) => {
                report.lose(why.label(), at);
                None
            }
        },
    };
    // A formula whose cached result has no type: the display text says what it was.
    if formula.is_some() && value == Value::Empty && value_type.is_none() && !text.is_empty() {
        value = match CellError::from_name(&text) {
            Some(e) => Value::Error(e),
            None => Value::Text(text.clone()),
        };
    }
    if simplified && value != Value::Empty {
        report.lose("number format simplified", at);
    }
    Ok(Cell {
        value,
        formula,
        format,
    })
}

/// A cached error as LibreOffice displays it: an OpenFormula name, or one of its own `Err:NNN`
/// codes. Only measured codes are mapped (`doc/odf-format.md` §3.1); any other is `#NAME?`, as
/// Part 4 §5.12 says of an error nobody knows.
fn lo_error(text: &str) -> CellError {
    match text.trim() {
        // `SQRT(-1)`, cached by Excel as `#NUM!`, recalculated by LibreOffice 26.8.
        "Err:502" => CellError::Num,
        t => CellError::from_name(t).unwrap_or(CellError::Name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_in_every_absolute_unit() {
        assert_eq!(points("72pt"), Some(72.0));
        assert_eq!(points("1in"), Some(72.0));
        assert!((points("2.54cm").unwrap() - 72.0).abs() < 1e-9);
        assert_eq!(points("10%"), None);
        assert_eq!(points("-1pt"), None);
    }
}
