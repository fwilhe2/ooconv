// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! [`Document`] → WordprocessingML, Transitional. Headings use the built-in `Heading1`…`Heading6`
//! styles with an outline level, so Word's navigation and TOC see them; list items use one
//! `w:num` per list, so each ordered list restarts. Two rules Word enforces that the schema does
//! not spell out are kept here: a `w:tc` ends with a `w:p`, and the body is never empty.

use std::collections::BTreeMap;

use super::model::{Align, Block, Document, ParaKind, Paragraph, Position, Run, Table};
use crate::names::WORD_T;
use crate::opc::write::{CORE_PROPERTIES_REL, PackageWriter, REL_BASE};
use crate::xml::write::Writer;
use crate::{Report, Result};

const CT_DOCUMENT: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
const CT_STYLES: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml";
const CT_NUMBERING: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";
const CT_CORE: &str = "application/vnd.openxmlformats-package.core-properties+xml";

/// A4 portrait, 2.5 cm margins, in twentieths of a point.
const PAGE: (u32, u32) = (11_906, 16_838);
const MARGIN: u32 = 1_418;

struct Ctx<'a> {
    pkg: &'a mut PackageWriter,
    /// Model list id → (`w:numId`, ordered).
    lists: BTreeMap<u32, (u32, bool)>,
    links: BTreeMap<String, String>,
}

pub fn write(doc: &Document, report: &mut Report) -> Result<Vec<u8>> {
    let mut pkg = PackageWriter::new();
    let mut ctx = Ctx {
        pkg: &mut pkg,
        lists: BTreeMap::new(),
        links: BTreeMap::new(),
    };
    let mut w = Writer::new();
    w.start("w:document")
        .attr("xmlns:w", WORD_T)
        .attr("xmlns:r", REL_BASE);
    w.start("w:body");
    blocks(&mut w, &doc.blocks, &mut ctx);
    // Word requires the body to end with a paragraph; a final table gets an empty one after it,
    // which every reader here drops again (`Document::drop_table_filler`).
    if !matches!(doc.blocks.last(), Some(Block::Paragraph(_))) {
        w.start("w:p").end();
    }
    w.start("w:sectPr");
    w.start("w:pgSz")
        .attr("w:w", PAGE.0.to_string())
        .attr("w:h", PAGE.1.to_string())
        .end();
    let m = MARGIN.to_string();
    w.start("w:pgMar")
        .attr("w:top", &m)
        .attr("w:right", &m)
        .attr("w:bottom", &m)
        .attr("w:left", &m)
        .attr("w:header", "708")
        .attr("w:footer", "708")
        .attr("w:gutter", "0")
        .end();
    w.end();
    w.end().end();
    if w.dropped > 0 {
        report.lose(
            "character XML cannot carry",
            format!("{} dropped", w.dropped),
        );
    }
    let lists = std::mem::take(&mut ctx.lists);
    pkg.part("word/document.xml", CT_DOCUMENT, w.finish());
    pkg.relate(
        "",
        &format!("{REL_BASE}/officeDocument"),
        "word/document.xml",
    );
    pkg.part("word/styles.xml", CT_STYLES, styles());
    pkg.relate(
        "word/document.xml",
        &format!("{REL_BASE}/styles"),
        "word/styles.xml",
    );
    if !lists.is_empty() {
        pkg.part("word/numbering.xml", CT_NUMBERING, numbering(&lists));
        pkg.relate(
            "word/document.xml",
            &format!("{REL_BASE}/numbering"),
            "word/numbering.xml",
        );
    }
    pkg.part(
        "docProps/core.xml",
        CT_CORE,
        crate::opc::write::core_properties(),
    );
    pkg.relate("", CORE_PROPERTIES_REL, "docProps/core.xml");
    pkg.finish()
}

fn blocks(w: &mut Writer, blocks: &[Block], ctx: &mut Ctx<'_>) {
    for block in blocks {
        match block {
            Block::Paragraph(p) => paragraph(w, p, ctx),
            Block::Table(t) => table(w, t, ctx),
        }
    }
}

