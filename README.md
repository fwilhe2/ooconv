<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# ooconv

Converts between Office Open XML and OpenDocument, in both directions:

| | |
|---|---|
| `.xlsx` | ↔ `.ods`, `.fods` |
| `.docx` | ↔ `.odt`, `.fodt` |

A Rust library and a command. The point is **reliability**: everything it writes validates
against its format's schema, and every document it writes is opened by real applications in
the test suite — LibreOffice, an Excel-compatible spreadsheet and a Word-compatible word
processor — which must see what was meant. What it cannot carry, it says, by name.

```sh
ooconv budget.xlsx                 # → budget.ods
ooconv report.odt report.docx
ooconv book.ods --to fods
ooconv book.xlsx out.ods --strict  # refuse (exit 3) rather than lose anything
```

```rust
let out = ooconv::convert(&std::fs::read("book.xlsx")?, ooconv::Format::Ods)?;
std::fs::write("book.ods", &out.bytes)?;
for loss in &out.report.losses {
    eprintln!("not carried: {loss}"); // e.g. "2 × comments (first: Data)"
}
```

The input's format is read from its bytes, never its name.

## What it carries

Sheets, values, formulas (translated between Excel's syntax and OpenFormula, function names as
measured against LibreOffice), number-format shapes, dates in both date systems, merged cells,
column widths. Paragraphs, headings, alignment, character formatting, hyperlinks, nested lists,
tables with spans. The full line, and what is reported instead, is in `doc/plan.md` § Scope.

## How it is held to that

`doc/plan.md` has the requirements; `doc/oracles.md` the test loops and how to get the oracles:

```sh
scripts/fetch-tools.sh                     # jing and the Open XML SDK validator
OOCONV_REQUIRE_ORACLES=all cargo test --workspace
```

The format notes (`doc/xlsx-format.md`, `doc/docx-format.md`, `doc/odf-format.md`) hold every
fact the code relies on, each marked `MEASURED`, `SPEC` or `UNVERIFIED` — and an `UNVERIFIED`
fact is not implemented.

## Licence

Code AGPL-3.0-or-later, prose CC-BY-4.0, test data CC0-1.0 (see `REUSE.toml`). Parts of the
reader are lifted from [grind](https://github.com/fwilhe2/grind), by the same author.
