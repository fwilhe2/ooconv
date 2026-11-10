// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! One formula AST, two dialects. **Parsed, never rewritten with string surgery**: every formula
//! is read into an [`Expr`] by its dialect's parser and printed by the other dialect's printer, so
//! a translated formula is one the target's own grammar produces — or it is not translated, and
//! the [`Refusal`] says which class of construct stopped it.
//!
//! | Excel (`<f>`) | OpenFormula (`of:=`) |
//! |---|---|
//! | `A1`, `$A$1`, `Sheet1!A1`, `'My Sheet'!A1` | `[.A1]`, `[.$A$1]`, `[Sheet1.A1]`, `['My Sheet'.A1]` |
//! | `Sheet1:Sheet3!A1` | `[Sheet1.A1:Sheet3.A1]` (Part 4 §4.8's cuboid) |
//! | `A:A`, `1:1` | `[.A:.A]`, `[.1:.1]` |
//! | `,` between arguments, `{1,2;3,4}` | `;` (§5.6), `{1;2|3;4}` (§5.13) |
//! | `TRUE`, `FALSE` | `TRUE()`, `FALSE()` (§6.15) |
//! | `A1:A2 B1:B2` (intersection), `(A1,B1)` (union) | `[.A1:.A2]![.B1:.B2]`, `[.A1]~[.B1]` |
//! | `_xlfn.XLOOKUP`, `CEILING` | `COM.MICROSOFT.XLOOKUP`, `COM.MICROSOFT.CEILING` (`names.rs`) |
//!
//! The precedence table is §5.5 Table 1, which Excel agrees with, surprises included: prefix `-`
//! binds tighter than `^` (`-2^2` is 4) and `^` is left-associative (`2^3^2` is 64). Parentheses
//! the source wrote are kept as [`Expr::Paren`], so the printers never need to invent any.

pub mod excel;
pub mod names;
pub mod odf;

use super::model::{CellError, MAX_COLS, MAX_ROWS};

/// Deeper than any formula a person writes, shallower than a stack overflow in the printers.
pub const MAX_DEPTH: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Number(f64),
    Text(String),
    Bool(bool),
    Error(CellError),
    Ref(Reference),
    /// A defined name.
    Name(String),
    /// A function call, by its **OpenFormula** name (`names.rs` maps Excel's).
    Call {
        name: String,
        args: Vec<Expr>,
    },
    Prefix(Op, Box<Expr>),
    Postfix(Op, Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    Paren(Box<Expr>),
    /// An omitted argument: `OFFSET(A1,1,,2)`.
    Empty,
    /// An inline array, by rows.
    Array(Vec<Vec<Expr>>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Percent,
    /// `:` between two computed references.
    Range,
    Intersect,
    Union,
}

impl Op {
    /// The operator's spelling where the two dialects agree.
    fn common(self) -> &'static str {
        match self {
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
            Op::Pow => "^",
            Op::Concat => "&",
            Op::Eq => "=",
            Op::Ne => "<>",
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::Percent => "%",
            Op::Range => ":",
            Op::Intersect => "!",
            Op::Union => "~",
        }
    }
}

// Binding powers, shared by both parsers.
pub(crate) const CMP_BP: u8 = 10;
pub(crate) const CONCAT_BP: u8 = 20;
pub(crate) const ADD_BP: u8 = 30;
pub(crate) const MUL_BP: u8 = 40;
pub(crate) const POW_BP: u8 = 50;
pub(crate) const PREFIX_BP: u8 = 70;
pub(crate) const UNION_BP: u8 = 80;
pub(crate) const INTERSECT_BP: u8 = 90;
pub(crate) const RANGE_BP: u8 = 100;

/// A reference: one cell, a range, a whole column or row, possibly across sheets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub start: CellRef,
    pub end: Option<CellRef>,
}

/// One end of a reference. An axis may be missing only in a range (`A:A`, `1:1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellRef {
    /// `None` means the sheet the formula is on — or, for an end, the start's sheet.
    pub sheet: Option<String>,
    pub col: Option<Axis>,
    pub row: Option<Axis>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Axis {
    /// 0-based.
    pub index: u32,
    pub absolute: bool,
}

