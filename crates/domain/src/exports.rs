//! Exports: rendering a work into a file a reader can keep (spec §13).
//!
//! # What this module is and is not
//!
//! It is **pure rendering**: a [`Document`], some metadata, a [`ExportFormat`],
//! and bytes. It has no database and no process, which is what makes every format
//! testable without a server — and what keeps the interesting question (may this
//! actor export this work?) in one place instead of two (see the plan's second
//! pitfall: the export loads through the same access check the reader does, and a
//! second check here would be the leak).
//!
//! It is **not** the converter. Spec §13.1's last three formats are produced by
//! external programs, and this module refuses them with
//! [`ExportError::NeedsConverter`] so that the refusal is a typed answer rather
//! than a string. The layer that owns the process — the app — decides whether the
//! program exists and reports the formats it cannot offer *before* a job is
//! queued.
//!
//! # Formats
//!
//! Spec §13.1's order is the order here: plain text, sanitized standalone HTML,
//! Markdown, EPUB, then the converter formats. The distinction that matters is
//! [`ExportFormat::is_builtin`]: four formats this build always has and three it
//! may not, and an interface must not offer what the server will refuse.

pub mod epub;

use serde::{Deserialize, Serialize};

use crate::document::{escape_text, Block, Document, Inline, Mark};

pub use epub::{EpubChapter, EpubError, EpubFacts, EpubInput, EpubProvenance};

/// A format a work can be exported to (spec §13.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// Plain text: the words, with no presentation.
    PlainText,
    /// A standalone HTML file: the sanitized rendering, wrapped in a document.
    Html,
    /// Markdown, with the simplifications stated inside the file itself.
    Markdown,
    /// An EPUB 3 package, built here.
    Epub,
    /// PDF, through an external converter.
    Pdf,
    /// AZW3, through an external converter.
    Azw3,
    /// MOBI, through an external converter.
    Mobi,
}

/// An external program a format depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Converter {
    /// Calibre's `ebook-convert`, which produces all three converter formats.
    EbookConvert,
    /// `pandoc`, which can produce a PDF but not AZW3 or MOBI.
    Pandoc,
}

impl Converter {
    /// The program's name, as it appears on `PATH` and in a doctor report.
    #[must_use]
    pub const fn binary(self) -> &'static str {
        match self {
            Self::EbookConvert => "ebook-convert",
            Self::Pandoc => "pandoc",
        }
    }

    /// What to tell an operator who does not have it.
    #[must_use]
    pub const fn install_hint(self) -> &'static str {
        match self {
            Self::EbookConvert => {
                "install Calibre (`ebook-convert`), which provides PDF, AZW3 and MOBI"
            }
            Self::Pandoc => "install pandoc, which provides PDF",
        }
    }
}

impl ExportFormat {
    /// Every format, in spec §13.1's order.
    pub const ALL: [Self; 7] = [
        Self::PlainText,
        Self::Html,
        Self::Markdown,
        Self::Epub,
        Self::Pdf,
        Self::Azw3,
        Self::Mobi,
    ];

    /// The stored and wire representation.
    ///
    /// Snake case rather than the enum's own name, because this string is in a
    /// database column and in URLs, and a rename of a Rust variant must not
    /// change either.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PlainText => "plain_text",
            Self::Html => "html",
            Self::Markdown => "markdown",
            Self::Epub => "epub",
            Self::Pdf => "pdf",
            Self::Azw3 => "azw3",
            Self::Mobi => "mobi",
        }
    }

    /// Parse the stored representation.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "plain_text" => Self::PlainText,
            "html" => Self::Html,
            "markdown" => Self::Markdown,
            "epub" => Self::Epub,
            "pdf" => Self::Pdf,
            "azw3" => Self::Azw3,
            "mobi" => Self::Mobi,
            _ => return None,
        })
    }

    /// Whether this build can produce the format on its own.
    ///
    /// The four that are `true` here are always available; the three that are
    /// `false` depend on a program the instance may not have, so an interface
    /// asks before offering them (spec §13.1: "disable unavailable formats with
    /// installation guidance").
    #[must_use]
    pub const fn is_builtin(self) -> bool {
        matches!(
            self,
            Self::PlainText | Self::Html | Self::Markdown | Self::Epub
        )
    }

    /// The converters that can produce this format, best first.
    ///
    /// A list rather than one program because the answer genuinely differs: an
    /// instance with `pandoc` and no Calibre can still export a PDF, and telling
    /// its operator that PDF is unavailable because Calibre is missing would be
    /// false.
    #[must_use]
    pub const fn converters(self) -> &'static [Converter] {
        match self {
            Self::PlainText | Self::Html | Self::Markdown | Self::Epub => &[],
            Self::Pdf => &[Converter::EbookConvert, Converter::Pandoc],
            Self::Azw3 | Self::Mobi => &[Converter::EbookConvert],
        }
    }

    /// The name a reader sees in a picker.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PlainText => "Plain text",
            Self::Html => "HTML",
            Self::Markdown => "Markdown",
            Self::Epub => "EPUB",
            Self::Pdf => "PDF",
            Self::Azw3 => "AZW3",
            Self::Mobi => "MOBI",
        }
    }

    /// What an operator would have to install for this format, if anything.
    ///
    /// Empty for the formats this instance builds itself. The point of the
    /// sentence is that a reader who wanted a PDF is told what would make one
    /// possible, rather than being told "not available" and left to guess.
    #[must_use]
    pub const fn requires(self) -> &'static str {
        match self.converters().first() {
            Some(converter) => converter.install_hint(),
            None => "",
        }
    }

    /// The file extension an exported file carries.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::PlainText => "txt",
            Self::Html => "html",
            Self::Markdown => "md",
            Self::Epub => "epub",
            Self::Pdf => "pdf",
            Self::Azw3 => "azw3",
            Self::Mobi => "mobi",
        }
    }

    /// The media type, for the `Content-Type` of a download.
    #[must_use]
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::PlainText => "text/plain; charset=utf-8",
            Self::Html => "text/html; charset=utf-8",
            Self::Markdown => "text/markdown; charset=utf-8",
            Self::Epub => "application/epub+zip",
            Self::Pdf => "application/pdf",
            Self::Azw3 => "application/vnd.amazon.ebook",
            Self::Mobi => "application/x-mobipocket-ebook",
        }
    }
}

