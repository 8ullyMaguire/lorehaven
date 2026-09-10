//! The restricted editor document (spec §8.3, ADR 0002).
//!
//! The editor's document is the source of truth; sanitized HTML and plain text
//! are *derived* representations produced here, once, on write. Nothing else in
//! the workspace is allowed to derive them, because a second renderer is a
//! second set of escaping rules and therefore a second chance to lose.
//!
//! The schema is deliberately tiny and closed:
//!
//! * blocks: paragraph, heading, bullet list, ordered list, blockquote, scene break;
//! * inlines: text (with bold / italic / link marks) and a hard line break.
//!
//! Anything else — an unknown node type, an unknown mark, an attribute we did
//! not ask for — is **rejected**, not stripped. Stripping would mean the stored
//! document is not the document the author sent, and it would leave the
//! dangerous case (an embedded `<script>`, an `onclick`) indistinguishable from
//! the harmless one. A rejection tells the caller plainly that it asked for
//! something this schema does not have.
//!
//! Security invariant, asserted by the tests in this module: no text, attribute
//! or URL from an untrusted document ever reaches the HTML output without being
//! escaped, and link targets are restricted to schemes that cannot execute.

use std::fmt;

use serde_json::{json, Map, Value};

/// Deepest nesting the parser accepts.
///
/// The limit exists so that a document cannot be used to exhaust the stack of
/// the process that parses it (spec §3.8, bounded resource use).
pub const MAX_DEPTH: usize = 20;

/// Most nodes the parser accepts in one document.
pub const MAX_NODES: usize = 20_000;

/// Most bytes of text one document may carry.
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;

/// Longest heading level we accept: four levels is already more structure than
/// fiction needs, and an unbounded `level` is an attribute we would have to
/// validate for no benefit.
///
/// Public because [`Block::Heading`]'s documentation states the bound and a
/// client assembling a document needs to know it before it is refused — the
/// same reason [`MAX_TEXT_BYTES`] is public. A private constant behind a public
/// promise leaves the number with two homes and only one of them readable.
pub const MAX_HEADING_LEVEL: u8 = 4;

/// Why a document was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentError {
    /// The value is not the shape the schema describes.
    Shape {
        /// Where in the document the problem is.
        path: String,
        /// What is wrong.
        detail: String,
    },
    /// The document is too large or too deeply nested to be safe.
    Bounds {
        /// Which bound was exceeded.
        detail: String,
    },
}

impl DocumentError {
    fn shape(path: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::Shape {
            path: path.into(),
            detail: detail.into(),
        }
    }
}

impl fmt::Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shape { path, detail } => {
                write!(f, "at {path}: {detail}")
            }
            Self::Bounds { detail } => f.write_str(detail),
        }
    }
}

impl std::error::Error for DocumentError {}

/// A mark applied to a run of text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    /// Strong emphasis.
    Bold,
    /// Emphasis.
    Italic,
    /// A link. The target has already been validated.
    Link {
        /// Absolute or site-relative target.
        href: String,
    },
}

/// A run of text within a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    /// Text with its marks.
    Text {
        /// The text itself.
        text: String,
        /// Marks, in the schema's canonical order.
        marks: Vec<Mark>,
    },
    /// A hard line break inside a block.
    LineBreak,
}

/// A block-level node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// A paragraph of inline content.
    Paragraph(Vec<Inline>),
    /// A heading.
    Heading {
        /// 1 to [`MAX_HEADING_LEVEL`].
        level: u8,
        /// Heading text.
        content: Vec<Inline>,
    },
    /// A list, ordered or bulleted. Each item is a sequence of blocks, which is
    /// what the editor produces (and what allows a nested list or paragraph).
    List {
        /// Whether the list is numbered.
        ordered: bool,
        /// First number of an ordered list; `1` for bullets.
        start: u32,
        /// The items.
        items: Vec<Vec<Block>>,
    },
    /// A quotation block.
    Blockquote(Vec<Block>),
    /// A scene break — the typographic separator between scenes.
    SceneBreak,
}

/// A validated document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Top-level blocks.
    pub blocks: Vec<Block>,
}

