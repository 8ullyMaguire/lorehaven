//! The EPUB container, built and validated (spec §13.3).
//!
//! # Why the container is written here rather than by a ZIP crate
//!
//! Two reasons, and the first is a requirement rather than a preference. The OCF
//! specification requires the `mimetype` entry to be the **first** entry in the
//! archive, **stored uncompressed**, and to contain exactly the string
//! `application/epub+zip` — a reader identifies a file as an EPUB by reading
//! those bytes at a fixed offset before it has parsed anything. A general ZIP
//! writer will happily reorder or deflate an entry, and several readers refuse an
//! EPUB that gets this wrong without saying why.
//!
//! The second is that the whole point of [`validate`] is to check an archive
//! against that specification, and a validator written on top of the same library
//! that wrote the file can only agree with itself. Both halves here are this
//! crate's, and the tests assert the invariants from the *parsed bytes*: the
//! first local header is the `mimetype`, its method field is `0`, and no extra
//! field is present on it.
//!
//! # Why the output is deterministic
//!
//! Every entry carries a fixed DOS timestamp rather than the moment of the
//! export. Rendered bytes go into `content_blobs`, which is content-addressed: a
//! deterministic export means exporting the same work twice stores one blob and
//! the second export is free, while a timestamp makes every export a distinct
//! file, defeating the deduplication the storage layer already does.

use std::io::Write;

use flate2::write::DeflateEncoder;
use flate2::Compression as Deflate;

use crate::document::escape_text;

/// What makes an archive an EPUB, at the offset a reader looks.
const MIMETYPE: &[u8] = b"application/epub+zip";

/// The path of the package document, and what `container.xml` points at.
const OPF_PATH: &str = "OEBPS/content.opf";

/// Characters are UTF-8 and the container is ZIP, which stores bytes. This is
/// the text a reader sees if it opens the file as a ZIP in a text editor.
const MIMETYPE_PATH: &str = "mimetype";
const CONTAINER_PATH: &str = "META-INF/container.xml";
const NAV_PATH: &str = "OEBPS/nav.xhtml";
const NCX_PATH: &str = "OEBPS/toc.ncx";
const STYLE_PATH: &str = "OEBPS/style.css";

/// How many chapters one export may hold.
///
/// A cap rather than a limit on the reader: an export is built in memory and
/// written as one blob, and a work larger than this is a mistake or an attack
/// rather than a reader's afternoon. One hundred thousand chapters is far above
/// any work the importer has recorded — the largest was 795 episodes — and far
/// below anything that would exhaust a small instance.
pub const MAX_CHAPTERS: usize = 100_000;

/// How large one chapter's XHTML may be.
///
/// The editor's own document limit is 2 MiB (see [`crate::document`]); this is
/// that limit with room for the wrapper, so a chapter that was accepted by the
/// editor cannot make the container unwritable.
pub const MAX_CHAPTER_BYTES: usize = 4 * 1024 * 1024;

/// Something went wrong building or reading a container.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EpubError {
    /// A work with no chapters. Spec §13's fourth pitfall: never produce an empty
    /// file and call it success — a reader can act on this, and a file they
    /// cannot open is not something they can act on.
    #[error("a work with no chapters cannot be exported")]
    Empty,
    /// More chapters than [`MAX_CHAPTERS`].
    #[error("the work has {count} chapters, above the {max} an export may hold")]
    TooManyChapters {
        /// How many the work has.
        count: usize,
        /// The ceiling.
        max: usize,
    },
    /// A chapter larger than [`MAX_CHAPTER_BYTES`].
    #[error("chapter {ordinal} is {bytes} bytes, above the {max} one chapter may be")]
    ChapterTooLarge {
        /// The chapter's position.
        ordinal: u32,
        /// Its size.
        bytes: usize,
        /// The ceiling.
        max: usize,
    },
    /// The bytes are not a ZIP archive this reader understands.
    #[error("not a readable ZIP archive: {0}")]
    NotAZip(String),
    /// The archive is a ZIP but not an EPUB.
    #[error("the archive is not a valid EPUB: {0}")]
    Invalid(String),
}

/// One chapter, as the container builder receives it.
#[derive(Debug, Clone, Copy)]
pub struct EpubChapter<'a> {
    /// 1-based position, which is also the file name's order.
    pub ordinal: u32,
    /// The chapter's title as shown to the reader.
    pub title: &'a str,
    /// The chapter body, as sanitized XHTML *fragments* — the output of
    /// [`crate::document::Document::to_sanitized_html`]. It is already an
    /// allow-listed subset with void elements self-closed, which is what makes it
    /// legal inside XHTML; the builder still escapes the title, which is text.
    pub body: &'a str,
}

/// Where the instance CTA goes in an export (spec §42.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CtaPlacement {
    /// After every chapter (the default).
    #[default]
    PerChapter,
    /// After the final chapter only.
    PerWork,
    /// Nowhere.
    Off,
}

impl CtaPlacement {
    /// Parse the TOML value. `None` for anything unrecognised so config
    /// validation can name the bad value rather than silently defaulting.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "per_chapter" => Some(Self::PerChapter),
            "per_work" => Some(Self::PerWork),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    /// The TOML value.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PerChapter => "per_chapter",
            Self::PerWork => "per_work",
            Self::Off => "off",
        }
    }

    /// Whether the chapter at `ordinal` (1-based) of `total` carries the
    /// CTA under this placement.
    #[must_use]
    pub fn carries_on(&self, ordinal: u32, total: u32) -> bool {
        match self {
            Self::PerChapter => true,
            Self::PerWork => ordinal == total,
            Self::Off => false,
        }
    }
}

/// The instance CTA to embed (spec §42.1). `html` must already be a
/// sanitized XHTML fragment — the builder embeds it verbatim and the
/// config layer sanitizes before constructing this.
#[derive(Debug, Clone, Copy)]
pub struct EpubCta<'a> {
    pub placement: CtaPlacement,
    /// Sanitized XHTML fragment.
    pub html: &'a str,
}

impl EpubCta<'_> {
    /// The class every rendered CTA block carries, so `validate` can find
    /// them again and tests can assert placement from the parsed bytes.
    pub const CLASS: &'static str = "instance-cta";
}

/// Where the work came from (spec §13.2: "source provenance"), and on what terms
/// it may be passed on.
#[derive(Debug, Clone, Copy)]
pub struct EpubProvenance<'a> {
    /// The source's name, as a reader should see it.
    pub source_name: &'a str,
    /// The work's address at the source.
    pub source_url: &'a str,
    /// When it was retrieved, RFC 3339.
    pub retrieved_at: &'a str,
    /// The work's own identifier at the source, when it has one.
    pub source_key: Option<&'a str>,
    /// A licence or permission statement, when the source states one.
    pub permission: Option<&'a str>,
}