/// Per-export choices (spec §13.2: "user-selected typography where supported").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ExportOptions {
    /// Whether the rendered file opens with a title page.
    pub title_page: bool,
    /// Whether each chapter is preceded by its own heading.
    pub chapter_headings: bool,
    /// A font stack to prefer, carried into formats that can express one.
    ///
    /// `None` means the export's own default rather than a reader's theme: the
    /// plan is explicit that "the reader's `reader_theme` is not the exporter's
    /// business", so a typography preference travels only when it was chosen for
    /// this export.
    pub font_family: Option<String>,
    /// A base font size in points, for formats that carry one.
    pub font_size_pt: Option<u16>,
}

impl ExportOptions {
    /// What a reader gets when they choose nothing: attribution and chapter
    /// headings, no title page, and no typography opinions.
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            title_page: false,
            chapter_headings: true,
            font_family: None,
            font_size_pt: None,
        }
    }

    /// The stored form.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Read the stored form.
    ///
    /// Anything unreadable becomes the defaults rather than an error: the options
    /// were written by this instance, and a row that somehow holds something else
    /// should produce an export with sensible choices rather than no export at
    /// all. The `deny_unknown_fields` attribute means an option removed later
    /// cannot resurrect a stale setting.
    #[must_use]
    pub fn from_json(raw: Option<&str>) -> Self {
        raw.and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_else(Self::defaults)
    }
}

/// A stable identifier for an export of one subject at one moment.
///
/// Name-based rather than random, so that exporting the same unchanged work twice
/// produces the same `dc:identifier` and the same bytes — which is what lets the
/// blob store recognise the second export as the first one's file instead of
/// storing a copy. A revised work gets a different moment and therefore a
/// different identifier, which is the other half of the same property: two
/// exports of different revisions must not claim to be the same publication.
///
/// FNV-1a, twice with different offsets, rather than SHA-256: the value names a
/// file, nothing depends on it being hard to invert, and a hash of the content is
/// already what `content_blobs` uses for that job.
#[must_use]
pub fn stable_identifier(parts: &[&str]) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut forward = OFFSET;
    let mut backward = OFFSET ^ 0x5555_5555_5555_5555;
    for part in parts {
        for byte in part.as_bytes() {
            forward = (forward ^ u64::from(*byte)).wrapping_mul(PRIME);
        }
        for byte in part.as_bytes().iter().rev() {
            backward = (backward ^ u64::from(*byte)).wrapping_mul(PRIME);
        }
        // A separator, so ("ab", "c") and ("a", "bc") cannot collide.
        forward = (forward ^ 0x1f).wrapping_mul(PRIME);
        backward = (backward ^ 0x1f).wrapping_mul(PRIME);
    }
    let hex = format!("{forward:016x}{backward:016x}");
    format!(
        "urn:uuid:{}-{}-4{}-8{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..32]
    )
}

/// Where a work came from, and on what terms it may be passed on (spec §13.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportProvenance {
    /// The source's name, as a reader should see it.
    pub source_name: String,
    /// The work's address at the source.
    pub source_url: String,
    /// When it was retrieved, RFC 3339.
    pub retrieved_at: String,
    /// The work's own identifier at the source, when it has one.
    pub source_key: Option<String>,
    /// A licence or permission statement, when the source states one.
    ///
    /// Optional because an invented licence is worse than none, and because an
    /// original work has no source to quote.
    pub permission: Option<String>,
}

/// What a chapter's content is, which is one of exactly two things.
///
/// An authored chapter is a [`Document`] — the editor's own format, with its
/// structure intact. An **imported** chapter is sanitized HTML, because that is
/// what the importer stores: the source's page, reduced to the allow-listed
/// subset, and never parsed back into the editor's model. Pretending the two are
/// one shape would mean either losing the editor's structure or inventing an
/// HTML-to-document parser whose mistakes would look like the author's.
///
/// The formats handle both, and the HTML side is a converter over exactly the
/// tag set the sanitizer permits — a closed set, so the conversion is not a guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChapterBody {
    /// An authored chapter, in the editor's format.
    Document(Document),
    /// An imported chapter: sanitized HTML from a source page.
    Html(String),
}

impl ChapterBody {
    /// The sanitized HTML rendering, which is what HTML and EPUB carry.
    #[must_use]
    pub fn to_sanitized_html(&self) -> String {
        match self {
            Self::Document(document) => document.to_sanitized_html(),
            // Already sanitized on the way in by the importer, which is the only
            // code that knew which parts of a foreign page were prose.
            Self::Html(html) => html.clone(),
        }
    }

    /// The plain-text rendering.
    #[must_use]
    pub fn to_plain_text(&self) -> String {
        match self {
            Self::Document(document) => document.to_plain_text(),
            Self::Html(html) => html_to_text(html),
        }
    }

    /// The Markdown rendering.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        match self {
            Self::Document(document) => {
                let mut out = String::new();
                for block in &document.blocks {
                    write_markdown_block(&mut out, block, 0);
                }
                out.trim_end().to_owned()
            }
            Self::Html(html) => html_to_markdown(html),
        }
    }

    /// Whether there is nothing to render.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.to_plain_text().trim().is_empty()
    }
}

/// One chapter, ready to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportChapter {
    /// 1-based reading position.
    pub ordinal: u32,
    /// The chapter's title as the author gave it. May be empty.
    pub title: String,
    /// The chapter's content.
    pub body: ChapterBody,
}

