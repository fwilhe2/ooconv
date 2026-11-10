// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Namespace URIs, relationship types, and the flavour they say the file is. Lifted from grind
//! (`ooxml/src/names.rs` and `core/src/odf/names.rs`, merged so one walker serves both families),
//! with their two rules: **dispatch on `(namespace-uri,
//! local-name)`, never the prefix**, and **an unrecognised URI is a value** ([`Ns::Other`]) that
//! every lookup misses.
//!
//! Every Part 1 namespace has two spellings, Transitional and Strict (ISO/IEC 29500); real files
//! are overwhelmingly Transitional. Both resolve to the same [`Ns`], so nothing downstream
//! branches on the flavour — the reader only records it for the report. Every URI is measured:
//! grind `doc/xlsx-format.md` §1.1–1.2 and `doc/docx-format.md` §1.
//!
//! The ODF URIs are OASIS's and have one spelling each.

/// A namespace this filter recognises. Everything else is [`Ns::Other`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ns {
    /// SpreadsheetML itself — `worksheet`, `sheetData`, `row`, `c`, `f`, `v`.
    Spreadsheet,
    /// WordprocessingML — `document`, `body`, `p`, `r`, `t`. The word processor's main
    /// namespace, and the one `grind-docx` dispatches on.
    Word,
    /// DrawingML's WordprocessingML placement vocabulary: `wp:inline`, `wp:anchor`, `wp:extent`
    /// — where a picture sits in a paragraph and how big it is.
    WordDrawing,
    /// DrawingML's picture: `pic:pic`, `pic:blipFill`.
    Picture,
    /// The `r:` attribute namespace: `r:id` on a `<sheet>` names a relationship.
    Relationships,
    /// The root namespace of a `_rels/*.rels` part — Part 2 (OPC), so the same in both flavours;
    /// measured.
    PackageRels,
    /// `[Content_Types].xml`. Part 2 as well, and likewise invariant.
    ContentTypes,
    /// Markup Compatibility and Extensibility — Part 3, invariant, and the reason `mce.rs`
    /// exists.
    Mce,
    /// DrawingML's main namespace: a theme's colour scheme (`<a:clrScheme>`) and a picture's
    /// `a:blip`. Both spellings; the Strict one measured on Word's Strict documents
    /// (grind `doc/docx-format.md` §1.2).
    Drawing,
    /// VML, the drawing language of Excel before DrawingML, still where a form control lives
    /// (grind `doc/xlsx-format.md` §4.11): `v:shape`.
    Vml,
    /// VML's Excel vocabulary: `x:ClientData` and what a control says in it.
    VmlExcel,
    // ---- OpenDocument ----
    Office,
    Table,
    Text,
    Style,
    /// `fo:` — the XSL-FO-compatible properties a style is mostly made of.
    Fo,
    Svg,
    Xlink,
    /// `number:` — the data-style namespace, whose URI says `datastyle`.
    Number,
    Draw,
    Meta,
    Dc,
    /// `META-INF/manifest.xml`'s own vocabulary.
    Manifest,
    /// LibreOffice's calc extension namespace: carries an alias for `office:value-type`.
    Calcext,
    /// Bound to `xml` by XML itself: `xml:space`, `xml:id`.
    Xml,
    /// No namespace at all. Most SpreadsheetML *attributes* are unprefixed (`<sheet name=…
    /// sheetId=…>`), so this is a real answer rather than an error case.
    None,
    Other,
}

/// Which spelling of Part 1 a file uses — a fact the report states, never a branch the reader
/// takes. `Mixed` is real: `universal-content-strict.xlsx` in LibreOffice's corpus carries Strict
/// relationship types beside Transitional ones (grind `doc/xlsx-format.md` §1.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Flavour {
    /// What Excel has written by default since 2007, and what every other producer writes.
    #[default]
    Transitional,
    /// ISO/IEC 29500 Strict. Recognised; not a supported configuration.
    Strict,
    /// Both, in one document.
    Mixed,
}

