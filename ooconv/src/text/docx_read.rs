// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! WordprocessingML → [`Document`]. Attributes are in the `w:` namespace (grind
//! grind `doc/docx-format.md` §1.4). Structure comes from styles and numbering: a paragraph is a heading
//! when its style (or its style's ancestry) has an outline level or is named `heading N`, and a
//! list item when it or its style carries a `numPr` whose level is not `none`.

use std::collections::HashMap;

use super::model::{
    Align, Block, Document, MAX_HEADING, MAX_LIST_LEVEL, ParaKind, Paragraph, Position, RunStyle,
    Table, TableCell,
};
use crate::names::{Ns, RelType, Seen};
use crate::opc::package::{Package, Rel};
use crate::xml::read::{Attrs, Handled, Name, Reader};
use crate::{Error, Report, Result};

fn w(name: &Name, local: &str) -> bool {
    name.in_ns(Ns::Word, local)
}

fn val(attrs: &Attrs) -> Option<&str> {
    attrs.get(Ns::Word, "val")
}

/// `ST_OnOff`: present means on, unless its value says off.
fn on(attrs: &Attrs) -> bool {
    !matches!(val(attrs), Some("0" | "false" | "off" | "none"))
}

/// Run properties as a set of *explicit* values, so a style chain and direct formatting combine:
/// `None` inherits.
#[derive(Clone, Copy, Debug, Default)]
struct RunProps {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    position: Option<Position>,
}

impl RunProps {
    fn over(self, base: RunProps) -> RunProps {
        RunProps {
            bold: self.bold.or(base.bold),
            italic: self.italic.or(base.italic),
            underline: self.underline.or(base.underline),
            strike: self.strike.or(base.strike),
            position: self.position.or(base.position),
        }
    }

    fn resolve(self) -> RunStyle {
        RunStyle {
            bold: self.bold.unwrap_or(false),
            italic: self.italic.unwrap_or(false),
            underline: self.underline.unwrap_or(false),
            strike: self.strike.unwrap_or(false),
            position: self.position.unwrap_or_default(),
        }
    }
}

#[derive(Clone, Default)]
struct Style {
    name: String,
    based_on: Option<String>,
    outline: Option<u8>,
    num: Option<(u32, u8)>,
    run: RunProps,
}

struct Ctx<'r> {
    report: &'r mut Report,
    styles: HashMap<String, Style>,
    /// numId → (abstract level → ordered?).
    numbering: HashMap<u32, Vec<Option<bool>>>,
    rels: Vec<Rel>,
}

pub fn read(bytes: &[u8], report: &mut Report) -> Result<Document> {
    let mut pkg = Package::open(bytes)?;
    let mut seen = Seen::default();
    let main = pkg
        .rel_of("", RelType::OfficeDocument, &mut seen)
        .ok_or(Error::UnknownFormat)?;
    let xml = pkg.part(&main.target).ok_or(Error::UnknownFormat)?;
    let rels = pkg.rels(&main.target, &mut seen);
    let part = |pkg: &mut Package<'_>, kind| {
        rels.iter()
            .find(|r| r.kind == kind && !r.external)
            .and_then(|r| pkg.part(&r.target))
    };
    let styles = part(&mut pkg, RelType::Styles)
        .map(|x| read_styles(&x))
        .transpose()?
        .unwrap_or_default();
    let numbering = part(&mut pkg, RelType::Numbering)
        .map(|x| read_numbering(&x))
        .transpose()?
        .unwrap_or_default();
    for rel in &rels {
        match rel.kind {
            RelType::Header | RelType::Footer => report.lose("header or footer", &rel.target),
            RelType::Footnotes | RelType::Endnotes => {}
            RelType::Comments => report.lose("comments", &rel.target),
            _ => {}
        }
    }
    let mut ctx = Ctx {
        report,
        styles,
        numbering,
        rels,
    };
    let mut doc = Document::default();
    let mut reader = Reader::new(&xml);
    reader.root()?;
    reader.children(|r, name, _| {
        if !w(name, "body") {
            return Ok(Handled::No);
        }
        doc.blocks = blocks(r, &mut ctx)?;
        Ok(Handled::Yes)
    })?;
    seen.transitional |= reader.seen.transitional;
    seen.strict |= reader.seen.strict;
    ctx.report.flavour = Some(seen.flavour());
    Ok(doc)
}

