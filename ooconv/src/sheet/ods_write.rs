// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! [`Workbook`] → OpenDocument spreadsheet, either form. Only ODF 1.4's own vocabulary, so every
//! document validates against the schema (`doc/plan.md`, R2); no `calcext:`, no `loext:`.

use std::collections::BTreeMap;

use super::dates;
use super::formula;
use super::model::{Cell, Criteria, NumFmt, Sheet, Value, Workbook};
use super::numfmt;
use crate::odf::{self, MIME_ODS, Parts};
use crate::xml::write::Writer;
use crate::{Report, Result};

#[derive(Default)]
struct Styles {
    /// Number formats used, by first use; cell style `ce{i+1}`, data style `N{i+1}`.
    formats: Vec<NumFmt>,
    /// Column widths used, in points as written; column style `co{i+1}`.
    widths: Vec<String>,
    hidden_table: bool,
}

impl Styles {
    fn cell(&mut self, f: NumFmt) -> Option<String> {
        if f == NumFmt::General {
            return None;
        }
        let i = match self.formats.iter().position(|g| *g == f) {
            Some(i) => i,
            None => {
                self.formats.push(f);
                self.formats.len() - 1
            }
        };
        Some(format!("ce{}", i + 1))
    }

    fn column(&mut self, points: f64) -> String {
        let width = format!("{:.3}pt", points.max(0.1));
        let i = match self.widths.iter().position(|w| *w == width) {
            Some(i) => i,
            None => {
                self.widths.push(width);
                self.widths.len() - 1
            }
        };
        format!("co{}", i + 1)
    }

    fn xml(&self) -> String {
        let mut w = Writer::fragment();
        for (i, width) in self.widths.iter().enumerate() {
            w.start("style:style")
                .attr("style:name", format!("co{}", i + 1))
                .attr("style:family", "table-column");
            w.start("style:table-column-properties")
                .attr("style:column-width", width)
                .end();
            w.end();
        }
        if self.hidden_table {
            w.start("style:style")
                .attr("style:name", "ta-hidden")
                .attr("style:family", "table");
            w.start("style:table-properties")
                .attr("table:display", "false")
                .end();
            w.end();
        }
        let mut out = w.finish();
        for (i, &f) in self.formats.iter().enumerate() {
            if let Some(data) = numfmt::odf_data_style(f, &format!("N{}", i + 1)) {
                out.push_str(&data);
            }
        }
        let mut w = Writer::fragment();
        for (i, _) in self.formats.iter().enumerate() {
            w.start("style:style")
                .attr("style:name", format!("ce{}", i + 1))
                .attr("style:family", "table-cell")
                .attr("style:parent-style-name", "Default")
                .attr("style:data-style-name", format!("N{}", i + 1))
                .end();
        }
        out.push_str(&w.finish());
        out
    }
}

pub fn write(book: &Workbook, flat: bool, report: &mut Report) -> Result<Vec<u8>> {
    let mut styles = Styles::default();
    let mut body = Writer::fragment();
    body.start("office:spreadsheet");
    // Always explicit: the default without it is regular expressions, not what a criterion from
    // Excel means.
    let (wild, regex) = match book.criteria {
        Criteria::Wildcards => ("true", "false"),
        Criteria::RegularExpressions => ("false", "true"),
        Criteria::Literal => ("false", "false"),
    };
    body.start("table:calculation-settings")
        .attr("table:use-regular-expressions", regex)
        .attr("table:use-wildcards", wild)
        .end();
    let fallback = [Sheet::new("Sheet1")];
    let sheets = match book.sheets.is_empty() {
        true => &fallback[..],
        false => &book.sheets[..],
    };
    let first_visible = sheets.iter().position(|s| !s.hidden).unwrap_or(0);
    for (i, sheet) in sheets.iter().enumerate() {
        let hidden = sheet.hidden && i != first_visible;
        styles.hidden_table |= hidden;
        table(&mut body, sheet, hidden, &mut styles);
    }
    body.end();
    if body.dropped > 0 {
        report.lose(
            "character XML cannot carry",
            format!("{} dropped", body.dropped),
        );
    }

    let mut common = Writer::fragment();
    common
        .start("style:style")
        .attr("style:name", "Default")
        .attr("style:family", "table-cell")
        .end();
    let parts = Parts {
        mimetype: MIME_ODS,
        styles: common.finish(),
        automatic_styles: styles.xml(),
        body: body.finish(),
        ..Parts::default()
    };
    odf::package::assemble(&parts, flat)
}