impl Document {
    /// Parse and validate an editor document from its JSON form.
    pub fn from_json(value: &Value) -> Result<Self, DocumentError> {
        let mut budget = Budget {
            nodes: 0,
            text_bytes: 0,
        };

        let object = value
            .as_object()
            .ok_or_else(|| DocumentError::shape("$", "a document must be an object"))?;

        match object.get("type").and_then(Value::as_str) {
            Some("doc") => {}
            Some(other) => {
                return Err(DocumentError::shape(
                    "$.type",
                    format!("expected `doc`, found `{other}`"),
                ))
            }
            None => {
                return Err(DocumentError::shape(
                    "$.type",
                    "a document must have a type",
                ))
            }
        }

        // `attrs` is not part of the discriminated schema; an unexpected one is
        // refused rather than ignored so that a client cannot attach data we do
        // not model and later believe it was stored.
        reject_unknown_keys(object, &["type", "content"], "$")?;

        let content = match object.get("content") {
            Some(value) => value
                .as_array()
                .ok_or_else(|| DocumentError::shape("$.content", "expected an array"))?,
            None => return Ok(Self { blocks: Vec::new() }),
        };

        let blocks = parse_blocks(content, 1, &mut budget)?;
        Ok(Self { blocks })
    }

    /// A document containing a single paragraph of `text`.
    ///
    /// Used where a derived representation is wanted from plain input (an
    /// imported file, a title that will be shown as prose).
    #[must_use]
    pub fn from_plain(text: &str) -> Self {
        let mut blocks = Vec::new();
        for paragraph in text.split("\n\n") {
            let trimmed = paragraph.trim();
            if trimmed.is_empty() {
                continue;
            }
            blocks.push(Block::Paragraph(vec![Inline::Text {
                text: trimmed.to_owned(),
                marks: Vec::new(),
            }]));
        }
        Self { blocks }
    }

    /// The canonical JSON form of the validated document.
    ///
    /// This — never the client's own JSON — is what gets stored, so that a
    /// stored document is provably one this module accepted.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "type": "doc",
            "content": blocks_to_json(&self.blocks),
        })
    }

    /// The sanitized HTML representation.
    #[must_use]
    pub fn to_sanitized_html(&self) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            write_block(&mut out, block);
        }
        out
    }

    /// The plain-text representation.
    #[must_use]
    pub fn to_plain_text(&self) -> String {
        let mut out = String::new();
        let mut first = true;
        for block in &self.blocks {
            if !first {
                out.push_str("\n\n");
            }
            first = false;
            write_block_text(&mut out, block, "");
        }
        out
    }

    /// Word count of the plain-text representation (spec §3.2: nonnegative).
    #[must_use]
    pub fn word_count(&self) -> u32 {
        let text = self.to_plain_text();
        let words = text.split_whitespace().count();
        u32::try_from(words).unwrap_or(u32::MAX)
    }

    /// Whether the document has no visible content.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.to_plain_text().trim().is_empty()
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

struct Budget {
    nodes: usize,
    text_bytes: usize,
}

fn parse_blocks(
    values: &[Value],
    depth: usize,
    budget: &mut Budget,
) -> Result<Vec<Block>, DocumentError> {
    if depth > MAX_DEPTH {
        return Err(DocumentError::Bounds {
            detail: format!("the document nests deeper than {MAX_DEPTH} levels"),
        });
    }

    let mut blocks = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let path = format!("$.content[{index}]");
        blocks.push(parse_block(value, &path, depth, budget)?);
    }
    Ok(blocks)
}