/// Everything the container needs.
#[derive(Debug, Clone, Copy)]
pub struct EpubInput<'a> {
    /// `urn:uuid:…`, and stable for one export of one revision of one work.
    pub identifier: &'a str,
    /// The work's title.
    pub title: &'a str,
    /// Attribution: who wrote it.
    pub author: &'a str,
    /// A BCP 47 language tag.
    pub language: &'a str,
    /// RFC 3339, the revision's moment — `dcterms:modified` requires it.
    pub modified: &'a str,
    /// The chapters, in reading order.
    pub chapters: &'a [EpubChapter<'a>],
    /// Provenance and permission, when the work has any to state.
    pub provenance: Option<EpubProvenance<'a>>,
    /// The instance CTA (spec §42), when the placement is not `off` and
    /// the work is not exempt (§42.2).
    pub cta: Option<EpubCta<'a>>,
}

/// What [`validate`] established about an archive.
///
/// Returned rather than merely asserted, because the interesting facts are the
/// ones a caller can check against what it asked for: the title, the language,
/// and the chapters in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpubFacts {
    /// `dc:identifier`.
    pub identifier: String,
    /// `dc:title`.
    pub title: String,
    /// `dc:language`.
    pub language: String,
    /// Which chapter ordinals carry the instance CTA (spec §42.3).
    pub cta_chapters: Vec<u32>,
    /// `dc:creator`, which attribution requires.
    pub author: String,
    /// Chapter titles, in spine order.
    pub chapter_titles: Vec<String>,
    /// Whether a navigation document with one entry per chapter is present.
    pub has_navigation: bool,
    /// Whether the package states where the work came from.
    pub has_provenance: bool,
    /// Whether the package carries a permission statement.
    pub has_permission: bool,
}

// ---------------------------------------------------------------------------
// Building
// ---------------------------------------------------------------------------

/// Build an EPUB (spec §13.1's fourth format, and §13.3's subject).
///
/// # Errors
/// [`EpubError::Empty`] for a work with no chapters, [`EpubError::TooManyChapters`]
/// or [`EpubError::ChapterTooLarge`] when the work exceeds what one export may
/// hold. A work with no chapters is an error rather than an empty package
/// precisely because an empty package is the failure that looks like success.
pub fn build(input: &EpubInput<'_>) -> Result<Vec<u8>, EpubError> {
    if input.chapters.is_empty() {
        return Err(EpubError::Empty);
    }
    if input.chapters.len() > MAX_CHAPTERS {
        return Err(EpubError::TooManyChapters {
            count: input.chapters.len(),
            max: MAX_CHAPTERS,
        });
    }
    for chapter in input.chapters {
        if chapter.body.len() > MAX_CHAPTER_BYTES {
            return Err(EpubError::ChapterTooLarge {
                ordinal: chapter.ordinal,
                bytes: chapter.body.len(),
                max: MAX_CHAPTER_BYTES,
            });
        }
    }

    let mut zip = Zip::new();
    // The first entry, stored: this is the only thing a reader looks at before it
    // knows the file is an EPUB at all.
    zip.add(MIMETYPE_PATH, MIMETYPE, Method::Stored);
    zip.add(
        CONTAINER_PATH,
        container_xml(OPF_PATH).as_bytes(),
        Method::Deflated,
    );
    zip.add(STYLE_PATH, STYLE.as_bytes(), Method::Deflated);
    zip.add(
        OPF_PATH,
        package_document(input).as_bytes(),
        Method::Deflated,
    );
    zip.add(
        NAV_PATH,
        navigation_document(input).as_bytes(),
        Method::Deflated,
    );
    zip.add(NCX_PATH, ncx_document(input).as_bytes(), Method::Deflated);
    for chapter in input.chapters {
        zip.add(
            &chapter_path(chapter.ordinal),
            chapter_xhtml(input, chapter).as_bytes(),
            Method::Deflated,
        );
    }
    Ok(zip.finish())
}

/// Where a chapter lives inside the package.
///
/// Zero-padded so that a reader sorting file names — and a person looking at the
/// archive — sees the reading order rather than `10` before `2`.
fn chapter_path(ordinal: u32) -> String {
    format!("OEBPS/text/chapter-{ordinal:05}.xhtml")
}

fn container_xml(opf_path: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="{opf_path}" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#
    )
}

/// The package document: metadata, manifest, spine, and the provenance.
fn package_document(input: &EpubInput<'_>) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"pub-id\" xml:lang=\"",
    );
    out.push_str(&escape_text(input.language));
    out.push_str("\">\n");
    out.push_str("  <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n");
    // Attribution (spec §13.2): title, author and language are the three a reader
    // needs to know whose work this is and in what language.
    out.push_str(&format!(
        "    <dc:identifier id=\"pub-id\">{}</dc:identifier>\n",
        escape_text(input.identifier)
    ));
    out.push_str(&format!(
        "    <dc:title>{}</dc:title>\n",
        escape_text(input.title)
    ));
    out.push_str(&format!(
        "    <dc:creator>{}</dc:creator>\n",
        escape_text(input.author)
    ));
    out.push_str(&format!(
        "    <dc:language>{}</dc:language>\n",
        escape_text(input.language)
    ));
    // EPUB 3 requires dcterms:modified in exactly this shape.
    out.push_str(&format!(
        "    <meta property=\"dcterms:modified\">{}</meta>\n",
        escape_text(input.modified)
    ));

    if let Some(provenance) = input.provenance {
        // Provenance as a `dc:source` rather than a private extension: it is the
        // element the schema has for "where this came from", so a reader that
        // knows nothing about Lorehaven still shows it.
        out.push_str(&format!(
            "    <dc:source>{}</dc:source>\n",
            escape_text(provenance.source_url)
        ));
        out.push_str(&format!(
            "    <meta property=\"dcterms:provenance\">{}</meta>\n",
            escape_text(&format!(
                "{} — retrieved {}",
                provenance.source_name, provenance.retrieved_at
            ))
        ));
        if let Some(key) = provenance.source_key {
            out.push_str(&format!(
                "    <meta property=\"lorehaven:sourceIdentifier\">{}</meta>\n",
                escape_text(key)
            ));
        }
        if let Some(permission) = provenance.permission {
            // The copyright statement belongs in `dc:rights`, and is only written
            // when the source states one: an invented licence is worse than none.
            out.push_str(&format!(
                "    <dc:rights>{}</dc:rights>\n",
                escape_text(permission)
            ));
        }
        out.push_str(
            "    <meta property=\"lorehaven:edition\">imported copy; text as retrieved, \
             not the source's current revision</meta>\n",
        );
    }
    out.push_str("  </metadata>\n");

    out.push_str("  <manifest>\n");
    out.push_str(
        "    <item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n",
    );
    out.push_str(
        "    <item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n",
    );
    out.push_str("    <item id=\"style\" href=\"style.css\" media-type=\"text/css\"/>\n");
    for chapter in input.chapters {
        // The href is relative to the package document, which sits one level
        // above `text/` — a path that is wrong here produces a package whose
        // every spine item is "missing" in a validator and silently blank in a
        // reader that is lenient.
        out.push_str(&format!(
            "    <item id=\"c{}\" href=\"text/chapter-{:05}.xhtml\" media-type=\"application/xhtml+xml\"/>\n",
            chapter.ordinal, chapter.ordinal
        ));
    }
    out.push_str("  </manifest>\n");

    // The spine states the reading order. `toc="ncx"` keeps EPUB 2 readers
    // working, which is most of the devices this format is used for.
    out.push_str("  <spine toc=\"ncx\">\n");
    for chapter in input.chapters {
        out.push_str(&format!("    <itemref idref=\"c{}\"/>\n", chapter.ordinal));
    }
    out.push_str("  </spine>\n");
    out.push_str("</package>\n");
    out
}

