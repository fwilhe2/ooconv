// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! OpenFormula as stored in `table:formula` (ODF 1.4 Part 4 §5): the `of:` namespace prefix and
//! `=`, references in brackets, `;` between arguments, `!` and `~` for intersection and union.

use super::{
    Axis, CellRef, Cursor, Expr, INTERSECT_BP, MAX_DEPTH, Op, PREFIX_BP, Reference, Refusal,
    UNION_BP, column_letters, number, quote_string,
};
use crate::sheet::model::CellError;

/// A `table:formula` attribute → an expression. The namespace prefix decides the grammar: `of:`
/// is OpenFormula; no prefix is read as OpenFormula too, as LibreOffice does; anything else is
/// another application's syntax and refused.
pub fn parse_attribute(attr: &str) -> Result<Expr, Refusal> {
    let body = match attr.split_once(':') {
        Some((ns, rest)) if !ns.contains(['=', '[', '(', '"']) => match ns {
            "of" => rest,
            _ => return Err(Refusal::ForeignSyntax),
        },
        _ => attr,
    };
    parse(body)
}

/// OpenFormula text, with or without its leading `=`.
pub fn parse(src: &str) -> Result<Expr, Refusal> {
    let trimmed = src.trim();
    let chars: Vec<char> = trimmed
        .strip_prefix('=')
        .unwrap_or(trimmed)
        .chars()
        .collect();
    if chars.is_empty() {
        return Err(Refusal::Syntax);
    }
    let mut p = Parser {
        c: Cursor {
            src: &chars,
            at: 0,
            nesting: 0,
        },
    };
    let expr = p.expr(0)?;
    p.c.space();
    match p.c.at == chars.len() {
        true => Ok(expr),
        false => Err(Refusal::Syntax),
    }
}

struct Parser<'a> {
    c: Cursor<'a>,
}