/// Which family a Part 1 URI belongs to. Collected as the reader resolves names; `Flavour`
/// falls out of the set at the end.
#[derive(Clone, Copy, Debug, Default)]
pub struct Seen {
    pub transitional: bool,
    pub strict: bool,
}

impl Seen {
    pub fn flavour(self) -> Flavour {
        match (self.transitional, self.strict) {
            (true, true) => Flavour::Mixed,
            (false, true) => Flavour::Strict,
            // Neither declared: Transitional by default. Such a file has no SpreadsheetML in it,
            // and the caller has already refused it for that.
            _ => Flavour::Transitional,
        }
    }
}

// ---- Part 1, Transitional (ECMA-376 1st edition spellings) ----

pub const MAIN_T: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
/// Measured from `styles/colors.xlsx`'s theme part, 2026-09-21.
pub const DRAWING_T: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
pub const REL_T: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// Measured from LibreOffice's own `.docx` output and Word's, 2026-10-05.
pub const WORD_T: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
pub const WORD_DRAWING_T: &str =
    "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
pub const PICTURE_T: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";

// ---- Part 1, Strict (ISO/IEC 29500 Strict) ----

pub const MAIN_S: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
pub const REL_S: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
/// The four below measured from `sw/qa/extras/ooxmlexport/data/strict.docx` and its
/// neighbours, 2026-10-05 (grind `doc/docx-format.md` §1.2).
pub const WORD_S: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
pub const WORD_DRAWING_S: &str = "http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing";
pub const PICTURE_S: &str = "http://purl.oclc.org/ooxml/drawingml/picture";
pub const DRAWING_S: &str = "http://purl.oclc.org/ooxml/drawingml/main";

// ---- Parts 2 and 3: one spelling each, in both flavours ----

pub const PACKAGE_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
pub const CONTENT_TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
pub const MCE: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
/// VML and its Excel vocabulary — one spelling in every file, since VML predates both flavours.
pub const VML: &str = "urn:schemas-microsoft-com:vml";
pub const VML_EXCEL: &str = "urn:schemas-microsoft-com:office:excel";

// ---- OpenDocument (OASIS, one spelling) ----

pub const ODF_OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
pub const ODF_TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
pub const ODF_TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
pub const ODF_STYLE: &str = "urn:oasis:names:tc:opendocument:xmlns:style:1.0";
pub const ODF_FO: &str = "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0";
pub const ODF_SVG: &str = "urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0";
pub const ODF_NUMBER: &str = "urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0";
pub const ODF_DRAW: &str = "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0";
pub const ODF_META: &str = "urn:oasis:names:tc:opendocument:xmlns:meta:1.0";
pub const ODF_MANIFEST: &str = "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0";
pub const ODF_CALCEXT: &str =
    "urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0";
pub const XLINK: &str = "http://www.w3.org/1999/xlink";
pub const DC: &str = "http://purl.org/dc/elements/1.1/";
pub const XML: &str = "http://www.w3.org/XML/1998/namespace";

impl Ns {
    pub fn from_uri(uri: &str) -> Ns {
        match uri {
            ODF_OFFICE => Ns::Office,
            ODF_TABLE => Ns::Table,
            ODF_TEXT => Ns::Text,
            ODF_STYLE => Ns::Style,
            ODF_FO => Ns::Fo,
            ODF_SVG => Ns::Svg,
            ODF_NUMBER => Ns::Number,
            ODF_DRAW => Ns::Draw,
            ODF_META => Ns::Meta,
            ODF_MANIFEST => Ns::Manifest,
            ODF_CALCEXT => Ns::Calcext,
            XLINK => Ns::Xlink,
            DC => Ns::Dc,
            XML => Ns::Xml,
            MAIN_T | MAIN_S => Ns::Spreadsheet,
            WORD_T | WORD_S => Ns::Word,
            WORD_DRAWING_T | WORD_DRAWING_S => Ns::WordDrawing,
            PICTURE_T | PICTURE_S => Ns::Picture,
            REL_T | REL_S => Ns::Relationships,
            PACKAGE_RELS => Ns::PackageRels,
            CONTENT_TYPES => Ns::ContentTypes,
            MCE => Ns::Mce,
            DRAWING_T | DRAWING_S => Ns::Drawing,
            VML => Ns::Vml,
            VML_EXCEL => Ns::VmlExcel,
            "" => Ns::None,
            _ => Ns::Other,
        }
    }
}

