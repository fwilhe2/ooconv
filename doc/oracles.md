<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# The oracles

R7 says correctness is the applications' judgement. This is which applications, how each is
asked, and what each loop asserts.

| Loop | Asserts | Oracle | File |
|---|---|---|---|
| R | `read(write(m)) == m` for the fixtures and every corpus document, every format; `FLOOR` on exact round trips | none — **never skips** | `ooconv/tests/roundtrip.rs` |
| S | everything written validates (R2) | `jing` + ODF 1.4 RNG; Open XML SDK `OpenXmlValidator` | `ooconv/tests/schema.rs` |
| L | LibreOffice reads what we write as we meant it, and calculates every formula the same from our xlsx as from our ods | `soffice` | `ooconv/tests/libreoffice.rs` |
| G | an Excel-compatible spreadsheet opens and round-trips every xlsx, calculates the fixture to its true caches, and agrees with LibreOffice wherever a corpus cache is disputed | `gridcraft-cli check` | `ooconv/tests/gridcraft.rs` |
| W | a Word-compatible word processor sees every block, character format, link and table cell we meant | `wordcraft-cli inspect` | `ooconv/tests/wordcraft.rs` |
| H | each hostile file has exactly one outcome | none — **never skips** | `ooconv/tests/hostile.rs` |

Rules, grind's unchanged:

- A **`FLOOR`** is a ratchet: raise it, never lower it.
- A **divergence table** names a difference by *what it is*, never by file. It is checked
  **both ways**: a difference not in the table fails, and an entry that no longer occurs fails
  too — delete it. Each loop has its named divergences and no others.
- A corpus file is never special-cased by name. (The zip bomb is skipped in loop R and S for
  time, and held by loop H.)
- **Add to `ooconv/tests/common/fixtures.rs` when adding a feature**, so every oracle sees it
  whatever the corpus contains.

## Getting the oracles

```sh
scripts/fetch-tools.sh          # jing (pinned by sha256) and the Open XML SDK validator
```

| Oracle | Found as | Pinned |
|---|---|---|
| `jing` 20220510 | `.tools/jing/jing.jar`, or `OOCONV_JING` | by checksum in `scripts/fetch-tools.sh`; needs `java` |
| `ooxml-validate` | `.tools/ooxml-validate/ooxml-validate`, or `OOCONV_OOXML_VALIDATE` | `DocumentFormat.OpenXml` 3.5.1 in `tools/ooxml-validate/*.csproj`; needs the .NET SDK |
| `soffice` | `PATH` | `ci/libreoffice-image` by digest; `scripts/soffice-docker/soffice` puts it on `PATH` |
| `gridcraft-cli`, `wordcraft-cli` | `PATH` | not pinned: installed system-wide; versions measured with are 0.17.0 and 0.16.0 |

A missing oracle **skips with a notice**. `OOCONV_REQUIRE_ORACLES=all` (or a list:
`jing,ooxml-validate,soffice`) turns a skip into a failure; CI sets it for what CI has.

```sh
OOCONV_REQUIRE_ORACLES=all cargo test --workspace                       # everything, locally
PATH="$PWD/scripts/soffice-docker:$PATH" cargo test -p ooconv --test libreoffice   # the pin
cargo test --release -p ooconv --test hostile                           # includes the zip bomb
```

## How each oracle is asked

**LibreOffice.** Every document is converted with `soffice --headless --convert-to fods|fodt`
under a private profile whose `registrymodifications.xcu` sets `OOXMLRecalcMode` and
`ODFRecalcMode` to 0, *always recalculate*: a cached value is the producer's claim, LibreOffice's
calculation is the check. Our reader reads LibreOffice's flat ODF — not schema-valid, which is
fine, since the claim is about meaning. Formula results from the corpus are compared *across
formats* (our xlsx against our ods) because the corpus's caches are often synthetic; the
fixture's caches are true and are compared directly.

**gridcraft.** `gridcraft-cli check <files> --json` opens each workbook, recalculates every
formula against its cached value, and round-trips it through its own xlsx writer. Disputed
corpus caches are settled by asking LibreOffice the same cell: if two independent engines agree,
the cache was wrong, not our translation.

**wordcraft.** `wordcraft-cli inspect <file>` gives blocks with text, style, list level,
alignment and run properties by UTF-16 offset; the test compares each character's formatting
and link, and each table's grid of cell texts.

**The validators.** `jing -i` validates flat documents and each XML part of a package;
`ooxml-validate` (`tools/ooxml-validate/Program.cs`) runs `OpenXmlValidator` at
`FileFormatVersions.Office2007`, the strictest level, and prints one line per error.

## What the loops have found

Recorded because a loop that never failed proves nothing.

- **Loop L, first run:** a space before a formatting change vanished (`Plain, bold` →
  `Plain,bold`) — the ODF whitespace collapser forgot it had produced text once a run was taken.
  Our own round trip could not see it, because our writer escapes edge spaces.
- **Loop L:** LibreOffice keeps a single trailing space it writes literally; our reader stripped
  it. `doc/odf-format.md` §2.1.
- **Loop L:** `COUNTIF(…;"A-*")` was 0 in LibreOffice and 3 in Excel: an ODF document without
  `table:use-wildcards="true"` reads criteria as regular expressions. The model now carries the
  criteria mode and the ODS writer always states it.
- **Loop L:** `Data!A:A` printed as `Data!A` after a change for 3-D references.
- **Loop W:** a document ending in a table needs a paragraph after it in Word.
- **Loop G:** none of ours: every disputed cache was synthetic, confirmed by LibreOffice.
