// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! [`Document`] → OpenDocument text, either form, in ODF 1.4's own vocabulary only. Headings are
//! `text:h` with an outline level and LibreOffice's `Heading_20_N` styles; list items are nested
//! `text:list`s under one of two list styles; character formatting is automatic text styles.

use super::model::{Align, Block, Document, ParaKind, Paragraph, Position, RunStyle, Table};
use crate::odf::{self, MIME_ODT, Parts};
use crate::xml::write::Writer;
use crate::{Report, Result};

#[derive(Default)]
struct Styles {
    text: Vec<RunStyle>,
    /// (parent style, alignment) → `P{n}`.
    paragraphs: Vec<(String, Align)>,
    table_cell: bool,
}

impl Styles {
    fn text(&mut self, s: RunStyle) -> Option<String> {
        if s == RunStyle::default() {
            return None;
        }
        let i = self.text.iter().position(|t| *t == s).unwrap_or_else(|| {
            self.text.push(s);
            self.text.len() - 1
        });
        Some(format!("T{}", i + 1))
    }

    fn paragraph(&mut self, parent: &str, align: Option<Align>) -> String {
        let Some(align) = align else {
            return parent.to_owned();
        };
        let key = (parent.to_owned(), align);
        let i = self
            .paragraphs
            .iter()
            .position(|p| *p == key)
            .unwrap_or_else(|| {
                self.paragraphs.push(key);
                self.paragraphs.len() - 1
            });
        format!("P{}", i + 1)
    }

    fn automatic(&self) -> String {
        let mut w = Writer::fragment();
        for (i, (parent, align)) in self.paragraphs.iter().enumerate() {
            w.start("style:style")
                .attr("style:name", format!("P{}", i + 1))
                .attr("style:family", "paragraph")
                .attr("style:parent-style-name", parent);
            w.start("style:paragraph-properties")
                .attr(
                    "fo:text-align",
                    match align {
                        Align::Center => "center",
                        Align::End => "end",
                        Align::Justify => "justify",
                    },
                )
                .end();
            w.end();
        }
        for (i, s) in self.text.iter().enumerate() {
            w.start("style:style")
                .attr("style:name", format!("T{}", i + 1))
                .attr("style:family", "text");
            w.start("style:text-properties");
            if s.bold {
                w.attr("fo:font-weight", "bold");
            }
            if s.italic {
                w.attr("fo:font-style", "italic");
            }
            if s.underline {
                w.attr("style:text-underline-style", "solid")
                    .attr("style:text-underline-width", "auto")
                    .attr("style:text-underline-color", "font-color");
            }
            if s.strike {
                w.attr("style:text-line-through-style", "solid")
                    .attr("style:text-line-through-type", "single");
            }
            match s.position {
                Position::Superscript => {
                    w.attr("style:text-position", "super 58%");
                }
                Position::Subscript => {
                    w.attr("style:text-position", "sub 58%");
                }
                Position::Normal => {}
            }
            w.end().end();
        }
        if self.table_cell {
            w.start("style:style")
                .attr("style:name", "TC")
                .attr("style:family", "table-cell");
            w.start("style:table-cell-properties")
                .attr("fo:padding", "0.05cm")
                .attr("fo:border", "0.5pt solid #000000")
                .end();
            w.end();
        }
        w.finish()
    }
}