fn paragraph(w: &mut Writer, p: &Paragraph, ctx: &mut Ctx<'_>) {
    w.start("w:p");
    let style = match p.kind {
        ParaKind::Heading(level) => Some(format!("Heading{level}")),
        ParaKind::ListItem { .. } => Some("ListParagraph".to_owned()),
        ParaKind::Normal => None,
    };
    let jc = p.align.map(|a| match a {
        Align::Center => "center",
        Align::End => "right",
        Align::Justify => "both",
    });
    if style.is_some() || jc.is_some() || matches!(p.kind, ParaKind::ListItem { .. }) {
        w.start("w:pPr");
        if let Some(style) = style {
            w.start("w:pStyle").attr("w:val", style).end();
        }
        if let ParaKind::ListItem {
            list,
            ordered,
            level,
        } = p.kind
        {
            let next = ctx.lists.len() as u32 + 1;
            let (num_id, _) = *ctx.lists.entry(list).or_insert((next, ordered));
            w.start("w:numPr");
            w.start("w:ilvl").attr("w:val", level.to_string()).end();
            w.start("w:numId").attr("w:val", num_id.to_string()).end();
            w.end();
        }
        if let Some(jc) = jc {
            w.start("w:jc").attr("w:val", jc).end();
        }
        w.end();
    }
    let mut i = 0;
    while i < p.runs.len() {
        match &p.runs[i].link {
            Some(target) => {
                // Consecutive runs of one link are one `w:hyperlink`.
                let mut j = i;
                while j < p.runs.len() && p.runs[j].link.as_ref() == Some(target) {
                    j += 1;
                }
                w.start("w:hyperlink");
                match target.strip_prefix('#') {
                    Some(anchor) => {
                        w.attr("w:anchor", anchor);
                    }
                    None => {
                        let id = match ctx.links.get(target) {
                            Some(id) => id.clone(),
                            None => {
                                let id = ctx.pkg.relate_external(
                                    "word/document.xml",
                                    &format!("{REL_BASE}/hyperlink"),
                                    target,
                                );
                                ctx.links.insert(target.clone(), id.clone());
                                id
                            }
                        };
                        w.attr("r:id", id);
                    }
                }
                for run in &p.runs[i..j] {
                    write_run(w, run, true);
                }
                w.end();
                i = j;
            }
            None => {
                write_run(w, &p.runs[i], false);
                i += 1;
            }
        }
    }
    w.end();
}

fn write_run(w: &mut Writer, run: &Run, link: bool) {
    w.start("w:r");
    let s = run.style;
    if link || s.bold || s.italic || s.underline || s.strike || s.position != Position::Normal {
        // `CT_RPr`'s order: rStyle, b, i, strike, u, vertAlign.
        w.start("w:rPr");
        if link {
            w.start("w:rStyle").attr("w:val", "Hyperlink").end();
        }
        if s.bold {
            w.start("w:b").end();
        }
        if s.italic {
            w.start("w:i").end();
        }
        if s.strike {
            w.start("w:strike").end();
        }
        if s.underline {
            w.start("w:u").attr("w:val", "single").end();
        }
        match s.position {
            Position::Superscript => {
                w.start("w:vertAlign").attr("w:val", "superscript").end();
            }
            Position::Subscript => {
                w.start("w:vertAlign").attr("w:val", "subscript").end();
            }
            Position::Normal => {}
        }
        w.end();
    }
    let mut buf = String::new();
    let flush = |w: &mut Writer, buf: &mut String| {
        if buf.is_empty() {
            return;
        }
        w.start("w:t");
        if buf.starts_with(char::is_whitespace)
            || buf.ends_with(char::is_whitespace)
            || buf.contains("  ")
        {
            w.attr("xml:space", "preserve");
        }
        w.text(buf).end();
        buf.clear();
    };
    for c in run.text.chars() {
        match c {
            '\t' => {
                flush(w, &mut buf);
                w.start("w:tab").end();
            }
            '\n' => {
                flush(w, &mut buf);
                w.start("w:br").end();
            }
            '\r' => {}
            '\u{2011}' => {
                flush(w, &mut buf);
                w.start("w:noBreakHyphen").end();
            }
            '\u{AD}' => {
                flush(w, &mut buf);
                w.start("w:softHyphen").end();
            }
            c => buf.push(c),
        }
    }
    flush(w, &mut buf);
    w.end();
}

