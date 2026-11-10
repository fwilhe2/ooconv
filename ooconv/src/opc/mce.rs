// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Markup Compatibility and Extensibility (ECMA-376 Part 3) — how a producer writes content a
//! consumer may not understand, and so in nearly every file written since about 2010 (15 of
//! LibreOffice's 360 test workbooks carry an `mc:AlternateContent` in a worksheet,
//! grind `doc/xlsx-format.md` §1.3).
//!
//! - **`mc:Ignorable="x14ac xr"`** names prefixes whose attributes may be skipped. Nothing to do:
//!   an attribute in an unknown namespace already misses every lookup.
//! - **`mc:AlternateContent`** is the one that needs code. See [`Alternative`].
//! - **`mc:MustUnderstand`** is reported and **never refused**: it guards a feature, not the
//!   content an import is for.
//!
//! All pure functions over names and attribute strings. The handling sits once, in `xml.rs`'s
//! dispatcher, since `mc:AlternateContent` can wrap a sparkline group, a slicer, a validation, a
//! drawing or a `sheetPr`, and per-site handling is per-site bugs.

/// What an element in the markup-compatibility namespace is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alternative {
    /// `<mc:AlternateContent>` — a wrapper holding the alternatives below.
    Wrapper,
    /// `<mc:Choice Requires="x14">` — content for a consumer that understands `x14`.
    Choice,
    /// `<mc:Fallback>` — content for one that understands none of the choices.
    Fallback,
    /// Something else in that namespace. Ignored like any other unknown element.
    Other,
}

pub fn classify(local: &str) -> Alternative {
    match local {
        "AlternateContent" => Alternative::Wrapper,
        "Choice" => Alternative::Choice,
        "Fallback" => Alternative::Fallback,
        _ => Alternative::Other,
    }
}

/// Do we satisfy a `Requires="…"`? Only the empty requirement: no extension namespace is
/// understood, so the `mc:Fallback` is what gets read — correctly, since a `Choice` requiring
/// `x14` holds `x14` content. A function so that understanding one has one place to go.
pub fn satisfies(requires: &str) -> bool {
    requires.split_whitespace().next().is_none()
}

/// The namespace prefixes a `mc:MustUnderstand="x14 x15"` names, in order. The report carries
/// URIs, resolved by `xml::uri_of` while the declaring element's scope is live.
pub fn must_understand(value: &str) -> impl Iterator<Item = &str> {
    value.split_whitespace()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_elements_that_matter() {
        assert_eq!(classify("AlternateContent"), Alternative::Wrapper);
        assert_eq!(classify("Choice"), Alternative::Choice);
        assert_eq!(classify("Fallback"), Alternative::Fallback);
        assert_eq!(classify("Ignorable"), Alternative::Other);
    }

    /// The rule, stated as a test so that changing it is deliberate.
    #[test]
    fn we_require_nothing_so_we_understand_nothing() {
        assert!(!satisfies("x14"));
        assert!(!satisfies("x14 x15"));
        // Whitespace-only is an empty requirement, not a requirement for a namespace named
        // with a space.
        assert!(satisfies(""));
        assert!(satisfies("   "));
    }

    #[test]
    fn must_understand_is_a_list_of_prefixes() {
        let got: Vec<_> = must_understand("x14  x15\nxr").collect();
        assert_eq!(got, ["x14", "x15", "xr"]);
        assert_eq!(must_understand("").count(), 0);
    }
}