impl ExportChapter {
    /// An authored chapter.
    #[must_use]
    pub fn authored(ordinal: u32, title: impl Into<String>, document: Document) -> Self {
        Self {
            ordinal,
            title: title.into(),
            body: ChapterBody::Document(document),
        }
    }

    /// An imported chapter, whose body is sanitized HTML.
    #[must_use]
    pub fn imported(ordinal: u32, title: impl Into<String>, html: impl Into<String>) -> Self {
        Self {
            ordinal,
            title: title.into(),
            body: ChapterBody::Html(html.into()),
        }
    }
}

impl ExportChapter {
    /// The label to show, which is the author's title or an honest number.
    ///
    /// The fallback says `Chapter N` and nothing more, because that is what is
    /// known: the eFiction port's `format!("Chapter {i}")` fallback looked
    /// plausible and discarded the author's real title wherever the selector
    /// missed, with nothing raised.
    #[must_use]
    pub fn label(&self) -> String {
        let title = self.title.trim();
        if title.is_empty() {
            format!("Chapter {}", self.ordinal)
        } else {
            title.to_owned()
        }
    }
}

/// A whole work, ready to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportWork {
    /// The work's title.
    pub title: String,
    /// Attribution: who wrote it.
    pub author: String,
    /// A BCP 47 language tag.
    pub language: String,
    /// The chapters, in reading order.
    pub chapters: Vec<ExportChapter>,
    /// Where it came from, for an imported work.
    pub provenance: Option<ExportProvenance>,
    /// A stable identifier for this export, which becomes the EPUB's
    /// `dc:identifier`. Not the work's id: an export of a *revision* is its own
    /// artifact, and two exports of a revised work must not claim to be the same
    /// publication.
    pub identifier: String,
    /// The revision's moment, RFC 3339, for `dcterms:modified`.
    pub modified: String,
}

/// Why an export could not be rendered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExportError {
    /// The format needs an external program, which the caller owns.
    #[error("{format:?} export needs an external converter")]
    NeedsConverter {
        /// The format asked for.
        format: ExportFormat,
    },
    /// The work has nothing to export. Spec §13's fourth pitfall: an empty file
    /// reported as success is the failure that looks like success.
    #[error("this work has no chapters, so there is nothing to export")]
    Empty,
    /// The EPUB container could not be built.
    #[error(transparent)]
    Epub(#[from] EpubError),
}

/// Render a work (spec §13.1).
///
/// # Errors
/// [`ExportError::NeedsConverter`] for the three converter formats, which the
/// caller produces instead; [`ExportError::Empty`] for a work with no chapters;
/// and [`ExportError::Epub`] for a container that cannot be built.
pub fn render(
    work: &ExportWork,
    format: ExportFormat,
    options: &ExportOptions,
) -> Result<Vec<u8>, ExportError> {
    if work.chapters.is_empty() {
        return Err(ExportError::Empty);
    }
    Ok(match format {
        ExportFormat::PlainText => render_plain_text(work, options).into_bytes(),
        ExportFormat::Html => render_html(work, options).into_bytes(),
        ExportFormat::Markdown => render_markdown(work, options).into_bytes(),
        ExportFormat::Epub => {
            // The chapter borrows are assembled here rather than in a helper,
            // because they borrow from a vector that has to outlive the call and
            // a helper returning them would return borrows of its own locals.
            // The bodies are rendered into strings this function owns, and the
            // chapter list borrows from them: `EpubChapter` carries `&str`, so
            // something has to outlive it, and a helper returning the vector
            // would be returning borrows of its own locals.
            let bodies: Vec<String> = work
                .chapters
                .iter()
                .map(|chapter| chapter.body.to_sanitized_html())
                .collect();
            let chapters: Vec<EpubChapter<'_>> = work
                .chapters
                .iter()
                .zip(&bodies)
                .map(|(chapter, body)| EpubChapter {
                    ordinal: chapter.ordinal,
                    title: chapter.title.as_str(),
                    body: body.as_str(),
                })
                .collect();
            let provenance = work.provenance.as_ref().map(|provenance| EpubProvenance {
                source_name: provenance.source_name.as_str(),
                source_url: provenance.source_url.as_str(),
                retrieved_at: provenance.retrieved_at.as_str(),
                source_key: provenance.source_key.as_deref(),
                permission: provenance.permission.as_deref(),
            });
            epub::build(&EpubInput {
                identifier: &work.identifier,
                title: &work.title,
                author: &work.author,
                language: &work.language,
                modified: &work.modified,
                chapters: &chapters,
                provenance,
            })?
        }
        ExportFormat::Pdf | ExportFormat::Azw3 | ExportFormat::Mobi => {
            return Err(ExportError::NeedsConverter { format })
        }
    })
}

// ---------------------------------------------------------------------------
// Imported chapters: sanitized HTML in, text and Markdown out
// ---------------------------------------------------------------------------
//
// These two functions exist because an imported chapter is stored as HTML rather
// than as the editor's document (see [`ChapterBody`]). They are written against
// **exactly the tag set `lorehaven_scrapers::sanitize` permits**, which is a
// closed set rather than "whatever the web contains":
//
//   p, br, hr, em, strong, blockquote, ul, ol, li, h1..h4, a[href], ruby/rt
//
// Anything outside it was already dropped on the way in, so the converters do not
// need to handle it — and a test asserts the two lists still agree, so a tag
// added to the sanitizer without a rule here is a failing test rather than a
// paragraph that silently loses its text.