/// A `w:rPr`: its properties, and the character style it names.
fn run_props(r: &mut Reader<'_>) -> Result<(RunProps, Option<String>)> {
    let mut p = RunProps::default();
    let mut style = None;
    r.children(|_, name, attrs| {
        if name.ns != Ns::Word {
            return Ok(Handled::No);
        }
        match name.local.as_str() {
            "rStyle" => style = val(attrs).map(str::to_owned),
            "b" => p.bold = Some(on(attrs)),
            "i" => p.italic = Some(on(attrs)),
            "u" => p.underline = Some(on(attrs)),
            "strike" | "dstrike" => p.strike = Some(on(attrs)),
            "vertAlign" => {
                p.position = Some(match val(attrs) {
                    Some("superscript") => Position::Superscript,
                    Some("subscript") => Position::Subscript,
                    _ => Position::Normal,
                })
            }
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    })?;
    Ok((p, style))
}

fn num_pr(r: &mut Reader<'_>) -> Result<Option<(u32, u8)>> {
    let mut id = None;
    let mut level = 0u8;
    r.children(|_, name, attrs| {
        if w(name, "numId") {
            id = val(attrs).and_then(|v| v.parse::<u32>().ok());
        } else if w(name, "ilvl") {
            level = val(attrs).and_then(|v| v.parse().ok()).unwrap_or(0);
        }
        Ok(Handled::Yes)
    })?;
    Ok(id.map(|id| (id, level)))
}

fn read_styles(xml: &[u8]) -> Result<HashMap<String, Style>> {
    let mut out = HashMap::new();
    let mut r = Reader::new(xml);
    r.root()?;
    r.children(|r, name, attrs| {
        if !w(name, "style") {
            return Ok(Handled::No);
        }
        let id = attrs.get(Ns::Word, "styleId").unwrap_or("").to_owned();
        let mut style = Style::default();
        r.children(|r, name, attrs| {
            if name.ns != Ns::Word {
                return Ok(Handled::No);
            }
            match name.local.as_str() {
                "name" => style.name = val(attrs).unwrap_or("").to_owned(),
                "basedOn" => style.based_on = val(attrs).map(str::to_owned),
                "rPr" => style.run = run_props(r)?.0,
                "pPr" => r.children(|r, name, attrs| {
                    if w(name, "outlineLvl") {
                        style.outline = val(attrs).and_then(|v| v.parse().ok());
                    } else if w(name, "numPr") {
                        style.num = num_pr(r)?;
                    } else {
                        return Ok(Handled::No);
                    }
                    Ok(Handled::Yes)
                })?,
                _ => return Ok(Handled::No),
            }
            Ok(Handled::Yes)
        })?;
        out.insert(id, style);
        Ok(Handled::Yes)
    })?;
    Ok(out)
}

fn read_numbering(xml: &[u8]) -> Result<HashMap<u32, Vec<Option<bool>>>> {
    let mut abstracts: HashMap<String, Vec<Option<bool>>> = HashMap::new();
    let mut nums: Vec<(u32, String)> = Vec::new();
    let mut r = Reader::new(xml);
    r.root()?;
    r.children(|r, name, attrs| {
        if w(name, "abstractNum") {
            let id = attrs
                .get(Ns::Word, "abstractNumId")
                .unwrap_or("")
                .to_owned();
            let mut levels = vec![None; MAX_LIST_LEVEL as usize];
            r.children(|r, name, attrs| {
                if !w(name, "lvl") {
                    return Ok(Handled::No);
                }
                let level: usize = attrs
                    .get(Ns::Word, "ilvl")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let mut fmt = None;
                r.children(|_, name, attrs| {
                    if w(name, "numFmt") {
                        fmt = val(attrs).map(str::to_owned);
                    }
                    Ok(Handled::Yes)
                })?;
                if level < levels.len() {
                    levels[level] = match fmt.as_deref() {
                        Some("none") => None,
                        Some("bullet") => Some(false),
                        Some(_) => Some(true),
                        None => Some(true),
                    };
                }
                Ok(Handled::Yes)
            })?;
            abstracts.insert(id, levels);
            return Ok(Handled::Yes);
        }
        if w(name, "num") {
            let id = attrs.get(Ns::Word, "numId").and_then(|v| v.parse().ok());
            let mut abs = None;
            r.children(|_, name, attrs| {
                if w(name, "abstractNumId") {
                    abs = val(attrs).map(str::to_owned);
                }
                Ok(Handled::Yes)
            })?;
            if let (Some(id), Some(abs)) = (id, abs) {
                nums.push((id, abs));
            }
            return Ok(Handled::Yes);
        }
        Ok(Handled::No)
    })?;
    Ok(nums
        .into_iter()
        .filter_map(|(id, abs)| abstracts.get(&abs).map(|l| (id, l.clone())))
        .collect())
}

impl Ctx<'_> {
    /// A style's chain, nearest first, cycle-safe.
    fn chain(&self, id: &str) -> Vec<&Style> {
        let mut out = Vec::new();
        let mut at = Some(id.to_owned());
        while let Some(id) = at {
            let Some(style) = self.styles.get(&id) else {
                break;
            };
            if out.len() > 32 {
                break;
            }
            out.push(style);
            at = style.based_on.clone();
        }
        out
    }

    fn heading(&self, style: &str) -> Option<u8> {
        for s in self.chain(style) {
            if let Some(level) = s.outline {
                return (level < MAX_HEADING).then_some(level + 1);
            }
            let lower = s.name.to_ascii_lowercase();
            if let Some(n) = lower
                .strip_prefix("heading ")
                .and_then(|n| n.trim().parse::<u8>().ok())
            {
                return (1..=MAX_HEADING).contains(&n).then_some(n);
            }
        }
        None
    }

    fn style_num(&self, style: &str) -> Option<(u32, u8)> {
        self.chain(style).into_iter().find_map(|s| s.num)
    }

    fn char_style(&self, style: &str) -> RunProps {
        self.chain(style)
            .into_iter()
            .rev()
            .fold(RunProps::default(), |acc, s| s.run.over(acc))
    }

    fn is_hyperlink_style(&self, id: &str) -> bool {
        id == "Hyperlink"
            || self
                .styles
                .get(id)
                .is_some_and(|s| s.name.eq_ignore_ascii_case("hyperlink"))
    }

    fn hyperlink(&self, attrs: &Attrs) -> Option<String> {
        if let Some(id) = attrs.get(Ns::Relationships, "id") {
            return self
                .rels
                .iter()
                .find(|r| r.id == id && r.kind == RelType::Hyperlink)
                .map(|r| r.target.clone());
        }
        attrs.get(Ns::Word, "anchor").map(|a| format!("#{a}"))
    }
}