/// The EPUB 3 navigation document, which is also what a reader's own table of
/// contents shows.
fn navigation_document(input: &EpubInput<'_>) -> String {
    let mut out = String::new();
    out.push_str(XHTML_OPEN);
    out.push_str(&format!(
        "<title>{}</title>\n</head>\n<body>\n",
        escape_text(input.title)
    ));
    out.push_str("<nav epub:type=\"toc\" id=\"toc\">\n<h1>Contents</h1>\n<ol>\n");
    for chapter in input.chapters {
        out.push_str(&format!(
            "<li><a href=\"text/chapter-{:05}.xhtml\">{}</a></li>\n",
            chapter.ordinal,
            escape_text(&chapter_label(chapter))
        ));
    }
    out.push_str("</ol>\n</nav>\n");
    // Spec §13.2 asks for a table of contents; the attribution belongs where a
    // reader lands, not only in the metadata a reading system may never show.
    out.push_str(&attribution_section(input));
    out.push_str("</body>\n</html>\n");
    out
}

/// The EPUB 2 table of contents, for readers that predate the nav document.
fn ncx_document(input: &EpubInput<'_>) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\" xml:lang=\"",
    );
    out.push_str(&escape_text(input.language));
    out.push_str("\">\n<head>\n");
    out.push_str(&format!(
        "<meta name=\"dtb:uid\" content=\"{}\"/>\n",
        escape_text(input.identifier)
    ));
    out.push_str("<meta name=\"dtb:depth\" content=\"1\"/>\n");
    out.push_str("</head>\n<docTitle>\n");
    out.push_str(&format!(
        "<text>{}</text>\n</docTitle>\n",
        escape_text(input.title)
    ));
    out.push_str("<navMap>\n");
    for chapter in input.chapters {
        out.push_str(&format!(
            "<navPoint id=\"n{}\" playOrder=\"{}\">\n<navLabel><text>{}</text></navLabel>\n\
             <content src=\"text/chapter-{:05}.xhtml\"/>\n</navPoint>\n",
            chapter.ordinal,
            chapter.ordinal,
            escape_text(&chapter_label(chapter)),
            chapter.ordinal
        ));
    }
    out.push_str("</navMap>\n</ncx>\n");
    out
}

/// A chapter's own title, or its number when the author did not give one.
///
/// The fallback is a number and says so; it does not invent a title, which is the
/// ported defect the eFiction work found (`Chapter {i}` looking plausible and
/// being wrong).
fn chapter_label(chapter: &EpubChapter<'_>) -> String {
    let title = chapter.title.trim();
    if title.is_empty() {
        format!("Chapter {}", chapter.ordinal)
    } else {
        title.to_owned()
    }
}

fn chapter_xhtml(input: &EpubInput<'_>, chapter: &EpubChapter<'_>) -> String {
    let mut out = String::new();
    out.push_str(XHTML_OPEN);
    out.push_str(&format!(
        "<title>{} — {}</title>\n<link rel=\"stylesheet\" type=\"text/css\" href=\"../style.css\"/>\n</head>\n<body>\n",
        escape_text(&chapter_label(chapter)),
        escape_text(input.title)
    ));
    out.push_str(&format!(
        "<h1 class=\"chapter-title\">{}</h1>\n",
        escape_text(&chapter_label(chapter))
    ));
    // A provenance line on every chapter rather than only in the package: a
    // chapter is the unit that gets copied out of an EPUB, and an extracted
    // chapter that no longer says where it came from is an orphan.
    if let Some(provenance) = input.provenance {
        out.push_str(&format!(
            "<p class=\"provenance\">{} — <a href=\"{}\">{}</a></p>\n",
            escape_text(provenance.source_name),
            escape_text(provenance.source_url),
            escape_text(input.title)
        ));
    }
    // The body is already an allow-listed XHTML fragment (see the module docs).
    out.push_str(chapter.body);
    // The instance CTA (spec §42): appended after the last paragraph, never
    // interleaved, and never inside the attribution block — attribution is
    // legal notice, not growth surface (§42.3).
    if let Some(cta) = input.cta {
        if cta.placement.carries_on(chapter.ordinal, input.chapters.len() as u32) {
            out.push_str(&format!(
                "\n<div class=\"{}\">{}</div>\n",
                EpubCta::CLASS,
                cta.html
            ));
        }
    }
    out.push_str("\n</body>\n</html>\n");
    out
}

/// The attribution block a nav document ends with.
fn attribution_section(input: &EpubInput<'_>) -> String {
    let mut out = String::from("<section class=\"attribution\">\n");
    out.push_str(&format!(
        "<p>{} — {}</p>\n",
        escape_text(input.title),
        escape_text(input.author)
    ));
    if let Some(provenance) = input.provenance {
        out.push_str(&format!(
            "<p>Imported from {} on {}.</p>\n",
            escape_text(provenance.source_name),
            escape_text(provenance.retrieved_at)
        ));
        if let Some(permission) = provenance.permission {
            out.push_str(&format!("<p>{}</p>\n", escape_text(permission)));
        }
    }
    out.push_str("</section>\n");
    out
}

const XHTML_OPEN: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE html>\n\
<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"en\">\n\
<head>\n<meta charset=\"utf-8\"/>\n";

