<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# ooconv — the plan

A library and a command that convert between Office Open XML and OpenDocument, **both ways**:
`.xlsx` ↔ `.ods`/`.fods`, `.docx` ↔ `.odt`/`.fodt`. It exists because
[grind](https://github.com/fwilhe2/grind) reads Microsoft formats but will not write them, and a
converter that writes them is a separate product with its own obligations. Its method is grind's:
a spec-first clean room, measured facts, and external applications as the test oracle.

## The requirements

These are normative. A change that breaks one is wrong even if every test passes.

- **R1 — Parsed, never patched.** Every document is read into a neutral model (`sheet::Workbook`,
  `text::Document`) and written from it. Formulas are parsed into one AST and printed by the
  target dialect's printer; no string surgery on markup or formula text anywhere.
- **R2 — Everything written validates.** ODF output against the ODF 1.4 RELAX NG schema (`jing`),
  OOXML output against the Open XML SDK's `OpenXmlValidator`. Both forms of ODF, every
  fixture, every corpus document (`ooconv/tests/schema.rs`).
- **R3 — Minimal output.** Only the parts, namespaces and styles the content uses. A document
  is small enough to read, and nothing in it is there by habit.
- **R4 — Standard vocabulary only.** No `calcext:`, `loext:`, `x14:`, `w14:` or any other
  extension is *written*. Strictness on the way out.
- **R5 — Tolerant on the way in.** An unrecognised element is skipped with its subtree, markup
  compatibility takes the `mc:Fallback`, LibreOffice's aliases are understood. A malformed
  fragment costs that fragment, not the document.
- **R6 — Every loss is reported.** Anything the reader *recognises* as content that the model or
  the target cannot hold is a `Loss` in the `Report`, by stable class and count. Nothing is
  dropped silently. `--strict` (`Options::strict`) refuses instead of writing.
- **R7 — The applications read it as meant.** Correctness is the oracles' judgement, not ours:
  LibreOffice, an Excel-compatible spreadsheet (`gridcraft-cli`) and a Word-compatible word
  processor (`wordcraft-cli`) open every document written and must see what the model says
  (`doc/oracles.md`). A difference is a bug or a named divergence, never ignored.
- **R8 — Clean room.** Other implementations — LibreOffice, the Open XML SDK, grind's own
  filters' sources — are oracles and corpora. A fact learned from one is written into the format
  notes (`doc/xlsx-format.md`, `doc/docx-format.md`, `doc/odf-format.md`) with its evidence
  *before* it reaches code. A fact marked **`UNVERIFIED` may not be implemented.**
- **R9 — Library first, no environment.** Every operation is bytes in, bytes out; the library
  touches no filesystem and no network. The CLI is a thin shell over it.
- **R10 — Hostile input is expected.** Zip bombs, entity bombs, zip-slip, nesting and
  repetition bombs are capped and tested (`ooconv/tests/hostile.rs`).

## Architecture

```
          xlsx_read ─┐                 ┌─ xlsx_write          (sheet/)
 ods_read (both forms)┼─▶ Workbook ───▶┼─ ods_write (both forms)
                                        
          docx_read ─┐                 ┌─ docx_write          (text/)
 odt_read (both forms)┼─▶ Document ───▶┼─ odt_write (both forms)
```

| Module | Holds |
|---|---|
| `names` | Every namespace URI both families use, relationship types, the OOXML flavour (Transitional/Strict) |
| `xml::read` | The tolerant walker (lifted from grind's `ooxml/src/xml.rs`), markup compatibility, mixed content |
| `xml::write` | The one XML writer: escaping, balance, and no character XML cannot carry |
| `opc` | OPC packages: reading (lifted from grind), writing with generated content types and relationships |
| `odf` | ODF packages and the flat form; ODF whitespace in both directions |
| `sheet` | `Workbook`, the formula AST with an Excel and an OpenFormula dialect, number formats, dates |
| `text` | `Document`: paragraphs, headings, lists, tables, runs |
| `report` | `Loss` and `Report` |

The model holds only what both formats spell the same way. Its canonical form — what every
reader returns and every round trip must reproduce — is documented on its types: dates against
1899-12-30, start alignment as `None`, lists numbered by first appearance, no filler paragraph
after a final table, at least one sheet.

## Scope

**Carried, both ways.** Sheets (order, names, hidden), cell values (number, text, boolean,
error), formulas with their cached results (references in every form, 3-D ranges, whole rows
and columns, operators including intersection and union, inline arrays, array formulas, shared
formulas expanded, function names mapped as measured), number-format *shapes* (fixed, percent,
scientific, date, time, date-time, text), merged cells, column widths, the criteria mode.
Paragraphs, headings 1–6, alignment, bold/italic/underline/strike/super/subscript, hyperlinks,
tabs and line breaks, bullet and numbered lists nested to nine levels, tables with row and
column spans and nesting.

**Reported, not carried (yet).** Defined names, cell styling (fonts, fills, borders), number
formats beyond their shape, comments, drawings, charts, pictures, conditional formatting, data
validation, auto-filters, pivot tables, headers and footers, footnotes, page and section setup,
indexes (kept as their text), page breaks. Each is a `Loss` class; `ooconv --strict` names them.

**Not in scope.** `.xls`, `.doc`, `.xlsb`, presentations, macros, encryption (a locked file is
`Error::Encrypted`).

## Relation to grind

grind's `grind-ooxml` became `opc` and `xml::read` here, and grind's measured facts about
reading `.xlsx` and `.docx` are cited from its `doc/xlsx-format.md` and `doc/docx-format.md`
rather than copied. grind could depend on this crate to *write* Microsoft formats — an adapter
from its model to `Workbook`/`Document` — without its own model learning anything about them.
Both are AGPL-3.0-or-later.