/// The text of a sanitized HTML fragment, with block structure as newlines.
#[must_use]
pub fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut chars = html.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '<' => {
                let mut tag = String::new();
                for next in chars.by_ref() {
                    if next == '>' {
                        break;
                    }
                    tag.push(next);
                }
                let trimmed = tag.trim();
                let closing = trimmed.starts_with('/');
                let name = trimmed
                    .trim_start_matches('/')
                    .split(|c: char| c.is_whitespace() || c == '/')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                match name.as_str() {
                    // A break, a scene divider and the end of a block all read as
                    // a line ending in plain text; a list item as its own line.
                    "br" | "hr" | "li" => out.push('\n'),
                    "p" | "blockquote" | "ul" | "ol" | "h1" | "h2" | "h3" | "h4" => {
                        if closing && !out.ends_with("\n\n") && !out.is_empty() {
                            out.push('\n');
                        }
                    }
                    // A ruby annotation's own text is pronunciation rather than
                    // prose, and running it into the base text reads as a stutter:
                    // 漢字かんじ instead of 漢字. The annotation runs to its own
                    // closing tag, which the sanitizer never nests.
                    "rt" | "rp" if !closing => loop {
                        match chars.next() {
                            None => break,
                            Some('<') => {
                                let inner = read_tag_body(&mut chars);
                                if inner.trim_start().starts_with('/') {
                                    break;
                                }
                            }
                            Some(_) => {}
                        }
                    },
                    _ => {}
                }
            }
            '&' => out.push_str(&decode_entity(&read_entity(&mut chars))),
            _ => out.push(ch),
        }
    }
    tidy_lines(&out)
}

/// The text of a tag whose opening `<` has already been consumed, up to its `>`.
fn read_tag_body(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut body = String::new();
    for next in chars.by_ref() {
        if next == '>' {
            break;
        }
        body.push(next);
    }
    body
}

/// The name of an entity whose `&` has already been consumed.
fn read_entity(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut entity = String::new();
    for next in chars.by_ref() {
        if next == ';' || entity.len() > 8 {
            break;
        }
        entity.push(next);
    }
    entity
}

/// The Markdown of a sanitized HTML fragment.
#[must_use]
pub fn html_to_markdown(html: &str) -> String {
    let mut out = MarkdownWriter::default();
    // What is currently open, so a closing tag knows what it is closing. A stack
    // rather than a set of flags, because `<strong><a href=..>x</a></strong>` and
    // `<a href=..><strong>x</strong></a>` nest in opposite orders and both occur.
    let mut open: Vec<Open> = Vec::new();
    let mut list: Option<bool> = None;
    let mut list_index = 0_usize;
    let mut chars = html.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '<' {
            if ch == '&' {
                out.text(&decode_entity(&read_entity(&mut chars)));
            } else {
                out.text(&escape_markdown_char(ch));
            }
            continue;
        }

        let tag = read_tag_body(&mut chars);
        let trimmed = tag.trim();
        let closing = trimmed.starts_with('/');
        let name = trimmed
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();

        match name.as_str() {
            "p" => out.blank_line(),
            "br" => out.hard_break(),
            "hr" => {
                out.blank_line();
                out.text("* * *");
                out.blank_line();
            }
            "h1" | "h2" | "h3" | "h4" => {
                out.blank_line();
                if !closing {
                    out.text(&"#".repeat(name[1..].parse::<usize>().unwrap_or(1)));
                    out.text(" ");
                }
            }
            "blockquote" => {
                out.blank_line();
                // The marker belongs on the lines *inside* the quote, so it is
                // applied as they are written rather than appended after them.
                out.quoted = !closing;
            }
            "ul" | "ol" => {
                out.blank_line();
                if closing {
                    list = None;
                } else {
                    list = Some(name == "ol");
                    list_index = 0;
                }
            }
            "li" => {
                out.item_break();
                if !closing {
                    list_index += 1;
                    let marker = if list == Some(true) {
                        format!("{list_index}. ")
                    } else {
                        "- ".to_owned()
                    };
                    out.text(&marker);
                    // An item's first paragraph starts *at* the marker, not after
                    // it: a blank line here would turn every list item into a
                    // paragraph of its own with the marker stranded above it.
                    out.at_item_start = true;
                }
            }
            "strong" | "em" => {
                let mark = if name == "strong" { "**" } else { "*" };
                if closing {
                    if open.last() == Some(&Open::Emphasis(mark)) {
                        open.pop();
                        out.text(mark);
                    }
                } else {
                    open.push(Open::Emphasis(mark));
                    out.text(mark);
                }
            }
            "a" => {
                if closing {
                    if let Some(Open::Link(href)) = open.last().cloned() {
                        open.pop();
                        // A link with no href is not a link, and `[text]()` is not
                        // Markdown — the empty case never opens a bracket.
                        if !href.is_empty() {
                            out.text(&format!("]({href})"));
                        }
                    }
                } else {
                    let href = attribute(trimmed, "href").unwrap_or_default();
                    if !href.is_empty() {
                        out.text("[");
                    }
                    open.push(Open::Link(href));
                }
            }
            "ruby" | "rt" | "rp" => {}
            _ => {}
        }
    }
    out.finish()
}

/// A Markdown buffer that knows where its lines begin.
///
/// The line-start state is the whole reason this is a type rather than a
/// `String`: a block quote's marker has to be written after every newline *while*
/// inside the quote, and a writer that appends the marker when the quote closes
/// puts it in the wrong place — which is what the first version of this did, and
/// it produced `Quoted.` followed by two bare `>` lines.
#[derive(Default)]
struct MarkdownWriter {
    out: String,
    /// Whether a block quote is open, so its marker is written per line.
    quoted: bool,
    /// Whether the next character written starts a line.
    at_line_start: bool,
    /// Whether a list item's marker has been written and nothing else yet.
    at_item_start: bool,
}

impl MarkdownWriter {
    fn text(&mut self, text: &str) {
        self.at_item_start = false;
        for ch in text.chars() {
            if self.at_line_start {
                if self.quoted {
                    self.out.push_str("> ");
                }
                self.at_line_start = false;
            }
            self.out.push(ch);
            if ch == '\n' {
                self.at_line_start = true;
            }
        }
    }

