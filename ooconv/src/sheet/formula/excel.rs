// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Excel's A1 formula syntax as stored in `<f>` (ECMA-376 §18.17): no leading `=`, `,` between
//! arguments, function names in their *file* spelling (`_xlfn.XLOOKUP`). The parser follows
//! grind's (`xlsx/src/formula.rs`), generalised to the union, intersection and inline-array
//! operators the converter carries rather than refuses.

use super::{
    Axis, CellRef, Cursor, Expr, INTERSECT_BP, MAX_DEPTH, Op, PREFIX_BP, Reference, Refusal,
    column_letters, continues_word, names, number, quote_string,
};

/// `<f>`'s text → an expression. A leading `=` is accepted, though `<f>` never has one.
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
    fn expr(&mut self, min_bp: u8) -> Result<Expr, Refusal> {
        let mut lhs = self.prefix()?;
        loop {
            let spaced = self.c.space();
            let (op, bp, len) = match self.c.common_infix() {
                Some(found) => found,
                // Whitespace and then an operand: Excel's intersection operator.
                None if spaced && self.starts_operand() => (Op::Intersect, INTERSECT_BP, 0),
                None => break,
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

    fn starts_operand(&self) -> bool {
        matches!(
            self.c.peek(),
            Some(c) if c.is_alphanumeric() || matches!(c, '$' | '\'' | '(' | '_')
        )
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

    fn primary(&mut self) -> Result<Expr, Refusal> {
        self.c.space();
        match self.c.peek().ok_or(Refusal::Syntax)? {
            // The implicit-intersection marker: "one cell of this", which a non-array formula
            // does anyway.
            '@' => {
                self.c.at += 1;
                self.recurse(Self::primary)
            }
            '{' => self.array(),
            '[' => Err(Refusal::ExternalLink),
            '"' => self.c.string(),
            '#' => {
                // `#REF!` may be followed by nothing a reference could be; keep it an error.
                Ok(self.c.error_literal())
            }
            '(' => {
                self.c.at += 1;
                let mut inner = self.recurse(|p| p.expr(0))?;
                self.c.space();
                // `(A1:A2,C1:C2)`: a comma inside plain parentheses is the union operator.
                while self.c.eat(',') {
                    let rhs = self.recurse(|p| p.expr(0))?;
                    inner = Expr::Binary(Op::Union, Box::new(inner), Box::new(rhs));
                    self.c.space();
                }
                match self.c.eat(')') {
                    true => Ok(Expr::Paren(Box::new(inner))),
                    false => Err(Refusal::Syntax),
                }
            }
            c if c.is_ascii_digit()
                || matches!(c, '.' | '$' | '\'' | '_' | '\\')
                || c.is_alphabetic() =>
            {
                self.operand()
            }
            _ => Err(Refusal::Syntax),
        }
    }

    fn operand(&mut self) -> Result<Expr, Refusal> {
        let numeric = matches!(self.c.peek(), Some(c) if c.is_ascii_digit() || c == '.');
        let save = self.c.at;
        if let Some(word) = self.c.word() {
            match self.c.peek() {
                Some('(') => return self.recurse(|p| p.call(&word)),
                Some('[') => return Err(Refusal::StructuredReference),
                _ => self.c.at = save,
            }
        }
        if let Some(reference) = self.reference()? {
            return Ok(reference);
        }
        if numeric {
            return self.c.number();
        }
        if let Some(word) = self.c.word() {
            return Ok(match word.to_ascii_uppercase().as_str() {
                "TRUE" => Expr::Bool(true),
                "FALSE" => Expr::Bool(false),
                _ => Expr::Name(word),
            });
        }
        Err(Refusal::Syntax)
    }

    fn call(&mut self, word: &str) -> Result<Expr, Refusal> {
        self.c.at += 1; // `(`
        let mut args = Vec::new();
        self.c.space();
        if !self.c.eat(')') {
            loop {
                self.c.space();
                args.push(match matches!(self.c.peek(), Some(',' | ')')) {
                    true => Expr::Empty,
                    false => self.expr(0)?,
                });
                self.c.space();
                if self.c.eat(',') {
                    continue;
                }
                if self.c.eat(')') {
                    break;
                }
                return Err(Refusal::Syntax);
            }
        }
        let name = names::excel_to_odf(word);
        // `_xlfn.SINGLE(x)` means `@x`; only with the marker, since a workbook may define its own
        // `SINGLE`.
        if name == "SINGLE" && args.len() == 1 && word.len() > name.len() {
            return Ok(args.remove(0));
        }
        match name.as_str() {
            "TRUE" if args.is_empty() => Ok(Expr::Bool(true)),
            "FALSE" if args.is_empty() => Ok(Expr::Bool(false)),
            _ => Ok(Expr::Call { name, args }),
        }
    }

    /// `{1,2;3,4}` — constants only (§18.17.2.4).
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
                _ => match self.c.word().map(|w| w.to_ascii_uppercase()).as_deref() {
                    Some("TRUE") => Expr::Bool(true),
                    Some("FALSE") => Expr::Bool(false),
                    _ => return Err(Refusal::Syntax),
                },
            };
            rows.last_mut().expect("a row").push(item);
            self.c.space();
            match self.c.peek() {
                Some(',') => self.c.at += 1,
                Some(';') => {
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

    // ---- references ----

    fn reference(&mut self) -> Result<Option<Expr>, Refusal> {
        let save = self.c.at;
        let sheets = self.sheet_prefix();
        if sheets.is_some() && self.c.peek() == Some('#') {
            // `Sheet1!#REF!` — what Excel writes for a deleted cell.
            return Ok(Some(self.c.error_literal()));
        }
        let Some(start) = self.c.cell_part() else {
            if sheets.is_some() {
                // `Data!Name` — a sheet-scoped name: no ODF spelling.
                return Err(Refusal::Syntax);
            }
            self.c.at = save;
            return Ok(None);
        };
        let mut end = None;
        if self.c.peek() == Some(':') {
            let mark = self.c.at;
            self.c.at += 1;
            match self.c.cell_part() {
                Some(e) if same_shape(start, e) && !continues_word(self.c.peek()) => end = Some(e),
                _ => self.c.at = mark,
            }
        }
        if continues_word(self.c.peek())
            || (end.is_none() && !(start.0.is_some() && start.1.is_some()))
        {
            if sheets.is_some() {
                return Err(Refusal::Syntax);
            }
            self.c.at = save;
            return Ok(None);
        }
        let (first, last) = sheets.unwrap_or((None, None));
        let cell = |sheet: Option<String>, (col, row): (Option<Axis>, Option<Axis>)| CellRef {
            sheet,
            col,
            row,
        };
        Ok(Some(Expr::Ref(Reference {
            start: cell(first, start),
            end: match (end, last) {
                (Some(e), sheet) => Some(cell(sheet, e)),
                (None, Some(sheet)) => Some(cell(Some(sheet), start)),
                (None, None) => None,
            },
        })))
    }

    /// `Sheet1!`, `'My Sheet'!`, `Sheet1:Sheet3!`, consumed through the `!`, or nothing.
    fn sheet_prefix(&mut self) -> Option<(Option<String>, Option<String>)> {
        let save = self.c.at;
        let first = self.sheet_name()?;
        let second = match self.c.eat(':') {
            true => match self.sheet_name() {
                Some(name) => Some(name),
                None => {
                    self.c.at = save;
                    return None;
                }
            },
            false => None,
        };
        if !self.c.eat('!') {
            self.c.at = save;
            return None;
        }
        Some((Some(first), second))
    }

    fn sheet_name(&mut self) -> Option<String> {
        if self.c.peek() != Some('\'') {
            return self.c.word();
        }
        let save = self.c.at;
        self.c.at += 1;
        let mut out = String::new();
        loop {
            match self.c.peek() {
                Some('\'') if self.c.peek_at(1) == Some('\'') => {
                    out.push('\'');
                    self.c.at += 2;
                }
                Some('\'') => {
                    self.c.at += 1;
                    return Some(out);
                }
                Some(c) => {
                    out.push(c);
                    self.c.at += 1;
                }
                None => {
                    self.c.at = save;
                    return None;
                }
            }
        }
    }
}

fn same_shape(a: (Option<Axis>, Option<Axis>), b: (Option<Axis>, Option<Axis>)) -> bool {
    a.0.is_some() == b.0.is_some() && a.1.is_some() == b.1.is_some()
}

// ---- printing ----

/// An expression as `<f>` text: no `=`.
pub fn print(expr: &Expr) -> Result<String, Refusal> {
    let mut out = String::new();
    write(expr, &mut out)?;
    Ok(out)
}

fn write(expr: &Expr, out: &mut String) -> Result<(), Refusal> {
    match expr {
        Expr::Number(n) => out.push_str(&number(*n)),
        Expr::Text(s) => out.push_str(&quote_string(s)),
        Expr::Bool(b) => out.push_str(if *b { "TRUE" } else { "FALSE" }),
        Expr::Error(e) => out.push_str(e.as_str()),
        Expr::Ref(r) => reference(r, out)?,
        Expr::Name(n) => out.push_str(n),
        Expr::Call { name, args } => {
            if names::UNSPELLED.contains(&name.as_str()) {
                return Err(Refusal::UnspelledFunction);
            }
            out.push_str(&names::odf_to_excel(name));
            out.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(a, out)?;
            }
            out.push(')');
        }
        Expr::Prefix(op, e) => {
            out.push_str(op.common());
            write(e, out)?;
        }
        Expr::Postfix(op, e) => {
            write(e, out)?;
            out.push_str(op.common());
        }
        // A comma is the union operator only inside parentheses of its own: one pair around the
        // whole chain, whether or not the source had them.
        Expr::Binary(Op::Union, ..) => union(expr, out)?,
        Expr::Paren(e) if matches!(**e, Expr::Binary(Op::Union, ..)) => union(e, out)?,
        Expr::Binary(op, a, b) => {
            write(a, out)?;
            out.push_str(match op {
                Op::Intersect => " ",
                op => op.common(),
            });
            write(b, out)?;
        }
        Expr::Paren(e) => {
            out.push('(');
            write(e, out)?;
            out.push(')');
        }
        Expr::Empty => {}
        Expr::Array(rows) => {
            out.push('{');
            for (i, row) in rows.iter().enumerate() {
                if i > 0 {
                    out.push(';');
                }
                for (j, item) in row.iter().enumerate() {
                    if j > 0 {
                        out.push(',');
                    }
                    write(item, out)?;
                }
            }
            out.push('}');
        }
    }
    Ok(())
}

fn union(expr: &Expr, out: &mut String) -> Result<(), Refusal> {
    fn items<'e>(e: &'e Expr, into: &mut Vec<&'e Expr>) {
        match e {
            Expr::Binary(Op::Union, a, b) => {
                items(a, into);
                items(b, into);
            }
            other => into.push(other),
        }
    }
    let mut list = Vec::new();
    items(expr, &mut list);
    out.push('(');
    for (i, item) in list.into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write(item, out)?;
    }
    out.push(')');
    Ok(())
}

/// A sheet name as Excel needs it: bare only when it is a plain identifier that cannot be read
/// as anything else, quoted (with `'` doubled) otherwise. Quoting is always accepted.
pub fn sheet_name(name: &str) -> String {
    let plain = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        && !looks_like_reference(name)
        && !matches!(name.to_ascii_uppercase().as_str(), "TRUE" | "FALSE");
    match plain {
        true => name.to_owned(),
        false => format!("'{}'", name.replace('\'', "''")),
    }
}

/// `A1`, `XFD1048576`, `R1C1`, `R`, `C`: a name Excel would read as a reference.
fn looks_like_reference(name: &str) -> bool {
    let letters: String = name
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    let rest = &name[letters.len()..];
    let a1 = !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
        && super::column_index(&letters).is_some();
    let upper = name.to_ascii_uppercase();
    let r1c1 = upper.starts_with('R') || upper.starts_with('C');
    a1 || (r1c1
        && upper
            .chars()
            .skip(1)
            .all(|c| c.is_ascii_digit() || c == 'C'))
}

fn cell(c: &CellRef, out: &mut String) {
    if let Some(col) = c.col {
        if col.absolute {
            out.push('$');
        }
        out.push_str(&column_letters(col.index));
    }
    if let Some(row) = c.row {
        if row.absolute {
            out.push('$');
        }
        out.push_str(&(row.index + 1).to_string());
    }
}

fn reference(r: &Reference, out: &mut String) -> Result<(), Refusal> {
    let end_sheet = r.end.as_ref().and_then(|e| e.sheet.as_ref());
    match (&r.start.sheet, end_sheet) {
        (Some(a), Some(b)) if a != b => {
            out.push_str(&sheet_name(a));
            out.push(':');
            out.push_str(&sheet_name(b));
            out.push('!');
        }
        (Some(a), _) => {
            out.push_str(&sheet_name(a));
            out.push('!');
        }
        // `[.A1:Sheet2.B2]`: the start is on this sheet and the end on another.
        (None, Some(_)) => return Err(Refusal::UnspellableReference),
        (None, None) => {}
    }
    cell(&r.start, out);
    if let Some(end) = &r.end {
        // `Sheet1:Sheet3!A1` — a cuboid one cell deep has its cell written once.
        let cuboid = end.sheet.is_some() && end.sheet != r.start.sheet;
        if !cuboid || (end.col, end.row) != (r.start.col, r.start.row) {
            out.push(':');
            cell(end, out);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn round(src: &str) -> String {
        print(&parse(src).unwrap_or_else(|e| panic!("{src}: {e:?}"))).unwrap()
    }

    #[test]
    fn what_excel_writes_comes_back_as_written() {
        for f in [
            "A1",
            "$A$1+A$1*$A1",
            "SUM(A1:B3)",
            "Data!A1",
            "'My Sheet'!A2",
            "'O''Brien'!A3",
            "SUM(Data:Sheet3!A1:A2)",
            "SUM(Data:Sheet3!A1)",
            "SUM(Data!A:A)+SUM(Data!1:1)",
            "SUM(A:A)+SUM(1:1)+SUM($A:$C)+SUM($1:$3)",
            "-2^2",
            "(1+2)*3",
            "A1%%",
            "IF(A1>=2,\"a\"\"b\",FALSE)",
            "OFFSET(A1,1,,2)",
            "#DIV/0!",
            "SUM(A1:A3 A2:B2)",
            "SUM((A1,B1,C1))",
            "{1,2;3,4}",
            "_xlfn.XLOOKUP(1,A1:A3,B1:B3)",
            "1E+21+1.5E-7",
            "Rate*2",
            "LOG10(A1)",
        ] {
            assert_eq!(round(f), f);
        }
    }

    #[test]
    fn markers_excel_adds_are_dropped() {
        assert_eq!(round("@A1:A3"), "A1:A3");
        assert_eq!(round("_xlfn.SINGLE(A1:A3)"), "A1:A3");
        assert_eq!(round("=A1"), "A1");
    }

    #[test]
    fn constructs_without_a_translation_are_refused_by_class() {
        assert_eq!(parse("Table1[Col]"), Err(Refusal::StructuredReference));
        assert_eq!(parse("[1]Sheet1!A1"), Err(Refusal::ExternalLink));
        assert_eq!(parse("SUM(A1"), Err(Refusal::Syntax));
        assert_eq!(parse(""), Err(Refusal::Syntax));
        let deep = format!(
            "{}1{}",
            "(".repeat(MAX_DEPTH + 5),
            ")".repeat(MAX_DEPTH + 5)
        );
        assert_eq!(parse(&deep), Err(Refusal::Syntax));
    }

    #[test]
    fn a_sheet_is_quoted_whenever_bare_would_mean_something_else() {
        assert_eq!(sheet_name("Data"), "Data");
        assert_eq!(sheet_name("My Sheet"), "'My Sheet'");
        assert_eq!(sheet_name("A1"), "'A1'");
        assert_eq!(sheet_name("R1C1"), "'R1C1'");
        assert_eq!(sheet_name("2024"), "'2024'");
        assert_eq!(sheet_name("Größe"), "'Größe'");
        assert_eq!(sheet_name("TRUE"), "'TRUE'");
    }
}