/// Which flavour a URI is evidence of, if either. Only Part 1 URIs count: the package and
/// markup-compatibility namespaces are shared, and counting them for Transitional would make every
/// Strict file `Mixed`.
pub fn family(uri: &str) -> Option<Flavour> {
    // `starts_with` rather than equality because a relationship *type* extends the
    // relationships namespace with a segment, and both are evidence of the same family.
    if uri == MAIN_T || uri == WORD_T || uri.starts_with(REL_T) {
        Some(Flavour::Transitional)
    } else if uri == MAIN_S || uri == WORD_S || uri.starts_with(REL_S) {
        Some(Flavour::Strict)
    } else {
        None
    }
}

/// A relationship's `Type`, which is how a part is found — never by path convention, since nothing
/// in the spec promises `xl/workbook.xml`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelType {
    OfficeDocument,
    Worksheet,
    ChartSheet,
    SharedStrings,
    Styles,
    Theme,
    /// A worksheet's DrawingML drawing, where its charts and pictures hang.
    Drawing,
    /// A drawing's chart part.
    Chart,
    /// A worksheet's comments part, or a Word document's, which shares the type. Its VML anchor
    /// (`vmlDrawing`) is not a `Drawing`.
    Comments,
    /// A worksheet's legacy VML drawing: a comment's anchor, and every form control's own
    /// spelling (`controls.rs`).
    VmlDrawing,
    /// A worksheet's pivot table definition.
    PivotTable,
    /// A worksheet's table definition (`xl/tables/tableN.xml`) — read, because a structured
    /// reference is only a range once the table says which.
    Table,
    /// Recognised so it can be *ignored* cheaply: a calculation chain is Excel's evaluation
    /// order cache and says nothing this filter wants.
    CalcChain,
    // ---- WordprocessingML's parts (grind `doc/docx-format.md` §1.3) ----
    /// `word/numbering.xml` — what makes a paragraph a list item, and which kind.
    Numbering,
    /// A header part (`word/header1.xml`), named from a section's `w:headerReference`.
    Header,
    Footer,
    Footnotes,
    Endnotes,
    /// A picture's bytes (`word/media/image1.png`), named from an `a:blip`'s `r:embed`.
    Image,
    /// A hyperlink's target, always `TargetMode="External"` and **never fetched**.
    Hyperlink,
    /// An embedded OLE object — counted.
    OleObject,
    /// `word/settings.xml` — the default tab stop, and whether even pages have headers of
    /// their own.
    Settings,
    /// `word/fontTable.xml` — each font's family class (`swiss`, `roman`, …), which is what a
    /// reader without the font falls back on.
    FontTable,
    Other,
}

