<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# WordprocessingML, as measured

The facts the docx reader and writer stand on, under `doc/xlsx-format.md`'s markers:
**`MEASURED`**, **`SPEC`**, and **`UNVERIFIED`** (which may not be implemented). Reading facts
are grind's: grind `doc/docx-format.md` (namespaces and Strict §1.1–1.2, the parts by frequency
§1.3, `w:`-namespaced attributes §1.4).

---

## 1. What LibreOffice writes — `MEASURED`

LibreOffice 26.8.2.2, 2026-11-10, from an HTML page with a heading, inline formatting, a link,
nested and numbered lists and a table:

```sh
soffice --headless --convert-to 'docx:MS Word 2007 XML' t.html
```

- Parts: `word/{document,styles,numbering,fontTable,settings}.xml`, `word/theme/theme1.xml`,
  `docProps/{app,core}.xml`. The validator accepts it.
- A heading is `w:pStyle w:val="Heading1"`; bold `<w:b/>`, italic `<w:i/>`, underline
  `<w:u w:val="single"/>`, strike `<w:strike/>`.
- A link is `<w:hyperlink r:id>` around runs styled `Hyperlink`, its relationship
  `TargetMode="External"`.
- A list item is `w:numPr` (`w:ilvl`, `w:numId`) on a paragraph; bullets and numbers are two
  `w:num`s over two `w:abstractNum`s, nine levels each.
- A table is `w:tbl` › `w:tblGrid` › `w:tr` › `w:tc`, each cell ending in a `w:p`.

## 2. Rules Word keeps that the schema does not say

### 2.1 The body ends with a paragraph — `MEASURED`

A document whose last block is a table reads, in `wordcraft-cli inspect` 0.16.0 (a
Word-compatible processor), with one more block than it was written with: an empty paragraph
after the table (2026-11-10, `odt/03-tables-and-pagination.odt` → docx). The writer always ends
the body with a paragraph, and every reader treats an empty paragraph directly after a final
table as the format's filler (`Document::drop_table_filler`). LibreOffice writes the same filler.

### 2.2 A cell ends with a paragraph — `SPEC`

ECMA-376 §17.4.66: a `w:tc` must contain at least one block-level element, and its last must be
a `w:p`. The writer adds an empty paragraph after a nested table that ends a cell.

### 2.3 Every instance of a list shares its abstract list's counters — `SPEC`

§17.9.27 `lvlOverride`/`startOverride`: a `w:num` that does not override its start continues
the counters of the `w:abstractNum` it instantiates. So each numbered list is its own `w:num`
*with* `startOverride` 1 on every level, or the second numbered list would continue the first.

## 3. Resolution rules the reader applies

- **Headings** — `SPEC` §17.7.4.9 `outlineLvl` in the paragraph style's ancestry, else a style
  *named* `heading N` (the built-in name, §17.7.4.9's latent styles), levels 1–6.
- **Lists** — the paragraph's `w:numPr`, else its style's; a level whose `w:numFmt` is `none`
  is not a list item; `bullet` is unordered, every other format ordered.
- **Character styles** are resolved onto runs (`Strong` becomes bold). **Paragraph styles'** run
  properties are not: they are the style's look, and a heading's bold is its level.
- **The `Hyperlink` character style inside a link** is the link's appearance and is not
  resolved onto the run: carrying its underline would add formatting on every round trip.
  Found by loop L (2026-11-10).
- **Complex fields** — the result between `separate` and `end` is text; the instruction is not.
- **Revisions** — insertions are content, deletions are not.