/// The common styles: what `text:h` and list items refer to by name.
fn common_styles() -> String {
    let mut w = Writer::fragment();
    w.start("style:default-style")
        .attr("style:family", "paragraph");
    w.start("style:text-properties")
        .attr("fo:font-size", "11pt")
        .end();
    w.end();
    w.start("style:style")
        .attr("style:name", "Standard")
        .attr("style:family", "paragraph")
        .attr("style:class", "text");
    w.start("style:paragraph-properties")
        .attr("fo:margin-bottom", "0.21cm")
        .end();
    w.end();
    w.start("style:style")
        .attr("style:name", "Heading")
        .attr("style:family", "paragraph")
        .attr("style:parent-style-name", "Standard")
        .attr("style:next-style-name", "Standard")
        .attr("style:class", "text");
    w.start("style:paragraph-properties")
        .attr("fo:margin-top", "0.42cm")
        .attr("fo:keep-with-next", "always")
        .end();
    w.start("style:text-properties")
        .attr("fo:font-weight", "bold")
        .end();
    w.end();
    for level in 1..=6 {
        let size = ["16pt", "14pt", "13pt", "12pt", "11pt", "11pt"][level - 1];
        w.start("style:style")
            .attr("style:name", format!("Heading_20_{level}"))
            .attr("style:display-name", format!("Heading {level}"))
            .attr("style:family", "paragraph")
            .attr("style:parent-style-name", "Heading")
            .attr("style:next-style-name", "Standard")
            .attr("style:default-outline-level", level.to_string())
            .attr("style:class", "text");
        w.start("style:text-properties")
            .attr("fo:font-size", size)
            .end();
        w.end();
    }
    for (name, ordered) in [("List_20_Bullet", false), ("List_20_Number", true)] {
        w.start("text:list-style")
            .attr("style:name", name)
            .attr("style:display-name", name.replace("_20_", " "));
        for level in 1..=10u32 {
            match ordered {
                true => {
                    w.start("text:list-level-style-number")
                        .attr("text:level", level.to_string())
                        .attr("style:num-suffix", ".")
                        .attr("style:num-format", "1");
                }
                false => {
                    w.start("text:list-level-style-bullet")
                        .attr("text:level", level.to_string())
                        .attr(
                            "text:bullet-char",
                            ["\u{2022}", "\u{25E6}", "\u{25AA}"][(level as usize - 1) % 3],
                        );
                }
            }
            w.start("style:list-level-properties")
                .attr(
                    "text:space-before",
                    format!("{:.2}cm", 0.635 * (level - 1) as f64),
                )
                .attr("text:min-label-width", "0.635cm")
                .end();
            w.end();
        }
        w.end();
    }
    w.finish()
}

struct Ctx<'a> {
    styles: &'a mut Styles,
    tables: u32,
    /// Model list ids already written, so a list that resumes continues its numbering.
    seen_lists: Vec<u32>,
}

pub fn write(doc: &Document, flat: bool, report: &mut Report) -> Result<Vec<u8>> {
    let mut styles = Styles::default();
    let mut ctx = Ctx {
        styles: &mut styles,
        tables: 0,
        seen_lists: Vec::new(),
    };
    let mut w = Writer::fragment();
    w.start("office:text");
    blocks(&mut w, &doc.blocks, &mut ctx);
    if doc.blocks.is_empty() {
        w.start("text:p").attr("text:style-name", "Standard").end();
    }
    w.end();
    if w.dropped > 0 {
        report.lose(
            "character XML cannot carry",
            format!("{} dropped", w.dropped),
        );
    }
    let parts = Parts {
        mimetype: MIME_ODT,
        styles: common_styles(),
        automatic_styles: styles.automatic(),
        body: w.finish(),
        ..Parts::default()
    };
    odf::package::assemble(&parts, flat)
}

fn blocks(w: &mut Writer, blocks: &[Block], ctx: &mut Ctx<'_>) {
    let mut i = 0;
    while i < blocks.len() {
        match &blocks[i] {
            Block::Paragraph(p) => match p.kind {
                ParaKind::ListItem { list, ordered, .. } => {
                    let mut j = i;
                    let mut items = Vec::new();
                    while let Some(Block::Paragraph(q)) = blocks.get(j)
                        && let ParaKind::ListItem { list: l, level, .. } = q.kind
                        && l == list
                    {
                        items.push((level, q));
                        j += 1;
                    }
                    let resumes = ctx.seen_lists.contains(&list);
                    ctx.seen_lists.push(list);
                    w.start("text:list").attr(
                        "text:style-name",
                        if ordered {
                            "List_20_Number"
                        } else {
                            "List_20_Bullet"
                        },
                    );
                    if resumes && ordered {
                        w.attr("text:continue-numbering", "true");
                    }
                    list_items(w, &items, 0, ctx);
                    w.end();
                    i = j;
                    continue;
                }
                _ => paragraph(w, p, ctx),
            },
            Block::Table(t) => table(w, t, ctx),
        }
        i += 1;
    }
}

