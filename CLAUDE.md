# CLAUDE.md

<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

Guidance for Claude Code in this repo. The reasoning behind code lives in the code and in
`doc/*.md`; read those before touching an area.

## What this is

`ooconv`: a library and CLI converting `.xlsx` ↔ `.ods`/`.fods` and `.docx` ↔ `.odt`/`.fodt`
through a neutral model. A sibling of [grind](../grind), which reads Microsoft formats and never
writes them; grind may depend on this crate later. `doc/plan.md`'s R1–R10 are normative.

## The constraints that govern everything

1. **Clean room (R8).** LibreOffice, the Open XML SDK and grind's filters are oracles and corpora,
   never sources. A fact goes into `doc/xlsx-format.md`, `doc/docx-format.md` or
   `doc/odf-format.md` with its evidence (`MEASURED` + command + date, or `SPEC` + section)
   **before** it reaches code. **An `UNVERIFIED` fact may not be implemented** — refuse and
   report instead (see `names::UNSPELLED`).
2. **The applications decide (R7).** A change is right when the oracle loops say so, not when our
   own round trip does: loop R passing proves only that our reader and writer agree with each
   other. Two bugs so far were invisible to it and caught by LibreOffice (`doc/oracles.md`).
3. **Strict out, tolerant in (R2, R4, R5).** Writers emit only schema vocabulary and must
   validate; readers skip what they do not know. Every recognised thing not carried is a
   `Report` loss with a stable class label (R6) — never a silent drop.

## Commands

```sh
cargo test --workspace                          # oracles that are missing skip with a notice
OOCONV_REQUIRE_ORACLES=all cargo test --workspace
cargo test -p ooconv --test libreoffice         # one loop
cargo test --release -p ooconv --test hostile   # includes the zip bomb
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
RUSTDOCFLAGS=-D warnings cargo doc --no-deps
scripts/fetch-tools.sh                          # jing + the Open XML SDK validator into .tools/
PATH="$PWD/scripts/soffice-docker:$PATH" cargo test -p ooconv --test libreoffice   # the CI pin
cargo run -p ooconv-cli -- book.xlsx book.ods
```

## The loops

`doc/oracles.md` is the full table. R (round trip, never skips), S (schema: jing, Open XML SDK),
L (LibreOffice reads and recalculates), G (`gridcraft-cli`, an Excel-compatible spreadsheet),
W (`wordcraft-cli`, a Word-compatible word processor), H (hostile files, never skips).

- A `FLOOR` is a ratchet: raise it, never lower it.
- Divergence tables name a difference by what it is, never by file, and are checked **both
  ways**: an entry that stops occurring fails the test and must be deleted.
- **Adding a feature means adding it to `ooconv/tests/common/fixtures.rs`**, so every oracle
  sees it.
- Corpus caches (`ooconv/tests/data/corpus/xlsx`, from ooxmlgen) are often synthetic: a formula
  value is checked across formats and across engines, not against the cache.

## Architecture

| Path | Holds |
|---|---|
| `ooconv/src/names.rs`, `xml/read.rs`, `opc/{package,mce}.rs` | Lifted from grind's `grind-ooxml`, generalised to ODF namespaces |
| `ooconv/src/xml/write.rs` | The one XML writer — every byte goes through it |
| `ooconv/src/opc/write.rs`, `odf/` | Packages: OPC with generated content types and rels; ODF both forms; ODF whitespace |
| `ooconv/src/sheet/` | `Workbook`; `formula/` (one AST, `excel` and `odf` dialects, `names` tables); readers and writers |
| `ooconv/src/text/` | `Document`; readers and writers |
| `cli/` | The `ooconv` binary |
| `tools/ooxml-validate/` | The Open XML SDK validator (C#) |

Facts that are easy to get wrong:

- Positions are 0-based `(row, col)`. Dates in the model are serials against 1899-12-30;
  only the xlsx reader and writer know Excel's 1900 bug (`sheet/dates.rs`).
- Function names in the model are **OpenFormula** names (`COM.MICROSOFT.XLOOKUP`).
- The model has a canonical form (`doc/plan.md` § Architecture); readers produce it, and round
  trips compare against it.

## Conventions

- Comments say what code and names cannot, and stay true: no status, no "used to". A design
  argument goes in `doc/`.
- `ponytail:` marks a deliberate shortcut with a known ceiling and upgrade path.
- Every file carries SPDX metadata, or is covered by `REUSE.toml`. Code AGPL-3.0-or-later, prose
  CC-BY-4.0, test data CC0-1.0; `doc/OpenDocument-v1.4-schema.rng` is OASIS's, redistributed
  verbatim with a `.license` sidecar — never edit it.