/// One stylesheet for the package.
///
/// Deliberately plain: the reader's own typography is not the exporter's business
/// (the plan's words), so this carries only what an exported file needs to be
/// readable on a device that applies no styling of its own.
const STYLE: &str = "\
body { margin: 1em auto; max-width: 34em; padding: 0 1em; line-height: 1.5; }
h1.chapter-title { font-size: 1.3em; margin: 0 0 1em; }
p.provenance { font-size: 0.85em; color: #555; }
section.attribution { margin-top: 2em; font-size: 0.9em; color: #555; }
blockquote { margin: 1em 1.5em; }
";

// ---------------------------------------------------------------------------
// Validating
// ---------------------------------------------------------------------------

/// Check an archive against the EPUB specification (spec §13.3).
///
/// The checks are the ones the spec lists, and they are structural rather than
/// stylistic: the container is a ZIP whose first entry is a stored `mimetype`
/// holding exactly the right string, `container.xml` names a package document
/// that parses as XML with the required metadata, its spine lists every chapter
/// in order, and each spine item exists as an entry. Text is validated as UTF-8,
/// which is the only encoding an EPUB may use.
///
/// # Errors
/// [`EpubError::NotAZip`] when the bytes are not an archive at all, and
/// [`EpubError::Invalid`] when they are an archive that is not a valid EPUB, with
/// the reason. A refusal that says *what* is wrong is the difference between a
/// validator and a shrug.
pub fn validate(bytes: &[u8]) -> Result<EpubFacts, EpubError> {
    let archive = Archive::read(bytes)?;

    // 1. The mimetype entry: first, stored, and exactly right. A reader reads
    //    these bytes before it trusts anything else in the file.
    let first = archive
        .entries
        .first()
        .ok_or_else(|| EpubError::Invalid("the archive is empty".to_owned()))?;
    if first.name != MIMETYPE_PATH {
        return Err(EpubError::Invalid(format!(
            "the first entry is {:?} rather than {:?}",
            first.name, MIMETYPE_PATH
        )));
    }
    if first.method != 0 {
        return Err(EpubError::Invalid(
            "the mimetype entry is compressed; a reader cannot identify the file".to_owned(),
        ));
    }
    let mimetype = archive
        .read_stored(first)
        .map_err(|error| EpubError::Invalid(error.to_string()))?;
    if mimetype != MIMETYPE {
        return Err(EpubError::Invalid(format!(
            "the mimetype entry holds {:?}",
            String::from_utf8_lossy(&mimetype)
        )));
    }

    // 2. The container, which is how anything finds the package document.
    let container = archive.text(CONTAINER_PATH)?;
    let opf_path = require_between(&container, "full-path=\"", "\"")
        .ok_or_else(|| EpubError::Invalid("container.xml names no rootfile".to_owned()))?;
    let opf = archive.text(opf_path)?;

    // 3. The package's required metadata. Title, identifier and language are
    //    what a library needs; the creator is the attribution spec §13.2 asks
    //    for, and an export without it is a work with no author.
    let metadata = require_between(&opf, "<metadata", "</metadata>")
        .ok_or_else(|| EpubError::Invalid("the package has no metadata".to_owned()))?;
    let identifier = require_between(metadata, "<dc:identifier", "</dc:identifier>")
        .and_then(inner_text)
        .ok_or_else(|| EpubError::Invalid("the package states no identifier".to_owned()))?;
    let title = require_between(metadata, "<dc:title", "</dc:title>")
        .and_then(inner_text)
        .ok_or_else(|| EpubError::Invalid("the package states no title".to_owned()))?;
    let language = require_between(metadata, "<dc:language", "</dc:language>")
        .and_then(inner_text)
        .ok_or_else(|| EpubError::Invalid("the package states no language".to_owned()))?;
    let author = require_between(metadata, "<dc:creator", "</dc:creator>")
        .and_then(inner_text)
        .ok_or_else(|| {
            EpubError::Invalid(
                "the package states no creator, so it is attributed to nobody".to_owned(),
            )
        })?;
    if title.trim().is_empty() || author.trim().is_empty() {
        return Err(EpubError::Invalid(
            "the package's title or creator is empty".to_owned(),
        ));
    }
    let has_provenance = metadata.contains("<dc:source>");
    let has_permission = metadata.contains("<dc:rights>");

    // 4. The navigation document, and the chapters it must name.
    let nav = archive.text(NAV_PATH)?;
    // The nav's own links, filtered to the chapter files: a future nav that also
    // links a colophon or a licence must not shift the entry that corresponds to
    // each spine item.
    let nav_links: Vec<&str> = find_all_between(&nav, "<a href=\"", "\"")
        .into_iter()
        .filter(|href| href.starts_with("text/chapter-"))
        .collect();
    let has_navigation = nav.contains("epub:type=\"toc\"") && !nav_links.is_empty();

    // 5. The spine: every chapter, in order, each one a real entry.
    let spine = require_between(&opf, "<spine", "</spine>")
        .ok_or_else(|| EpubError::Invalid("the package has no spine".to_owned()))?;
    let spine_ids = find_all_between(spine, "idref=\"", "\"");
    if spine_ids.is_empty() {
        return Err(EpubError::Invalid(
            "the spine names no chapters, so the package has no reading order".to_owned(),
        ));
    }
    let manifest = require_between(&opf, "<manifest", "</manifest>")
        .ok_or_else(|| EpubError::Invalid("the package has no manifest".to_owned()))?;

    let mut chapter_titles = Vec::with_capacity(spine_ids.len());
    let mut cta_chapters = Vec::new();
    let mut previous: Option<u32> = None;
    for idref in &spine_ids {
        // The manifest maps the id to the file, which is the join a reader makes.
        let item =
            require_between(manifest, &format!("<item id=\"{idref}\""), "/>").ok_or_else(|| {
                EpubError::Invalid(format!(
                    "the spine names {idref:?}, which the manifest does not"
                ))
            })?;
        let href = require_between(item, "href=\"", "\"")
            .ok_or_else(|| EpubError::Invalid(format!("manifest item {idref:?} has no href")))?;
        let path = format!("OEBPS/{href}");

        let body = archive.text(&path)?;
        // The file has to be a well-formed-enough XHTML document and valid UTF-8.
        // `text` already refuses invalid UTF-8; this checks the skeleton, which
        // is what a reading system needs to render it at all.
        for required in ["<html", "</html>", "<body", "</body>"] {
            if !body.contains(required) {
                return Err(EpubError::Invalid(format!(
                    "{path} is not an XHTML document: it has no {required}"
                )));
            }
        }

        // The chapter title is what the nav document says it is, which is the
        // thing a reader sees; taking it from the file name would report the
        // ordinal to a reader's table of contents.
        let nav_entry = nav_links.get(chapter_titles.len()).ok_or_else(|| {
            EpubError::Invalid(format!("the navigation document names no entry for {path}"))
        })?;
        if *nav_entry != href {
            return Err(EpubError::Invalid(format!(
                "the navigation document lists {nav_entry:?} where the spine has {href:?}"
            )));
        }
        let label = require_between(&nav, &format!("<a href=\"{href}\">"), "</a>")
            .ok_or_else(|| EpubError::Invalid(format!("the nav entry for {href} has no label")))?;
        chapter_titles.push(unescape(label));

        // Order: the spine is the reading order, and a spine that runs backwards
        // is a work whose chapters are shuffled for every reader of the export.
        let ordinal = ordinal_from_href(href).ok_or_else(|| {
            EpubError::Invalid(format!("the spine item {href:?} is not a chapter file"))
        })?;
        if let Some(previous) = previous {
            if ordinal <= previous {
                return Err(EpubError::Invalid(format!(
                    "the spine puts chapter {ordinal} after chapter {previous}"
                )));
            }
        }
        previous = Some(ordinal);

        // Which chapters carry the instance CTA (spec §42.3): reported from
        // the parsed bytes so tests assert placement, not the builder's intent.
        if body.contains(&format!("class=\"{}\"", EpubCta::CLASS)) {
            cta_chapters.push(ordinal);
        }
    }

    // 6. Non-code-unit text: the metadata and every chapter had to decode as
    //    UTF-8 to get here, which is the check. This confirms the *content* is
    //    UTF-8 rather than merely ASCII-compatible, which is what a reader with
    //    a non-Latin script in a title depends on.
    for text in [&title, &author, &language] {
        if text
            .chars()
            .any(|c| c.is_control() && c != '\t' && c != '\n')
        {
            return Err(EpubError::Invalid(
                "the package metadata contains control characters".to_owned(),
            ));
        }
    }

    Ok(EpubFacts {
        identifier,
        title,
        language,
        author,
        chapter_titles,
        cta_chapters,
        has_navigation,
        has_provenance,
        has_permission,
    })
}

/// A `<dc:title>`-style element's own text, given everything from its name to
/// the start of its closing tag.
///
/// The slice has already been cut at the close by [`require_between`], so this
/// only has to step over the opening tag's attributes and its `>` — searching for
/// the closing tag here would search for something that is no longer in the
/// string, and would report every element as empty.
fn inner_text(section: &str) -> Option<String> {
    let start = section.find('>')?;
    Some(unescape(section[start + 1..].trim()))
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Everything between `open` and `close`, exclusive.
fn require_between<'a>(haystack: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = haystack.find(open)?;
    let after = &haystack[start + open.len()..];
    let end = after.find(close)?;
    Some(&after[..end])
}

/// Every occurrence of what sits between `open` and the next `close`.
fn find_all_between<'a>(haystack: &'a str, open: &str, close: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = haystack;
    while let Some(start) = rest.find(open) {
        let after = &rest[start + open.len()..];
        let Some(end) = after.find(close) else { break };
        out.push(&after[..end]);
        rest = &after[end..];
    }
    out
}

/// `text/chapter-00007.xhtml` is chapter 7.
fn ordinal_from_href(href: &str) -> Option<u32> {
    let stem = href.strip_prefix("text/chapter-")?.strip_suffix(".xhtml")?;
    stem.parse().ok()
}

// ---------------------------------------------------------------------------
// The ZIP container
// ---------------------------------------------------------------------------

fn dos_date_time() -> (u16, u16) {
    // 1980-01-01 00:00:00, the earliest a DOS timestamp can express, and
    // deliberately fixed: see the module docs on determinism.
    (0x0021, 0x0000)
}

/// How an entry's bytes are stored, which is a ZIP field and not the codec's
/// own level setting — the two are named apart deliberately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    /// Stored as-is. Required for `mimetype`, allowed for any entry.
    Stored,
    /// DEFLATE, which is what makes an archive smaller than its contents.
    Deflated,
}

