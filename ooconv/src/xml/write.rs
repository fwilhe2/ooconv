// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The one XML writer. **Every byte any writer emits goes through here**, so well-formedness is a
//! property of this file rather than of each format's code: escaping, balanced tags, and no
//! character XML 1.0 forbids (§2.2 `Char`) — those are dropped and counted, never written, since a
//! single U+0001 makes the whole part unreadable to every consumer.

/// Is `c` a character XML 1.0 can carry at all (§2.2)? Rust's `char` already excludes surrogates.
pub fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{FFFD}' | '\u{10000}'..)
        && c != '\u{FFFE}'
        && c != '\u{FFFF}'
}

#[derive(Default)]
pub struct Writer {
    out: String,
    /// Open elements, innermost last.
    stack: Vec<String>,
    /// The last `start` has not had its `>` written yet: attributes may still come.
    pending: bool,
    /// Characters dropped because XML cannot carry them.
    pub dropped: usize,
}

impl Writer {
    /// A writer that starts with the XML declaration every part carries.
    pub fn new() -> Self {
        let mut w = Self::default();
        w.out
            .push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
        w
    }

    /// A writer for a fragment: no declaration.
    pub fn fragment() -> Self {
        Self::default()
    }

    fn close_pending(&mut self) {
        if std::mem::take(&mut self.pending) {
            self.out.push('>');
        }
    }

    /// Open `<name`. Attributes follow with [`Writer::attr`].
    pub fn start(&mut self, name: &str) -> &mut Self {
        self.close_pending();
        self.out.push('<');
        self.out.push_str(name);
        self.stack.push(name.to_owned());
        self.pending = true;
        self
    }

    /// An attribute on the element just started. Panics if content was written since: that is
    /// a bug in the caller, and would otherwise be malformed output.
    pub fn attr(&mut self, name: &str, value: impl AsRef<str>) -> &mut Self {
        assert!(self.pending, "attribute {name} after content");
        self.out.push(' ');
        self.out.push_str(name);
        self.out.push_str("=\"");
        for c in value.as_ref().chars() {
            match c {
                '&' => self.out.push_str("&amp;"),
                '<' => self.out.push_str("&lt;"),
                '"' => self.out.push_str("&quot;"),
                // Attribute-value normalisation (XML §3.3.3) would turn these into spaces.
                '\t' => self.out.push_str("&#9;"),
                '\n' => self.out.push_str("&#10;"),
                '\r' => self.out.push_str("&#13;"),
                c if is_xml_char(c) => self.out.push(c),
                _ => self.dropped += 1,
            }
        }
        self.out.push('"');
        self
    }

    /// An attribute only when there is a value.
    pub fn attr_opt(&mut self, name: &str, value: Option<impl AsRef<str>>) -> &mut Self {
        if let Some(value) = value {
            self.attr(name, value);
        }
        self
    }

    /// Character content.
    pub fn text(&mut self, text: &str) -> &mut Self {
        self.close_pending();
        for c in text.chars() {
            match c {
                '&' => self.out.push_str("&amp;"),
                '<' => self.out.push_str("&lt;"),
                '>' => self.out.push_str("&gt;"),
                // A literal CR would be normalised away by every parser (XML §2.11).
                '\r' => self.out.push_str("&#13;"),
                c if is_xml_char(c) => self.out.push(c),
                _ => self.dropped += 1,
            }
        }
        self
    }

    /// Close the innermost open element — self-closed if nothing was written inside it.
    pub fn end(&mut self) -> &mut Self {
        let name = self.stack.pop().expect("end() with no open element");
        if std::mem::take(&mut self.pending) {
            self.out.push_str("/>");
        } else {
            self.out.push_str("</");
            self.out.push_str(&name);
            self.out.push('>');
        }
        self
    }

    /// `<name>text</name>`.
    pub fn leaf(&mut self, name: &str, text: &str) -> &mut Self {
        self.start(name).text(text).end()
    }

    /// Markup another [`Writer`] produced, balanced by construction.
    pub fn splice(&mut self, fragment: Writer) -> &mut Self {
        self.close_pending();
        self.dropped += fragment.dropped;
        self.out.push_str(&fragment.finish());
        self
    }

    pub fn finish(self) -> String {
        assert!(self.stack.is_empty(), "unclosed elements: {:?}", self.stack);
        let mut out = self.out;
        if self.pending {
            unreachable!("pending start with an empty stack");
        }
        if !out.ends_with('\n') && out.starts_with("<?xml") {
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_elements_self_close_and_content_closes_by_name() {
        let mut w = Writer::fragment();
        w.start("a").attr("x", "1");
        w.start("b").end();
        w.leaf("c", "t");
        w.end();
        assert_eq!(w.finish(), r#"<a x="1"><b/><c>t</c></a>"#);
    }

    #[test]
    fn everything_is_escaped_where_it_must_be() {
        let mut w = Writer::fragment();
        w.start("a").attr("v", "<&\"\t\n'>").text("<&>\"'\r").end();
        assert_eq!(
            w.finish(),
            "<a v=\"&lt;&amp;&quot;&#9;&#10;'>\">&lt;&amp;&gt;\"'&#13;</a>"
        );
    }

    #[test]
    fn characters_xml_cannot_carry_are_dropped_and_counted() {
        let mut w = Writer::fragment();
        w.start("a")
            .attr("v", "x\u{1}y")
            .text("\u{0}\u{FFFE}ok\u{1F600}")
            .end();
        assert_eq!(w.dropped, 3);
        assert_eq!(w.finish(), "<a v=\"xy\">ok\u{1F600}</a>");
    }

    #[test]
    #[should_panic(expected = "unclosed")]
    fn an_unbalanced_document_is_a_bug_not_output() {
        let mut w = Writer::fragment();
        w.start("a").start("b").end();
        w.finish();
    }
}