fn table(w: &mut Writer, t: &Table, ctx: &mut Ctx<'_>) {
    let cols = t.columns().max(1);
    let text_width = PAGE.0 - 2 * MARGIN;
    let col_width = (text_width / cols as u32).max(1);
    w.start("w:tbl");
    w.start("w:tblPr");
    w.start("w:tblW")
        .attr("w:w", "0")
        .attr("w:type", "auto")
        .end();
    w.start("w:tblBorders");
    for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
        w.start(&format!("w:{side}"))
            .attr("w:val", "single")
            .attr("w:sz", "4")
            .attr("w:space", "0")
            .attr("w:color", "auto")
            .end();
    }
    w.end();
    w.start("w:tblLayout").attr("w:type", "fixed").end();
    w.end();
    w.start("w:tblGrid");
    for _ in 0..cols {
        w.start("w:gridCol")
            .attr("w:w", col_width.to_string())
            .end();
    }
    w.end();
    for (r, row) in t.rows.iter().enumerate() {
        w.start("w:tr");
        let mut c = 0;
        while c < row.len() {
            let cell = &row[c];
            // A horizontally covered position belongs to the `gridSpan` before it.
            if cell.covered && !is_vertical_continuation(t, r, c) {
                c += 1;
                continue;
            }
            let (span, continuing) = match cell.covered {
                true => (master_span(t, r, c), true),
                false => (cell.col_span, false),
            };
            w.start("w:tc");
            w.start("w:tcPr");
            w.start("w:tcW")
                .attr("w:w", (col_width * span).to_string())
                .attr("w:type", "dxa")
                .end();
            if span > 1 {
                w.start("w:gridSpan").attr("w:val", span.to_string()).end();
            }
            if continuing {
                w.start("w:vMerge").end();
            } else if cell.row_span > 1 {
                w.start("w:vMerge").attr("w:val", "restart").end();
            }
            w.end();
            if !continuing {
                blocks(w, &cell.blocks, ctx);
            }
            if continuing || !matches!(cell.blocks.last(), Some(Block::Paragraph(_))) {
                w.start("w:p").end();
            }
            w.end();
            c += span.max(1) as usize;
        }
        w.end();
    }
    w.end();
}

/// Whether a covered cell is covered from above (a row span), rather than from the left.
fn is_vertical_continuation(t: &Table, r: usize, c: usize) -> bool {
    master(t, r, c).is_some_and(|(mr, mc)| mr < r && mc == c)
}

/// The span the master of a vertical continuation has, which the continuation repeats.
fn master_span(t: &Table, r: usize, c: usize) -> u32 {
    master(t, r, c).map_or(1, |(mr, mc)| t.rows[mr][mc].col_span)
}

/// The cell whose span covers `(r, c)`.
fn master(t: &Table, r: usize, c: usize) -> Option<(usize, usize)> {
    for mr in (0..=r).rev() {
        for mc in (0..=c).rev() {
            let cell = &t.rows[mr][mc];
            if !cell.covered && mr + cell.row_span as usize > r && mc + cell.col_span as usize > c {
                return Some((mr, mc));
            }
        }
    }
    None
}

