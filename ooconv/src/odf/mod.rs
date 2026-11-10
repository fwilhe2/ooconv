// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! OpenDocument's container, both forms (ODF 1.4 Part 2): the zip package and the flat XML
//! document. Content reading and writing live with each document kind.

pub mod package;

/// The ODF version every document here declares.
pub const VERSION: &str = "1.4";

pub const MIME_ODS: &str = "application/vnd.oasis.opendocument.spreadsheet";
pub const MIME_ODT: &str = "application/vnd.oasis.opendocument.text";

/// The namespace declarations every root element written here carries — only those a writer can
/// use, never LibreOffice's twenty-odd (`doc/plan.md`, R3).
pub const NAMESPACES: &[(&str, &str)] = &[
    ("xmlns:office", crate::names::ODF_OFFICE),
    ("xmlns:style", crate::names::ODF_STYLE),
    ("xmlns:text", crate::names::ODF_TEXT),
    ("xmlns:table", crate::names::ODF_TABLE),
    ("xmlns:fo", crate::names::ODF_FO),
    ("xmlns:number", crate::names::ODF_NUMBER),
    ("xmlns:svg", crate::names::ODF_SVG),
    ("xmlns:xlink", crate::names::XLINK),
    ("xmlns:of", "urn:oasis:names:tc:opendocument:xmlns:of:1.2"),
];

/// The pieces of one ODF document, each the *inside* of its element, which the two forms
/// assemble differently. Built by a kind's writer, assembled by [`package::assemble`].
#[derive(Default)]
pub struct Parts {
    pub mimetype: &'static str,
    /// `office:font-face-decls`'s children.
    pub font_faces: String,
    /// `office:styles`'s children — common styles, in `styles.xml` in a package.
    pub styles: String,
    /// `office:automatic-styles`'s children used by the body.
    pub automatic_styles: String,
    /// `office:automatic-styles`'s children used by `office:master-styles` (page layouts).
    pub page_layouts: String,
    /// `office:master-styles`'s children.
    pub master_styles: String,
    /// `office:body`'s single child, complete: `<office:spreadsheet>…</office:spreadsheet>`.
    pub body: String,
}

/// Text as ODF paragraph content (Part 3 §6.1.2–6.1.4): a space that whitespace collapsing would
/// eat — a second in a run, or one at either end — becomes `<text:s/>`, a tab `<text:tab/>`, a
/// line feed `<text:line-break/>`. What the reader's collapsing gives back is then exactly `s`.
pub fn write_text(w: &mut crate::xml::write::Writer, s: &str) {
    let chars: Vec<char> = s.chars().filter(|c| *c != '\r').collect();
    let mut buf = String::new();
    let flush = |w: &mut crate::xml::write::Writer, buf: &mut String| {
        if !buf.is_empty() {
            w.text(&std::mem::take(buf));
        }
    };
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            ' ' => {
                let start = i;
                while i < chars.len() && chars[i] == ' ' {
                    i += 1;
                }
                let edge = |c: Option<&char>| c.is_none_or(|c| matches!(c, '\t' | '\n'));
                let inner =
                    !edge(start.checked_sub(1).and_then(|p| chars.get(p))) && !edge(chars.get(i));
                let mut escaped = i - start;
                if inner {
                    buf.push(' ');
                    escaped -= 1;
                }
                if escaped > 0 {
                    flush(w, &mut buf);
                    w.start("text:s");
                    if escaped > 1 {
                        w.attr("text:c", escaped.to_string());
                    }
                    w.end();
                }
                continue;
            }
            c @ ('\t' | '\n') => {
                flush(w, &mut buf);
                w.start(if c == '\t' {
                    "text:tab"
                } else {
                    "text:line-break"
                })
                .end();
            }
            c => buf.push(c),
        }
        i += 1;
    }
    flush(w, &mut buf);
}

/// Builds a paragraph's text the way ODF's whitespace rules read it (Part 3 §6.1.2): runs of
/// space, tab, CR and LF in *character data* become one space, and such a space at the start of
/// the paragraph is dropped; what came from `<text:s/>`, `<text:tab/>` and `<text:line-break/>`
/// is literal and never collapsed. A collapsed space at the *end* is kept: LibreOffice writes a
/// single trailing space literally and reads it back (`doc/odf-format.md` §2.1, `MEASURED`).
#[derive(Default)]
pub struct Collapser {
    out: String,
    /// Whitespace has been seen since the last character: one space is owed, if anything
    /// precedes it.
    pending_space: bool,
    /// Anything has been produced — `out` may have been taken since, so it cannot say.
    started: bool,
    /// The last thing produced was a collapsed space, so whitespace right after it — even in
    /// the next run — belongs to the same sequence.
    after_space: bool,
}

