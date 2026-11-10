// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a conversion did not carry. Every reader and writer reports by **class** — a fixed label
//! such as `"merged cells"` — and a count, so a loss can be counted, explained and looked for,
//! and so `--strict` has something exact to refuse on.

use std::collections::BTreeMap;
use std::fmt;

/// One class of content that did not survive, and how often.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loss {
    /// A stable label, never a sentence assembled from the document: `"array formula"`.
    pub class: &'static str,
    /// How many instances were dropped.
    pub count: usize,
    /// The first instance, for a person to find it: a cell address, an element name.
    pub example: String,
}

impl fmt::Display for Loss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} × {}", self.count, self.class)?;
        if !self.example.is_empty() {
            write!(f, " (first: {})", self.example)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Sorted by class.
    pub losses: Vec<Loss>,
    /// The OOXML flavour the source used, when it was OOXML.
    pub flavour: Option<crate::Flavour>,
}

impl Report {
    /// Count one instance of `class`, remembering `example` if it is the first.
    pub fn lose(&mut self, class: &'static str, example: impl fmt::Display) {
        match self.losses.binary_search_by(|l| l.class.cmp(class)) {
            Ok(i) => self.losses[i].count += 1,
            Err(i) => self.losses.insert(
                i,
                Loss {
                    class,
                    count: 1,
                    example: example.to_string(),
                },
            ),
        }
    }

    pub fn count(&self, class: &str) -> usize {
        self.losses
            .iter()
            .find(|l| l.class == class)
            .map_or(0, |l| l.count)
    }

    /// The losses as a map, for tests that compare the whole report.
    pub fn by_class(&self) -> BTreeMap<&'static str, usize> {
        self.losses.iter().map(|l| (l.class, l.count)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn losses_aggregate_by_class_and_keep_the_first_example() {
        let mut r = Report::default();
        r.lose("merged cells", "Sheet1!A1");
        r.lose("array formula", "Sheet1!B2");
        r.lose("merged cells", "Sheet1!C3");
        assert_eq!(r.count("merged cells"), 2);
        assert_eq!(r.losses[0].class, "array formula", "sorted by class");
        assert_eq!(r.losses[1].example, "Sheet1!A1");
    }
}