/// Why a formula was not translated: a **class**, so an exclusion can be counted and looked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Refusal {
    /// `Table1[Column]` — a structured reference; the model has no tables.
    StructuredReference,
    /// `[1]Sheet1!A1`, `'file:///…'#$Sheet1.A1` — another workbook. Never followed.
    ExternalLink,
    /// A formula in a namespace other than OpenFormula's (`msoxl:=`, `ooow:=`).
    ForeignSyntax,
    /// A reference whose two ends say different sheets in a way the target cannot spell.
    UnspellableReference,
    /// A function whose spelling in the target format is not measured (`names::UNSPELLED`).
    UnspelledFunction,
    /// Nothing this grammar recognises, or nested past [`MAX_DEPTH`].
    Syntax,
}

impl Refusal {
    /// The [`crate::Report`] class a refusal is counted under.
    pub fn label(self) -> &'static str {
        match self {
            Refusal::StructuredReference => "formula: structured reference",
            Refusal::ExternalLink => "formula: external link",
            Refusal::ForeignSyntax => "formula: foreign syntax",
            Refusal::UnspellableReference => "formula: unspellable reference",
            Refusal::UnspelledFunction => "formula: function with no known spelling",
            Refusal::Syntax => "formula: unreadable",
        }
    }
}

impl Expr {
    /// Move every relative reference by `(rows, cols)` — how a shared formula's dependents are
    /// derived from its master (ECMA-376 §18.3.1.40). An axis moved off the grid makes the whole
    /// reference `#REF!`, as Excel does.
    pub fn shifted(&self, rows: i64, cols: i64) -> Expr {
        match self {
            Expr::Ref(r) => match shift_ref(r, rows, cols) {
                Some(r) => Expr::Ref(r),
                None => Expr::Error(CellError::Ref),
            },
            Expr::Call { name, args } => Expr::Call {
                name: name.clone(),
                args: args.iter().map(|a| a.shifted(rows, cols)).collect(),
            },
            Expr::Prefix(op, e) => Expr::Prefix(*op, Box::new(e.shifted(rows, cols))),
            Expr::Postfix(op, e) => Expr::Postfix(*op, Box::new(e.shifted(rows, cols))),
            Expr::Binary(op, a, b) => Expr::Binary(
                *op,
                Box::new(a.shifted(rows, cols)),
                Box::new(b.shifted(rows, cols)),
            ),
            Expr::Paren(e) => Expr::Paren(Box::new(e.shifted(rows, cols))),
            Expr::Array(rows_) => Expr::Array(
                rows_
                    .iter()
                    .map(|r| r.iter().map(|e| e.shifted(rows, cols)).collect())
                    .collect(),
            ),
            leaf => leaf.clone(),
        }
    }
}

fn shift_ref(r: &Reference, rows: i64, cols: i64) -> Option<Reference> {
    let axis = |a: Option<Axis>, by: i64, max: u32| -> Option<Option<Axis>> {
        match a {
            None => Some(None),
            Some(a) if a.absolute => Some(Some(a)),
            Some(a) => {
                let i = i64::from(a.index) + by;
                (0..i64::from(max)).contains(&i).then_some(Some(Axis {
                    index: i as u32,
                    absolute: false,
                }))
            }
        }
    };
    let cell = |c: &CellRef| -> Option<CellRef> {
        Some(CellRef {
            sheet: c.sheet.clone(),
            col: axis(c.col, cols, MAX_COLS)?,
            row: axis(c.row, rows, MAX_ROWS)?,
        })
    };
    Some(Reference {
        start: cell(&r.start)?,
        end: match &r.end {
            Some(e) => Some(cell(e)?),
            None => None,
        },
    })
}

/// `0` → `A`, `16383` → `XFD`.
pub fn column_letters(mut col: u32) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).expect("ASCII")
}

/// `A` → 0, `XFD` → 16383; `None` past the grid or for anything but letters.
pub fn column_index(letters: &str) -> Option<u32> {
    if letters.is_empty() || letters.len() > 3 || !letters.bytes().all(|b| b.is_ascii_alphabetic())
    {
        return None;
    }
    let mut col: u32 = 0;
    for c in letters.bytes() {
        col = col * 26 + u32::from(c.to_ascii_uppercase() - b'A' + 1);
    }
    (col <= MAX_COLS).then(|| col - 1)
}

/// `1` → 0; `None` past the grid, for `0` or a leading zero.
pub fn row_index(digits: &str) -> Option<u32> {
    if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let row: u32 = digits.parse().ok()?;
    (row <= MAX_ROWS).then(|| row - 1)
}

