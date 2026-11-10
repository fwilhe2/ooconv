// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The neutral workbook: what both readers produce and both writers consume. It holds only what
//! both formats can say the same way, so that a writer never has to guess.
//!
//! **Positions are 0-based**, `(row, col)`. **Dates are serials against ODF's default epoch**,
//! 1899-12-30, which is the day Excel's 1900 system also counts from from 1900-03-01 onwards
//! (`dates.rs`): the xlsx reader corrects into it and the xlsx writer out of it.

use std::collections::BTreeMap;

use super::formula::Expr;

/// Excel's grid: 1,048,576 rows by 16,384 columns (XFD). ODF sets no limit, so this is the one
/// a converter can promise in both directions; content beyond it is a loss on the way to xlsx.
pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLS: u32 = 16_384;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workbook {
    pub sheets: Vec<Sheet>,
    /// How `COUNTIF`, `MATCH` and the other criteria functions read a text criterion.
    pub criteria: Criteria,
}

/// What a criterion string means (ODF 1.4 Part 3 §9.4.1 `table:calculation-settings`). Excel
/// knows only [`Criteria::Wildcards`]; an ODF document without settings means
/// [`Criteria::RegularExpressions`] — the schema's default, and LibreOffice's reading of such a
/// file (grind `doc/small-group.md`, `MEASURED`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Criteria {
    /// `*`, `?` and `~` — Excel's.
    #[default]
    Wildcards,
    RegularExpressions,
    /// Neither: a criterion is matched as written.
    Literal,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sheet {
    /// Excel limits a name to 31 characters and forbids `[]:*?/\`; ODF forbids fewer. The xlsx
    /// writer makes a name legal when it has to, and reports it.
    pub name: String,
    pub cells: BTreeMap<(u32, u32), Cell>,
    /// Merged areas, each at least two cells, never overlapping.
    pub merges: Vec<Area>,
    /// Column widths in points, by 0-based column, where the source set one.
    pub col_widths: BTreeMap<u32, f64>,
    pub hidden: bool,
}

/// A rectangle of cells, inclusive at both ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub first: (u32, u32),
    pub last: (u32, u32),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
    /// The value — for a formula cell, the cached result of the last calculation.
    pub value: Value,
    pub formula: Option<Formula>,
    pub format: NumFmt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Formula {
    pub expr: Expr,
    /// An array (CSE) formula's extent: rows and columns from this cell. `None` for an ordinary
    /// formula.
    pub array: Option<(u32, u32)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    #[default]
    Empty,
    /// Always finite: readers refuse NaN and infinities, which neither format can carry.
    Number(f64),
    Text(String),
    Bool(bool),
    Error(CellError),
}

/// The errors OpenFormula (Part 4 §5.12) and Excel share. Anything else a file spells is
/// [`CellError::Na`]-shaped noise to the other application, so readers map unknown names to
/// [`CellError::Name`], as §5.12 says to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellError {
    Null,
    Div0,
    Value,
    Ref,
    Name,
    Num,
    Na,
}

impl CellError {
    pub const ALL: [CellError; 7] = [
        CellError::Null,
        CellError::Div0,
        CellError::Value,
        CellError::Ref,
        CellError::Name,
        CellError::Num,
        CellError::Na,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            CellError::Null => "#NULL!",
            CellError::Div0 => "#DIV/0!",
            CellError::Value => "#VALUE!",
            CellError::Ref => "#REF!",
            CellError::Name => "#NAME?",
            CellError::Num => "#NUM!",
            CellError::Na => "#N/A",
        }
    }

    pub fn from_name(name: &str) -> Option<CellError> {
        CellError::ALL
            .into_iter()
            .find(|e| e.as_str().eq_ignore_ascii_case(name.trim()))
    }
}

/// How a number is shown — the few shapes both formats spell identically and that carry
/// *meaning* (a date is not a number to a person). Everything else reads as [`NumFmt::General`]
/// and is reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NumFmt {
    #[default]
    General,
    /// `0`, `0.00`: fixed decimals, optionally grouped (`#,##0.00`).
    Fixed { decimals: u8, grouping: bool },
    /// `0%`, `0.00%`.
    Percent { decimals: u8 },
    /// `0.00E+00`.
    Scientific { decimals: u8 },
    /// `YYYY-MM-DD`.
    Date,
    /// `HH:MM:SS`.
    Time,
    /// `YYYY-MM-DD HH:MM:SS`.
    DateTime,
    /// A text cell format (`@`).
    Text,
}

impl NumFmt {
    pub fn is_temporal(self) -> bool {
        matches!(self, NumFmt::Date | NumFmt::Time | NumFmt::DateTime)
    }
}

impl Sheet {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Set a cell, dropping it when it carries nothing at all.
    pub fn set(&mut self, row: u32, col: u32, cell: Cell) {
        if cell == Cell::default() {
            self.cells.remove(&(row, col));
        } else {
            self.cells.insert((row, col), cell);
        }
    }

    /// The last row and column that hold anything, cell or merge.
    pub fn extent(&self) -> Option<(u32, u32)> {
        let cells = self.cells.keys().copied();
        let merges = self.merges.iter().map(|m| m.last);
        cells
            .chain(merges)
            .fold(None, |acc: Option<(u32, u32)>, (r, c)| match acc {
                None => Some((r, c)),
                Some((ar, ac)) => Some((ar.max(r), ac.max(c))),
            })
    }
}

impl Cell {
    pub fn number(n: f64) -> Self {
        Self {
            value: Value::Number(n),
            ..Self::default()
        }
    }

    pub fn text(s: impl Into<String>) -> Self {
        Self {
            value: Value::Text(s.into()),
            ..Self::default()
        }
    }
}