impl Method {
    const fn code(self) -> u16 {
        match self {
            Self::Stored => 0,
            Self::Deflated => 8,
        }
    }
}

struct ZipEntry {
    name: String,
    method: u16,
    crc32: u32,
    compressed_size: u32,
    uncompressed_size: u32,
    local_offset: u32,
}

/// A ZIP archive, written by hand.
///
/// Only what an EPUB needs: no data descriptors, no ZIP64 (an export large enough
/// for 4 GiB is refused before it gets here by [`MAX_CHAPTERS`]), no encryption.
/// Every field the format requires is written explicitly, which is what lets the
/// mimetype entry be exactly what OCF demands.
#[derive(Default)]
struct Zip {
    out: Vec<u8>,
    entries: Vec<ZipEntry>,
}

impl Zip {
    fn new() -> Self {
        Self::default()
    }

    fn add(&mut self, name: &str, data: &[u8], compression: Method) {
        let stored = match compression {
            Method::Stored => data.to_vec(),
            Method::Deflated => {
                let mut encoder = DeflateEncoder::new(Vec::new(), deflate_level());
                // Writing into a `Vec` cannot fail, and a `write_all` error here
                // would be a bug in this function rather than a condition the
                // caller could act on.
                let _ = encoder.write_all(data);
                encoder.finish().unwrap_or_else(|_| data.to_vec())
            }
        };
        let crc32 = crc32fast::hash(data);
        let local_offset = u32::try_from(self.out.len()).unwrap_or(u32::MAX);
        let (time, date) = dos_date_time();
        let name_bytes = name.as_bytes();

        // Local file header.
        self.out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        self.out.extend_from_slice(&20_u16.to_le_bytes()); // version needed
                                                           // Bit 11 marks the name as UTF-8, which is what all of ours are.
        self.out.extend_from_slice(&0x0800_u16.to_le_bytes());
        self.out
            .extend_from_slice(&compression.code().to_le_bytes());
        self.out.extend_from_slice(&time.to_le_bytes());
        self.out.extend_from_slice(&date.to_le_bytes());
        self.out.extend_from_slice(&crc32.to_le_bytes());
        self.out
            .extend_from_slice(&(stored.len() as u32).to_le_bytes());
        self.out
            .extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.out
            .extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        // No extra field: the specification allows one, and a reader that
        // identifies EPUBs by offset is happier without.
        self.out.extend_from_slice(&0_u16.to_le_bytes());
        self.out.extend_from_slice(name_bytes);
        self.out.extend_from_slice(&stored);

        self.entries.push(ZipEntry {
            name: name.to_owned(),
            method: compression.code(),
            crc32,
            compressed_size: stored.len() as u32,
            uncompressed_size: data.len() as u32,
            local_offset,
        });
    }

