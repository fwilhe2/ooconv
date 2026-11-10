// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! OpenDocument text → [`Document`], either form. A heading is a `text:h`; a list item is a
//! paragraph inside `text:list`s, its kind from the list style in force at its level. Character
//! formatting is resolved from automatic text styles and the common styles they inherit from;
//! a *common* paragraph style's text properties are the style's, not the run's, and stay behind.

use std::collections::HashMap;

use super::model::{
    Align, Block, Document, MAX_HEADING, MAX_LIST_LEVEL, ParaKind, Paragraph, Position, Table,
    TableCell,
};
use crate::names::Ns;
use crate::odf::{self, Collapser};
use crate::xml::read::{Attrs, Handled, Name, Reader};
use crate::{Report, Result};

#[derive(Clone, Copy, Debug, Default)]
struct Props {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    position: Option<Position>,
}

impl Props {
    fn over(self, base: Props) -> Props {
        Props {
            bold: self.bold.or(base.bold),
            italic: self.italic.or(base.italic),
            underline: self.underline.or(base.underline),
            strike: self.strike.or(base.strike),
            position: self.position.or(base.position),
        }
    }

    fn resolve(self) -> super::model::RunStyle {
        super::model::RunStyle {
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
    parent: Option<String>,
    props: Props,
    /// `Some(None)` is an explicit start alignment, which stops the search up the chain.
    align: Option<Option<Align>>,
    automatic: bool,
}

#[derive(Default)]
struct Ctx<'r> {
    report: Option<&'r mut Report>,
    /// Text and paragraph styles share no namespace in ODF, but names rarely collide; keyed by
    /// (family, name).
    styles: HashMap<(String, String), Style>,
    /// List style name → per level (0-based): ordered?
    lists: HashMap<String, Vec<bool>>,
    next_list: u32,
    last_list: Option<u32>,
}

impl Ctx<'_> {
    fn lose(&mut self, class: &'static str, example: &str) {
        if let Some(r) = self.report.as_deref_mut() {
            r.lose(class, example);
        }
    }

    fn chain(&self, family: &str, name: &str) -> Vec<&Style> {
        let mut out = Vec::new();
        let mut at = Some(name.to_owned());
        while let Some(n) = at {
            let Some(s) = self.styles.get(&(family.to_owned(), n)) else {
                break;
            };
            if out.len() > 32 {
                break;
            }
            out.push(s);
            at = s.parent.clone();
        }
        out
    }

    /// A text style's properties, inherited through common character styles too — except,
    /// inside a link, a common style that is how links look (`Internet_20_link`, `Hyperlink`):
    /// its underline is the link's appearance, not the text's formatting.
    fn text_props(&self, name: &str, in_link: bool) -> Props {
        let mut at = Some(name.to_owned());
        let mut chain = Vec::new();
        while let Some(n) = at {
            let Some(s) = self.styles.get(&("text".to_owned(), n.clone())) else {
                break;
            };
            if chain.len() > 32 {
                break;
            }
            let link_look = matches!(
                n.as_str(),
                "Internet_20_link"
                    | "Visited_20_Internet_20_Link"
                    | "Hyperlink"
                    | "FollowedHyperlink"
            );
            if !(in_link && !s.automatic && link_look) {
                chain.push(s);
            }
            at = s.parent.clone();
        }
        chain
            .into_iter()
            .rev()
            .fold(Props::default(), |acc, s| s.props.over(acc))
    }

    /// A paragraph style's *direct* text properties: only the automatic links of its chain.
    fn paragraph_props(&self, name: &str) -> Props {
        self.chain("paragraph", name)
            .into_iter()
            .filter(|s| s.automatic)
            .rev()
            .fold(Props::default(), |acc, s| s.props.over(acc))
    }

    fn paragraph_align(&self, name: &str) -> Option<Align> {
        self.chain("paragraph", name)
            .into_iter()
            .find_map(|s| s.align)
            .flatten()
    }
}

