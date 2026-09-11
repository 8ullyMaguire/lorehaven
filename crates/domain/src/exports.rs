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

/// One chapter, ready to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportChapter {
    /// 1-based reading position.
    pub ordinal: u32,
    /// The chapter's title as the author gave it. May be empty.
    pub title: String,
    /// The chapter's content.
    pub document: Document,
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
                .map(|chapter| chapter.document.to_sanitized_html())
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
        out.push_str(&chapter.document.to_plain_text());
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
        out.push_str(&chapter.document.to_sanitized_html());
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
        for block in &work_chapter_blocks(chapter) {
            write_markdown_block(&mut out, block, 0);
        }
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

fn work_chapter_blocks(chapter: &ExportChapter) -> Vec<Block> {
    chapter.document.blocks.clone()
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
