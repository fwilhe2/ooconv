<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# SpreadsheetML, as measured

The facts the xlsx reader and writer stand on. **ECMA-376 / ISO/IEC 29500 is the
specification**; applications are oracles, never sources.

Every fact carries one marker, and a fact moves up the ladder, never down quietly:

- **`MEASURED`** — observed, with the command and the date. Reproducible.
- **`SPEC`** — stated by ECMA-376, cited by section, not yet confirmed against a file.
- **`UNVERIFIED`** — from memory or inference. **May not be implemented.**

**Reading** is grind's filter's ground and its facts are cited, not repeated: grind
`doc/xlsx-format.md` (flavours and namespaces §1, markup compatibility §1.3, `xml:space` §1.4,
the 1900 date system §2.1, `_xHHHH_` escapes §2.6, function names on import §5.1). The reader
code lifted from grind keeps its citations.

---

## 1. The package we write

### 1.1 The parts — `MEASURED`

What LibreOffice 26.8.2.2 writes for a two-row CSV, measured 2026-11-10:

```sh
soffice --headless --convert-to xlsx s.csv && unzip -l s.xlsx
```

`[Content_Types].xml`, `_rels/.rels`, `docProps/{app,core}.xml`, `xl/workbook.xml`,
`xl/_rels/workbook.xml.rels`, `xl/styles.xml`, `xl/theme/theme1.xml`, `xl/sharedStrings.xml`,
`xl/worksheets/sheet1.xml`. The Open XML SDK validator accepts it with no errors.

The writer emits the subset the validator needs and nothing it does not: content types,
package and workbook relationships, `docProps/core.xml`, the workbook, styles, shared strings,
one part per sheet. No theme, no `app.xml`: neither is required (validated: `tests/schema.rs`),
and both oracles open the result (loops L and G).

### 1.2 Element order — `SPEC`

ECMA-376 Part 1 §18.3.1.99 `CT_Worksheet` is a sequence: `sheetPr`, `dimension`, `sheetViews`,
`sheetFormatPr`, `cols`, `sheetData`, … `mergeCells`, …; §18.8.39 `CT_Stylesheet`: `numFmts`,
`fonts`, `fills`, `borders`, `cellStyleXfs`, `cellXfs`, `cellStyles`. The writer follows both;
the validator checks it.

### 1.3 The minimal stylesheet — `SPEC`, validated

One font, the two fills every producer writes (`none`, `gray125`), one border, one
`cellStyleXfs` entry, the `Normal` cell style, and one `cellXfs` entry per number-format shape
used. Custom formats are numbered from 164, the first id ECMA-376 §18.8.30 leaves to files.

### 1.4 No `calcPr` — `MEASURED`

`calcPr fullCalcOnLoad="1"` makes `gridcraft-cli check` report the caches as stale and check
nothing (measured 2026-11-10, gridcraft 0.17.0: `staleCache: true`, `checked: 0`). Without it,
3 of 3 formulas checked and matched. The caches we write are the source application's results,
so they stay checkable.

---

## 2. Values

### 2.1 Dates below serial 61 — `MEASURED`

LibreOffice 26.8.2.2 and 26.2.5.2 (the CI pin) both read an xlsx date serial against
1899-12-30 *without* Excel's 1900 leap-year bug, measured 2026-11-10 with
`soffice --convert-to fods` on `values/dates-1900.xlsx` and on our own output of it:

| serial | Excel shows (grind §2.1) | LibreOffice reads |
|---|---|---|
| 1 | 1900-01-01 | 1899-12-31 |
| 59 | 1900-02-28 | 1900-02-27 |
| 61 | 1900-03-01 | 1900-03-01 |

The converter keeps Excel's meaning (`sheet::dates`); loop L names the difference
(`1900 leap-year bug`). Only serials below 61 under a date format are affected.

### 2.2 Text — `SPEC` and `MEASURED`

ECMA-376 §22.9.2.19 `ST_Xstring`: a character XML cannot carry is written `_xHHHH_`, and a literal
`_x` followed by four hex digits and `_` has its underscore written `_x005F_`. On reading, a
decoded control character other than tab, LF and CR is dropped, as LibreOffice does (grind
§2.6, `MEASURED`). So a control character survives neither application, and the writer escapes
it only to keep the part well-formed.

A formula's text result is written inline (`t="str"`), never as a shared string (§18.18.11).

### 2.3 Limits — `SPEC`

