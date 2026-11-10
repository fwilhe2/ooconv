// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Documents built in code, one per feature the model has, so every oracle sees every feature
//! whatever the corpus happens to contain. **Add to these when adding a feature.**
//!
//! Each is in the model's canonical form — what a reader produces — so `read(write(x)) == x` is
//! the round-trip claim with nothing to normalise.

use ooconv::sheet::formula::excel;
use ooconv::sheet::{Area, Cell, CellError, Formula, NumFmt, Sheet, Value, Workbook};
use ooconv::text::{
    Align, Block, Document, ParaKind, Paragraph, Position, Run, RunStyle, Table, TableCell,
};

fn f(src: &str, value: Value) -> Cell {
    Cell {
        value,
        formula: Some(Formula {
            expr: excel::parse(src).unwrap_or_else(|e| panic!("{src}: {e:?}")),
            array: None,
        }),
        format: NumFmt::General,
    }
}

fn fmt(value: f64, format: NumFmt) -> Cell {
    Cell {
        value: Value::Number(value),
        formula: None,
        format,
    }
}

/// Every value kind, format, formula shape and sheet property, with cached values that are
/// the formulas' true results — so a calculating oracle can check them.
pub fn workbook() -> Workbook {
    let mut data = Sheet::new("Data");
    let rows: Vec<Vec<Cell>> = vec![
        vec![Cell::text("Item"), Cell::text("Amount"), Cell::text("Kind")],
        vec![Cell::text("apples"), Cell::number(3.0), Cell::text("fruit")],
        vec![Cell::text("pears"), Cell::number(4.5), Cell::text("fruit")],
        vec![Cell::text("leeks"), Cell::number(-2.25), Cell::text("veg")],
    ];
    for (r, row) in rows.into_iter().enumerate() {
        for (c, cell) in row.into_iter().enumerate() {
            data.set(r as u32, c as u32, cell);
        }
    }
    let formulas = [
        ("SUM(B2:B4)", Value::Number(5.25)),
        ("B2*2+B3/3", Value::Number(7.5)),
        ("-2^2", Value::Number(4.0)),
        ("2^3^2", Value::Number(64.0)),
        ("IF(B2>3,\"big\",\"small\")", Value::Text("small".into())),
        ("A2&\" & \"&A3", Value::Text("apples & pears".into())),
        ("SUMIF(C2:C4,\"fruit\",B2:B4)", Value::Number(7.5)),
        ("COUNTIF(B2:B4,\">0\")", Value::Number(2.0)),
        ("ROUND(B3/7,3)", Value::Number(0.643)),
        ("VLOOKUP(\"pears\",A2:B4,2,FALSE)", Value::Number(4.5)),
        ("1/0", Value::Error(CellError::Div0)),
        ("NA()", Value::Error(CellError::Na)),
        ("AND(TRUE,B2>0)", Value::Bool(true)),
        ("ISNUMBER(B2)", Value::Bool(true)),
        ("SUM(B:B)", Value::Number(5.25)),
        ("_xlfn.IFNA(NA(),\"none\")", Value::Text("none".into())),
        ("_xlfn.XLOOKUP(\"leeks\",A2:A4,B2:B4)", Value::Number(-2.25)),
        ("CEILING(B3,2)", Value::Number(6.0)),
        ("'O''Brien & Co'!A1*2", Value::Number(84.0)),
        ("B2%", Value::Number(0.03)),
        ("MAX(B2:B4)-MIN(B2:B4)", Value::Number(6.75)),
        ("SUM({1,2;3,4})", Value::Number(10.0)),
        ("LEN(\"a\"\"b\")", Value::Number(3.0)),
    ];
    for (i, (src, value)) in formulas.into_iter().enumerate() {
        data.set(i as u32 + 1, 4, Cell::text(src));
        data.set(i as u32 + 1, 5, f(src, value));
    }
    data.col_widths.insert(0, 90.0);
    data.col_widths.insert(4, 180.0);

    let mut values = Sheet::new("Values & Formats");
    let cells = [
        Cell::text("plain"),
        Cell::text("  two leading, two inner  spaces, trailing "),
        Cell::text("tab\there"),
        Cell::text("line one\nline two"),
        Cell::text("Grüße – 日本語 – emoji 😀"),
        Cell::text("_x0041_ looks like an escape"),
        Cell::text("<xml> & \"quotes\" 'apos'"),
        Cell::number(0.0),
        Cell::number(1e21),
        Cell::number(1.5e-7),
        Cell::number(-123456789.125),
        Cell {
            value: Value::Bool(true),
            ..Cell::default()
        },
        Cell {
            value: Value::Bool(false),
            ..Cell::default()
        },
        Cell {
            value: Value::Error(CellError::Na),
            ..Cell::default()
        },
        fmt(46336.0, NumFmt::Date),
        fmt(46336.75, NumFmt::DateTime),
        fmt(0.5, NumFmt::Time),
        fmt(61.0, NumFmt::Date),
        fmt(0.125, NumFmt::Percent { decimals: 1 }),
        fmt(0.5, NumFmt::Percent { decimals: 0 }),
        fmt(
            1234.5678,
            NumFmt::Fixed {
                decimals: 2,
                grouping: true,
            },
        ),
        fmt(
            42.0,
            NumFmt::Fixed {
                decimals: 0,
                grouping: false,
            },
        ),
        fmt(12345.678, NumFmt::Scientific { decimals: 2 }),
    ];
    for (i, cell) in cells.into_iter().enumerate() {
        values.set(i as u32, 0, cell);
    }
    values.set(0, 2, Cell::text("merged"));
    values.merges.push(Area {
        first: (0, 2),
        last: (1, 3),
    });
    values.set(3, 2, Cell::text("wide"));
    values.merges.push(Area {
        first: (3, 2),
        last: (3, 4),
    });

    let mut quoted = Sheet::new("O'Brien & Co");
    quoted.set(0, 0, Cell::number(42.0));
    quoted.set(1, 0, f("Data!B2+Data!B3", Value::Number(7.5)));

    let mut hidden = Sheet::new("Hidden");
    hidden.hidden = true;
    hidden.set(4, 2, Cell::text("far from A1"));

    Workbook {
        sheets: vec![data, values, quoted, hidden, Sheet::new("Empty")],
        ..Workbook::default()
    }
}