fn styles() -> String {
    let mut w = Writer::new();
    w.start("w:styles").attr("xmlns:w", WORD_T);
    w.start("w:docDefaults");
    w.start("w:rPrDefault").start("w:rPr");
    w.start("w:sz").attr("w:val", "22").end();
    w.end().end();
    w.start("w:pPrDefault").start("w:pPr");
    w.start("w:spacing").attr("w:after", "120").end();
    w.end().end();
    w.end();

    w.start("w:style")
        .attr("w:type", "paragraph")
        .attr("w:default", "1")
        .attr("w:styleId", "Normal");
    w.start("w:name").attr("w:val", "Normal").end();
    w.start("w:qFormat").end();
    w.end();

    for level in 1..=6u8 {
        // Sizes in half-points: 16, 14, 13, 12, 11, 11 pt.
        let size = [32, 28, 26, 24, 22, 22][level as usize - 1];
        w.start("w:style")
            .attr("w:type", "paragraph")
            .attr("w:styleId", format!("Heading{level}"));
        w.start("w:name")
            .attr("w:val", format!("heading {level}"))
            .end();
        w.start("w:basedOn").attr("w:val", "Normal").end();
        w.start("w:next").attr("w:val", "Normal").end();
        w.start("w:qFormat").end();
        w.start("w:pPr");
        w.start("w:keepNext").end();
        w.start("w:spacing")
            .attr("w:before", "240")
            .attr("w:after", "120")
            .end();
        w.start("w:outlineLvl")
            .attr("w:val", (level - 1).to_string())
            .end();
        w.end();
        w.start("w:rPr");
        w.start("w:b").end();
        w.start("w:sz").attr("w:val", size.to_string()).end();
        w.end();
        w.end();
    }

    w.start("w:style")
        .attr("w:type", "paragraph")
        .attr("w:styleId", "ListParagraph");
    w.start("w:name").attr("w:val", "List Paragraph").end();
    w.start("w:basedOn").attr("w:val", "Normal").end();
    w.start("w:qFormat").end();
    w.start("w:pPr");
    w.start("w:spacing").attr("w:after", "0").end();
    w.end();
    w.end();

    w.start("w:style")
        .attr("w:type", "character")
        .attr("w:styleId", "Hyperlink");
    w.start("w:name").attr("w:val", "Hyperlink").end();
    w.start("w:rPr");
    w.start("w:color").attr("w:val", "0563C1").end();
    w.start("w:u").attr("w:val", "single").end();
    w.end();
    w.end();

    w.end();
    w.finish()
}

fn numbering(lists: &BTreeMap<u32, (u32, bool)>) -> String {
    let mut w = Writer::new();
    w.start("w:numbering").attr("xmlns:w", WORD_T);
    // Abstract 0: bullets; abstract 1: decimal, `1.`, `1.1.` … as a level deepens.
    for (abs, ordered) in [(0, false), (1, true)] {
        w.start("w:abstractNum")
            .attr("w:abstractNumId", abs.to_string());
        w.start("w:multiLevelType")
            .attr("w:val", "hybridMultilevel")
            .end();
        for level in 0..9u32 {
            w.start("w:lvl").attr("w:ilvl", level.to_string());
            w.start("w:start").attr("w:val", "1").end();
            w.start("w:numFmt")
                .attr("w:val", if ordered { "decimal" } else { "bullet" })
                .end();
            let text = match ordered {
                true => format!("%{}.", level + 1),
                false => ["\u{2022}", "\u{25E6}", "\u{25AA}"][level as usize % 3].to_owned(),
            };
            w.start("w:lvlText").attr("w:val", text).end();
            w.start("w:lvlJc").attr("w:val", "left").end();
            w.start("w:pPr");
            w.start("w:ind")
                .attr("w:left", (720 * (level + 1)).to_string())
                .attr("w:hanging", "360")
                .end();
            w.end();
            w.end();
        }
        w.end();
    }
    for (num_id, ordered) in lists.values() {
        w.start("w:num").attr("w:numId", num_id.to_string());
        w.start("w:abstractNumId")
            .attr("w:val", if *ordered { "1" } else { "0" })
            .end();
        // Instances of one abstract list share its counters unless told to start over, so every
        // ordered list restarts explicitly (ECMA-376 §17.9.27).
        if *ordered {
            for level in 0..9 {
                w.start("w:lvlOverride").attr("w:ilvl", level.to_string());
                w.start("w:startOverride").attr("w:val", "1").end();
                w.end();
            }
        }
        w.end();
    }
    w.end();
    w.finish()
}