fn parse_block(
    value: &Value,
    path: &str,
    depth: usize,
    budget: &mut Budget,
) -> Result<Block, DocumentError> {
    count_node(budget)?;

    let object = value
        .as_object()
        .ok_or_else(|| DocumentError::shape(path, "a block must be an object"))?;

    let node_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| DocumentError::shape(path, "a block must have a type"))?;

    match node_type {
        "paragraph" => {
            reject_unknown_keys(object, &["type", "content"], path)?;
            Ok(Block::Paragraph(parse_inlines(
                object, path, depth, budget,
            )?))
        }
        "heading" => {
            reject_unknown_keys(object, &["type", "attrs", "content"], path)?;
            let attrs = attrs_of(object, path)?;
            let level = attrs
                .and_then(|attrs| attrs.get("level"))
                .and_then(Value::as_u64)
                .ok_or_else(|| DocumentError::shape(path, "a heading needs a numeric level"))?;
            let level = u8::try_from(level)
                .ok()
                .filter(|level| (1..=MAX_HEADING_LEVEL).contains(level))
                .ok_or_else(|| {
                    DocumentError::shape(
                        path,
                        format!("a heading level must be between 1 and {MAX_HEADING_LEVEL}"),
                    )
                })?;
            Ok(Block::Heading {
                level,
                content: parse_inlines(object, path, depth, budget)?,
            })
        }
        "bulletList" | "orderedList" => {
            reject_unknown_keys(object, &["type", "attrs", "content"], path)?;
            let ordered = node_type == "orderedList";
            let start =
                if ordered {
                    let attrs = attrs_of(object, path)?;
                    match attrs.and_then(|attrs| attrs.get("start")) {
                        Some(value) => u32::try_from(value.as_u64().ok_or_else(|| {
                            DocumentError::shape(path, "`start` must be a number")
                        })?)
                        .ok()
                        .filter(|start| (1..=100_000).contains(start))
                        .ok_or_else(|| {
                            DocumentError::shape(path, "`start` must be between 1 and 100000")
                        })?,
                        None => 1,
                    }
                } else {
                    1
                };

            let items = match object.get("content") {
                Some(value) => value
                    .as_array()
                    .ok_or_else(|| DocumentError::shape(path, "expected an array of list items"))?,
                None => &Vec::new(),
            };

            let mut parsed = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let item_path = format!("{path}.content[{index}]");
                count_node(budget)?;
                let object = item.as_object().ok_or_else(|| {
                    DocumentError::shape(&item_path, "a list item must be an object")
                })?;
                if object.get("type").and_then(Value::as_str) != Some("listItem") {
                    return Err(DocumentError::shape(
                        &item_path,
                        "a list may contain only list items",
                    ));
                }
                reject_unknown_keys(object, &["type", "content"], &item_path)?;
                let inner = match object.get("content") {
                    Some(value) => value.as_array().ok_or_else(|| {
                        DocumentError::shape(&item_path, "expected an array of blocks")
                    })?,
                    None => &Vec::new(),
                };
                parsed.push(parse_blocks(inner, depth + 1, budget)?);
            }

            Ok(Block::List {
                ordered,
                start,
                items: parsed,
            })
        }
        "blockquote" => {
            reject_unknown_keys(object, &["type", "content"], path)?;
            let inner = match object.get("content") {
                Some(value) => value
                    .as_array()
                    .ok_or_else(|| DocumentError::shape(path, "expected an array of blocks"))?,
                None => &Vec::new(),
            };
            Ok(Block::Blockquote(parse_blocks(inner, depth + 1, budget)?))
        }
        "horizontalRule" => {
            reject_unknown_keys(object, &["type"], path)?;
            Ok(Block::SceneBreak)
        }
        other => Err(DocumentError::shape(
            path,
            format!("`{other}` is not part of the editor schema"),
        )),
    }
}

fn parse_inlines(
    object: &Map<String, Value>,
    path: &str,
    depth: usize,
    budget: &mut Budget,
) -> Result<Vec<Inline>, DocumentError> {
    let Some(value) = object.get("content") else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| DocumentError::shape(path, "expected an array of inline nodes"))?;

    let mut inlines = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let child_path = format!("{path}.content[{index}]");
        count_node(budget)?;

        if depth > MAX_DEPTH {
            return Err(DocumentError::Bounds {
                detail: format!("the document nests deeper than {MAX_DEPTH} levels"),
            });
        }

        let child = value
            .as_object()
            .ok_or_else(|| DocumentError::shape(&child_path, "an inline node must be an object"))?;

        match child.get("type").and_then(Value::as_str) {
            Some("text") => {
                reject_unknown_keys(child, &["type", "text", "marks"], &child_path)?;
                let text = child
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| DocumentError::shape(&child_path, "text must be a string"))?;

                if text.chars().any(|c| c.is_control() && c != '\t') {
                    return Err(DocumentError::shape(
                        &child_path,
                        "text may not contain control characters",
                    ));
                }

                budget.text_bytes += text.len();
                if budget.text_bytes > MAX_TEXT_BYTES {
                    return Err(DocumentError::Bounds {
                        detail: format!(
                            "a document may hold at most {MAX_TEXT_BYTES} bytes of text"
                        ),
                    });
                }

                inlines.push(Inline::Text {
                    text: text.to_owned(),
                    marks: parse_marks(child, &child_path)?,
                });
            }
            Some("hardBreak") => {
                reject_unknown_keys(child, &["type"], &child_path)?;
                inlines.push(Inline::LineBreak);
            }
            Some(other) => {
                return Err(DocumentError::shape(
                    &child_path,
                    format!("`{other}` is not part of the editor schema"),
                ))
            }
            None => {
                return Err(DocumentError::shape(
                    &child_path,
                    "an inline node needs a type",
                ))
            }
        }
    }

    Ok(inlines)
}