    /// A list item's break: a new line, but not a blank one.
    fn item_break(&mut self) {
        self.trim_trailing_spaces();
        while self.out.ends_with("\n\n") {
            self.out.pop();
        }
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        self.at_line_start = true;
        self.at_item_start = false;
    }

    /// A paragraph break: exactly one blank line, however many were implied.
    fn blank_line(&mut self) {
        // A paragraph opening inside a list item is the item's own text.
        if self.at_item_start {
            return;
        }
        self.trim_trailing_spaces();
        while self.out.ends_with("\n\n") || self.out.is_empty() {
            if self.out.is_empty() {
                return;
            }
            self.out.pop();
        }
        if !self.out.is_empty() {
            self.out.push_str("\n\n");
            self.at_line_start = true;
        }
    }

    /// A line break inside a paragraph, which Markdown spells as two trailing
    /// spaces — and which therefore must survive the trailing-space tidy-up.
    fn hard_break(&mut self) {
        if !self.out.ends_with(' ') && !self.out.is_empty() {
            self.out.push_str("  ");
        }
        self.out.push('\n');
        self.at_line_start = true;
    }

    fn trim_trailing_spaces(&mut self) {
        while self.out.ends_with(' ') {
            self.out.pop();
            self.at_line_start = false;
        }
    }

    fn finish(mut self) -> String {
        self.trim_trailing_spaces();
        while self.out.ends_with('\n') {
            self.out.pop();
        }
        self.out
    }
}

/// An inline construct waiting for its closing tag.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Open {
    /// `**` or `*`, written on open and again on close.
    Emphasis(&'static str),
    /// A link's target, written as `](href)` on close.
    Link(String),
}

/// The value of an attribute in a tag's own text, quoted either way.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let at = tag.find(&needle)?;
    let after = tag[at + needle.len()..].trim_start();
    let (quote, rest) = after.split_at(1);
    if quote == "\"" || quote == "'" {
        let end = rest.find(quote)?;
        Some(rest[..end].to_owned())
    } else {
        let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
        Some(rest[..end].to_owned())
    }
}

fn escape_markdown_char(ch: char) -> String {
    match ch {
        '*' | '_' | '`' | '[' | ']' => format!("\\{ch}"),
        _ => ch.to_string(),
    }
}

/// Decode the entities the sanitizer emits.
fn decode_entity(entity: &str) -> String {
    match entity {
        "amp" => "&".to_owned(),
        "lt" => "<".to_owned(),
        "gt" => ">".to_owned(),
        "quot" => "\"".to_owned(),
        "apos" | "#39" => "'".to_owned(),
        "nbsp" => " ".to_owned(),
        _ => {
            if let Some(code) = entity.strip_prefix('#') {
                if let Ok(value) = code.parse::<u32>() {
                    if let Some(ch) = char::from_u32(value) {
                        return ch.to_string();
                    }
                }
            }
            format!("&{entity};")
        }
    }
}