impl Collapser {
    /// Character data straight from the XML.
    pub fn raw(&mut self, text: &str) {
        for c in text.chars() {
            if matches!(c, ' ' | '\t' | '\r' | '\n') {
                self.pending_space |= !self.after_space;
            } else {
                self.flush_space();
                self.out.push(c);
                self.started = true;
                self.after_space = false;
            }
        }
    }

    /// Text from an element that stands for it.
    pub fn literal(&mut self, text: &str) {
        self.flush_space();
        self.out.push_str(text);
        self.started |= !text.is_empty();
        self.after_space = false;
    }

    fn flush_space(&mut self) {
        if std::mem::take(&mut self.pending_space) && self.started {
            self.out.push(' ');
            self.after_space = true;
        }
    }

    /// Everything since the last `take`: one run's text. A space owed at a run boundary belongs
    /// to the run it was written in, which is this one.
    pub fn take(&mut self) -> String {
        self.flush_space();
        std::mem::take(&mut self.out)
    }

    /// The paragraph has ended: a space still owed is a trailing space.
    pub fn end(&mut self) {
        self.flush_space();
    }

    pub fn finish(mut self) -> String {
        self.end();
        self.out
    }
}

/// The text of the paragraph-like element just started — `text:p`, `text:h`, `text:span`,
/// `text:a` — flattened.
pub fn read_text(r: &mut crate::xml::read::Reader<'_>, into: &mut Collapser) -> crate::Result<()> {
    use crate::names::Ns;
    use crate::xml::read::Handled;
    let into = std::cell::RefCell::new(into);
    r.mixed(
        |t| into.borrow_mut().raw(t),
        |r, name, attrs| {
            if name.ns != Ns::Text {
                return Ok(Handled::No);
            }
            match name.local.as_str() {
                "s" => {
                    let n = attrs
                        .get(Ns::Text, "c")
                        .and_then(|c| c.parse::<usize>().ok())
                        .unwrap_or(1)
                        .min(10_000);
                    into.borrow_mut().literal(&" ".repeat(n));
                }
                "tab" => into.borrow_mut().literal("\t"),
                "line-break" => into.borrow_mut().literal("\n"),
                "span" | "a" | "meta" => read_text(r, &mut into.borrow_mut())?,
                _ => return Ok(Handled::No),
            }
            Ok(Handled::Yes)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::write::Writer;

    fn text(s: &str) -> String {
        let mut w = Writer::fragment();
        w.start("p");
        write_text(&mut w, s);
        w.end();
        w.finish()
    }

    /// What the writer escapes, the reader's collapsing gives back exactly.
    #[test]
    fn written_text_reads_back_as_itself() {
        for s in [
            "a b",
            "  lead",
            "trail  ",
            "a\tb\nc",
            " ",
            "x \t y",
            "many     spaces",
        ] {
            let xml = format!(
                "<text:p xmlns:text=\"{}\">{}</text:p>",
                crate::names::ODF_TEXT,
                text(s).trim_start_matches("<p>").trim_end_matches("</p>")
            );
            let mut r = crate::xml::read::Reader::new(xml.as_bytes());
            r.root().unwrap();
            let mut c = Collapser::default();
            read_text(&mut r, &mut c).unwrap();
            assert_eq!(c.finish(), s, "{xml}");
        }
    }

    /// A space before a span boundary belongs between the two runs, even though the first run
    /// has already been taken — found by LibreOffice's loop, not by our own round trip.
    #[test]
    fn a_space_survives_being_split_across_runs() {
        let mut c = Collapser::default();
        c.raw("Plain, ");
        assert_eq!(c.take(), "Plain, ", "the space is the first run's");
        c.raw("  bold");
        assert_eq!(c.take(), "bold", "and collapses with what follows it");
    }

    #[test]
    fn character_data_collapses_and_its_edges_vanish() {
        let xml = format!(
            "<text:p xmlns:text=\"{}\">\n   a \n  <text:span>b </text:span> c\n</text:p>",
            crate::names::ODF_TEXT
        );
        let mut r = crate::xml::read::Reader::new(xml.as_bytes());
        r.root().unwrap();
        let mut c = Collapser::default();
        read_text(&mut r, &mut c).unwrap();
        assert_eq!(c.finish(), "a b c ");
    }

    #[test]
    fn only_the_spaces_collapsing_would_eat_are_escaped() {
        assert_eq!(text("a b"), "<p>a b</p>");
        assert_eq!(text("a  b"), "<p>a <text:s/>b</p>");
        assert_eq!(text("a   b"), "<p>a <text:s text:c=\"2\"/>b</p>");
        assert_eq!(text(" a"), "<p><text:s/>a</p>");
        assert_eq!(text("a "), "<p>a<text:s/></p>");
        assert_eq!(text("a  "), "<p>a<text:s text:c=\"2\"/></p>");
        assert_eq!(text("a\tb\nc"), "<p>a<text:tab/>b<text:line-break/>c</p>");
        assert_eq!(text(" "), "<p><text:s/></p>");
    }
}