fn parse_marks(object: &Map<String, Value>, path: &str) -> Result<Vec<Mark>, DocumentError> {
    let Some(value) = object.get("marks") else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| DocumentError::shape(path, "`marks` must be an array"))?;

    let mut bold = false;
    let mut italic = false;
    let mut href: Option<String> = None;

    for (index, value) in values.iter().enumerate() {
        let mark_path = format!("{path}.marks[{index}]");
        let mark = value
            .as_object()
            .ok_or_else(|| DocumentError::shape(&mark_path, "a mark must be an object"))?;

        match mark.get("type").and_then(Value::as_str) {
            Some("bold") => {
                reject_unknown_keys(mark, &["type"], &mark_path)?;
                bold = true;
            }
            Some("italic") => {
                reject_unknown_keys(mark, &["type"], &mark_path)?;
                italic = true;
            }
            Some("link") => {
                reject_unknown_keys(mark, &["type", "attrs"], &mark_path)?;
                let attrs = attrs_of(mark, &mark_path)?;
                if let Some(attrs) = attrs {
                    reject_unknown_keys(attrs, &["href"], &mark_path)?;
                }
                let raw = attrs
                    .and_then(|attrs| attrs.get("href"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| DocumentError::shape(&mark_path, "a link needs an href"))?;
                href = Some(normalise_href(raw).ok_or_else(|| {
                    DocumentError::shape(
                        &mark_path,
                        "a link must be http, https, mailto or a site-relative path",
                    )
                })?);
            }
            Some(other) => {
                return Err(DocumentError::shape(
                    &mark_path,
                    format!("`{other}` is not part of the editor schema"),
                ))
            }
            None => return Err(DocumentError::shape(&mark_path, "a mark needs a type")),
        }
    }

    let mut marks = Vec::new();
    if bold {
        marks.push(Mark::Bold);
    }
    if italic {
        marks.push(Mark::Italic);
    }
    if let Some(href) = href {
        marks.push(Mark::Link { href });
    }
    Ok(marks)
}

fn count_node(budget: &mut Budget) -> Result<(), DocumentError> {
    budget.nodes += 1;
    if budget.nodes > MAX_NODES {
        return Err(DocumentError::Bounds {
            detail: format!("a document may hold at most {MAX_NODES} nodes"),
        });
    }
    Ok(())
}

fn attrs_of<'a>(
    object: &'a Map<String, Value>,
    path: &str,
) -> Result<Option<&'a Map<String, Value>>, DocumentError> {
    match object.get("attrs") {
        Some(value) => value
            .as_object()
            .map(Some)
            .ok_or_else(|| DocumentError::shape(path, "`attrs` must be an object")),
        None => Ok(None),
    }
}

