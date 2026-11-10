// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The neutral text document: paragraphs, headings, list items and tables of them, with the
//! character formatting both formats spell the same way. Formatting is **direct** — resolved onto
//! runs — except for what is structure (a heading's level, a list's kind), which is kept as
//! structure so each writer can use its format's own semantic markup.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paragraph {
    pub kind: ParaKind,
    pub align: Option<Align>,
    pub runs: Vec<Run>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParaKind {
    #[default]
    Normal,
    /// 1 to 6 — the levels both formats' default templates name.
    Heading(u8),
    /// An item of list `list` (consecutive items of one list share the id), at nesting `level`
    /// (0-based, below [`MAX_LIST_LEVEL`]).
    ListItem { list: u32, ordered: bool, level: u8 },
}

/// Word allows nine levels (0–8), ODF ten; nine is what both can hold.
pub const MAX_LIST_LEVEL: u8 = 9;
pub const MAX_HEADING: u8 = 6;

/// A paragraph's alignment where it is not the default. Start — left in a left-to-right
/// paragraph — *is* the default in both formats, so it has no variant: one meaning, one spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Center,
    End,
    Justify,
}

/// A run of text with one formatting. `text` may hold `\t` (a tab) and `\n` (a line break inside
/// the paragraph); nothing else below U+0020.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub text: String,
    pub style: RunStyle,
    /// A hyperlink's target: a URL, or `#name` for a place in the document.
    pub link: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RunStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub position: Position,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Position {
    #[default]
    Normal,
    Superscript,
    Subscript,
}

/// A table as a full grid: every row has one entry per grid column, and a position a span
/// covers is a [`TableCell`] with `covered` set — how ODF spells it, and what Word's
/// `gridSpan`/`vMerge` expand to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub rows: Vec<Vec<TableCell>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableCell {
    pub blocks: Vec<Block>,
    pub col_span: u32,
    pub row_span: u32,
    pub covered: bool,
}

impl Default for TableCell {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            col_span: 1,
            row_span: 1,
            covered: false,
        }
    }
}

impl Document {
    /// Drop an empty paragraph that directly follows a table ending the document: Word requires
    /// it and LibreOffice writes it, so it is the formats' filler rather than the author's
    /// content, and the canonical model does not have it.
    pub fn drop_table_filler(&mut self) {
        let n = self.blocks.len();
        if n >= 2
            && matches!(self.blocks[n - 2], Block::Table(_))
            && matches!(&self.blocks[n - 1], Block::Paragraph(p)
                if p.runs.is_empty() && p.kind == ParaKind::Normal && p.align.is_none())
        {
            self.blocks.pop();
        }
    }

    /// Renumber lists 1, 2, 3 … in order of first appearance — the canonical form every reader
    /// returns, since each format's own ids (`w:numId`, document order) mean nothing outside it.
    pub fn number_lists(&mut self) {
        fn walk(blocks: &mut [Block], ids: &mut Vec<u32>) {
            for block in blocks {
                match block {
                    Block::Paragraph(p) => {
                        if let ParaKind::ListItem { list, .. } = &mut p.kind {
                            let i = match ids.iter().position(|id| id == list) {
                                Some(i) => i,
                                None => {
                                    ids.push(*list);
                                    ids.len() - 1
                                }
                            };
                            *list = i as u32 + 1;
                        }
                    }
                    Block::Table(t) => {
                        for cell in t.rows.iter_mut().flatten() {
                            walk(&mut cell.blocks, ids);
                        }
                    }
                }
            }
        }
        walk(&mut self.blocks, &mut Vec::new());
    }
}

impl Paragraph {
    pub fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }

    /// Append text, merging with the last run when formatting and link agree.
    pub fn push(&mut self, text: &str, style: RunStyle, link: Option<&str>) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.runs.last_mut()
            && last.style == style
            && last.link.as_deref() == link
        {
            last.text.push_str(text);
            return;
        }
        self.runs.push(Run {
            text: text.to_owned(),
            style,
            link: link.map(str::to_owned),
        });
    }
}

impl Table {
    /// The grid's width.
    pub fn columns(&self) -> usize {
        self.rows.iter().map(Vec::len).max().unwrap_or(0)
    }

    /// Make the grid rectangular and the spans consistent: pad short rows, clamp spans to the
    /// grid, mark what each span covers, and clear what no span covers. Every reader ends here,
    /// so every writer can rely on it.
    pub fn normalise(&mut self) {
        let width = self.columns().max(1);
        for row in &mut self.rows {
            row.resize_with(width, TableCell::default);
        }
        let height = self.rows.len();
        let mut covered = vec![vec![false; width]; height];
        for r in 0..height {
            for c in 0..width {
                if covered[r][c] {
                    continue;
                }
                let cell = &mut self.rows[r][c];
                cell.covered = false;
                cell.col_span = cell.col_span.clamp(1, (width - c) as u32);
                cell.row_span = cell.row_span.clamp(1, (height - r) as u32);
                for (dr, row) in covered
                    .iter_mut()
                    .enumerate()
                    .skip(r)
                    .take(cell.row_span as usize)
                {
                    for (dc, slot) in row
                        .iter_mut()
                        .enumerate()
                        .skip(c)
                        .take(cell.col_span as usize)
                    {
                        if (dr, dc) != (r, c) {
                            *slot = true;
                        }
                    }
                }
            }
        }
        for (r, row) in self.rows.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                if covered[r][c] {
                    cell.covered = true;
                    cell.col_span = 1;
                    cell.row_span = 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_merge_when_nothing_tells_them_apart() {
        let mut p = Paragraph::default();
        let bold = RunStyle {
            bold: true,
            ..RunStyle::default()
        };
        p.push("a", RunStyle::default(), None);
        p.push("b", RunStyle::default(), None);
        p.push("c", bold, None);
        p.push("d", bold, Some("https://x"));
        assert_eq!(p.runs.len(), 3);
        assert_eq!(p.text(), "abcd");
    }

    #[test]
    fn a_table_normalises_to_a_full_grid() {
        let mut t = Table {
            rows: vec![
                vec![TableCell {
                    col_span: 2,
                    row_span: 2,
                    ..TableCell::default()
                }],
                vec![
                    TableCell::default(),
                    TableCell::default(),
                    TableCell::default(),
                ],
            ],
        };
        t.normalise();
        assert_eq!(t.columns(), 3);
        assert!(t.rows[0][1].covered && t.rows[1][0].covered && t.rows[1][1].covered);
        assert!(!t.rows[0][2].covered && !t.rows[1][2].covered);
    }
}