fn table(w: &mut Writer, sheet: &Sheet, hidden: bool, styles: &mut Styles) {
    w.start("table:table").attr("table:name", &sheet.name);
    if hidden {
        w.attr("table:style-name", "ta-hidden");
    }
    let (last_row, last_col) = sheet.extent().unwrap_or((0, 0));
    let last_col = last_col.max(sheet.col_widths.keys().next_back().copied().unwrap_or(0));

    // Columns: at least one (the schema requires it), runs of equal style repeated.
    let mut runs: Vec<(Option<String>, u32)> = Vec::new();
    for c in 0..=last_col {
        let style = sheet.col_widths.get(&c).map(|p| styles.column(*p));
        match runs.last_mut() {
            Some((s, n)) if *s == style => *n += 1,
            _ => runs.push((style, 1)),
        }
    }
    for (style, n) in runs {
        w.start("table:table-column");
        w.attr_opt("table:style-name", style);
        if n > 1 {
            w.attr("table:number-columns-repeated", n.to_string());
        }
        w.end();
    }

    // Which positions a merge covers, and where each starts.
    let mut covered = std::collections::HashSet::new();
    let mut spans = BTreeMap::new();
    for m in &sheet.merges {
        spans.insert(
            m.first,
            (m.last.0 - m.first.0 + 1, m.last.1 - m.first.1 + 1),
        );
        for r in m.first.0..=m.last.0 {
            for c in m.first.1..=m.last.1 {
                if (r, c) != m.first {
                    covered.insert((r, c));
                }
            }
        }
    }

    let mut by_row: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for &(r, c) in sheet.cells.keys() {
        by_row.entry(r).or_default().push(c);
    }
    for &(r, c) in spans.keys().chain(covered.iter()) {
        by_row.entry(r).or_default().push(c);
    }

    let mut next_row = 0;
    for (r, mut cols) in by_row {
        cols.sort_unstable();
        cols.dedup();
        if r > next_row {
            empty_rows(w, r - next_row);
        }
        w.start("table:table-row");
        let mut next_col = 0;
        for c in cols {
            if c > next_col {
                w.start("table:table-cell");
                if c - next_col > 1 {
                    w.attr("table:number-columns-repeated", (c - next_col).to_string());
                }
                w.end();
            }
            let element = match covered.contains(&(r, c)) {
                true => "table:covered-table-cell",
                false => "table:table-cell",
            };
            let default = Cell::default();
            let cell = sheet.cells.get(&(r, c)).unwrap_or(&default);
            write_cell(w, element, cell, spans.get(&(r, c)).copied(), styles);
            next_col = c + 1;
        }
        w.end();
        next_row = r + 1;
    }
    if next_row <= last_row || next_row == 0 {
        empty_rows(w, (last_row + 1).saturating_sub(next_row).max(1));
    }
    w.end();
}

fn empty_rows(w: &mut Writer, n: u32) {
    w.start("table:table-row");
    if n > 1 {
        w.attr("table:number-rows-repeated", n.to_string());
    }
    w.start("table:table-cell").end();
    w.end();
}

fn write_cell(
    w: &mut Writer,
    element: &str,
    cell: &Cell,
    span: Option<(u32, u32)>,
    styles: &mut Styles,
) {
    w.start(element);
    w.attr_opt("table:style-name", styles.cell(cell.format));
    if let Some((rows, cols)) = span {
        w.attr("table:number-columns-spanned", cols.to_string())
            .attr("table:number-rows-spanned", rows.to_string());
    }
    // An error with no formula has no ODF value type; the formula `=#N/A` says it exactly.
    let constant_error = match (&cell.value, &cell.formula) {
        (Value::Error(e), None) => Some(formula::Expr::Error(*e)),
        _ => None,
    };
    if let Some(f) = &cell.formula {
        w.attr("table:formula", formula::odf::print_attribute(&f.expr));
        if let Some((rows, cols)) = f.array {
            w.attr("table:number-matrix-columns-spanned", cols.to_string())
                .attr("table:number-matrix-rows-spanned", rows.to_string());
        }
    } else if let Some(e) = &constant_error {
        w.attr("table:formula", formula::odf::print_attribute(e));
    }
    let mut text: Option<String> = None;
    match &cell.value {
        Value::Empty => {}
        Value::Number(n) => match cell.format {
            NumFmt::Date | NumFmt::DateTime => {
                w.attr("office:value-type", "date")
                    .attr("office:date-value", dates::to_iso(*n));
            }
            NumFmt::Time => {
                w.attr("office:value-type", "time")
                    .attr("office:time-value", dates::to_duration(*n));
            }
            NumFmt::Percent { .. } => {
                w.attr("office:value-type", "percentage")
                    .attr("office:value", formula::number(*n));
            }
            _ => {
                w.attr("office:value-type", "float")
                    .attr("office:value", formula::number(*n));
            }
        },
        Value::Text(s) => {
            w.attr("office:value-type", "string");
            if cell.formula.is_some() {
                w.attr("office:string-value", s);
            }
            text = Some(s.clone());
        }
        Value::Bool(b) => {
            w.attr("office:value-type", "boolean")
                .attr("office:boolean-value", if *b { "true" } else { "false" });
            text = Some(if *b { "TRUE" } else { "FALSE" }.into());
        }
        // A cached error has no value type in ODF 1.4; the display text carries it.
        Value::Error(e) => text = Some(e.as_str().into()),
    }
    if let Some(text) = text {
        // A cell's lines are paragraphs (Part 3 §9.1.4).
        for line in text.split('\n') {
            w.start("text:p");
            odf::write_text(w, line);
            w.end();
        }
    }
    w.end();
}