/// Block-level content: of `w:body`, `w:tc`, `w:sdtContent`.
fn blocks(r: &mut Reader<'_>, ctx: &mut Ctx<'_>) -> Result<Vec<Block>> {
    let mut out = Vec::new();
    r.children(|r, name, _| {
        if name.ns != Ns::Word {
            return Ok(Handled::No);
        }
        match name.local.as_str() {
            "p" => out.push(Block::Paragraph(paragraph(r, ctx)?)),
            "tbl" => out.push(Block::Table(table(r, ctx)?)),
            "sdt" => {
                r.children(|r, name, _| {
                    if w(name, "sdtContent") {
                        out.extend(blocks(r, ctx)?);
                        return Ok(Handled::Yes);
                    }
                    Ok(Handled::No)
                })?;
            }
            "customXml" | "ins" => out.extend(blocks(r, ctx)?),
            "sectPr" | "bookmarkStart" | "bookmarkEnd" | "proofErr" => {}
            "altChunk" => ctx.report.lose("embedded document", "w:altChunk"),
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    })?;
    Ok(out)
}

struct ParaState {
    para: Paragraph,
    /// Inside a complex field: `true` while reading its instruction, which is not text.
    in_instruction: Vec<bool>,
}

fn paragraph(r: &mut Reader<'_>, ctx: &mut Ctx<'_>) -> Result<Paragraph> {
    let mut style: Option<String> = None;
    let mut num: Option<Option<(u32, u8)>> = None;
    let mut align = None;
    let mut state = ParaState {
        para: Paragraph::default(),
        in_instruction: Vec::new(),
    };
    r.children(|r, name, attrs| {
        if w(name, "pPr") {
            r.children(|r, name, attrs| {
                if name.ns != Ns::Word {
                    return Ok(Handled::No);
                }
                match name.local.as_str() {
                    "pStyle" => style = val(attrs).map(str::to_owned),
                    "numPr" => num = Some(num_pr(r)?),
                    "jc" => {
                        align = match val(attrs) {
                            Some("center") => Some(Align::Center),
                            Some("right" | "end") => Some(Align::End),
                            Some("both" | "distribute") => Some(Align::Justify),
                            _ => None,
                        }
                    }
                    _ => return Ok(Handled::No),
                }
                Ok(Handled::Yes)
            })?;
            return Ok(Handled::Yes);
        }
        inline(r, name, attrs, ctx, &mut state, None)
    })?;
    let mut para = state.para;
    para.align = align;
    let style = style.unwrap_or_default();
    let num = num.unwrap_or_else(|| ctx.style_num(&style));
    para.kind = match num {
        Some((id, level)) if id != 0 => {
            let level = level.min(MAX_LIST_LEVEL - 1);
            match ctx
                .numbering
                .get(&id)
                .and_then(|l| l.get(level as usize).copied().flatten())
            {
                Some(ordered) => ParaKind::ListItem {
                    list: id,
                    ordered,
                    level,
                },
                None => ParaKind::Normal,
            }
        }
        _ => match ctx.heading(&style) {
            Some(level) => ParaKind::Heading(level),
            None => ParaKind::Normal,
        },
    };
    Ok(para)
}

/// Paragraph-level content: runs, and the wrappers runs come in.
fn inline(
    r: &mut Reader<'_>,
    name: &Name,
    attrs: &Attrs,
    ctx: &mut Ctx<'_>,
    state: &mut ParaState,
    link: Option<&str>,
) -> Result<Handled> {
    if name.ns != Ns::Word {
        return Ok(Handled::No);
    }
    match name.local.as_str() {
        "r" => run(r, ctx, state, link)?,
        "hyperlink" => {
            let target = ctx.hyperlink(attrs);
            if target.is_none() {
                ctx.report.lose("hyperlink without a target", "w:hyperlink");
            }
            let target = target.or(link.map(str::to_owned));
            r.children(|r, name, attrs| inline(r, name, attrs, ctx, state, target.as_deref()))?;
        }
        // Accepted insertions, simple fields' results, smart tags and content controls are
        // their content.
        "ins" | "fldSimple" | "smartTag" | "customXml" | "sdtContent" | "moveTo" | "dir"
        | "bdo" => {
            r.children(|r, name, attrs| inline(r, name, attrs, ctx, state, link))?;
        }
        "sdt" => {
            r.children(|r, name, attrs| match w(name, "sdtContent") {
                true => inline(r, name, attrs, ctx, state, link),
                false => Ok(Handled::No),
            })?;
        }
        // A deletion's text is not the document's.
        "del" | "moveFrom" | "pPr" | "bookmarkStart" | "bookmarkEnd" | "proofErr" => {}
        _ => return Ok(Handled::No),
    }
    Ok(Handled::Yes)
}

fn run(
    r: &mut Reader<'_>,
    ctx: &mut Ctx<'_>,
    state: &mut ParaState,
    link: Option<&str>,
) -> Result<()> {
    let mut props = RunProps::default();
    let mut char_style = None;
    let mut pieces: Vec<String> = Vec::new();
    r.children(|r, name, attrs| {
        if name.ns != Ns::Word {
            return Ok(Handled::No);
        }
        let in_instr = state.in_instruction.last().copied().unwrap_or(false);
        match name.local.as_str() {
            "rPr" => (props, char_style) = run_props(r)?,
            "t" if !in_instr => pieces.push(r.text()?),
            "tab" | "ptab" if !in_instr => pieces.push("\t".into()),
            "br" | "cr" if !in_instr => match attrs.get(Ns::Word, "type") {
                Some("page") | Some("column") => ctx.report.lose("page or column break", "w:br"),
                _ => pieces.push("\n".into()),
            },
            "noBreakHyphen" if !in_instr => pieces.push("\u{2011}".into()),
            "softHyphen" if !in_instr => pieces.push("\u{AD}".into()),
            "fldChar" => match attrs.get(Ns::Word, "fldCharType") {
                Some("begin") => state.in_instruction.push(true),
                Some("separate") => {
                    if let Some(top) = state.in_instruction.last_mut() {
                        *top = false;
                    }
                }
                Some("end") => {
                    state.in_instruction.pop();
                }
                _ => {}
            },
            "drawing" | "pict" | "object" => ctx.report.lose("picture or drawing", "w:drawing"),
            "footnoteReference" | "endnoteReference" => {
                ctx.report.lose("footnote", "w:footnoteReference")
            }
            "commentReference" => {}
            "sym" => ctx.report.lose("symbol character", "w:sym"),
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    })?;
    // Inside a link, the `Hyperlink` character style is how links look, not formatting the text
    // has: carrying its underline onto the run would add formatting on every round trip.
    let base = match char_style {
        Some(s) if link.is_some() && ctx.is_hyperlink_style(&s) => RunProps::default(),
        Some(s) => ctx.char_style(&s),
        None => RunProps::default(),
    };
    let style = props.over(base).resolve();
    for p in pieces {
        state.para.push(&p, style, link);
    }
    Ok(())
}

fn table(r: &mut Reader<'_>, ctx: &mut Ctx<'_>) -> Result<Table> {
    // Rows as read: each cell with its grid span and vertical-merge state.
    struct Raw {
        cell: TableCell,
        span: u32,
        /// `Some(true)` restarts a vertical merge, `Some(false)` continues one.
        vmerge: Option<bool>,
    }
    let mut rows: Vec<Vec<Raw>> = Vec::new();
    r.children(|r, name, _| {
        if !w(name, "tr") {
            return Ok(Handled::No);
        }
        let mut row = Vec::new();
        r.children(|r, name, _| {
            if !w(name, "tc") {
                return Ok(Handled::No);
            }
            let mut span = 1u32;
            let mut vmerge = None;
            let mut blocks_ = Vec::new();
            r.children(|r, name, attrs| {
                if w(name, "tcPr") {
                    r.children(|_, name, attrs| {
                        if w(name, "gridSpan") {
                            span = val(attrs)
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(1)
                                .clamp(1, 1000);
                        } else if w(name, "vMerge") {
                            vmerge = Some(val(attrs) == Some("restart"));
                        }
                        Ok(Handled::Yes)
                    })?;
                    return Ok(Handled::Yes);
                }
                let _ = attrs;
                match name.ns == Ns::Word {
                    true if name.local == "p" => {
                        blocks_.push(Block::Paragraph(paragraph(r, ctx)?));
                        Ok(Handled::Yes)
                    }
                    true if name.local == "tbl" => {
                        blocks_.push(Block::Table(table(r, ctx)?));
                        Ok(Handled::Yes)
                    }
                    _ => Ok(Handled::No),
                }
            })?;
            row.push(Raw {
                cell: TableCell {
                    blocks: blocks_,
                    col_span: span,
                    ..TableCell::default()
                },
                span,
                vmerge,
            });
            Ok(Handled::Yes)
        })?;
        rows.push(row);
        Ok(Handled::Yes)
    })?;

    // Expand to the grid, then fold vertical merges into their first cell's row span.
    let mut grid: Vec<Vec<TableCell>> = Vec::new();
    let mut continues: Vec<Vec<bool>> = Vec::new();
    for row in rows {
        let mut cells = Vec::new();
        let mut cont = Vec::new();
        for raw in row {
            let is_cont = raw.vmerge == Some(false);
            cells.push(raw.cell);
            cont.push(is_cont);
            for _ in 1..raw.span {
                cells.push(TableCell {
                    covered: true,
                    ..TableCell::default()
                });
                cont.push(is_cont);
            }
        }
        grid.push(cells);
        continues.push(cont);
    }
    for r in 0..grid.len() {
        for c in 0..grid[r].len() {
            if !continues[r][c] || grid[r][c].covered {
                continue;
            }
            // Find the cell this continues: the nearest non-continuing cell above.
            let mut up = r;
            while up > 0 && continues[up][c] {
                up -= 1;
            }
            if up == r || c >= grid[up].len() {
                continue;
            }
            let span = (r - up + 1) as u32;
            grid[up][c].row_span = grid[up][c].row_span.max(span);
            let content = std::mem::take(&mut grid[r][c].blocks);
            if content
                .iter()
                .any(|b| !matches!(b, Block::Paragraph(p) if p.runs.is_empty()))
            {
                ctx.report.lose("content under a merged cell", "w:vMerge");
            }
        }
    }
    let mut table = Table { rows: grid };
    table.normalise();
    Ok(table)
}