fn run(text: &str, style: RunStyle) -> Run {
    Run {
        text: text.into(),
        style,
        link: None,
    }
}

fn para(kind: ParaKind, runs: Vec<Run>) -> Block {
    Block::Paragraph(Paragraph {
        kind,
        align: None,
        runs,
    })
}

fn plain(text: &str) -> Block {
    para(ParaKind::Normal, vec![run(text, RunStyle::default())])
}

fn item(list: u32, ordered: bool, level: u8, text: &str) -> Block {
    para(
        ParaKind::ListItem {
            list,
            ordered,
            level,
        },
        vec![run(text, RunStyle::default())],
    )
}

fn cell(blocks: Vec<Block>) -> TableCell {
    TableCell {
        blocks,
        ..TableCell::default()
    }
}

/// Every block kind, run style, list shape and table shape.
pub fn document() -> Document {
    let b = RunStyle {
        bold: true,
        ..RunStyle::default()
    };
    let i = RunStyle {
        italic: true,
        ..RunStyle::default()
    };
    let all = RunStyle {
        bold: true,
        italic: true,
        underline: true,
        strike: true,
        position: Position::Normal,
    };
    let sup = RunStyle {
        position: Position::Superscript,
        ..RunStyle::default()
    };
    let sub = RunStyle {
        position: Position::Subscript,
        ..RunStyle::default()
    };
    let u = RunStyle {
        underline: true,
        ..RunStyle::default()
    };
    let s = RunStyle {
        strike: true,
        ..RunStyle::default()
    };

    let mut blocks = Vec::new();
    for level in 1..=6 {
        blocks.push(para(
            ParaKind::Heading(level),
            vec![run(&format!("Heading {level}"), RunStyle::default())],
        ));
    }
    blocks.push(para(
        ParaKind::Normal,
        vec![
            run("Plain, ", RunStyle::default()),
            run("bold", b),
            run(", ", RunStyle::default()),
            run("italic", i),
            run(", ", RunStyle::default()),
            run("underlined", u),
            run(", ", RunStyle::default()),
            run("struck", s),
            run(", ", RunStyle::default()),
            run("everything", all),
            run(", E=mc", RunStyle::default()),
            run("2", sup),
            run(" and H", RunStyle::default()),
            run("2", sub),
            run("O.", RunStyle::default()),
        ],
    ));
    blocks.push(para(
        ParaKind::Normal,
        vec![
            run("A link to ", RunStyle::default()),
            Run {
                text: "example.org".into(),
                style: RunStyle::default(),
                link: Some("https://example.org/path?q=1&r=2".into()),
            },
            Run {
                text: " in bold".into(),
                style: b,
                link: Some("https://example.org/path?q=1&r=2".into()),
            },
            run(" and back.", RunStyle::default()),
        ],
    ));
    blocks.push(plain(
        "  Two leading spaces, a\ttab, two  inner spaces,\na line break, trailing  ",
    ));
    blocks.push(plain(
        "Unicode: Grüße – 日本語 – 😀 – non\u{2011}breaking hyphen",
    ));
    for (align, text) in [
        (Align::Center, "Centred."),
        (Align::End, "End-aligned."),
        (
            Align::Justify,
            "Justified text that runs on long enough to wrap onto a second line, so that justification is visible to a person looking at the rendered page.",
        ),
    ] {
        blocks.push(Block::Paragraph(Paragraph {
            kind: ParaKind::Normal,
            align: Some(align),
            runs: vec![run(text, RunStyle::default())],
        }));
    }
    blocks.push(item(1, false, 0, "bullet one"));
    blocks.push(item(1, false, 1, "nested bullet"));
    blocks.push(item(1, false, 2, "deeper"));
    blocks.push(item(1, false, 0, "bullet two"));
    blocks.push(plain("Between the lists."));
    blocks.push(item(2, true, 0, "first"));
    blocks.push(item(2, true, 0, "second"));
    blocks.push(item(2, true, 1, "second, a"));
    blocks.push(plain("An interruption; the numbered list resumes below."));
    blocks.push(item(2, true, 0, "third"));
    blocks.push(para(
        ParaKind::Heading(2),
        vec![run("Tables", RunStyle::default())],
    ));

    let mut spans = Table {
        rows: vec![
            vec![
                TableCell {
                    blocks: vec![plain("two columns, two rows")],
                    col_span: 2,
                    row_span: 2,
                    covered: false,
                },
                TableCell {
                    covered: true,
                    ..TableCell::default()
                },
                cell(vec![plain("C1")]),
            ],
            vec![
                TableCell {
                    covered: true,
                    ..TableCell::default()
                },
                TableCell {
                    covered: true,
                    ..TableCell::default()
                },
                cell(vec![plain("C2")]),
            ],
            vec![
                cell(vec![plain("A3")]),
                TableCell {
                    blocks: vec![plain("two columns")],
                    col_span: 2,
                    ..TableCell::default()
                },
                TableCell {
                    covered: true,
                    ..TableCell::default()
                },
            ],
        ],
    };
    spans.normalise();
    blocks.push(Block::Table(spans));
    blocks.push(plain("A table inside a table:"));
    let inner = Table {
        rows: vec![vec![cell(vec![plain("inner")]), cell(vec![plain("cells")])]],
    };
    blocks.push(Block::Table(Table {
        rows: vec![vec![
            cell(vec![
                plain("outer"),
                para(ParaKind::Normal, vec![run("bold", b)]),
            ]),
            cell(vec![Block::Table(inner), plain("after the inner table")]),
        ]],
    }));
    blocks.push(plain("The end."));
    Document { blocks }
}