Excel's specifications and limits: 1,048,576 rows, 16,384 columns, 32,767 characters in a cell,
8,192 in a formula, 31 in a sheet name, which may not contain `[ ] : * ? / \`, start or end with
`'`, or be `History`. The writer keeps each and reports it (`cell beyond Excel's grid`, `text
longer than Excel allows`, `formula longer than Excel allows`, `sheet renamed for Excel`).
`hostile`-adjacent: `document/sheets.xlsx` (Open XML SDK) carries `Has[Brackets]` and
`Has/Slash`, which the SDK's validator accepts and Excel's rules forbid.

---

## 3. Formulas

### 3.1 Function names on export — `MEASURED`

What LibreOffice 26.8.2.2 writes in `<f>` for each OpenFormula name, measured 2026-11-10 by
converting `ooconv/tests/data/measured/function-names.fods` (one cell per name) to xlsx. The
whole table is vendored as `function-names-lo-export.txt` and replayed by a unit test in
`sheet/formula/names.rs`. In summary:

- `COM.MICROSOFT.X` is written `_xlfn.X` for the Excel 2010+ functions, `X` for `CEILING`,
  `FLOOR`, `ISO.CEILING`, `NETWORKDAYS.INTL`, `WORKDAY.INTL`.
- `FILTER` and `SORT` carry a second marker: `_xlfn._xlws.FILTER`, `_xlfn._xlws.SORT`.
- `LEGACY.X` is written `X`; `FDIST`, `FINV`, `FORMULA`, `SKEWP` become `_xlfn.F.DIST`,
  `_xlfn.F.INV`, `_xlfn.FORMULATEXT`, `_xlfn.SKEW.P`.
- OpenFormula names newer than Excel 2007 gain `_xlfn.`: `IFNA`, `XOR`, `DAYS`, `ISOWEEKNUM`,
  `SHEET(S)`, `ISFORMULA`, `NUMBERVALUE`, `UNICHAR`, `UNICODE`, `ARABIC`, `BASE`, `DECIMAL`, the
  `BIT*` functions, `COT`/`COTH`/`CSC`/`CSCH`/`SEC`/`SECH`/`ACOT`/`ACOTH`, `COMBINA`,
  `PERMUTATIONA`, `PHI`, `GAUSS`, `PDURATION`, `RRI`, `GAMMA`, `MUNIT`.
- `IMSEC`, `IMCOT` and the other new complex functions are written **without** `_xlfn.`. That is
  LibreOffice's choice, not established as Excel's; the writer follows the measurement.

### 3.2 Names with no measured spelling — `UNVERIFIED`, so refused

LibreOffice writes `com.microsoft.lambda` (lower case, no marker) for `LAMBDA`, `LET`'s relatives
`MAP`, `REDUCE`, `SCAN`, `BYCOL`, `BYROW`, `MAKEARRAY`, `ISOMITTED` and `ARRAYTOTEXT` — names it
cannot export. Excel's spelling (`_xlfn.LAMBDA`, with `_xlpm.` parameters) is `UNVERIFIED`, so
the writer refuses such a formula (`formula: function with no known spelling`) and keeps its
value. `ODF`'s `B` is exported as `B`; Excel's `BINOM.DIST.RANGE` is the same function and its
spelling is likewise unmeasured.

### 3.3 Reference syntax — `SPEC`

§18.17: `Sheet!A1`, `'Quoted Sheet'!A1` with `'` doubled, `Sheet1:Sheet3!A1` for a span of
sheets, `A:A` and `1:1`, a space for intersection and `,` inside parentheses for union,
`{1,2;3,4}` for an inline array. A sheet name is written bare only when it is a plain identifier
that could not be read as a cell or R1C1 reference or a boolean; quoting is always accepted.

### 3.4 Shared formulas — `SPEC`, held by the corpus

§18.3.1.40: a dependent's formula is the master's with every relative reference moved by the
dependent's offset; an axis moved off the grid is `#REF!` for that reference only.
`formulas/shared-groups.xlsx`'s manifest says the same, and both calculating oracles compute our
expansion to the same values (loop G).

---

## 4. Number formats — `SPEC`

Built-in ids (§18.8.30) used for writing: 0 General, 1 `0`, 2 `0.00`, 3 `#,##0`, 4 `#,##0.00`,
9 `0%`, 10 `0.00%`, 11 `0.00E+00`, 49 `@`. Dates and times are written as custom codes
(`yyyy\-mm\-dd`, `hh:mm:ss`, `yyyy\-mm\-dd hh:mm:ss`) because ids 14–22 render by locale. On
reading, ids 14–17 are dates, 18–21 and 45–47 times, 22 a date-time.