/// Items all at `depth` or deeper, as the `text:list-item`s of one `text:list`. An item deeper
/// than the one before it opens a nested list inside the previous item.
fn list_items(w: &mut Writer, items: &[(u8, &Paragraph)], depth: u8, ctx: &mut Ctx<'_>) {
    let mut i = 0;
    while i < items.len() {
        w.start("text:list-item");
        if items[i].0 == depth {
            paragraph(w, items[i].1, ctx);
            i += 1;
        }
        let start = i;
        while i < items.len() && items[i].0 > depth {
            i += 1;
        }
        if i > start {
            w.start("text:list");
            list_items(w, &items[start..i], depth + 1, ctx);
            w.end();
        }
        w.end();
    }
}

fn paragraph(w: &mut Writer, p: &Paragraph, ctx: &mut Ctx<'_>) {
    match p.kind {
        ParaKind::Heading(level) => {
            let style = ctx
                .styles
                .paragraph(&format!("Heading_20_{level}"), p.align);
            w.start("text:h")
                .attr("text:style-name", style)
                .attr("text:outline-level", level.to_string());
        }
        _ => {
            let style = ctx.styles.paragraph("Standard", p.align);
            w.start("text:p").attr("text:style-name", style);
        }
    }
    let mut i = 0;
    while i < p.runs.len() {
        let link = p.runs[i].link.clone();
        let mut j = i;
        while j < p.runs.len() && p.runs[j].link == link {
            j += 1;
        }
        if let Some(href) = &link {
            w.start("text:a")
                .attr("xlink:type", "simple")
                .attr("xlink:href", href);
        }
        for run in &p.runs[i..j] {
            match ctx.styles.text(run.style) {
                Some(style) => {
                    w.start("text:span").attr("text:style-name", style);
                    odf::write_text(w, &run.text);
                    w.end();
                }
                None => odf::write_text(w, &run.text),
            }
        }
        if link.is_some() {
            w.end();
        }
        i = j;
    }
    w.end();
}

fn table(w: &mut Writer, t: &Table, ctx: &mut Ctx<'_>) {
    ctx.tables += 1;
    ctx.styles.table_cell = true;
    let cols = t.columns().max(1);
    w.start("table:table")
        .attr("table:name", format!("Table{}", ctx.tables));
    w.start("table:table-column");
    if cols > 1 {
        w.attr("table:number-columns-repeated", cols.to_string());
    }
    w.end();
    for row in &t.rows {
        w.start("table:table-row");
        for cell in row {
            if cell.covered {
                w.start("table:covered-table-cell").end();
                continue;
            }
            w.start("table:table-cell").attr("table:style-name", "TC");
            if cell.col_span > 1 {
                w.attr("table:number-columns-spanned", cell.col_span.to_string());
            }
            if cell.row_span > 1 {
                w.attr("table:number-rows-spanned", cell.row_span.to_string());
            }
            blocks(w, &cell.blocks, ctx);
            if cell.blocks.is_empty() {
                w.start("text:p").attr("text:style-name", "Standard").end();
            }
            w.end();
        }
        // A row needs at least one cell.
        if row.is_empty() {
            w.start("table:table-cell").end();
        }
        w.end();
    }
    if t.rows.is_empty() {
        w.start("table:table-row")
            .start("table:table-cell")
            .end()
            .end();
    }
    w.end();
}