/// Refuse any key the schema does not define.
///
/// Being strict here is what makes "the stored document is the document you
/// sent" true: an attribute we silently dropped would come back missing from
/// the stored revision with no explanation.
fn reject_unknown_keys(
    object: &Map<String, Value>,
    allowed: &[&str],
    path: &str,
) -> Result<(), DocumentError> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(DocumentError::shape(
                path,
                format!("`{key}` is not part of the editor schema"),
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn blocks_to_json(blocks: &[Block]) -> Value {
    Value::Array(blocks.iter().map(block_to_json).collect())
}

fn block_to_json(block: &Block) -> Value {
    match block {
        Block::Paragraph(content) => json!({
            "type": "paragraph",
            "content": inlines_to_json(content),
        }),
        Block::Heading { level, content } => json!({
            "type": "heading",
            "attrs": { "level": level },
            "content": inlines_to_json(content),
        }),
        Block::List {
            ordered,
            start,
            items,
        } => {
            let node_type = if *ordered {
                "orderedList"
            } else {
                "bulletList"
            };
            let items: Vec<Value> = items
                .iter()
                .map(|item| {
                    json!({
                        "type": "listItem",
                        "content": blocks_to_json(item),
                    })
                })
                .collect();
            if *ordered {
                json!({
                    "type": node_type,
                    "attrs": { "start": start },
                    "content": items,
                })
            } else {
                json!({ "type": node_type, "content": items })
            }
        }
        Block::Blockquote(content) => json!({
            "type": "blockquote",
            "content": blocks_to_json(content),
        }),
        Block::SceneBreak => json!({ "type": "horizontalRule" }),
    }
}

fn inlines_to_json(inlines: &[Inline]) -> Value {
    Value::Array(
        inlines
            .iter()
            .map(|inline| match inline {
                Inline::LineBreak => json!({ "type": "hardBreak" }),
                Inline::Text { text, marks } => {
                    if marks.is_empty() {
                        json!({ "type": "text", "text": text })
                    } else {
                        let marks: Vec<Value> = marks
                            .iter()
                            .map(|mark| match mark {
                                Mark::Bold => json!({ "type": "bold" }),
                                Mark::Italic => json!({ "type": "italic" }),
                                Mark::Link { href } => {
                                    json!({ "type": "link", "attrs": { "href": href } })
                                }
                            })
                            .collect();
                        json!({ "type": "text", "text": text, "marks": marks })
                    }
                }
            })
            .collect(),
    )
}

fn write_block(out: &mut String, block: &Block) {
    match block {
        Block::Paragraph(content) => {
            out.push_str("<p>");
            write_inlines(out, content);
            out.push_str("</p>");
        }
        Block::Heading { level, content } => {
            // The schema allows levels 1–4; the reader maps them to its own
            // scale, so the stored HTML keeps the author's intent.
            out.push_str("<h");
            out.push(char::from(b'0' + *level));
            out.push('>');
            write_inlines(out, content);
            out.push_str("</h");
            out.push(char::from(b'0' + *level));
            out.push('>');
        }
        Block::List {
            ordered,
            start,
            items,
        } => {
            if *ordered && *start != 1 {
                out.push_str(&format!("<ol start=\"{start}\">"));
            } else if *ordered {
                out.push_str("<ol>");
            } else {
                out.push_str("<ul>");
            }
            for item in items {
                out.push_str("<li>");
                for child in item {
                    write_block(out, child);
                }
                out.push_str("</li>");
            }
            out.push_str(if *ordered { "</ol>" } else { "</ul>" });
        }
        Block::Blockquote(content) => {
            out.push_str("<blockquote>");
            for child in content {
                write_block(out, child);
            }
            out.push_str("</blockquote>");
        }
        Block::SceneBreak => out.push_str("<hr />"),
    }
}

fn write_inlines(out: &mut String, inlines: &[Inline]) {
    for inline in inlines {
        match inline {
            Inline::LineBreak => out.push_str("<br />"),
            Inline::Text { text, marks } => {
                let mut open = String::new();
                let mut close = String::new();
                for mark in marks {
                    match mark {
                        Mark::Bold => {
                            open.push_str("<strong>");
                            close.insert_str(0, "</strong>");
                        }
                        Mark::Italic => {
                            open.push_str("<em>");
                            close.insert_str(0, "</em>");
                        }
                        Mark::Link { href } => {
                            open.push_str(&format!(
                                "<a href=\"{}\" rel=\"nofollow noopener ugc\">",
                                escape_attribute(href)
                            ));
                            close.insert_str(0, "</a>");
                        }
                    }
                }
                out.push_str(&open);
                out.push_str(&escape_text(text));
                out.push_str(&close);
            }
        }
    }
}

fn write_block_text(out: &mut String, block: &Block, prefix: &str) {
    match block {
        Block::Paragraph(content) => {
            out.push_str(prefix);
            write_inlines_text(out, content);
        }
        Block::Heading { content, .. } => {
            out.push_str(prefix);
            write_inlines_text(out, content);
        }
        Block::List { items, .. } => {
            let mut first = true;
            for item in items {
                if !first {
                    out.push('\n');
                }
                first = false;
                let mut item_first = true;
                for child in item {
                    if !item_first {
                        out.push('\n');
                    }
                    item_first = false;
                    write_block_text(out, child, "- ");
                }
            }
        }
        Block::Blockquote(content) => {
            let mut first = true;
            for child in content {
                if !first {
                    out.push_str("\n\n");
                }
                first = false;
                write_block_text(out, child, "");
            }
        }
        Block::SceneBreak => out.push_str("* * *"),
    }
}

fn write_inlines_text(out: &mut String, inlines: &[Inline]) {
    for inline in inlines {
        match inline {
            Inline::Text { text, .. } => out.push_str(text),
            Inline::LineBreak => out.push('\n'),
        }
    }
}

/// Escape text for an HTML text node.
///
/// The four characters that can change the meaning of markup are escaped; the
/// rest of the input is preserved.
#[must_use]
pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
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

fn escape_attribute(value: &str) -> String {
    escape_text(value)
}

/// Accept only link targets that cannot execute.
///
/// `javascript:` and `data:` URLs are the classic ways an editor becomes an
/// injection point, so the scheme is matched against an allowlist instead of
/// against a denylist of tricks.
#[must_use]
pub fn normalise_href(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().any(|c| c.is_control()) {
        return None;
    }
    // Characters that could change how the attribute is read if escaping were
    // ever weakened, and that no real destination needs.
    if trimmed
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`'))
    {
        return None;
    }
    if trimmed.len() > 2048 {
        return None;
    }

    // A site-relative path is always acceptable: it cannot leave the origin.
    if trimmed.starts_with('/') && !trimmed.starts_with("//") {
        return Some(trimmed.to_owned());
    }

    let lower = trimmed.to_ascii_lowercase();
    for scheme in ["http://", "https://", "mailto:"] {
        if lower.starts_with(scheme) {
            // The scheme is on the allowlist and the target passed the
            // character checks above, so it cannot execute.
            return Some(trimmed.to_owned());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(content: Vec<Value>) -> Value {
        json!({ "type": "doc", "content": content })
    }

    #[test]
    fn a_simple_document_round_trips_through_json() {
        let source = doc(vec![
            json!({
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "Hello " },
                    { "type": "text", "text": "world", "marks": [{ "type": "bold" }] },
                ],
            }),
            json!({ "type": "horizontalRule" }),
        ]);

        let parsed = Document::from_json(&source).expect("valid document");
        assert_eq!(parsed.to_json(), source, "the canonical form is stable");
        assert_eq!(document_text(&parsed), "Hello world");
    }

    fn document_text(document: &Document) -> String {
        let html = document.to_sanitized_html();
        html.replace("<p>", "")
            .replace("</p>", "")
            .replace("<strong>", "")
            .replace("</strong>", "")
            .replace("<hr />", "")
            .trim()
            .to_owned()
    }

    #[test]
    fn a_document_without_content_is_empty_rather_than_an_error() {
        let parsed = Document::from_json(&json!({ "type": "doc" })).expect("valid");
        assert!(parsed.is_empty());
        assert_eq!(parsed.word_count(), 0);
    }

    #[test]
    fn an_unknown_node_type_is_refused() {
        let source = doc(vec![json!({ "type": "script" })]);
        let error = Document::from_json(&source).expect_err("must be refused");
        assert!(error.to_string().contains("not part of the editor schema"));
    }

    #[test]
    fn an_unknown_mark_is_refused() {
        let source = doc(vec![json!({
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "hi",
                "marks": [{ "type": "onclick" }],
            }],
        })]);
        assert!(Document::from_json(&source).is_err());
    }

    #[test]
    fn an_unexpected_attribute_is_refused() {
        let source = doc(vec![json!({
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "hi",
                "marks": [{ "type": "link", "attrs": { "href": "https://a.example", "target": "_blank" } }],
            }],
        })]);
        assert!(Document::from_json(&source).is_err());
    }

    #[test]
    fn a_javascript_link_is_refused() {
        assert!(normalise_href("javascript:alert(1)").is_none());
        assert!(normalise_href("JaVaScRiPt:alert(1)").is_none());
        assert!(normalise_href("data:text/html,<script>alert(1)</script>").is_none());
        assert!(normalise_href("vbscript:msgbox(1)").is_none());
        assert!(normalise_href("  ").is_none());
        assert!(normalise_href("https://a.example").is_some());
        assert!(normalise_href("mailto:a@b.example").is_some());
        assert!(normalise_href("/works/abc").is_some());
        // A protocol-relative URL is *not* site-relative.
        assert!(normalise_href("//evil.example/x").is_none());
    }

    #[test]
    fn markup_in_text_is_escaped_in_the_html_output() {
        let source = doc(vec![json!({
            "type": "paragraph",
            "content": [{ "type": "text", "text": "<script>alert('x')</script>" }],
        })]);
        let parsed = Document::from_json(&source).expect("valid");
        let html = parsed.to_sanitized_html();
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
        // The plain text keeps the characters, because it is not markup.
        assert!(parsed.to_plain_text().contains("<script>"));
    }

    #[test]
    fn an_attribute_value_cannot_break_out_of_its_quotes() {
        let href = "https://a.example/\"><script>alert(1)</script>";
        // The whitespace and the angle brackets make this not a URL at all, so
        // it never reaches the attribute.
        assert!(normalise_href(href).is_none());
        assert!(normalise_href("https://a.example/?q=\"x\"").is_none());
    }

    #[test]
    fn the_parser_refuses_a_document_that_nests_too_deeply() {
        // Build blockquote(blockquote(...)) past the limit.
        let mut inner = doc(vec![json!({ "type": "paragraph" })]);
        for _ in 0..(MAX_DEPTH + 2) {
            inner = doc(vec![
                json!({ "type": "blockquote", "content": inner["content"].clone() }),
            ]);
        }
        let error = Document::from_json(&inner).expect_err("must be bounded");
        assert!(matches!(error, DocumentError::Bounds { .. }), "{error:?}");
    }

    #[test]
    fn scene_breaks_lists_and_quotes_render_as_expected() {
        let source = doc(vec![
            json!({
                "type": "bulletList",
                "content": [
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                ],
            }),
            json!({ "type": "orderedList", "attrs": { "start": 3 }, "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] },
            ] }),
            json!({ "type": "blockquote", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "said" }] }] }),
            json!({ "type": "horizontalRule" }),
        ]);

        let parsed = Document::from_json(&source).expect("valid");
        let html = parsed.to_sanitized_html();
        assert!(
            html.contains("<ul><li><p>one</p></li><li><p>two</p></li></ul>"),
            "{html}"
        );
        assert!(
            html.contains("<ol start=\"3\"><li><p>three</p></li></ol>"),
            "{html}"
        );
        assert!(
            html.contains("<blockquote><p>said</p></blockquote>"),
            "{html}"
        );
        assert!(html.ends_with("<hr />"), "{html}");
    }

    #[test]
    fn word_count_ignores_markup_and_punctuation_between_words() {
        let source = doc(vec![json!({
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "one two" },
                { "type": "hardBreak" },
                { "type": "text", "text": "three" },
            ],
        })]);
        let parsed = Document::from_json(&source).expect("valid");
        assert_eq!(parsed.word_count(), 3);
    }

    #[test]
    fn a_raw_newline_inside_text_is_refused_because_breaks_are_nodes() {
        // The editor represents a line break as a node, not as a newline
        // character, so a newline in `text` is off-schema and is refused rather
        // than stored in a form the editor cannot reopen.
        let source = doc(vec![json!({
            "type": "paragraph",
            "content": [{ "type": "text", "text": "one\ntwo" }],
        })]);
        assert!(Document::from_json(&source).is_err());
    }

    #[test]
    fn plain_text_is_derived_from_the_document_not_from_the_html() {
        let source = doc(vec![json!({
            "type": "paragraph",
            "content": [{ "type": "text", "text": "a & b < c" }],
        })]);
        let parsed = Document::from_json(&source).expect("valid");
        assert_eq!(parsed.to_plain_text(), "a & b < c");
        assert!(parsed.to_sanitized_html().contains("a &amp; b &lt; c"));
    }

    #[test]
    fn an_oversized_document_is_refused() {
        let mut content = Vec::new();
        for _ in 0..(MAX_NODES + 10) {
            content.push(json!({ "type": "paragraph" }));
        }
        let error = Document::from_json(&doc(content)).expect_err("must be bounded");
        assert!(error.to_string().contains("nodes"), "{error}");
    }

    #[test]
    fn a_heading_level_outside_the_schema_is_refused() {
        let source = doc(vec![json!({ "type": "heading", "attrs": { "level": 9 } })]);
        assert!(Document::from_json(&source).is_err());
        let ok = doc(vec![json!({ "type": "heading", "attrs": { "level": 3 } })]);
        assert!(Document::from_json(&ok).is_ok());
    }
}