pub fn read(bytes: &[u8], report: &mut Report) -> Result<Document> {
    let mut ctx = Ctx {
        report: Some(report),
        ..Ctx::default()
    };
    let mut doc = Document::default();
    for part in odf::package::documents(bytes)? {
        let mut r = Reader::new(&part);
        r.root()?;
        r.children(|r, name, _| {
            if name.ns != Ns::Office {
                return Ok(Handled::No);
            }
            match name.local.as_str() {
                "styles" => read_styles(r, &mut ctx, false)?,
                "automatic-styles" => read_styles(r, &mut ctx, true)?,
                "master-styles" => {
                    r.children(|r, name, _| {
                        if name.in_ns(Ns::Style, "master-page") {
                            r.children(|_, name, _| {
                                if name.ns == Ns::Style
                                    && (name.local.starts_with("header")
                                        || name.local.starts_with("footer"))
                                {
                                    ctx.lose("header or footer", &name.local);
                                }
                                Ok(Handled::No)
                            })?;
                            return Ok(Handled::Yes);
                        }
                        Ok(Handled::No)
                    })?;
                }
                "body" => r.children(|r, name, _| {
                    if !name.in_ns(Ns::Office, "text") {
                        return Ok(Handled::No);
                    }
                    doc.blocks = blocks(r, &mut ctx, None)?;
                    Ok(Handled::Yes)
                })?,
                _ => return Ok(Handled::No),
            }
            Ok(Handled::Yes)
        })?;
    }
    Ok(doc)
}

fn read_styles(r: &mut Reader<'_>, ctx: &mut Ctx<'_>, automatic: bool) -> Result<()> {
    r.children(|r, name, attrs| {
        if name.in_ns(Ns::Text, "list-style") {
            let n = attrs.get(Ns::Style, "name").unwrap_or("").to_owned();
            let mut levels = vec![false; MAX_LIST_LEVEL as usize + 1];
            r.children(|_, name, attrs| {
                let level: usize = attrs
                    .get(Ns::Text, "level")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1);
                if (1..=levels.len()).contains(&level) {
                    levels[level - 1] = match name.local.as_str() {
                        "list-level-style-number" => {
                            // `num-format=""` is a numbered level showing no number.
                            !matches!(attrs.get(Ns::Style, "num-format"), Some(""))
                        }
                        _ => false,
                    };
                }
                Ok(Handled::Yes)
            })?;
            ctx.lists.insert(n, levels);
            return Ok(Handled::Yes);
        }
        if !name.in_ns(Ns::Style, "style") {
            return Ok(Handled::No);
        }
        let family = attrs.get(Ns::Style, "family").unwrap_or("").to_owned();
        let n = attrs.get(Ns::Style, "name").unwrap_or("").to_owned();
        let mut style = Style {
            parent: attrs.get(Ns::Style, "parent-style-name").map(str::to_owned),
            automatic,
            ..Style::default()
        };
        r.children(|_, name, attrs| {
            if name.in_ns(Ns::Style, "text-properties") {
                style.props = text_properties(attrs);
            } else if name.in_ns(Ns::Style, "paragraph-properties") {
                style.align = match attrs.get(Ns::Fo, "text-align") {
                    Some("start" | "left") => Some(None),
                    Some("center") => Some(Some(Align::Center)),
                    Some("end" | "right") => Some(Some(Align::End)),
                    Some("justify") => Some(Some(Align::Justify)),
                    _ => None,
                };
            }
            Ok(Handled::Yes)
        })?;
        ctx.styles.insert((family, n), style);
        Ok(Handled::Yes)
    })
}

fn text_properties(attrs: &Attrs) -> Props {
    let line = |a: Option<&str>| a.map(|v| v != "none");
    Props {
        bold: attrs
            .get(Ns::Fo, "font-weight")
            .map(|w| w == "bold" || w.parse::<u32>().is_ok_and(|n| n >= 600)),
        italic: attrs
            .get(Ns::Fo, "font-style")
            .map(|s| s == "italic" || s == "oblique"),
        underline: line(attrs.get(Ns::Style, "text-underline-style")),
        strike: line(attrs.get(Ns::Style, "text-line-through-style")),
        position: attrs.get(Ns::Style, "text-position").map(|p| {
            let first = p.split_whitespace().next().unwrap_or("0");
            match first {
                "super" => Position::Superscript,
                "sub" => Position::Subscript,
                n => match n.trim_end_matches('%').parse::<f64>() {
                    Ok(v) if v > 0.0 => Position::Superscript,
                    Ok(v) if v < 0.0 => Position::Subscript,
                    _ => Position::Normal,
                },
            }
        }),
    }
}

/// The list a paragraph is in: its id, the list style in force, and its depth.
#[derive(Clone)]
struct InList {
    id: u32,
    style: Option<String>,
    level: u8,
}