impl Parser<'_> {
    fn recurse<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, Refusal>,
    ) -> Result<T, Refusal> {
        if self.c.nesting >= MAX_DEPTH {
            return Err(Refusal::Syntax);
        }
        self.c.nesting += 1;
        let out = f(self);
        self.c.nesting -= 1;
        out
    }

    fn infix(&self) -> Option<(Op, u8, usize)> {
        match self.c.peek()? {
            '!' => Some((Op::Intersect, INTERSECT_BP, 1)),
            '~' => Some((Op::Union, UNION_BP, 1)),
            _ => self.c.common_infix(),
        }
    }

    fn expr(&mut self, min_bp: u8) -> Result<Expr, Refusal> {
        let mut lhs = self.prefix()?;
        loop {
            self.c.space();
            let Some((op, bp, len)) = self.infix() else {
                break;
            };
            if bp < min_bp {
                break;
            }
            if super::depth(&lhs) >= MAX_DEPTH {
                return Err(Refusal::Syntax);
            }
            self.c.at += len;
            let rhs = self.recurse(|p| p.expr(bp + 1))?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn prefix(&mut self) -> Result<Expr, Refusal> {
        self.c.space();
        match self.c.peek() {
            Some(sign @ ('-' | '+')) => {
                self.c.at += 1;
                let op = if sign == '-' { Op::Sub } else { Op::Add };
                let inner = self.recurse(|p| p.expr(PREFIX_BP))?;
                Ok(Expr::Prefix(op, Box::new(inner)))
            }
            _ => {
                let mut expr = self.primary()?;
                loop {
                    let save = self.c.at;
                    self.c.space();
                    if !self.c.eat('%') {
                        self.c.at = save;
                        break;
                    }
                    expr = Expr::Postfix(Op::Percent, Box::new(expr));
                }
                Ok(expr)
            }
        }
    }

    fn primary(&mut self) -> Result<Expr, Refusal> {
        self.c.space();
        match self.c.peek().ok_or(Refusal::Syntax)? {
            '[' => self.reference(),
            '{' => self.array(),
            '"' => self.c.string(),
            '#' => Ok(self.c.error_literal()),
            '(' => {
                self.c.at += 1;
                let inner = self.recurse(|p| p.expr(0))?;
                self.c.space();
                match self.c.eat(')') {
                    true => Ok(Expr::Paren(Box::new(inner))),
                    false => Err(Refusal::Syntax),
                }
            }
            c if c.is_ascii_digit() || c == '.' => self.c.number(),
            c if c.is_alphabetic() || c == '_' || c == '\\' => {
                let word = self.c.word().ok_or(Refusal::Syntax)?;
                self.c.space();
                if self.c.peek() == Some('(') {
                    return self.recurse(|p| p.call(&word));
                }
                Ok(match word.to_ascii_uppercase().as_str() {
                    // Not OpenFormula, but LibreOffice reads them, and so must a tolerant reader.
                    "TRUE" => Expr::Bool(true),
                    "FALSE" => Expr::Bool(false),
                    _ => Expr::Name(word),
                })
            }
            _ => Err(Refusal::Syntax),
        }
    }

    fn call(&mut self, word: &str) -> Result<Expr, Refusal> {
        self.c.at += 1;
        let name = word.to_ascii_uppercase();
        let mut args = Vec::new();
        self.c.space();
        if !self.c.eat(')') {
            loop {
                self.c.space();
                args.push(match matches!(self.c.peek(), Some(';' | ')')) {
                    true => Expr::Empty,
                    false => self.expr(0)?,
                });
                self.c.space();
                if self.c.eat(';') {
                    continue;
                }
                if self.c.eat(')') {
                    break;
                }
                return Err(Refusal::Syntax);
            }
        }
        Ok(match name.as_str() {
            "TRUE" if args.is_empty() => Expr::Bool(true),
            "FALSE" if args.is_empty() => Expr::Bool(false),
            _ => Expr::Call { name, args },
        })
    }

    /// `{1;2|3;4}` (§5.13): `;` between columns, `|` between rows.
    fn array(&mut self) -> Result<Expr, Refusal> {
        self.c.at += 1;
        let mut rows = vec![Vec::new()];
        loop {
            self.c.space();
            let item = match self.c.peek().ok_or(Refusal::Syntax)? {
                '"' => self.c.string()?,
                '#' => self.c.error_literal(),
                '-' => {
                    self.c.at += 1;
                    match self.c.number()? {
                        Expr::Number(n) => Expr::Number(-n),
                        _ => return Err(Refusal::Syntax),
                    }
                }
                c if c.is_ascii_digit() || c == '.' => self.c.number()?,
                _ => {
                    let word = self.c.word().map(|w| w.to_ascii_uppercase());
                    // `TRUE()` inside an array is legal OpenFormula.
                    if self.c.peek() == Some('(') && self.c.peek_at(1) == Some(')') {
                        self.c.at += 2;
                    }
                    match word.as_deref() {
                        Some("TRUE") => Expr::Bool(true),
                        Some("FALSE") => Expr::Bool(false),
                        _ => return Err(Refusal::Syntax),
                    }
                }
            };
            rows.last_mut().expect("a row").push(item);
            self.c.space();
            match self.c.peek() {
                Some(';') => self.c.at += 1,
                Some('|') => {
                    self.c.at += 1;
                    rows.push(Vec::new());
                }
                Some('}') => {
                    self.c.at += 1;
                    break;
                }
                _ => return Err(Refusal::Syntax),
            }
        }
        let width = rows[0].len();
        match rows.iter().all(|r| r.len() == width) {
            true => Ok(Expr::Array(rows)),
            false => Err(Refusal::Syntax),
        }
    }

    /// `[.A1]`, `[$Sheet1.A1:.B2]`, `['My Sheet'.A:.A]`, `[.#REF!]`, `['file:…'#$S.A1]`.
    fn reference(&mut self) -> Result<Expr, Refusal> {
        self.c.at += 1;
        let start = self.c.at;
        let mut depth_quote = false;
        while let Some(c) = self.c.peek() {
            match c {
                '\'' => depth_quote = !depth_quote,
                ']' if !depth_quote => break,
                _ => {}
            }
            self.c.at += 1;
        }
        let body: String = self.c.src[start..self.c.at].iter().collect();
        if !self.c.eat(']') {
            return Err(Refusal::Syntax);
        }
        if body.contains("#REF!") {
            return Ok(Expr::Error(CellError::Ref));
        }
        let chars: Vec<char> = body.chars().collect();
        let mut r = Cursor {
            src: &chars,
            at: 0,
            nesting: 0,
        };
        let first = ref_end(&mut r)?;
        let second = match r.eat(':') {
            true => Some(ref_end(&mut r)?),
            false => None,
        };
        if r.at != chars.len() {
            return Err(Refusal::Syntax);
        }
        let reference = Reference {
            start: first,
            end: second.map(|mut e| {
                // An end without a sheet inherits the start's (§5.8); keep the model canonical.
                if e.sheet.is_none() {
                    e.sheet = None;
                }
                e
            }),
        };
        if reference.end.is_none()
            && (reference.start.col.is_none() || reference.start.row.is_none())
        {
            return Err(Refusal::Syntax);
        }
        if let Some(end) = &reference.end
            && (end.col.is_some() != reference.start.col.is_some()
                || end.row.is_some() != reference.start.row.is_some())
        {
            return Err(Refusal::Syntax);
        }
        Ok(Expr::Ref(reference))
    }
}

/// One end of a bracketed reference: an optional sheet, a `.`, and a cell part.
fn ref_end(r: &mut Cursor<'_>) -> Result<CellRef, Refusal> {
    let sheet = match r.peek() {
        Some('.') => None,
        _ => {
            r.eat('$');
            let name = match r.peek() {
                Some('\'') => {
                    r.at += 1;
                    let mut out = String::new();
                    loop {
                        match r.peek().ok_or(Refusal::Syntax)? {
                            '\'' if r.peek_at(1) == Some('\'') => {
                                out.push('\'');
                                r.at += 2;
                            }
                            '\'' => {
                                r.at += 1;
                                break;
                            }
                            c => {
                                out.push(c);
                                r.at += 1;
                            }
                        }
                    }
                    out
                }
                _ => {
                    let start = r.at;
                    while matches!(r.peek(), Some(c) if !matches!(c, '.' | ']' | ':' | '#' | '\'' | ' ' | '$'))
                    {
                        r.at += 1;
                    }
                    r.src[start..r.at].iter().collect()
                }
            };
            // `'file:///book.ods'#$Sheet1.A1` — another document.
            if r.peek() == Some('#') {
                return Err(Refusal::ExternalLink);
            }
            if name.is_empty() {
                return Err(Refusal::Syntax);
            }
            Some(name)
        }
    };
    if !r.eat('.') {
        return Err(Refusal::Syntax);
    }
    let (col, row) = r.cell_part().ok_or(Refusal::Syntax)?;
    Ok(CellRef { sheet, col, row })
}

// ---- printing ----

/// The `table:formula` attribute value: `of:=` and the expression.
pub fn print_attribute(expr: &Expr) -> String {
    format!("of:={}", print(expr))
}

pub fn print(expr: &Expr) -> String {
    let mut out = String::new();
    write(expr, &mut out);
    out
}

fn write(expr: &Expr, out: &mut String) {
    match expr {
        Expr::Number(n) => out.push_str(&number(*n)),
        Expr::Text(s) => out.push_str(&quote_string(s)),
        Expr::Bool(b) => out.push_str(if *b { "TRUE()" } else { "FALSE()" }),
        Expr::Error(e) => out.push_str(e.as_str()),
        Expr::Ref(r) => reference(r, out),
        Expr::Name(n) => out.push_str(n),
        Expr::Call { name, args } => {
            out.push_str(name);
            out.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    out.push(';');
                }
                write(a, out);
            }
            out.push(')');
        }
        Expr::Prefix(op, e) => {
            out.push_str(op.common());
            write(e, out);
        }
        Expr::Postfix(op, e) => {
            write(e, out);
            out.push_str(op.common());
        }
        Expr::Binary(op, a, b) => {
            write(a, out);
            out.push_str(op.common());
            write(b, out);
        }
        Expr::Paren(e) => {
            out.push('(');
            write(e, out);
            out.push(')');
        }
        Expr::Empty => {}
        Expr::Array(rows) => {
            out.push('{');
            for (i, row) in rows.iter().enumerate() {
                if i > 0 {
                    out.push('|');
                }
                for (j, item) in row.iter().enumerate() {
                    if j > 0 {
                        out.push(';');
                    }
                    write(item, out);
                }
            }
            out.push('}');
        }
    }
}