    fn finish(mut self) -> Vec<u8> {
        let (time, date) = dos_date_time();
        let directory_offset = u32::try_from(self.out.len()).unwrap_or(u32::MAX);
        let entries = std::mem::take(&mut self.entries);

        for entry in &entries {
            let name_bytes = entry.name.as_bytes();
            self.out.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
            self.out.extend_from_slice(&20_u16.to_le_bytes()); // version made by
            self.out.extend_from_slice(&20_u16.to_le_bytes()); // version needed
            self.out.extend_from_slice(&0x0800_u16.to_le_bytes());
            self.out.extend_from_slice(&entry.method.to_le_bytes());
            self.out.extend_from_slice(&time.to_le_bytes());
            self.out.extend_from_slice(&date.to_le_bytes());
            self.out.extend_from_slice(&entry.crc32.to_le_bytes());
            self.out
                .extend_from_slice(&entry.compressed_size.to_le_bytes());
            self.out
                .extend_from_slice(&entry.uncompressed_size.to_le_bytes());
            self.out
                .extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            self.out.extend_from_slice(&0_u16.to_le_bytes()); // extra
            self.out.extend_from_slice(&0_u16.to_le_bytes()); // comment
            self.out.extend_from_slice(&0_u16.to_le_bytes()); // disk
            self.out.extend_from_slice(&0_u16.to_le_bytes()); // internal attrs
            self.out.extend_from_slice(&0_u32.to_le_bytes()); // external attrs
            self.out
                .extend_from_slice(&entry.local_offset.to_le_bytes());
            self.out.extend_from_slice(name_bytes);
        }

        let directory_size = u32::try_from(self.out.len()).unwrap_or(u32::MAX) - directory_offset;
        let count = u16::try_from(entries.len()).unwrap_or(u16::MAX);
        self.out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        self.out.extend_from_slice(&0_u16.to_le_bytes()); // this disk
        self.out.extend_from_slice(&0_u16.to_le_bytes()); // directory disk
        self.out.extend_from_slice(&count.to_le_bytes());
        self.out.extend_from_slice(&count.to_le_bytes());
        self.out.extend_from_slice(&directory_size.to_le_bytes());
        self.out.extend_from_slice(&directory_offset.to_le_bytes());
        self.out.extend_from_slice(&0_u16.to_le_bytes()); // no comment
        self.out
    }
}

/// The deflate level. Six is the usual balance, and the level is stated here
/// rather than left to a default so that an output's size is reproducible.
const fn deflate_level() -> Deflate {
    Deflate::new(6)
}

/// A ZIP archive, read back for validation.
struct Archive<'a> {
    bytes: &'a [u8],
    entries: Vec<ArchiveEntry>,
}

struct ArchiveEntry {
    name: String,
    method: u16,
    crc32: u32,
    compressed_size: usize,
    uncompressed_size: usize,
    data_offset: usize,
}

impl<'a> Archive<'a> {
    fn read(bytes: &'a [u8]) -> Result<Self, EpubError> {
        // The end-of-central-directory record, found from the end: it is the last
        // thing in the file, and a comment (which we never write) is the only
        // thing that may follow it.
        let eocd = find_eocd(bytes)
            .ok_or_else(|| EpubError::NotAZip("no end-of-central-directory record".to_owned()))?;
        let count = read_u16(bytes, eocd + 10)? as usize;
        let directory_offset = read_u32(bytes, eocd + 16)? as usize;

        let mut entries = Vec::with_capacity(count);
        let mut cursor = directory_offset;
        for _ in 0..count {
            if read_u32(bytes, cursor)? != 0x0201_4b50 {
                return Err(EpubError::NotAZip(
                    "the central directory is malformed".to_owned(),
                ));
            }
            let method = read_u16(bytes, cursor + 10)?;
            let crc32 = read_u32(bytes, cursor + 16)?;
            let compressed_size = read_u32(bytes, cursor + 20)? as usize;
            let uncompressed_size = read_u32(bytes, cursor + 24)? as usize;
            let name_len = read_u16(bytes, cursor + 28)? as usize;
            let extra_len = read_u16(bytes, cursor + 30)? as usize;
            let comment_len = read_u16(bytes, cursor + 32)? as usize;
            let local_offset = read_u32(bytes, cursor + 42)? as usize;
            let name = String::from_utf8(
                bytes
                    .get(cursor + 46..cursor + 46 + name_len)
                    .ok_or_else(|| EpubError::NotAZip("a truncated entry name".to_owned()))?
                    .to_vec(),
            )
            .map_err(|_| EpubError::NotAZip("an entry name is not UTF-8".to_owned()))?;

            // The local header repeats the name and its own extra field length,
            // which is not necessarily the central one — so the data offset has
            // to come from the local record rather than from arithmetic on the
            // central one.
            if read_u32(bytes, local_offset)? != 0x0403_4b50 {
                return Err(EpubError::NotAZip(format!(
                    "the entry {name:?} has no local header"
                )));
            }
            let local_name_len = read_u16(bytes, local_offset + 26)? as usize;
            let local_extra_len = read_u16(bytes, local_offset + 28)? as usize;
            let data_offset = local_offset + 30 + local_name_len + local_extra_len;

            entries.push(ArchiveEntry {
                name,
                method,
                crc32,
                compressed_size,
                uncompressed_size,
                data_offset,
            });
            cursor += 46 + name_len + extra_len + comment_len;
        }

        Ok(Self { bytes, entries })
    }

    fn find(&self, name: &str) -> Result<&ArchiveEntry, EpubError> {
        self.entries
            .iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| EpubError::Invalid(format!("the archive has no {name}")))
    }

    fn raw(&self, entry: &ArchiveEntry) -> Result<Vec<u8>, EpubError> {
        let end = entry
            .data_offset
            .checked_add(entry.compressed_size)
            .ok_or_else(|| {
                EpubError::NotAZip("an entry runs past the end of the archive".to_owned())
            })?;
        let stored = self
            .bytes
            .get(entry.data_offset..end)
            .ok_or_else(|| EpubError::NotAZip("a truncated entry".to_owned()))?;
        if stored.len() != entry.compressed_size {
            return Err(EpubError::NotAZip("a truncated entry".to_owned()));
        }
        match entry.method {
            0 => Ok(stored.to_vec()),
            8 => {
                // `flate2`'s raw deflate decoder, which is the inverse of what
                // `Zip::add` wrote with a raw `DeflateEncoder`.
                let mut decoder = flate2::read::DeflateDecoder::new(stored);
                let mut out = Vec::with_capacity(entry.uncompressed_size);
                std::io::Read::read_to_end(&mut decoder, &mut out).map_err(|error| {
                    EpubError::NotAZip(format!("a deflated entry could not be read: {error}"))
                })?;
                Ok(out)
            }
            other => Err(EpubError::Invalid(format!(
                "an entry uses compression method {other}, which is not allowed in an EPUB"
            ))),
        }
    }

    /// An entry's bytes, with its checksum verified.
    ///
    /// The CRC is checked rather than trusted because it is the only thing that
    /// distinguishes "the archive is complete" from "the bytes happen to
    /// decompress": a truncated or corrupted entry frequently still produces
    /// *something*, which is precisely the failure that reaches a reader.
    fn read_stored(&self, entry: &ArchiveEntry) -> Result<Vec<u8>, EpubError> {
        let data = self.raw(entry)?;
        if crc32fast::hash(&data) != entry.crc32 {
            return Err(EpubError::Invalid(format!(
                "the entry {:?} fails its own checksum",
                entry.name
            )));
        }
        Ok(data)
    }

    /// An entry's bytes as text, which an EPUB's are.
    fn text(&self, name: &str) -> Result<String, EpubError> {
        let entry = self.find(name)?;
        let data = self.read_stored(entry)?;
        String::from_utf8(data).map_err(|_| {
            EpubError::Invalid(format!("{name} is not valid UTF-8, which an EPUB must be"))
        })
    }
}