/// `A1` for a 0-based position.
pub fn a1(row: u32, col: u32) -> String {
    format!("{}{}", column_letters(col), row + 1)
}

/// A number as both dialects read it: shortest round-trip digits, exponent form only where the
/// plain form would be long.
pub fn number(n: f64) -> String {
    let a = n.abs();
    if a != 0.0 && !(1e-5..1e15).contains(&a) {
        let s = format!("{n:e}");
        let (mantissa, exp) = s.split_once('e').expect("exponent form");
        match exp.strip_prefix('-') {
            Some(e) => format!("{mantissa}E-{e}"),
            None => format!("{mantissa}E+{exp}"),
        }
    } else {
        format!("{n}")
    }
}

/// A string literal, `"` doubled — the same in both dialects.
pub(crate) fn quote_string(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Shared character cursor for the two parsers.
pub(crate) struct Cursor<'a> {
    pub src: &'a [char],
    pub at: usize,
    pub nesting: usize,
}

impl Cursor<'_> {
    pub fn peek(&self) -> Option<char> {
        self.src.get(self.at).copied()
    }

    pub fn peek_at(&self, offset: usize) -> Option<char> {
        self.src.get(self.at + offset).copied()
    }

    pub fn eat(&mut self, c: char) -> bool {
        let found = self.peek() == Some(c);
        if found {
            self.at += 1;
        }
        found
    }

    pub fn space(&mut self) -> bool {
        let start = self.at;
        while matches!(self.peek(), Some(' ' | '\t' | '\r' | '\n')) {
            self.at += 1;
        }
        self.at > start
    }

    /// The binary operator at the cursor that both dialects spell alike.
    pub fn common_infix(&self) -> Option<(Op, u8, usize)> {
        Some(match self.peek()? {
            '<' if self.peek_at(1) == Some('>') => (Op::Ne, CMP_BP, 2),
            '<' if self.peek_at(1) == Some('=') => (Op::Le, CMP_BP, 2),
            '>' if self.peek_at(1) == Some('=') => (Op::Ge, CMP_BP, 2),
            '<' => (Op::Lt, CMP_BP, 1),
            '>' => (Op::Gt, CMP_BP, 1),
            '=' => (Op::Eq, CMP_BP, 1),
            '&' => (Op::Concat, CONCAT_BP, 1),
            '+' => (Op::Add, ADD_BP, 1),
            '-' => (Op::Sub, ADD_BP, 1),
            '*' => (Op::Mul, MUL_BP, 1),
            '/' => (Op::Div, MUL_BP, 1),
            '^' => (Op::Pow, POW_BP, 1),
            ':' => (Op::Range, RANGE_BP, 1),
            _ => return None,
        })
    }

    pub fn string(&mut self) -> Result<Expr, Refusal> {
        self.at += 1;
        let mut out = String::new();
        loop {
            match self.peek().ok_or(Refusal::Syntax)? {
                '"' if self.peek_at(1) == Some('"') => {
                    out.push('"');
                    self.at += 2;
                }
                '"' => {
                    self.at += 1;
                    return Ok(Expr::Text(out));
                }
                c => {
                    out.push(c);
                    self.at += 1;
                }
            }
        }
    }

    pub fn error_literal(&mut self) -> Expr {
        let start = self.at;
        self.at += 1;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || matches!(c, '/' | '_'))
        {
            self.at += 1;
        }
        if matches!(self.peek(), Some('!' | '?')) {
            self.at += 1;
        }
        let name: String = self.src[start..self.at].iter().collect();
        Expr::Error(CellError::from_name(&name).unwrap_or(CellError::Name))
    }

    pub fn number(&mut self) -> Result<Expr, Refusal> {
        let start = self.at;
        let digits = |c: &mut Self| {
            while matches!(c.peek(), Some(d) if d.is_ascii_digit()) {
                c.at += 1;
            }
        };
        digits(self);
        if self.eat('.') {
            digits(self);
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let save = self.at;
            self.at += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.at += 1;
            }
            if matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                digits(self);
            } else {
                self.at = save;
            }
        }
        let text: String = self.src[start..self.at].iter().collect();
        match text.parse::<f64>() {
            Ok(n) if n.is_finite() => Ok(Expr::Number(n)),
            _ => Err(Refusal::Syntax),
        }
    }

    /// An identifier: letters, digits, `_`, `.` — a function name, a defined name.
    pub fn word(&mut self) -> Option<String> {
        let start = self.at;
        while matches!(self.peek(), Some(c) if c.is_alphanumeric() || matches!(c, '_' | '.' | '\\'))
        {
            self.at += 1;
        }
        (self.at > start).then(|| self.src[start..self.at].iter().collect())
    }

    /// `$`? letters, `$`? digits — either half may be missing, not both. The cursor is left
    /// unmoved on `None`.
    pub fn cell_part(&mut self) -> Option<(Option<Axis>, Option<Axis>)> {
        let save = self.at;
        let col_abs = self.eat('$');
        let start = self.at;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphabetic()) {
            self.at += 1;
        }
        let letters: String = self.src[start..self.at].iter().collect();
        let mark = self.at;
        let row_abs = self.eat('$');
        let start = self.at;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.at += 1;
        }
        let digits: String = self.src[start..self.at].iter().collect();
        if digits.is_empty() {
            self.at = mark;
        }
        let col = match letters.is_empty() {
            true => None,
            false => match column_index(&letters) {
                Some(index) => Some(Axis {
                    index,
                    absolute: col_abs,
                }),
                None => {
                    self.at = save;
                    return None;
                }
            },
        };
        let row = match digits.is_empty() {
            true => None,
            false => match row_index(&digits) {
                Some(index) => Some(Axis {
                    index,
                    absolute: row_abs || (letters.is_empty() && col_abs),
                }),
                None => {
                    self.at = save;
                    return None;
                }
            },
        };
        if col.is_none() && row.is_none() {
            self.at = save;
            return None;
        }
        Some((col, row))
    }
}

