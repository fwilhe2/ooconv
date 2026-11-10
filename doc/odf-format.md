<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# OpenDocument, as measured

The facts the ODF readers and writers stand on, under `doc/xlsx-format.md`'s markers. **ODF 1.4
is the specification** (`doc/OpenDocument-v1.4-schema.rng`, Part 3; Part 4 for formulas). grind
`doc/ods-format.md` and `doc/odt-format.md` hold the reading facts this file does not repeat.

---

## 1. What we write

### 1.1 Only the schema's vocabulary — `SPEC`, validated

Root namespaces are exactly those the writers use (`odf::NAMESPACES`), `office:version="1.4"`,
and no `calcext:`/`loext:` anywhere. Both forms are validated with `jing -i` (`tests/schema.rs`):
the flat document, and a package's `content.xml` and `styles.xml`.

### 1.2 The `of:` prefix must be declared — `MEASURED`

A `table:formula="of:=…"` in a document that does not bind `of` is still read by LibreOffice,
but exported verbatim — `<f>of:=IF(1,TRUE())</f>` — as measured 2026-11-10 while measuring
function names (`doc/xlsx-format.md` §3.1). The writers declare
`xmlns:of="urn:oasis:names:tc:opendocument:xmlns:of:1.2"`.

### 1.3 The criteria mode is always stated — `SPEC`, `MEASURED`

Part 3 §9.4.1 `table:calculation-settings`: `table:use-regular-expressions` defaults to `true`
and `table:use-wildcards` (ODF 1.3+) to `false`. So a document without the element reads every
`COUNTIF`/`MATCH`/`VLOOKUP` criterion as a regular expression — and LibreOffice does: loop L
measured `COUNTIF(…;"A-*")` as 0 where Excel's wildcard reading gives 3 (2026-11-10). The model
carries the mode (`sheet::Criteria`); the ODS writer always writes the element; the xlsx writer
reports `regular-expression criteria` when a workbook in that mode calls a criteria function.

## 2. Text

### 2.1 Whitespace — `MEASURED`

Part 3 §6.1.2 collapses runs of space, tab, CR and LF in character data. What LibreOffice does
with the ends of a paragraph, measured 2026-11-10 from its own `.fods`:

```xml
<text:p><text:s text:c="2"/>two leading, two inner <text:s/>spaces, trailing </text:p>
```

A leading space is written `<text:s/>` (it would be dropped); a **single trailing space is
written literally**, and LibreOffice reads it back. The reader therefore drops collapsible space
at the start of a paragraph and keeps one at the end (`odf::Collapser`); the writer escapes
both ends anyway, so either reading gives back what was written.

A space at a **span boundary** belongs to the run it was written in, and whitespace starting
the next run collapses into it — `Plain, <span>bold</span>` is `"Plain, "` then `"bold"`.

### 2.2 Tabs and line breaks — grind `doc/odt-format.md` §3.3, `MEASURED` there

A tab or newline written as itself comes back as a space, so every one is `text:tab` or
`text:line-break`. In a spreadsheet cell, a line break is a new `text:p` (Part 3 §9.1.4).

## 3. Spreadsheet values

### 3.1 How LibreOffice spells what ODF 1.4 cannot — `MEASURED`

From LibreOffice 26.8.2.2's `.fods` of our files, 2026-11-10:

- A **cached error** is `office:value-type="string" office:string-value=""
  calcext:value-type="error"` with the display text in `text:p`. The reader reads the
  `calcext:` alias.
- `#NUM!` from `SQRT(-1)` is displayed **`Err:502`**, LibreOffice's own code. The reader maps
  `Err:502` to `#NUM!`; any other `Err:NNN` is unmeasured and reads as `#NAME?` (Part 4 §5.12).
- A **boolean constant** from xlsx comes back as the formula `TRUE()`/`FALSE()`.
- An **empty-string** cell comes back empty.

### 3.2 An error value we write — `SPEC`

ODF 1.4 has no error value type, so a cell holding an error *constant* is written as the
formula `of:=#N/A` (Part 4 §5.12), with the error's text displayed; the reader reads a formula
that is a bare error constant back as the value.

### 3.3 LibreOffice recalculates on load — `MEASURED`

With `ODFRecalcMode`/`OOXMLRecalcMode` 0 in the profile, and in fact also with the default
profile for these files, LibreOffice recalculates on load: `formulas/shared-groups.xlsx`
caches `C2` as 4 and LibreOffice shows 6, the value of `A2+B2` (2026-11-10). Loop L relies on
this to use LibreOffice as a calculation oracle.