/// Collapse runs of blank lines, and trim each line.
///
/// Both conversions produce ragged whitespace — an inline tag boundary can leave
/// a space before a line ending, and a closed block can add a newline that is
/// already there — and leaving it makes an exported file look careless in a way
/// the author's text was not.
fn tidy_lines(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            if lines.last().is_none_or(|last| !last.is_empty()) {
                lines.push(String::new());
            }
        } else {
            lines.push(trimmed.trim_start_matches(' ').to_owned());
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Plain text
// ---------------------------------------------------------------------------

fn render_plain_text(work: &ExportWork, options: &ExportOptions) -> String {
    let mut out = String::new();
    if options.title_page {
        out.push_str(&work.title);
        out.push('\n');
        out.push_str(&work.author);
        out.push_str("\n\n");
    }
    for chapter in &work.chapters {
        if options.chapter_headings {
            out.push_str(&chapter.label());
            out.push_str("\n\n");
        }
        out.push_str(&chapter.body.to_plain_text());
        out.push_str("\n\n");
    }
    if let Some(provenance) = &work.provenance {
        out.push_str(&provenance_footer(provenance));
    }
    out
}

fn provenance_footer(provenance: &ExportProvenance) -> String {
    let mut out = String::from("—\n");
    out.push_str(&format!(
        "{} — retrieved {} from {}\n",
        provenance.source_url, provenance.retrieved_at, provenance.source_name
    ));
    if let Some(permission) = &provenance.permission {
        out.push_str(permission);
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------
// Standalone HTML
// ---------------------------------------------------------------------------

fn render_html(work: &ExportWork, options: &ExportOptions) -> String {
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html lang=\"");
    out.push_str(&escape_text(&work.language));
    out.push_str("\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>");
    out.push_str(&escape_text(&work.title));
    out.push_str("</title>\n<style>\n");
    out.push_str(&standalone_css(options));
    out.push_str("</style>\n</head>\n<body>\n");

    if options.title_page {
        out.push_str(&format!(
            "<header class=\"title-page\">\n<h1>{}</h1>\n<p class=\"author\">{}</p>\n</header>\n",
            escape_text(&work.title),
            escape_text(&work.author)
        ));
    }

    // The table of contents (spec §13.2). Anchored rather than linked to files,
    // because this is one document.
    out.push_str("<nav class=\"toc\" aria-label=\"Contents\">\n<h2>Contents</h2>\n<ol>\n");
    for chapter in &work.chapters {
        out.push_str(&format!(
            "<li><a href=\"#chapter-{}\">{}</a></li>\n",
            chapter.ordinal,
            escape_text(&chapter.label())
        ));
    }
    out.push_str("</ol>\n</nav>\n");

    for chapter in &work.chapters {
        out.push_str(&format!(
            "<section class=\"chapter\" id=\"chapter-{}\">\n",
            chapter.ordinal
        ));
        if options.chapter_headings {
            out.push_str(&format!("<h2>{}</h2>\n", escape_text(&chapter.label())));
        }
        out.push_str(&chapter.body.to_sanitized_html());
        out.push_str("\n</section>\n");
    }

    out.push_str(&format!(
        "<footer class=\"attribution\">\n<p>{} — {}</p>\n",
        escape_text(&work.title),
        escape_text(&work.author)
    ));
    if let Some(provenance) = &work.provenance {
        out.push_str(&format!(
            "<p>Imported from <a href=\"{}\">{}</a> on {}.</p>\n",
            escape_text(&provenance.source_url),
            escape_text(&provenance.source_name),
            escape_text(&provenance.retrieved_at)
        ));
        if let Some(permission) = &provenance.permission {
            out.push_str(&format!("<p>{}</p>\n", escape_text(permission)));
        }
    }
    out.push_str("</footer>\n</body>\n</html>\n");
    out
}

fn standalone_css(options: &ExportOptions) -> String {
    let mut css = String::from(
        "body { margin: 0 auto; max-width: 36em; padding: 1em; line-height: 1.5; }\n\
         .toc ol { padding-left: 1.2em; }\n\
         .chapter { margin-top: 2.5em; }\n\
         .attribution { margin-top: 3em; font-size: 0.9em; color: #555; }\n",
    );
    // Typography travels only when it was chosen for this export (spec §13.2).
    if let Some(font) = &options.font_family {
        css.push_str(&format!("body {{ font-family: {}; }}\n", escape_text(font)));
    }
    if let Some(size) = options.font_size_pt {
        css.push_str(&format!("body {{ font-size: {size}pt; }}\n"));
    }
    css
}

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------

/// Render Markdown, stating its own simplifications inside the file (spec §13.1).
fn render_markdown(work: &ExportWork, options: &ExportOptions) -> String {
    let mut out = String::new();
    // An HTML comment rather than front matter: front matter is a convention some
    // tools interpret, while a comment is inert everywhere and is still there for
    // a person who opens the file to read.
    out.push_str("<!--\n");
    out.push_str("Markdown export. Markdown cannot express every feature of the original, so:\n");
    out.push_str("  * bold and italic are preserved;\n");
    out.push_str("  * links are preserved as links;\n");
    out.push_str("  * headings become #-prefixed headings, at the author's level;\n");
    out.push_str("  * lists become - or numbered lists, and their nesting is flattened;\n");
    out.push_str("  * block quotes become > lines, and stay nested;\n");
    out.push_str("  * a scene break becomes a line of three asterisks;\n");
    out.push_str("  * typography is not carried: Markdown has none, and the reader's own\n");
    out.push_str("    font choice is not this export's business.\n");
    out.push_str("-->\n\n");

    out.push_str(&format!("# {}\n\n", markdown_escape(&work.title)));
    out.push_str(&format!("by {}\n\n", markdown_escape(&work.author)));

    // The table of contents, linking to headings by their text.
    out.push_str("## Contents\n\n");
    for chapter in &work.chapters {
        out.push_str(&format!(
            "- [{}](#{})\n",
            markdown_escape(&chapter.label()),
            markdown_anchor(&chapter.label())
        ));
    }
    out.push('\n');

    for chapter in &work.chapters {
        if options.chapter_headings {
            out.push_str(&format!("## {}\n\n", markdown_escape(&chapter.label())));
        }
        out.push_str(&chapter.body.to_markdown());
        out.push('\n');
    }

    out.push_str("---\n\n");
    out.push_str(&format!("{} — {}\n", work.title, work.author));
    if let Some(provenance) = &work.provenance {
        out.push_str(&format!(
            "Imported from [{}]({}) on {}.\n",
            provenance.source_name, provenance.source_url, provenance.retrieved_at
        ));
        if let Some(permission) = &provenance.permission {
            out.push_str(&format!("{permission}\n"));
        }
    }
    out
}

fn write_markdown_block(out: &mut String, block: &Block, depth: usize) {
    let indent = "  ".repeat(depth);
    match block {
        Block::Paragraph(content) => {
            out.push_str(&indent);
            write_markdown_inlines(out, content);
            out.push_str("\n\n");
        }
        Block::Heading { level, content } => {
            out.push_str(&indent);
            // Markdown's own levels start at `#`; the document's start at 1, so
            // the mapping is direct and a level-4 heading stays level 4.
            for _ in 0..*level {
                out.push('#');
            }
            out.push(' ');
            write_markdown_inlines(out, content);
            out.push_str("\n\n");
        }
        Block::List { ordered, items, .. } => {
            for (index, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}. ", index + 1)
                } else {
                    "- ".to_owned()
                };
                out.push_str(&indent);
                out.push_str(&marker);
                // A list item's first block sits on the marker's line; the rest
                // are indented under it, which is the only shape Markdown has for
                // a multi-block item.
                let mut first = true;
                for child in item {
                    if first {
                        write_markdown_block_inline(out, child);
                        first = false;
                    } else {
                        out.push('\n');
                        write_markdown_block(out, child, depth + 1);
                    }
                }
                if first {
                    out.push('\n');
                }
            }
            out.push('\n');
        }
        Block::Blockquote(content) => {
            let mut inner = String::new();
            for child in content {
                write_markdown_block(&mut inner, child, 0);
            }
            for line in inner.lines() {
                out.push_str(&indent);
                out.push('>');
                if !line.is_empty() {
                    out.push(' ');
                    out.push_str(line);
                }
                out.push('\n');
            }
            out.push('\n');
        }
        // An unambiguous scene break, and the one Markdown has a convention for.
        Block::SceneBreak => out.push_str("* * *\n\n"),
    }
}

/// A block rendered without its trailing blank line, for a list item's first line.
fn write_markdown_block_inline(out: &mut String, block: &Block) {
    let mut buffer = String::new();
    write_markdown_block(&mut buffer, block, 0);
    out.push_str(buffer.trim_end());
}