/// A sheet name inside a reference: bare when it holds nothing §5.8's `SheetName` forbids
/// unquoted, quoted with `'` doubled otherwise.
pub fn sheet_name(name: &str) -> String {
    let bare = !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !name.chars().next().is_some_and(|c| c.is_ascii_digit());
    match bare {
        true => name.to_owned(),
        false => format!("'{}'", name.replace('\'', "''")),
    }
}

fn cell(c: &CellRef, out: &mut String) {
    if let Some(sheet) = &c.sheet {
        out.push_str(&sheet_name(sheet));
    }
    out.push('.');
    if let Some(Axis { index, absolute }) = c.col {
        if absolute {
            out.push('$');
        }
        out.push_str(&column_letters(index));
    }
    if let Some(Axis { index, absolute }) = c.row {
        if absolute {
            out.push('$');
        }
        out.push_str(&(index + 1).to_string());
    }
}

fn reference(r: &Reference, out: &mut String) {
    out.push('[');
    cell(&r.start, out);
    if let Some(end) = &r.end {
        out.push(':');
        // The end's sheet is written only when it differs: `[Data.A1:.B2]`.
        let mut end = end.clone();
        if end.sheet == r.start.sheet {
            end.sheet = None;
        }
        cell(&end, out);
    }
    out.push(']');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sheet::formula::excel;

    #[track_caller]
    fn x2o(src: &str) -> String {
        print(&excel::parse(src).unwrap_or_else(|e| panic!("{src}: {e:?}")))
    }

    #[track_caller]
    fn o2x(src: &str) -> String {
        excel::print(&parse(src).unwrap_or_else(|e| panic!("{src}: {e:?}"))).unwrap()
    }

    /// The translation table in `mod.rs`, row by row, both directions.
    #[test]
    fn the_table_holds_in_both_directions() {
        for (x, o) in [
            ("A1", "[.A1]"),
            ("$A$1", "[.$A$1]"),
            ("Sheet1!A1", "[Sheet1.A1]"),
            ("'My Sheet'!A1", "['My Sheet'.A1]"),
            ("SUM(Data!A1:B3)", "SUM([Data.A1:.B3])"),
            ("SUM(Sheet1:Sheet3!A1)", "SUM([Sheet1.A1:Sheet3.A1])"),
            ("SUM(A:A)", "SUM([.A:.A])"),
            ("SUM(1:1)", "SUM([.1:.1])"),
            ("ROUND(A1/3,2)", "ROUND([.A1]/3;2)"),
            ("{1,2;3,4}", "{1;2|3;4}"),
            ("TRUE", "TRUE()"),
            ("IF(A1,TRUE,FALSE)", "IF([.A1];TRUE();FALSE())"),
            ("SUM(A1:A2 A2:B2)", "SUM([.A1:.A2]![.A2:.B2])"),
            ("SUM((A1,B1))", "SUM(([.A1]~[.B1]))"),
            (
                "_xlfn.XLOOKUP(1,A1:A3,B1:B3)",
                "COM.MICROSOFT.XLOOKUP(1;[.A1:.A3];[.B1:.B3])",
            ),
            ("CEILING(A1,2)", "COM.MICROSOFT.CEILING([.A1];2)"),
            ("NORMSDIST(1)", "LEGACY.NORMSDIST(1)"),
            ("_xlfn.FORMULATEXT(A1)", "FORMULA([.A1])"),
            ("\"a\"\"b\"&A1", "\"a\"\"b\"&[.A1]"),
            ("#N/A", "#N/A"),
            ("-2^2", "-2^2"),
            ("A1%", "[.A1]%"),
            ("OFFSET(A1,1,,2)", "OFFSET([.A1];1;;2)"),
        ] {
            assert_eq!(x2o(x), o, "excel → odf");
            assert_eq!(o2x(o), x, "odf → excel");
        }
    }

    #[test]
    fn a_union_outside_parentheses_gains_its_own() {
        assert_eq!(o2x("SUM([.A1]~[.B1])"), "SUM((A1,B1))");
    }

    #[test]
    fn the_attribute_names_its_grammar() {
        assert!(parse_attribute("of:=[.A1]+1").is_ok());
        assert!(parse_attribute("=[.A1]+1").is_ok());
        assert_eq!(parse_attribute("msoxl:=A1+1"), Err(Refusal::ForeignSyntax));
        assert_eq!(
            parse("['file:///x.ods'#$Sheet1.A1]"),
            Err(Refusal::ExternalLink)
        );
        assert_eq!(parse("[.#REF!]+1").unwrap(), {
            Expr::Binary(
                Op::Add,
                Box::new(Expr::Error(CellError::Ref)),
                Box::new(Expr::Number(1.0)),
            )
        });
    }

    #[test]
    fn libreoffices_own_spellings_are_read() {
        // Absolute sheets, spaces around operators and a lower-case function.
        assert_eq!(
            o2x("of:=sum([$Sheet1.A1:.B2]) + 1".trim_start_matches("of:")),
            "SUM(Sheet1!A1:B2)+1"
        );
    }

    #[test]
    fn a_reference_from_here_to_another_sheet_has_no_excel_spelling() {
        let e = parse("[.A1:Data.B2]").unwrap();
        assert_eq!(excel::print(&e), Err(Refusal::UnspellableReference));
    }
}