/// Whether a cell part is followed by something that makes it a name instead (`A1B`, `LOG10_x`).
pub(crate) fn continues_word(c: Option<char>) -> bool {
    matches!(c, Some(c) if c.is_alphanumeric() || matches!(c, '_' | '.' | '('))
}

/// Check every reference in a tree lies on the grid; used by the writers' debug assertions.
pub fn depth(expr: &Expr) -> usize {
    match expr {
        Expr::Call { args, .. } => 1 + args.iter().map(depth).max().unwrap_or(0),
        Expr::Prefix(_, e) | Expr::Postfix(_, e) | Expr::Paren(e) => 1 + depth(e),
        Expr::Binary(_, a, b) => 1 + depth(a).max(depth(b)),
        Expr::Array(rows) => 1 + rows.iter().flatten().map(depth).max().unwrap_or(0),
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_both_ways_at_the_edges() {
        for (i, s) in [
            (0, "A"),
            (25, "Z"),
            (26, "AA"),
            (701, "ZZ"),
            (702, "AAA"),
            (16383, "XFD"),
        ] {
            assert_eq!(column_letters(i), s);
            assert_eq!(column_index(s), Some(i));
        }
        assert_eq!(column_index("XFE"), None);
        assert_eq!(row_index("1048576"), Some(1_048_575));
        assert_eq!(row_index("1048577"), None);
        assert_eq!(row_index("01"), None);
    }

    #[test]
    fn numbers_print_in_a_form_both_grammars_read() {
        assert_eq!(number(1.0), "1");
        assert_eq!(number(0.1), "0.1");
        assert_eq!(number(-2.5), "-2.5");
        assert_eq!(number(1e21), "1E+21");
        assert_eq!(number(1.5e-7), "1.5E-7");
        assert_eq!(number(123456789012345.0), "123456789012345");
        for n in [1e21, 1.5e-7, 0.1, 1.0 / 3.0, f64::MAX, f64::MIN_POSITIVE] {
            assert_eq!(number(n).parse::<f64>().unwrap(), n, "{n} round-trips");
        }
    }

    #[test]
    fn a_shared_formula_shifts_only_what_is_relative() {
        let e = excel::parse("A1+$B$2+C$3+$D4+SUM(A:A)").unwrap();
        let moved = e.shifted(1, 1);
        assert_eq!(excel::print(&moved).unwrap(), "B2+$B$2+D$3+$D5+SUM(B:B)");
        // Off the top edge is a broken reference, not a wrap-around.
        assert_eq!(
            excel::print(&e.shifted(-1, 0)).unwrap(),
            "#REF!+$B$2+C$3+$D3+SUM(A:A)"
        );
    }
}