fn write_markdown_inlines(out: &mut String, inlines: &[Inline]) {
    for inline in inlines {
        match inline {
            Inline::LineBreak => out.push_str("  \n"),
            Inline::Text { text, marks } => {
                let mut rendered = markdown_escape(text);
                // Marks nest in the document's canonical order, so applying them
                // in reverse produces the same nesting as the source.
                for mark in marks.iter().rev() {
                    match mark {
                        Mark::Bold => rendered = format!("**{rendered}**"),
                        Mark::Italic => rendered = format!("*{rendered}*"),
                        Mark::Link { href } => {
                            rendered = format!("[{rendered}]({href})");
                        }
                    }
                }
                out.push_str(&rendered);
            }
        }
    }
}

/// Escape what Markdown would otherwise interpret.
///
/// Only the characters that change meaning: a title containing `*` or `_` would
/// otherwise render as emphasis, and one beginning with `#` as a heading.
fn markdown_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (index, ch) in text.chars().enumerate() {
        match ch {
            '*' | '_' | '`' | '[' | ']' => {
                out.push('\\');
                out.push(ch);
            }
            '#' if index == 0 => {
                out.push('\\');
                out.push('#');
            }
            _ => out.push(ch),
        }
    }
    out
}

/// A heading's anchor, which is what a Markdown table of contents links to.
fn markdown_anchor(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    for ch in label.chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
        } else if ch == ' ' || ch == '-' || ch == '_' {
            out.push('-');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work(chapters: Vec<ExportChapter>) -> ExportWork {
        ExportWork {
            title: "A Work".to_owned(),
            author: "An Author".to_owned(),
            language: "en".to_owned(),
            chapters,
            provenance: Some(ExportProvenance {
                source_name: "Archive of Our Own".to_owned(),
                source_url: "https://archiveofourown.org/works/1".to_owned(),
                retrieved_at: "2026-09-11T09:00:00Z".to_owned(),
                source_key: Some("1".to_owned()),
                permission: None,
            }),
            identifier: "urn:uuid:1".to_owned(),
            modified: "2026-09-11T00:00:00Z".to_owned(),
        }
    }

    fn imported() -> ExportWork {
        work(vec![
            ExportChapter::imported(1, "One", "<p>First <strong>bold</strong> word.</p>"),
            ExportChapter::imported(
                2,
                "",
                "<p>Second.</p><hr /><blockquote><p>Quoted.</p></blockquote>\
                 <ul><li><p>An item.</p></li></ul>",
            ),
        ])
    }

    #[test]
    fn format_strings_are_stable_and_round_trip() {
        // These strings are in a database column and in URLs.
        for format in ExportFormat::ALL {
            assert_eq!(ExportFormat::parse(format.as_str()), Some(format));
        }
        assert_eq!(ExportFormat::parse("docx"), None);
        assert_eq!(ExportFormat::PlainText.as_str(), "plain_text");
        assert_eq!(ExportFormat::Epub.as_str(), "epub");
    }

    #[test]
    fn the_builtin_formats_are_exactly_the_ones_with_no_converter() {
        // The two facts have to agree: an interface asks `is_builtin` to decide
        // what to offer, and the renderer refuses what has a converter.
        for format in ExportFormat::ALL {
            assert_eq!(
                format.is_builtin(),
                format.converters().is_empty(),
                "{format:?}"
            );
        }
        assert!(!ExportFormat::Pdf.is_builtin());
        assert_eq!(
            ExportFormat::Pdf.converters(),
            &[Converter::EbookConvert, Converter::Pandoc]
        );
        assert_eq!(ExportFormat::Mobi.converters(), &[Converter::EbookConvert]);
    }

    #[test]
    fn the_plain_text_export_matches_the_rendered_text() {
        // The plan's own criterion. An imported body is HTML, so the text is the
        // converter's, and it has to equal what a reader would read.
        let export = imported();
        let bytes = render(&export, ExportFormat::PlainText, &ExportOptions::defaults())
            .expect("plain text");
        let text = String::from_utf8(bytes).expect("utf-8");
        assert!(text.contains("First bold word."), "{text}");
        assert!(text.contains("Quoted."), "{text}");
        assert!(text.contains("An item."), "{text}");
        assert!(!text.contains('<'), "no markup survives: {text}");
        assert!(
            text.contains("One"),
            "the chapter's title is a heading: {text}"
        );
        // A chapter with no title gets a number and says so.
        assert!(text.contains("Chapter 2"), "{text}");
    }

    #[test]
    fn a_work_with_no_chapters_is_refused_in_every_format() {
        let empty = work(Vec::new());
        for format in ExportFormat::ALL {
            let error =
                render(&empty, format, &ExportOptions::defaults()).expect_err("must refuse");
            // A converter format is refused for needing a converter, which is
            // also true; the point is that none of them produces an empty file.
            assert!(
                matches!(
                    error,
                    ExportError::Empty | ExportError::NeedsConverter { .. }
                ),
                "{format:?}: {error:?}"
            );
        }
    }

    #[test]
    fn the_converter_formats_are_refused_by_the_renderer() {
        let error = render(&imported(), ExportFormat::Pdf, &ExportOptions::defaults())
            .expect_err("must refuse");
        assert_eq!(
            error,
            ExportError::NeedsConverter {
                format: ExportFormat::Pdf
            }
        );
    }

    #[test]
    fn the_standalone_html_has_attribution_a_contents_list_and_no_theme() {
        let export = imported();
        let html = String::from_utf8(
            render(&export, ExportFormat::Html, &ExportOptions::defaults()).expect("html"),
        )
        .expect("utf-8");
        assert!(html.starts_with("<!DOCTYPE html>"), "{html}");
        assert!(html.contains("<a href=\"#chapter-1\">One</a>"), "{html}");
        assert!(html.contains("A Work — An Author"), "attribution: {html}");
        assert!(
            html.contains("archiveofourown.org/works/1"),
            "provenance: {html}"
        );
        // The reader's own theme is not the exporter's business, so a default
        // export carries no font choice at all.
        assert!(!html.contains("font-family"), "{html}");

        // …unless one was chosen for this export.
        let options = ExportOptions {
            font_family: Some("Literata, serif".to_owned()),
            font_size_pt: Some(12),
            ..ExportOptions::defaults()
        };
        let html = String::from_utf8(render(&export, ExportFormat::Html, &options).expect("html"))
            .expect("utf-8");
        assert!(html.contains("font-family: Literata, serif"), "{html}");
        assert!(html.contains("font-size: 12pt"), "{html}");
    }

    #[test]
    fn markdown_states_its_own_simplifications() {
        let export = imported();
        let markdown = String::from_utf8(
            render(&export, ExportFormat::Markdown, &ExportOptions::defaults()).expect("markdown"),
        )
        .expect("utf-8");
        assert!(markdown.starts_with("<!--"), "{markdown}");
        assert!(markdown.contains("typography is not carried"), "{markdown}");
        assert!(markdown.contains("# A Work"), "{markdown}");
        assert!(markdown.contains("## Contents"), "{markdown}");
    }

    #[test]
    fn markdown_keeps_emphasis_links_and_structure_from_an_imported_chapter() {
        let export = work(vec![ExportChapter::imported(
            1,
            "One",
            "<p>Some <strong>bold</strong> and <em>italic</em> and \
             <a href=\"https://example.org/x\">a link</a>.</p>\
             <blockquote><p>Quoted.</p></blockquote>\
             <ul><li><p>First item.</p></li><li><p>Second item.</p></li></ul><hr />",
        )]);
        let markdown = String::from_utf8(
            render(&export, ExportFormat::Markdown, &ExportOptions::defaults()).expect("markdown"),
        )
        .expect("utf-8");
        assert!(
            markdown.contains("Some **bold** and *italic* and"),
            "{markdown}"
        );
        assert!(
            markdown.contains("[a link](https://example.org/x)"),
            "a link survives with its target: {markdown}"
        );
        assert!(markdown.contains("> Quoted."), "{markdown}");
        assert!(markdown.contains("- First item."), "{markdown}");
        assert!(markdown.contains("- Second item."), "{markdown}");
        assert!(markdown.contains("* * *"), "the scene break: {markdown}");
    }

    #[test]
    fn markdown_from_an_authored_chapter_keeps_its_headings_and_order() {
        // The editor's own format takes the other path, and it must produce the
        // same shape of Markdown: this is the pair that would drift apart if only
        // one were tested.
        let document = Document::from_plain("First paragraph.\n\nSecond paragraph.");
        let export = work(vec![ExportChapter::authored(1, "One", document)]);
        let markdown = String::from_utf8(
            render(&export, ExportFormat::Markdown, &ExportOptions::defaults()).expect("markdown"),
        )
        .expect("utf-8");
        assert!(markdown.contains("## One"), "{markdown}");
        assert!(markdown.contains("First paragraph."), "{markdown}");
        assert!(markdown.contains("Second paragraph."), "{markdown}");
    }

    #[test]
    fn the_sanitizers_tag_set_is_the_one_the_converters_cover() {
        // If the importer's allow-list grows, this fails rather than a paragraph
        // silently losing its text on the way out. The list is duplicated here on
        // purpose: the assertion is that the two *sources* agree.
        // The tags whose text a reader must see.
        const SANITIZER_ALLOWS: [&str; 15] = [
            "p",
            "br",
            "hr",
            "em",
            "strong",
            "blockquote",
            "ul",
            "ol",
            "li",
            "h1",
            "h2",
            "h3",
            "h4",
            "a",
            "ruby",
        ];
        for tag in SANITIZER_ALLOWS {
            let html = format!("<{tag}>x</{tag}>");
            assert!(
                html_to_text(&html).contains('x'),
                "the converter drops <{tag}>"
            );
        }

        // …and the two that are *intentionally* dropped, because a ruby
        // annotation is pronunciation rather than prose. Asserted rather than
        // omitted, so a change to the sanitizer's list is noticed here.
        for tag in ["rt", "rp"] {
            let html = format!("<p><ruby>base<{tag}>note</{tag}></ruby></p>");
            let text = html_to_text(&html);
            assert!(text.contains("base"), "{tag}: {text}");
            assert!(
                !text.contains("note"),
                "{tag} is dropped on purpose: {text}"
            );
        }
    }

    #[test]
    fn a_ruby_annotation_does_not_stutter_the_base_text() {
        // CJK sources carry pronunciation in <rt>, and running it into the base
        // text reads as a repeat of the word.
        let text = html_to_text("<p><ruby>漢字<rt>かんじ</rt></ruby> means kanji.</p>");
        assert!(text.contains("漢字 means kanji."), "{text}");
        assert!(!text.contains("かんじ"), "{text}");
    }

    #[test]
    fn entities_are_decoded_so_text_is_not_double_escaped() {
        let text = html_to_text("<p>A &amp; B &lt;tag&gt; &quot;quoted&quot; &#8212; end.</p>");
        assert!(text.contains("A & B <tag> \"quoted\""), "{text}");
        assert!(
            text.contains('\u{2014}'),
            "a numeric entity decodes: {text}"
        );
    }

    #[test]
    fn an_imported_chapter_to_epub_carries_its_prose_and_no_wrapper() {
        let export = imported();
        let bytes = render(&export, ExportFormat::Epub, &ExportOptions::defaults()).expect("epub");
        let facts = epub::validate(&bytes).expect("valid");
        assert_eq!(facts.chapter_titles, ["One", "Chapter 2"]);
    }
}