/// Block content: of `office:text`, a section, a table cell, a list item.
fn blocks(r: &mut Reader<'_>, ctx: &mut Ctx<'_>, list: Option<&InList>) -> Result<Vec<Block>> {
    let mut out = Vec::new();
    r.children(|r, name, attrs| {
        match (name.ns, name.local.as_str()) {
            (Ns::Text, "p" | "h") => {
                let mut p = paragraph(r, name, attrs, ctx)?;
                if let Some(l) = list {
                    let ordered = l
                        .style
                        .as_ref()
                        .and_then(|s| ctx.lists.get(s))
                        .and_then(|levels| levels.get(l.level as usize).copied())
                        .unwrap_or(false);
                    p.kind = ParaKind::ListItem {
                        list: l.id,
                        ordered,
                        level: l.level.min(MAX_LIST_LEVEL - 1),
                    };
                }
                out.push(Block::Paragraph(p));
            }
            (Ns::Text, "list") => {
                let style = attrs
                    .get(Ns::Text, "style-name")
                    .map(str::to_owned)
                    .or_else(|| list.and_then(|l| l.style.clone()));
                let inner = match list {
                    Some(l) => InList {
                        id: l.id,
                        style,
                        level: l.level + 1,
                    },
                    None => {
                        let continues = attrs.get(Ns::Text, "continue-numbering") == Some("true")
                            || attrs.get(Ns::Text, "continue-list").is_some();
                        let id = match (continues, ctx.last_list) {
                            (true, Some(id)) => id,
                            _ => {
                                ctx.next_list += 1;
                                ctx.next_list
                            }
                        };
                        ctx.last_list = Some(id);
                        InList {
                            id,
                            style,
                            level: 0,
                        }
                    }
                };
                r.children(|r, name, _| {
                    match (name.ns, name.local.as_str()) {
                        (Ns::Text, "list-item") => out.extend(blocks(r, ctx, Some(&inner))?),
                        // A header is an unnumbered paragraph of the list: plain paragraphs here.
                        (Ns::Text, "list-header") => out.extend(blocks(r, ctx, None)?),
                        _ => return Ok(Handled::No),
                    }
                    Ok(Handled::Yes)
                })?;
            }
            (Ns::Table, "table") => out.push(Block::Table(table(r, ctx)?)),
            (Ns::Text, "section") => out.extend(blocks(r, ctx, list)?),
            (
                Ns::Text,
                "table-of-content" | "alphabetical-index" | "illustration-index" | "table-index"
                | "object-index" | "user-index" | "bibliography",
            ) => {
                ctx.lose("index (kept as text)", &name.local);
                r.children(|r, name, _| match name.in_ns(Ns::Text, "index-body") {
                    true => {
                        out.extend(blocks(r, ctx, list)?);
                        Ok(Handled::Yes)
                    }
                    false => Ok(Handled::No),
                })?;
            }
            (Ns::Draw, _) => ctx.lose("picture or drawing", &name.local),
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    })?;
    Ok(out)
}

fn paragraph(
    r: &mut Reader<'_>,
    name: &Name,
    attrs: &Attrs,
    ctx: &mut Ctx<'_>,
) -> Result<Paragraph> {
    let style = attrs.get(Ns::Text, "style-name").unwrap_or("").to_owned();
    let mut p = Paragraph {
        align: ctx.paragraph_align(&style),
        ..Paragraph::default()
    };
    if name.local == "h" {
        let level = attrs
            .get(Ns::Text, "outline-level")
            .and_then(|v| v.parse::<u8>().ok())
            .unwrap_or(1);
        p.kind = match level {
            1..=MAX_HEADING => ParaKind::Heading(level),
            _ => {
                ctx.lose("heading below level 6", &style);
                ParaKind::Heading(MAX_HEADING)
            }
        };
    }
    let base = ctx.paragraph_props(&style);
    let mut collapser = Collapser::default();
    inline(r, ctx, &mut p, &mut collapser, base, None)?;
    collapser.end();
    flush(&mut p, &mut collapser, base, None);
    Ok(p)
}

/// Move what the collapser holds into the paragraph as one run.
fn flush(p: &mut Paragraph, c: &mut Collapser, props: Props, link: Option<&str>) {
    let text = c.take();
    p.push(&text, props.resolve(), link);
}

fn inline(
    r: &mut Reader<'_>,
    ctx: &mut Ctx<'_>,
    p: &mut Paragraph,
    c: &mut Collapser,
    props: Props,
    link: Option<&str>,
) -> Result<()> {
    // The collapser spans the whole paragraph, so whitespace collapses across spans; each change
    // of formatting moves what it holds into a run first.
    let state = std::cell::RefCell::new((p, c));
    let ctx = std::cell::RefCell::new(ctx);
    r.mixed(
        |t| state.borrow_mut().1.raw(t),
        |r, name, attrs| {
            let mut st = state.borrow_mut();
            let (p, c) = &mut *st;
            if name.ns == Ns::Text {
                match name.local.as_str() {
                    "s" => {
                        let n = attrs
                            .get(Ns::Text, "c")
                            .and_then(|v| v.parse::<usize>().ok())
                            .unwrap_or(1)
                            .min(10_000);
                        c.literal(&" ".repeat(n));
                    }
                    "tab" => c.literal("\t"),
                    "line-break" => c.literal("\n"),
                    "span" => {
                        flush(p, c, props, link);
                        let own = attrs
                            .get(Ns::Text, "style-name")
                            .map(|s| ctx.borrow().text_props(s, link.is_some()))
                            .unwrap_or_default();
                        let inner = own.over(props);
                        inline(r, &mut ctx.borrow_mut(), p, c, inner, link)?;
                        flush(p, c, inner, link);
                    }
                    "a" => {
                        flush(p, c, props, link);
                        let href = attrs.get(Ns::Xlink, "href").map(str::to_owned);
                        let target = href.as_deref().or(link);
                        // `text:a`'s own style is the link's look (`Internet_20_link`).
                        inline(r, &mut ctx.borrow_mut(), p, c, props, target)?;
                        flush(p, c, props, target);
                    }
                    "note" => {
                        ctx.borrow_mut().lose("footnote", "text:note");
                        return Ok(Handled::No);
                    }
                    "soft-page-break"
                    | "bookmark"
                    | "bookmark-start"
                    | "bookmark-end"
                    | "reference-mark"
                    | "reference-mark-start"
                    | "reference-mark-end"
                    | "alphabetical-index-mark"
                    | "toc-mark"
                    | "change-start"
                    | "change-end"
                    | "change" => {}
                    "ruby" => {
                        // The base text is the text; the annotation is not.
                        r.children(|r, name, _| match name.in_ns(Ns::Text, "ruby-base") {
                            true => {
                                inline(r, &mut ctx.borrow_mut(), p, c, props, link)?;
                                Ok(Handled::Yes)
                            }
                            false => Ok(Handled::No),
                        })?;
                    }
                    // Fields and anything else textual: their content is what the reader sees.
                    _ => inline(r, &mut ctx.borrow_mut(), p, c, props, link)?,
                }
                return Ok(Handled::Yes);
            }
            match name.ns {
                Ns::Draw => ctx.borrow_mut().lose("picture or drawing", &name.local),
                Ns::Office if name.local == "annotation" => {
                    ctx.borrow_mut().lose("comments", "office:annotation")
                }
                _ => {}
            }
            Ok(Handled::No)
        },
    )
}

fn table(r: &mut Reader<'_>, ctx: &mut Ctx<'_>) -> Result<Table> {
    let mut rows: Vec<Vec<TableCell>> = Vec::new();
    rows_of(r, ctx, &mut rows)?;
    let mut t = Table { rows };
    t.normalise();
    Ok(t)
}

fn rows_of(r: &mut Reader<'_>, ctx: &mut Ctx<'_>, rows: &mut Vec<Vec<TableCell>>) -> Result<()> {
    r.children(|r, name, attrs| {
        if name.ns != Ns::Table {
            return Ok(Handled::No);
        }
        match name.local.as_str() {
            "table-header-rows" | "table-rows" | "table-row-group" => rows_of(r, ctx, rows)?,
            "table-row" => {
                let repeat = attrs
                    .get(Ns::Table, "number-rows-repeated")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, 1000);
                let mut row = Vec::new();
                r.children(|r, name, attrs| {
                    let covered = name.in_ns(Ns::Table, "covered-table-cell");
                    if !(covered || name.in_ns(Ns::Table, "table-cell")) {
                        return Ok(Handled::No);
                    }
                    let n = |a: &str| {
                        attrs
                            .get(Ns::Table, a)
                            .and_then(|v| v.parse::<u32>().ok())
                            .unwrap_or(1)
                            .clamp(1, 1000)
                    };
                    let cell = TableCell {
                        blocks: blocks(r, ctx, None)?,
                        col_span: n("number-columns-spanned"),
                        row_span: n("number-rows-spanned"),
                        covered,
                    };
                    for _ in 0..n("number-columns-repeated") {
                        row.push(cell.clone());
                    }
                    Ok(Handled::Yes)
                })?;
                for _ in 0..repeat {
                    rows.push(row.clone());
                }
            }
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    })
}
