//! HTML under construction, with the escaping decided by the API.
//!
//! The reader surface is **server-rendered HTML** (spec §8.5, §21.1): a reader
//! must be able to open a work, read a unit, move to the next one and come
//! back with JavaScript switched off, and "accessibility is a floor". That
//! makes this module load-bearing — a reader page built by string concatenation
//! is a page whose escaping is a decision nobody made.
//!
//! # The rule
//!
//! **Everything is escaped unless it is inserted with [`Html::raw`].** There
//! is no other way to emit markup, so the trust decision is visible at the
//! call site rather than distributed across the file.
//!
//! Exactly one thing in this instance is inserted raw: the unit's
//! `sanitized_html`, which the *server* produced from the validated editor
//! document (spec §8.3, ADR 0002). That is the one value in the reader's page
//! that is markup rather than text, and it is markup the server has already
//! made safe. A client-side renderer would be a second set of escaping rules
//! to get wrong, and the reader would then have two sanitisers — which is the
//! same class of mistake as two numbering schemes in one statement.
//!
//! # Why not a template engine
//!
//! A template engine is a dependency, a compile step and a second syntax to
//! learn; the reader's page is a document with a handful of elements, and the
//! escaping is the whole of the risk. The builder below is the template.
//!
//! ```
//! use darkhold_app::html::Html;
//!
//! let mut page = Html::new();
//! page.open("p", &[]).text("5 < 6 & true").close("p");
//! assert_eq!(page.finish(), "<p>5 &lt; 6 &amp; true</p>");
//! ```

use std::fmt::Write as _;

/// Escape a string for use in HTML text or in an attribute value.
///
/// The five characters that matter: `&` first (so the ampersands this
/// function introduces are not escaped again), then `<` and `>`, then the
/// quote and the apostrophe. The apostrophe is escaped as `&#39;` rather than
/// `&apos;`, because `&apos;` is not in HTML 4 and this instance serves
/// readers on browsers that predate HTML 5.
pub fn escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// An HTML document under construction.
///
/// See the module's documentation for the escaping rule.
#[derive(Debug, Default, Clone)]
pub struct Html {
    buffer: String,
}

impl Html {
    /// An empty document.
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    /// Append text, escaped.
    pub fn text(&mut self, value: &str) -> &mut Self {
        self.buffer.push_str(&escape(value));
        self
    }

    /// Append **markup**, unescaped.
    ///
    /// The one way to emit markup, and therefore the one place a trust
    /// decision is made. The value must already be safe: the server's
    /// sanitiser, or a literal in this crate.
    pub fn raw(&mut self, markup: &str) -> &mut Self {
        self.buffer.push_str(markup);
        self
    }

    /// Open an element with its attributes: `<name attr="value">`.
    pub fn open(&mut self, name: &str, attrs: &[(&str, &str)]) -> &mut Self {
        self.write_open(name);
        for (key, value) in attrs {
            self.write_attr(key, value);
        }
        self.buffer.push('>');
        self
    }

    /// Open an element whose attributes may be absent: `<name attr="value">`,
    /// with an attribute whose value is `None` written not at all.
    pub fn open_opt(&mut self, name: &str, attrs: &[(&str, Option<&str>)]) -> &mut Self {
        self.write_open(name);
        for (key, value) in attrs {
            if let Some(value) = value {
                self.write_attr(key, value);
            }
        }
        self.buffer.push('>');
        self
    }

    /// A void element, which has no closing tag: `<meta attr="value">`.
    pub fn void(&mut self, name: &str, attrs: &[(&str, &str)]) -> &mut Self {
        self.write_open(name);
        for (key, value) in attrs {
            self.write_attr(key, value);
        }
        self.buffer.push('>');
        self
    }

    /// Close an element: `</name>`.
    pub fn close(&mut self, name: &str) -> &mut Self {
        self.buffer.push_str("</");
        self.buffer.push_str(name);
        self.buffer.push('>');
        self
    }

    /// The document as a string.
    pub fn finish(&self) -> String {
        self.buffer.clone()
    }

    /// The document's length, for a test that wants to assert something
    /// cheaper than comparing the whole whole string.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Whether the document is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    fn write_open(&mut self, name: &str) {
        self.buffer.push('<');
        self.buffer.push_str(name);
    }

    fn write_attr(&mut self, name: &str, value: &str) {
        self.buffer.push(' ');
        self.buffer.push_str(name);
        self.buffer.push_str("=\"");
        self.buffer.push_str(&escape(value));
        self.buffer.push('"');
    }
}

impl std::fmt::Write for Html {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.buffer.push_str(s);
        Ok(())
    }
}

/// A `<style>` block carrying CSS custom properties.
///
/// The reader's appearance is the reader's own (spec §10.1, §21.1), and the
/// values live in the account's typography preference rather than in the
/// site's stylesheet — so they are written **inline**, on the document root,
/// where they override the stylesheet's own reader scale without touching the
/// site's scale at all.
///
/// The property *names* are literals here and the values are numbers and
/// keywords the store validated, so nothing needs escaping — but the block is
/// built through the same escaper as the rest of the document anyway, because
/// "this one is safe" is the sentence that starts the next bug.
pub fn style_block(properties: &[(&str, &str)]) -> String {
    let mut out = String::from(":root{");
    for (name, value) in properties {
        let _ = write!(out, "{name}:{};", escape(value));
    }
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_escaped() {
        let mut page = Html::new();
        page.text("<script>alert('x')</script> & \"more\"");
        assert_eq!(
            page.finish(),
            "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; &quot;more&quot;"
        );
    }

    #[test]
    fn raw_is_not_escaped() {
        let mut page = Html::new();
        page.raw("<p>body</p>");
        assert_eq!(page.finish(), "<p>body</p>");
    }

    #[test]
    fn attributes_are_escaped_and_the_name_is_not() {
        let mut page = Html::new();
        page.void("a", &[("href", "/x?a=1&b=2")]);
        assert_eq!(page.finish(), "<a href=\"/x?a=1&amp;b=2\">");
    }

    #[test]
    fn an_absent_attribute_writes_nothing() {
        let mut page = Html::new();
        page.open_opt("input", &[("value", None), ("value", Some("v"))]);
        assert_eq!(page.finish(), "<input value=\"v\">");
    }

    #[test]
    fn the_ampersand_is_escaped_first_so_the_others_are_not_double_escaped() {
        // `&lt;` in the input must not become `&amp;lt;`.
        assert_eq!(escape("&lt;"), "&amp;lt;");
    }

    #[test]
    fn the_style_block_carries_escaped_values() {
        let block = style_block(&[("--reader-font-scale", "1.0"), ("--reader-theme", "sepia")]);
        assert_eq!(
            block,
            ":root{--reader-font-scale:1.0;--reader-theme:sepia;}"
        );
    }
}