impl RelType {
    /// Both families, by stripping the family's own prefix and matching the rest — not just the
    /// last segment, which would accept `http://example.invalid/relationships/worksheet`.
    pub fn from_uri(uri: &str) -> RelType {
        let rest = uri
            .strip_prefix(REL_T)
            .or_else(|| uri.strip_prefix(REL_S))
            .and_then(|rest| rest.strip_prefix('/'));
        match rest {
            Some("officeDocument") => RelType::OfficeDocument,
            Some("worksheet") => RelType::Worksheet,
            Some("chartsheet") => RelType::ChartSheet,
            Some("sharedStrings") => RelType::SharedStrings,
            Some("styles") => RelType::Styles,
            Some("theme") => RelType::Theme,
            Some("drawing") => RelType::Drawing,
            Some("chart") => RelType::Chart,
            Some("comments") => RelType::Comments,
            Some("vmlDrawing") => RelType::VmlDrawing,
            Some("pivotTable") => RelType::PivotTable,
            Some("table") => RelType::Table,
            Some("calcChain") => RelType::CalcChain,
            Some("numbering") => RelType::Numbering,
            Some("header") => RelType::Header,
            Some("footer") => RelType::Footer,
            Some("footnotes") => RelType::Footnotes,
            Some("endnotes") => RelType::Endnotes,
            Some("image") => RelType::Image,
            Some("hyperlink") => RelType::Hyperlink,
            Some("oleObject") => RelType::OleObject,
            Some("settings") => RelType::Settings,
            Some("fontTable") => RelType::FontTable,
            _ => RelType::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_families_resolve_to_one_key() {
        assert_eq!(Ns::from_uri(MAIN_T), Ns::Spreadsheet);
        assert_eq!(Ns::from_uri(MAIN_S), Ns::Spreadsheet);
        assert_eq!(Ns::from_uri(REL_T), Ns::Relationships);
        assert_eq!(Ns::from_uri(REL_S), Ns::Relationships);
    }

    #[test]
    fn an_unknown_uri_is_a_value_not_a_failure() {
        assert_eq!(
            Ns::from_uri("http://schemas.microsoft.com/office/spreadsheetml/2014/revision"),
            Ns::Other
        );
        assert_eq!(Ns::from_uri(""), Ns::None);
    }

    /// The whole point of the flavour machinery: only Part 1 votes.
    #[test]
    fn the_invariant_namespaces_are_evidence_of_nothing() {
        assert_eq!(family(MAIN_T), Some(Flavour::Transitional));
        assert_eq!(family(MAIN_S), Some(Flavour::Strict));
        assert_eq!(family(PACKAGE_RELS), None);
        assert_eq!(family(CONTENT_TYPES), None);
        assert_eq!(family(MCE), None);
    }

    #[test]
    fn a_relationship_type_votes_too() {
        assert_eq!(
            family("http://purl.oclc.org/ooxml/officeDocument/relationships/worksheet"),
            Some(Flavour::Strict)
        );
    }

    #[test]
    fn flavour_from_what_was_seen() {
        let both = Seen {
            transitional: true,
            strict: true,
        };
        assert_eq!(both.flavour(), Flavour::Mixed);
        assert_eq!(Seen::default().flavour(), Flavour::Transitional);
        assert_eq!(
            Seen {
                strict: true,
                ..Seen::default()
            }
            .flavour(),
            Flavour::Strict
        );
    }

    #[test]
    fn relationship_types_in_both_spellings() {
        for base in [REL_T, REL_S] {
            assert_eq!(
                RelType::from_uri(&format!("{base}/officeDocument")),
                RelType::OfficeDocument
            );
            assert_eq!(
                RelType::from_uri(&format!("{base}/worksheet")),
                RelType::Worksheet
            );
            assert_eq!(
                RelType::from_uri(&format!("{base}/sharedStrings")),
                RelType::SharedStrings
            );
        }
    }

    /// `universal-content-strict.xlsx` really carries this, with a lower-case `officedocument`
    /// (grind `doc/xlsx-format.md` §1.2): unrecognised, so ignored rather than guessed at.
    #[test]
    fn a_misspelt_type_is_unrecognised_rather_than_guessed() {
        assert_eq!(
            RelType::from_uri(
                "http://schemas.openxmlformats.org/officedocument/2006/relationships/metadata/core-properties"
            ),
            RelType::Other
        );
    }

    /// Matching the last segment instead of the prefix would accept this.
    #[test]
    fn a_familiar_word_in_a_foreign_vocabulary_is_not_a_relationship_type() {
        assert_eq!(
            RelType::from_uri("http://example.invalid/relationships/worksheet"),
            RelType::Other
        );
    }
}