fn find_eocd(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 22 {
        return None;
    }
    // A comment may follow the record, so it is searched backwards from the end.
    let start = bytes.len().saturating_sub(22 + 0xffff);
    (start..=bytes.len() - 22)
        .rev()
        .find(|offset| read_u32(bytes, *offset).ok() == Some(0x0605_4b50))
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16, EpubError> {
    let slice = bytes
        .get(at..at + 2)
        .ok_or_else(|| EpubError::NotAZip("an unexpected end of archive".to_owned()))?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, EpubError> {
    let slice = bytes
        .get(at..at + 4)
        .ok_or_else(|| EpubError::NotAZip("an unexpected end of archive".to_owned()))?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapters() -> Vec<EpubChapter<'static>> {
        vec![
            EpubChapter {
                ordinal: 1,
                title: "An Arrival",
                body: "<p>It began, as these things do, at an awkward hour.</p>",
            },
            EpubChapter {
                ordinal: 2,
                // No title: the builder must not invent one, and must not omit
                // the chapter either.
                title: "  ",
                body: "<p>The second movement.</p><hr /><p>And then silence.</p>",
            },
            EpubChapter {
                // A title with characters that are XML-significant, because a
                // reader's own work is where those appear.
                ordinal: 3,
                title: "A & B <not a tag>",
                body: "<p>Ending.</p>",
            },
        ]
    }

    fn input<'a>(chapters: &'a [EpubChapter<'a>]) -> EpubInput<'a> {
        EpubInput {
            identifier: "urn:uuid:11111111-2222-3333-4444-555555555555",
            title: "A Work & Its Title",
            author: "Someone",
            language: "en",
            modified: "2026-09-11T00:00:00Z",
            chapters,
            provenance: Some(EpubProvenance {
                source_name: "Archive of Our Own",
                source_url: "https://archiveofourown.org/works/1",
                retrieved_at: "2026-09-11T09:00:00Z",
                source_key: Some("1"),
                permission: Some("The author permits redistribution of unaltered copies."),
            }),
            cta: None,
        }
    }

    // --- CTA placement (spec §42) -----------------------------------------

    fn input_with_cta<'a>(
        chapters: &'a [EpubChapter<'a>],
        placement: CtaPlacement,
    ) -> EpubInput<'a> {
        EpubInput {
            cta: Some(EpubCta {
                placement,
                html: "<p>If you enjoyed this, <strong>leave a comment</strong> or <strong>share</strong> with a friend.</p>",
            }),
            ..input(chapters)
        }
    }

    #[test]
    fn cta_default_is_per_chapter_on_every_chapter() {
        let chapters = chapters();
        let bytes = build(&input_with_cta(&chapters, CtaPlacement::PerChapter)).expect("build");
        let facts = validate(&bytes).expect("validate");
        assert_eq!(facts.cta_chapters, vec![1, 2, 3]);
    }

    #[test]
    fn cta_per_work_lands_on_the_last_chapter_only() {
        let chapters = chapters();
        let bytes = build(&input_with_cta(&chapters, CtaPlacement::PerWork)).expect("build");
        let facts = validate(&bytes).expect("validate");
        assert_eq!(facts.cta_chapters, vec![3]);
    }

    #[test]
    fn cta_off_places_nothing_anywhere() {
        let chapters = chapters();
        let bytes = build(&input_with_cta(&chapters, CtaPlacement::Off)).expect("build");
        let facts = validate(&bytes).expect("validate");
        assert!(facts.cta_chapters.is_empty());
    }

    #[test]
    fn cta_absent_leaves_the_export_untouched() {
        let chapters = chapters();
        let bytes = build(&input(&chapters)).expect("build");
        let facts = validate(&bytes).expect("validate");
        assert!(facts.cta_chapters.is_empty());
    }

    #[test]
    fn cta_never_reaches_the_attribution_block() {
        let chapters = chapters();
        let bytes = build(&input_with_cta(&chapters, CtaPlacement::PerChapter)).expect("build");
        // The attribution section is a separate document; the CTA class must
        // appear in no document other than the chapters (§42.3).
        let archive = Archive::read(&bytes).expect("archive");
        for entry in &archive.entries {
            if entry.name.starts_with("OEBPS/text/chapter-") {
                continue;
            }
            let text = archive.text(&entry.name).unwrap_or_default();
            assert!(
                !text.contains(EpubCta::CLASS),
                "CTA leaked into {}",
                entry.name
            );
        }
    }

    #[test]
    fn cta_placement_parses_and_round_trips() {
        assert_eq!(CtaPlacement::parse("per_chapter"), Some(CtaPlacement::PerChapter));
        assert_eq!(CtaPlacement::parse("per_work"), Some(CtaPlacement::PerWork));
        assert_eq!(CtaPlacement::parse("off"), Some(CtaPlacement::Off));
        assert_eq!(CtaPlacement::parse("everywhere"), None);
        assert_eq!(CtaPlacement::default().as_str(), "per_chapter");
        // carries_on: the placement rule, stated directly.
        assert!(CtaPlacement::PerChapter.carries_on(1, 3));
        assert!(CtaPlacement::PerChapter.carries_on(3, 3));
        assert!(!CtaPlacement::PerWork.carries_on(1, 3));
        assert!(CtaPlacement::PerWork.carries_on(3, 3));
        assert!(!CtaPlacement::Off.carries_on(3, 3));
    }


    #[test]
    fn an_epub_export_opens_and_contains_every_chapter() {
        // The plan's own criterion for this format.
        let chapters = chapters();
        let bytes = build(&input(&chapters)).expect("an epub");
        let facts = validate(&bytes).expect("the epub validates");

        assert_eq!(facts.chapter_titles.len(), 3, "{facts:?}");
        assert_eq!(
            facts.chapter_titles,
            ["An Arrival", "Chapter 2", "A & B <not a tag>"]
        );
        assert_eq!(facts.title, "A Work & Its Title");
        assert_eq!(facts.author, "Someone");
        assert_eq!(facts.language, "en");
        assert!(facts.has_navigation, "a table of contents is required");
        assert!(
            facts.has_provenance,
            "spec §13.2 asks for source provenance"
        );
        assert!(facts.has_permission, "the source stated terms; they travel");

        // Every chapter's prose is in the container, and so is the one whose
        // title needed escaping.
        let archive = Archive::read(&bytes).expect("a zip");
        let third = archive
            .text("OEBPS/text/chapter-00003.xhtml")
            .expect("chapter 3");
        assert!(third.contains("Ending."));
        assert!(third.contains("A &amp; B &lt;not a tag&gt;"), "{third}");
    }

    #[test]
    fn the_container_identifies_itself_the_way_a_reader_looks() {
        // The first entry, stored, with no extra field: a reader reads these
        // bytes at a fixed offset before it has parsed anything else, and
        // several refuse the file outright if this is wrong.
        let chapters = chapters();
        let bytes = build(&input(&chapters)).expect("an epub");
        assert_eq!(&bytes[0..4], b"PK\x03\x04", "the first local header");
        let method = u16::from_le_bytes([bytes[8], bytes[9]]);
        assert_eq!(method, 0, "mimetype must be stored, not deflated");
        let name_len = u16::from_le_bytes([bytes[26], bytes[27]]) as usize;
        let extra_len = u16::from_le_bytes([bytes[28], bytes[29]]) as usize;
        assert_eq!(extra_len, 0, "the mimetype entry carries no extra field");
        let name = &bytes[30..30 + name_len];
        assert_eq!(name, b"mimetype");
        let data = &bytes[30 + name_len..30 + name_len + MIMETYPE.len()];
        assert_eq!(data, MIMETYPE);
    }

    #[test]
    fn the_bytes_are_the_same_every_time() {
        // The output goes into a content-addressed blob store: two exports of an
        // unchanged work must be one blob, which a timestamp would prevent.
        let chapters = chapters();
        let first = build(&input(&chapters)).expect("an epub");
        let second = build(&input(&chapters)).expect("an epub");
        assert_eq!(first, second);
    }

    #[test]
    fn a_work_with_no_chapters_is_an_error_rather_than_an_empty_package() {
        // The plan's fourth pitfall, and the reason this is an error: a file that
        // exists and opens to nothing looks like success.
        let error = build(&input(&[])).expect_err("must refuse");
        assert_eq!(error, EpubError::Empty);
    }

    #[test]
    fn the_chapter_order_is_the_spine_and_the_navigation() {
        let chapters = chapters();
        let bytes = build(&input(&chapters)).expect("an epub");
        let archive = Archive::read(&bytes).expect("a zip");
        let opf = archive.text(OPF_PATH).expect("the package");
        let spine = require_between(&opf, "<spine", "</spine>").expect("a spine");
        let refs = find_all_between(spine, "idref=\"", "\"");
        assert_eq!(refs, ["c1", "c2", "c3"], "the reading order");

        let nav = archive.text(NAV_PATH).expect("the nav");
        let links = find_all_between(&nav, "href=\"text/chapter-", "\"");
        assert_eq!(
            links,
            ["00001.xhtml", "00002.xhtml", "00003.xhtml"],
            "the table of contents matches the spine"
        );
    }

    #[test]
    fn a_package_that_states_no_author_is_refused() {
        // Attribution is not decoration: an export that names nobody is one a
        // reader cannot cite, and the validator is where that becomes an error
        // rather than a missing line in a library.
        let chapters = chapters();
        let bytes = build(&input(&chapters)).expect("an epub");

        // Damage the package's attribution and confirm the validator says why.
        let damaged = replace_entry(&bytes, OPF_PATH, |opf| {
            opf.replace("Someone", "")
                .replace("<dc:creator></dc:creator>", "")
        });
        let error = validate(&damaged).expect_err("must refuse");
        assert!(
            error.to_string().contains("creator"),
            "the refusal should name the missing attribution: {error}"
        );
    }

    #[test]
    fn a_file_that_is_not_a_zip_is_reported_as_such() {
        let error = validate(b"not a zip at all, not even close").expect_err("must refuse");
        assert!(matches!(error, EpubError::NotAZip(_)), "{error:?}");
        assert!(error.to_string().contains("end-of-central-directory"));
    }

    #[test]
    fn a_zip_whose_first_entry_is_not_the_mimetype_is_not_an_epub() {
        // A reader identifies the file by the first entry. Anything else is a
        // generic archive, however correct its contents.
        let mut zip = Zip::new();
        zip.add("OEBPS/content.opf", b"<package/>", Method::Deflated);
        zip.add(MIMETYPE_PATH, MIMETYPE, Method::Stored);
        let error = validate(&zip.finish()).expect_err("must refuse");
        assert!(
            error.to_string().contains("first entry"),
            "the refusal should name the ordering problem: {error}"
        );
    }

    #[test]
    fn a_deflated_mimetype_is_refused() {
        // The specification requires it stored, and a reader that has to inflate
        // to identify a file is a reader that may not be able to.
        let mut zip = Zip::new();
        zip.add(MIMETYPE_PATH, MIMETYPE, Method::Deflated);
        let error = validate(&zip.finish()).expect_err("must refuse");
        assert!(
            error.to_string().contains("compressed"),
            "the refusal should say why: {error}"
        );
    }

    /// Rebuild the archive with one entry's text put through `edit`.
    ///
    /// Used to damage a valid EPUB in a specific way, which is how a validator's
    /// checks get tests without hand-writing an archive for each one.
    fn replace_entry(bytes: &[u8], path: &str, edit: impl Fn(&str) -> String) -> Vec<u8> {
        let archive = Archive::read(bytes).expect("a zip");
        let mut zip = Zip::new();
        for entry in &archive.entries {
            let stored = archive.raw(entry).expect("an entry");
            let text = String::from_utf8_lossy(&stored).into_owned();
            let method = if entry.method == 0 {
                Method::Stored
            } else {
                Method::Deflated
            };
            if entry.name == path {
                zip.add(path, edit(&text).as_bytes(), method);
            } else {
                zip.add(&entry.name, &stored, method);
            }
        }
        zip.finish()
    }
}
