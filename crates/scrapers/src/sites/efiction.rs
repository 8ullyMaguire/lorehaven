//! The eFiction family.
//!
//! eFiction is not a site. It is a PHP script that a few hundred small archives
//! installed between roughly 2005 and 2012, each with its own name, its own
//! skin, its own moderator, and — often — its own idea of what a date looks
//! like. One adapter serves all of them because they share an engine and
//! therefore a URL shape and a set of container names, and because the
//! alternative is nineteen near-identical files that drift apart.
//!
//! # Recognised URLs
//!
//! ```text
//! https://{member}/viewstory.php?sid={work}
//! https://{member}/viewstory.php?sid={work}&index={page}
//! https://{member}/viewstory.php?sid={work}&chapter={n}
//! https://{member}/viewstory.php?sid={work}&textsize=0&chapter={n}
//! https://{member}/viewstory.php?action=printable&sid={work}&chapter=all
//! ```
//!
//! Any of them previews the work, because `sid` names the work and everything
//! else is a view of it. `index=` addresses a page of the chapter list and
//! `chapter=` addresses one chapter's text; neither changes which work is being
//! read, so an adapter that refused them would refuse URLs a reader can paste.
//!
//! Two members keep their archive in a subdirectory rather than at the host
//! root — `dark-solace.org/elysian` and the two archives on `sinfuldreams.com` —
//! so the member table carries a path as well as a host, and a URL is only
//! claimed when its path begins with the member's own.
//!
//! # What the markup actually is
//!
//! Derived from fourteen pages recorded on 2026-09-10 from five members (see
//! `tests/fixtures/efiction/`), and written down here because the ported code
//! was wrong about several of them in ways that would have been invisible:
//!
//! * `div.infobox` is the **chapter page's** metadata block, not the work
//!   page's. The work page's block is a bare `div.content` — the same `content`
//!   class, with no `infobox` around it. An adapter selecting
//!   `.infobox .content` therefore reads a chapter page and returns nothing at
//!   all from a work page, which is the shape of bug that looks like "this
//!   source is unsupported" rather than "this selector is wrong".
//! * A chapter has two URLs and they are the reading view and the print view,
//!   not two skins. `&chapter={n}` — the one a table of contents links to — is
//!   the **print** view on both members recorded: it links `printable.css` and
//!   fires `window.print()` on load. `&textsize=0&chapter={n}` is the reading
//!   view. This adapter reads the reading view; the prose is identical, and the
//!   reason is in `Efiction::chapter_url`.
//! * `div#chapterlist` is one member's skin, not the family's. On
//!   `tgstorytime.com` each chapter is a `div#chapterlist` — nineteen of them on
//!   a nineteen-chapter work, all sharing one id, which is invalid HTML that
//!   every browser accepts. `giantessworld.net` emits no `#chapterlist` at all
//!   and wraps its entries in `<p>` elements instead. The chapter list is
//!   therefore read from the thing both members do share — anchors carrying
//!   `chapter=<number>` — rather than from a container.
//! * Every chapter's stable key is the `chapid` parameter on its **review**
//!   link (`reviews.php?type=ST&item=6369&chapid=32368`), which both members
//!   render for every chapter and which is not the ordinal.
//! * The author's notes live in `div.notes` / `div.noteinfo` on both members
//!   and on every page kind, and are **not** captured into the chapter body.
//!   See "What this adapter does not do".
//! * Text is Windows-1252 wearing an `ISO-8859-1` label. That is not this
//!   adapter's problem — `crate::safety::decode_body` handles it — but it is why
//!   the fixtures must be read through that decoder and not as UTF-8.
//!
//! # The three ways a page is not a story
//!
//! An eFiction archive answers a request for a work it will not show with HTTP
//! **200** and a page of the site's ordinary furniture. There is no status code
//! to read, so the adapter classifies the document instead, and the outcomes are
//! genuinely different:
//!
//! * **A moderation hold.** `Access denied. This story has not been validated by
//!   the administrators of this site.` The work exists; the source will not
//!   serve it. Reported as [`SourceError::Withheld`], because no retry resolves
//!   it and calling it `NotFound` would send an operator looking for a typo.
//! * **A content gate.** The work is rated adult and the member wants an
//!   acknowledgement before showing it. Reported as
//!   [`SourceError::AuthRequired`], which pauses the job and asks the reader,
//!   and which a credential with [`Credentials::adult_allowed`] satisfies — the
//!   gate is a link on the page and following it is what a browser does.
//! * **Nothing at all.** Reported as [`SourceError::NotFound`].
//!
//! A Cloudflare interstitial is a fourth outcome and is
//! [`SourceError::Blocked`]; one member of this family is known to sit behind
//! one.
//!
//! # The trap in reading a content gate
//!
//! Two of the three recorded gates carry the archive's own statistics —
//! `Members:`, `Series:`, `Stories:`, `Chapters:`, `Word count:`, `Reviewers:` —
//! and `Chapters:` and `Word count:` are *also* the names of story metadata
//! fields. A parser that reads label spans generically reports the whole
//! archive's chapter count and word count as the work's, and nothing about the
//! result looks wrong: a 47-million-word work is merely implausible, not
//! obviously impossible. The defence is structural rather than a name list: the
//! archive statistics sit in `div#infoblock`, the story metadata sits in a
//! `div.content` that also carries `Completed:`, and this adapter reads only the
//! latter. `tests/efiction_fixtures.rs` asserts it on all three gates.
//!
//! # What this adapter does not do
//!
//! * **Author's notes are parsed and dropped.** `div.notes`/`div.noteinfo` is
//!   read, and the text is not put into the chapter body. A chapter body here is
//!   the chapter's prose, and the notes are a separate editorial block: folding
//!   them in would mean inventing markup to delimit them inside prose the author
//!   wrote. The selector is recorded so the follow-up is a change to
//!   `chapter_body` and nothing else.
//! * **Most class labels become tags.** `Categories`, `Genre`, `Characters` and
//!   each member's own story codes are carried as display text in
//!   [`SourceWork::tags`], following the AO3 adapter's precedent of carrying
//!   without mapping — Lorehaven's taxonomy arrives in Milestone 9, and choosing
//!   one here would fix a schema this crate has no business fixing. `Rated` and
//!   `Warnings` have their own fields.
//! * **No bibliography.** `bibliography` is false: a member's author page would
//!   have to be recorded before an adapter read it, and it has not been.
//! * **The chapter list is not paginated.** Most members list every chapter of a
//!   work on one page. If one does not, the stated `Chapters:` count and the
//!   number of links found disagree, and the adapter refuses loudly rather than
//!   importing the first page of a long work — the failure mode
//!   `tests/fixtures/README.md` records as the reason this crate exists.

use scraper::{ElementRef, Html, Selector};
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time};
use url::Url;

use async_trait::async_trait;

use crate::sanitize::sanitize_fragment;
use crate::{
    collapse_whitespace, strip_tags, ChapterRef, Credentials, Fetcher, SourceAdapter,
    SourceCapabilities, SourceChapter, SourceError, SourceKey, SourceResult, SourceWork,
    WorkStatus,
};

/// The key every archive in this family is stored under.
///
/// One key for nineteen hosts is a deliberate trade. The alternative — a key per
/// member — would make each archive its own line in the catalogue, which is more
/// accurate and less useful: a reader who pastes a URL does not know or care
/// that `ninelivesarchive.com` and `ncisfiction.com` are two installs of one
/// script, and the operator's per-source settings (pacing, health, credentials)
/// are identical for all of them.
///
/// The cost is on the security side and is worth naming: the fetcher's allow-list
/// for this source is every host in the `MEMBERS` table, so an import from one member
/// may reach any of them. That is inherent in treating the family as one source,
/// and it is bounded by the list being a compile-time constant rather than
/// anything a page can influence.
pub const SOURCE_KEY: &str = "efiction";

/// One archive running eFiction.
///
/// `archive_path` is empty for the members installed at the host root, and the
/// subdirectory for the four that are not.
struct Member {
    /// The host, with no `www.` — the matcher trims that prefix from both sides.
    host: &'static str,
    /// The directory the archive is installed in, or `""` at the host root.
    archive_path: &'static str,
}

/// Every archive this adapter serves.
///
/// Derived from the ported adapter's table, which listed nineteen entries and
/// eighteen distinct hosts (two archives share `sinfuldreams.com`). The three
/// `www.`-prefixed entries there are stored without the prefix, because the
/// matcher trims it and a table carrying both spellings would be two entries for
/// one host.
///
/// Five of these — `FanFiction.net`'s neighbours in the family — answer 403 to a
/// plain request; see the milestone notes. They are listed because the list is
/// what the adapter claims, not what currently answers, and a member that
/// starts answering should not need an adapter change.
const MEMBERS: &[Member] = &[
    Member {
        host: "dark-solace.org",
        archive_path: "/elysian",
    },
    Member {
        host: "giantessworld.net",
        archive_path: "",
    },
    Member {
        host: "gluttonyfiction.com",
        archive_path: "",
    },
    Member {
        host: "libraryofmoria.com",
        archive_path: "",
    },
    Member {
        host: "mttjustonce.net",
        archive_path: "",
    },
    Member {
        host: "mugglenetfanfiction.com",
        archive_path: "",
    },
    Member {
        host: "naiceanilme.net",
        archive_path: "",
    },
    Member {
        host: "narutofic.org",
        archive_path: "",
    },
    Member {
        host: "ncisfiction.com",
        archive_path: "",
    },
    Member {
        host: "ninelivesarchive.com",
        archive_path: "",
    },
    Member {
        host: "sinfuldreams.com",
        archive_path: "/unicornfic",
    },
    Member {
        host: "sinfuldreams.com",
        archive_path: "/wickedtemptation",
    },
    Member {
        host: "spikeluver.com",
        archive_path: "",
    },
    Member {
        host: "starslibrary.net",
        archive_path: "",
    },
    Member {
        host: "sunnydaleafterdark.com",
        archive_path: "",
    },
    Member {
        host: "tgstorytime.com",
        archive_path: "",
    },
    Member {
        host: "thedelphicexpanse.com",
        archive_path: "",
    },
    Member {
        host: "thehookupzone.net",
        archive_path: "",
    },
    Member {
        host: "valentchamber.com",
        archive_path: "",
    },
];

/// The script that renders a story, on every member.
const VIEW_SCRIPT: &str = "viewstory.php";

/// eFiction's story-view script, for every archive in the family.
#[derive(Debug, Clone)]
pub struct Efiction {
    key: SourceKey,
}

impl Default for Efiction {
    fn default() -> Self {
        Self::new()
    }
}

impl Efiction {
    /// Build the adapter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: SourceKey::new(SOURCE_KEY),
        }
    }

    /// The member a URL belongs to, if any.
    fn member_for(url: &Url) -> Option<&'static Member> {
        let host = url.host_str()?.trim_start_matches("www.");
        let path = url.path();
        MEMBERS.iter().find(|member| {
            member.host.eq_ignore_ascii_case(host) && path.starts_with(member.archive_path)
        })
    }

    /// Whether a URL is a story the member serves.
    fn is_story_url(url: &Url) -> bool {
        Self::member_for(url).is_some() && url.path().ends_with(VIEW_SCRIPT)
    }

    /// The work's `sid`, which is the source's own identifier for it.
    fn sid_of(url: &Url) -> Option<String> {
        url.query_pairs()
            .find(|(name, _)| name == "sid")
            .map(|(_, value)| value.into_owned())
            .filter(|sid| !sid.is_empty() && sid.chars().all(|c| c.is_ascii_digit()))
    }

    /// The canonical page for a work: its first page of chapters.
    ///
    /// Normalising here rather than at each call site is what lets a reader
    /// paste a chapter URL, or the printable view of a chapter, and be shown the
    /// work.
    fn work_url(url: &Url) -> Option<Url> {
        let member = Self::member_for(url)?;
        let sid = Self::sid_of(url)?;
        Url::parse(&format!(
            "{}://{}{}/{VIEW_SCRIPT}?sid={sid}&index=1",
            url.scheme(),
            url.host_str()?,
            member.archive_path
        ))
        .ok()
    }

    /// A chapter's own page.
    ///
    /// # Which of the two views this is, and why
    ///
    /// eFiction renders one chapter at two URLs, and they are not two skins of
    /// one page — they are the reading view and the print view:
    ///
    /// * `&textsize=0&chapter={n}` is the **reading view**. It links the member's
    ///   own `style.css`, it runs no script on load, and the prose is in
    ///   `div#story`.
    /// * `&chapter={n}` is the **print view**. It links `printable.css` and fires
    ///   `window.print()` on load — which one of the recorded members does with a
    ///   bare `if (window.print)`, so it is not a fallback path.
    ///
    /// This adapter asks for the reading view. The print URL is the one a table
    /// of contents happens to link to, so it is a *valid* URL and was the first
    /// choice here — but a print view is a request to print, and at least one
    /// member in this family says so in its own `robots.txt`, disallowing
    /// `viewstory.php?action=printable&*` by name while leaving the reading view
    /// alone. Importing a library is reading, so it reads the reading view.
    ///
    /// The prose is byte-identical between the two, which the fixtures assert, so
    /// this is a decision about what is being asked of the member rather than
    /// about what can be parsed.
    fn chapter_url(member: &Member, host: &str, scheme: &str, sid: &str, ordinal: u32) -> String {
        format!(
            "{scheme}://{host}{}/{VIEW_SCRIPT}?sid={sid}&textsize=0&chapter={ordinal}",
            member.archive_path
        )
    }

    /// Read a work page.
    fn parse_work(&self, html: &str, url: &Url) -> SourceResult<SourceWork> {
        let document = Html::parse_document(html);
        let member = Self::member_for(url).ok_or_else(|| {
            SourceError::Unsupported(format!("{url} is not an eFiction archive's URL"))
        })?;
        let sid = Self::sid_of(url).ok_or_else(|| {
            SourceError::Parse(format!("{url} carries no sid to identify the work"))
        })?;

        match classify(&document) {
            Page::Work => {}
            other => return Err(other.into_error(url)),
        }

        let block = story_block(&document).ok_or_else(|| {
            SourceError::Parse(
                "the page has no story metadata block, so there is nothing to import".into(),
            )
        })?;
        let labels = merged_labels(&document, &label_values(block));

        let (title, author_text, author_url) =
            title_and_author(&document, url).ok_or_else(|| {
                SourceError::Parse("the page has no title header, so it is not a story page".into())
            })?;

        let summary = summary_of(&document, &labels);
        let chapters = chapter_refs(&document, &labels)?;

        let key = format!("{}{}/{sid}", member.host, member.archive_path);
        let source_url = Self::work_url(url)
            .map(|url| url.to_string())
            .unwrap_or_else(|| url.to_string());

        Ok(SourceWork {
            source_key: self.key.clone(),
            source_work_key: key,
            source_url,
            title,
            author_text,
            author_url,
            summary,
            word_count: label_number(&labels, "Word count"),
            language: None,
            status: label_value(&labels, "Completed")
                .map(|value| work_status(&value))
                .unwrap_or(WorkStatus::Unknown),
            published_at: label_value(&labels, "Published").and_then(|value| parse_date(&value)),
            updated_at: label_value(&labels, "Updated").and_then(|value| parse_date(&value)),
            chapters,
            rating_text: label_value(&labels, "Rated").or_else(|| label_value(&labels, "Rating")),
            warning_texts: warnings_of(&labels),
            tags: tags_of(&labels),
        })
    }

    /// Read one chapter out of a chapter page.
    ///
    /// `ordinal` is passed when the caller already knows it — a chapter fetched
    /// by ordinal does — and is otherwise recovered from the page's own heading,
    /// which is what lets the fixture test assert a chapter's position.
    fn parse_chapter(
        &self,
        html: &str,
        url: &Url,
        work: &SourceWork,
        ordinal: Option<u32>,
    ) -> SourceResult<SourceChapter> {
        let document = Html::parse_document(html);
        match classify(&document) {
            Page::Work | Page::Chapter => {}
            other => return Err(other.into_error(url)),
        }

        let body = chapter_body(&document).ok_or_else(|| {
            SourceError::Parse(format!(
                "the chapter page at {url} has no chapter body, so the chapter cannot be read"
            ))
        })?;

        // `div.chaptertitle` reads `TITLE by AUTHOR`. The suffix is the work's
        // author, and stripping it is how the chapter's own title comes back.
        let heading = text_of_selector(&document, "div.chaptertitle");
        let title = heading
            .as_deref()
            .map(|heading| {
                let heading = heading.trim();
                match heading.rsplit_once(" by ") {
                    Some((title, _author)) => title.trim().to_owned(),
                    None => heading.to_owned(),
                }
            })
            .unwrap_or_default();

        // The ordinal comes from the heading when the caller did not supply one:
        // the chapter list is the work's own order, so matching on the title is
        // a lookup rather than a guess. A heading that matches nothing keeps the
        // ordinal it was given, or zero, and the caller sees an empty key.
        let reference = ordinal
            .and_then(|ordinal| work.chapters.iter().find(|c| c.ordinal == ordinal))
            .or_else(|| {
                work.chapters
                    .iter()
                    .find(|candidate| candidate.title == title)
            });

        let Some(reference) = reference else {
            // Two different faults, and they send a reader to different places:
            // a page that does not name its own chapter cannot be placed at all
            // without being told, and a page that names a chapter the work does
            // not list means the two have drifted apart.
            return Err(SourceError::Parse(if title.is_empty() {
                format!(
                    "the page at {url} does not name which chapter it is, so it can only \
                     be read when the caller knows the ordinal"
                )
            } else {
                format!(
                    "the chapter at {url} is not in the work's chapter list ({} chapters)",
                    work.chapters.len()
                )
            }));
        };

        Ok(SourceChapter {
            ordinal: reference.ordinal,
            source_chapter_key: reference.source_chapter_key.clone(),
            title: if title.is_empty() {
                reference.title.clone()
            } else {
                title
            },
            content_html: sanitize_fragment(&body, Some(url)),
            image_urls: crate::sanitize::extract_image_urls(&body, Some(url)),
        })
    }
}

#[async_trait]
impl SourceAdapter for Efiction {
    fn key(&self) -> SourceKey {
        self.key.clone()
    }

    fn display_name(&self) -> &'static str {
        "eFiction archives"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            metadata: true,
            chapters: true,
            // `&chapter=n` addresses one chapter, so a failed chapter is retried
            // without re-reading the rest of the work.
            per_chapter_fetch: true,
            bibliography: false,
            // No revision marker is published per work. `Updated:` is a displayed
            // date, and a date is not a validator: two revisions can share one.
            incremental: false,
            // No account is needed to read public works. The content gate is a
            // cookie-less acknowledgement rather than a login, and it is
            // satisfied by `Credentials::adult_allowed` rather than by a
            // username.
            authentication: crate::AuthKind::None,
            min_interval_millis: None,
        }
    }

    fn can_handle(&self, url: &Url) -> bool {
        Self::is_story_url(url)
    }

    fn hosts(&self) -> Vec<String> {
        let mut hosts: Vec<String> = MEMBERS
            .iter()
            .map(|member| member.host.to_owned())
            .collect();
        hosts.sort();
        hosts.dedup();
        hosts
    }

    async fn preview(
        &self,
        fetch: &dyn Fetcher,
        url: &Url,
        creds: Option<&Credentials>,
    ) -> SourceResult<SourceWork> {
        let target = Self::work_url(url)
            .ok_or_else(|| {
                SourceError::Unsupported(format!("{url} is not an eFiction archive's URL"))
            })?
            .to_string();

        let page = fetch.get(&target).await?;
        let fetched_url = Url::parse(&page.final_url).unwrap_or_else(|_| url.clone());

        // A content gate is answered by following the page's own acknowledgement
        // link, once, and only for a credential that says the instance has
        // already consented to adult material (spec §11.6). Without that the
        // gate is reported, which pauses the job and asks the reader — the
        // behaviour the spec requires, rather than an adapter that decides on
        // the operator's behalf.
        // Classified into an owned value before the branch, because a `scraper`
        // document is not `Send` and holding one across an `await` makes this
        // future unusable by any executor that moves it between threads.
        let classified = classify(&Html::parse_document(&page.body));
        match classified {
            Page::ContentGate { ack } if creds.is_some_and(|c| c.adult_allowed) => {
                let ack = resolve(&fetched_url, &ack).ok_or_else(|| {
                    SourceError::Parse("the content gate's acknowledgement link is unusable".into())
                })?;
                let acknowledged = fetch.get(ack.as_str()).await?;
                self.parse_work(&acknowledged.body, &fetched_url)
            }
            _ => self.parse_work(&page.body, &fetched_url),
        }
    }

    async fn fetch_chapters(
        &self,
        fetch: &dyn Fetcher,
        work: &SourceWork,
        creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>> {
        // The work page is re-read first, so the chapter list and its keys are
        // the member's current ones rather than whatever a preview saw some time
        // ago. A re-read that fails falls back to the stored list rather than
        // failing the import: the chapter URLs do not depend on it, and a work
        // whose listing page is briefly broken should not lose every chapter.
        let work_url = Url::parse(&work.source_url)
            .map_err(|e| SourceError::Parse(format!("the stored work URL is unusable: {e}")))?;
        let refreshed = self
            .preview(fetch, &work_url, creds)
            .await
            .unwrap_or_else(|_| work.clone());

        let member = self.member(work)?;
        let host = host_of(&refreshed.source_url)
            .ok_or_else(|| SourceError::Parse("the work URL has no host".into()))?;
        let scheme = scheme_of(&refreshed.source_url).unwrap_or_else(|| "https".into());
        let sid = refreshed
            .source_work_key
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned();

        let mut chapters = Vec::with_capacity(refreshed.chapters.len());
        for reference in &refreshed.chapters {
            let target = Self::chapter_url(member, &host, &scheme, &sid, reference.ordinal);
            let page = fetch.get(&target).await?;
            let url = Url::parse(&page.final_url).unwrap_or_else(|_| {
                Url::parse(&target).expect("a chapter URL this adapter built must parse")
            });
            chapters.push(self.parse_chapter(
                &page.body,
                &url,
                &refreshed,
                Some(reference.ordinal),
            )?);
        }
        Ok(chapters)
    }

    async fn fetch_chapter(
        &self,
        fetch: &dyn Fetcher,
        work: &SourceWork,
        ordinal: u32,
        _creds: Option<&Credentials>,
    ) -> SourceResult<SourceChapter> {
        let reference = work
            .chapters
            .iter()
            .find(|candidate| candidate.ordinal == ordinal)
            .ok_or(SourceError::NotFound)?;

        let member = self.member(work)?;
        let host = host_of(&work.source_url)
            .ok_or_else(|| SourceError::Parse("the work URL has no host".into()))?;
        let scheme = scheme_of(&work.source_url).unwrap_or_else(|| "https".into());
        let sid = work.source_work_key.rsplit('/').next().unwrap_or_default();

        let target = Self::chapter_url(member, &host, &scheme, sid, ordinal);
        let page = fetch.get(&target).await?;
        let url = Url::parse(&page.final_url).unwrap_or_else(|_| {
            Url::parse(&target).expect("a chapter URL this adapter built must parse")
        });

        let mut chapter = self.parse_chapter(&page.body, &url, work, Some(ordinal))?;
        chapter.source_chapter_key = reference.source_chapter_key.clone();
        Ok(chapter)
    }

    fn preview_from_html(&self, html: &str, url: &Url) -> SourceResult<SourceWork> {
        self.parse_work(html, url)
    }

    fn chapters_from_html(
        &self,
        html: &str,
        work: &SourceWork,
    ) -> SourceResult<Vec<SourceChapter>> {
        // A chapter page holds one chapter, so this returns a list of one. The
        // ordinal is recovered by matching the page's own heading against the
        // work's chapter list, which is what lets the fixture test assert a
        // chapter's position without a network.
        let url = Url::parse(&work.source_url)
            .map_err(|e| SourceError::Parse(format!("the work URL is unusable: {e}")))?;
        self.parse_chapter(html, &url, work, None)
            .map(|chapter| vec![chapter])
    }
}

impl Efiction {
    /// The member a stored work belongs to.
    fn member(&self, work: &SourceWork) -> SourceResult<&'static Member> {
        let url = Url::parse(&work.source_url)
            .map_err(|e| SourceError::Parse(format!("the work URL is unusable: {e}")))?;
        Self::member_for(&url).ok_or_else(|| {
            SourceError::Unsupported(format!("{} is not an eFiction archive", work.source_url))
        })
    }
}

/// How a fetched document classified.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Page {
    /// A work page: the metadata block is present and the chapter list is readable.
    Work,
    /// A chapter page: the chapter body is present.
    Chapter,
    /// The member is asking for an acknowledgement before showing an adult work.
    ContentGate {
        /// The page's own acknowledgement link, as it appears in the markup.
        ack: String,
    },
    /// The member holds the work and will not serve it.
    Withheld,
    /// Cloudflare, or something shaped like it, answered instead of the member.
    Blocked,
    /// The member serves nothing at that `sid`.
    Missing,
}

impl Page {
    /// The error this classification becomes.
    fn into_error(self, url: &Url) -> SourceError {
        match self {
            // Not reachable through a caller that classified first; kept total
            // rather than panicking, because a preview is not a place to panic.
            Self::Work | Self::Chapter => SourceError::Parse(format!(
                "{url} was classified as readable and then read as unreadable"
            )),
            Self::ContentGate { .. } => SourceError::AuthRequired(format!(
                "{url} is gated: this member asks for an acknowledgement before \
                 showing the work. Granting the import a credential with adult \
                 content allowed satisfies the gate."
            )),
            Self::Withheld => SourceError::Withheld(format!(
                "{url} was refused by the member: this story has not been validated \
                 by its administrators, and no retry changes that"
            )),
            Self::Blocked => SourceError::Blocked,
            Self::Missing => SourceError::NotFound,
        }
    }
}

/// Classify a document without knowing which URL it came from.
///
/// Ordered, and the order matters: Cloudflare is checked first because its page
/// contains none of the markers below and would otherwise land in `Missing`,
/// reporting a challenge wall as a work that does not exist.
fn classify(document: &Html) -> Page {
    if is_challenge(document) {
        return Page::Blocked;
    }
    if story_block(document).is_some() {
        return Page::Work;
    }
    if chapter_body(document).is_some() {
        return Page::Chapter;
    }
    if let Some(ack) = acknowledgement_link(document) {
        return Page::ContentGate { ack };
    }
    if says_access_denied(document) {
        return Page::Withheld;
    }
    Page::Missing
}

/// Whether the document is a bot challenge rather than the member's own page.
///
/// Matched on the interstitial's own markers — `cf_chl` appears in its script
/// and frame ids, `challenge-platform` in its script source — plus the title,
/// which is the one string that survives Cloudflare re-skinning the page.
/// Deliberately not matched on the absence of content: identification by
/// absence is what puts a new gate into `Missing`.
fn is_challenge(document: &Html) -> bool {
    let title = text_of_selector(document, "title").unwrap_or_default();
    if title.to_ascii_lowercase().contains("just a moment") {
        return true;
    }
    let html = document.html();
    html.contains("cf_chl") || html.contains("challenge-platform")
}

/// The work's metadata block, or `None` when the page has none.
///
/// The rule is structural and it exists to dodge a specific trap: an archive's
/// *statistics* block also carries `Chapters:` and `Word count:`, so a reader
/// that accepts any block with those labels reports the whole archive's totals
/// as one work's. The difference is `Completed:`, which only story metadata has,
/// and the container, which is `div.content` rather than `div#infoblock`.
///
/// `Chapters:` together with `Published:` is accepted as a fallback for a member
/// that omits `Completed:` on a one-shot, since neither appears in a statistics
/// block.
fn story_block(document: &Html) -> Option<ElementRef<'_>> {
    let selector = Selector::parse("div.content").ok()?;
    document.select(&selector).find(|block| {
        if !labels_are_flat(*block) {
            return false;
        }
        let names = label_names(*block);
        names.iter().any(|name| name == "completed")
            || (names.iter().any(|name| name == "chapters")
                && names.iter().any(|name| name == "published"))
    })
}

/// Whether the page says the work is held back.
///
/// Matched on the phrase and not on the sentence: the two members that produce
/// it spell their own message differently — `giantessworld.net` writes
/// "adminstrators" — and a parser that matched the full sentence would read one
/// member's moderation hold as a missing work.
fn says_access_denied(document: &Html) -> bool {
    let text = document.root_element().text().collect::<String>();
    let text = text.to_ascii_lowercase();
    text.contains("access denied")
}

/// The acknowledgement link a content gate offers, if the page is one.
fn acknowledgement_link(document: &Html) -> Option<String> {
    let selector = Selector::parse("a[href]").ok()?;
    document
        .select(&selector)
        .filter_map(|anchor| anchor.value().attr("href"))
        .find(|href| href.contains("ageconsent") || href.contains("warning="))
        .map(str::to_owned)
}

/// The title and author from the page's heading.
///
/// The header is `<a>TITLE</a> by <a href="viewuser.php?uid=N">AUTHOR</a>`, and
/// finding it by the author link rather than by taking the first `#pagetitle` on
/// the page is what makes this correct on a member whose action box is emitted
/// first. `tgstorytime.com` puts a second `#pagetitle` on its work pages holding
/// only a "Report" link; an adapter that took the first `#pagetitle` would be
/// reading whichever of the two the skin happened to emit first.
fn title_and_author(document: &Html, page: &Url) -> Option<(String, String, Option<String>)> {
    let headers = Selector::parse("#pagetitle").ok()?;
    let anchors = Selector::parse("a[href]").ok()?;

    for header in document.select(&headers) {
        let links: Vec<ElementRef<'_>> = header.select(&anchors).collect();
        let Some(author_link) = links
            .iter()
            .find(|anchor| {
                anchor
                    .value()
                    .attr("href")
                    .is_some_and(|href| href.contains("viewuser.php"))
            })
            .copied()
        else {
            continue;
        };

        let title = links
            .first()
            .map(|anchor| collapse_whitespace(&anchor.text().collect::<String>()))
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }

        let author = collapse_whitespace(&author_link.text().collect::<String>());
        let author_url = author_link
            .value()
            .attr("href")
            .and_then(|href| page.join(href).ok())
            .map(|url| url.to_string());

        return Some((title, author, author_url));
    }
    None
}

/// One label from a metadata block, with the value that followed it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Label {
    /// The label with its trailing colon removed and lowercased (`word count`).
    name: String,
    /// The label as the source wrote it, without the colon (`Word count`).
    display: String,
    /// The value that followed, as plain text with the chrome stripped out.
    value: String,
    /// The value's items, when the member rendered them as links.
    ///
    /// An eFiction archive renders each class value as a link to its own browse
    /// page — one link per value — which makes the markup *more* precise than
    /// the text: `Characters: <a>Male to Female, Young Adult (20-26 yrs)</a>` is
    /// one value with a comma in its name, and
    /// `Categories: <a>Breasts</a> <a>Fantasy</a>` is two values with no
    /// separator between them. Splitting the text on commas gets both wrong, so
    /// where the member has already told us where the values are, that is what
    /// is used.
    items: Vec<String>,
}

/// Read every label and value out of a metadata block.
///
/// The block is a run of `<span class="label">Name:</span> value` pairs, so the
/// walk is over the block's own children with the current label carried forward
/// — the same shape the ported adapter used, which is one of the few things it
/// got right, and the reason this is not a regex over raw HTML: a value contains
/// markup, and a value's end is "the next label" rather than "the next tag".
///
/// The walk is flat, over direct children, because that is what every member
/// recorded writes. [`labels_are_flat`] checks it rather than assuming it, and a
/// block whose labels are nested is refused rather than half-read: descending
/// into a nested label would end the value in one place and start the next in
/// another, and the result would be a page of mislabelled metadata that no
/// assertion could distinguish from a correct read.
fn label_values(block: ElementRef<'_>) -> Vec<Label> {
    let mut labels: Vec<Label> = Vec::new();
    let mut current: Option<Label> = None;

    for node in block.children() {
        let element = ElementRef::wrap(node);
        match element {
            Some(element) if is_label_span(element) => {
                if let Some(label) = current.take() {
                    labels.push(label);
                }
                current = Some(label_from(element));
            }
            Some(element) => {
                if let Some(label) = current.as_mut() {
                    collect_element(element, label);
                }
            }
            None => {
                // A comment or a doctype is not text either; only `Node::Text`
                // carries a value a reader could see.
                if let (Some(label), scraper::Node::Text(text)) = (current.as_mut(), node.value()) {
                    push_text(text, label);
                }
            }
        }
    }
    if let Some(label) = current {
        labels.push(label);
    }
    labels
}

/// Whether every label in a block sits directly inside it.
///
/// The precondition of the flat walk above, checked rather than assumed. A skin
/// that wrapped each `label`/`value` pair in a `<div>` would put the labels one
/// level down, and this is how that is noticed instead of misread.
fn labels_are_flat(block: ElementRef<'_>) -> bool {
    let Ok(selector) = Selector::parse("span.label") else {
        return false;
    };
    block.select(&selector).all(|span| {
        span.parent()
            .is_some_and(|parent| parent.id() == block.id())
    })
}

/// Whether an element is a `<span class="label">`.
fn is_label_span(element: ElementRef<'_>) -> bool {
    element.value().name() == "span"
        && element
            .value()
            .attr("class")
            .is_some_and(|class| class.split_whitespace().any(|name| name == "label"))
}

/// A fresh label from its span, with the trailing colon removed.
fn label_from(span: ElementRef<'_>) -> Label {
    let display = collapse_whitespace(&span.text().collect::<String>());
    let display = display.trim_end_matches(':').trim().to_owned();
    Label {
        name: display.to_ascii_lowercase(),
        display,
        value: String::new(),
        items: Vec::new(),
    }
}

/// Append an element's text and items to the label being read.
///
/// # What counts as part of a value, and what is chrome
///
/// A member renders a class value as a link to its own browse page
/// (`browse.php?type=class&type_id=100&classid=173`), and everything else a
/// block contains is furniture: a monthly rating's stars, a review count, a
/// `Download ePub` link, a `Report` link. Both kinds are anchors and neither is
/// marked, so the distinction has to be structural, and the browse script is
/// what it is: an anchor whose href is a browse link contributes its text as a
/// value, and an anchor whose href is anything else contributes nothing at all.
///
/// The difference is not cosmetic. `tgstorytime.com` writes
/// `Rated: Adult <a href="modules/epubversion/…">Download ePub</a>`, so a walk
/// that harvested every anchor's text reported the work's rating as
/// `Adult Download ePub` — a rating that is not a rating, produced by a rule
/// that looked reasonable.
fn collect_element(element: ElementRef<'_>, label: &mut Label) {
    // An anchor reached as a descendant is handled here rather than by `select`,
    // which searches descendants only and would miss the element itself.
    if element.value().name() == "a" {
        if let Some(href) = element.value().attr("href") {
            if is_classification_link(href) {
                let text = collapse_whitespace(&element.text().collect::<String>());
                if !text.is_empty() {
                    label.items.push(text);
                }
            }
        }
        return;
    }

    for child in element.children() {
        match ElementRef::wrap(child) {
            Some(inner) => collect_element(inner, label),
            None => {
                if let scraper::Node::Text(text) = child.value() {
                    push_text(text, label);
                }
            }
        }
    }
}

/// Whether an href is the archive's own link for a class value.
///
/// `browse.php` is the script that lists works by category, character, warning
/// or story code, and it is the script every member points a class value at.
/// Matching the script rather than the label it appears under is what keeps a
/// value and the furniture around it apart without a list of label names.
fn is_classification_link(href: &str) -> bool {
    href.contains("browse.php")
}

/// Append a run of text to a label's value.
fn push_text(text: &str, label: &mut Label) {
    if text.trim().is_empty() {
        return;
    }
    if !label.value.is_empty() {
        label.value.push(' ');
    }
    label.value.push_str(text.trim());
}

/// The label spans of a block, as display text.
fn label_names(block: ElementRef<'_>) -> Vec<String> {
    block
        .select(&label_selector())
        .map(|span| {
            collapse_whitespace(&span.text().collect::<String>())
                .trim_end_matches(':')
                .trim()
                .to_ascii_lowercase()
        })
        .collect()
}

/// A label's value as one string: what the page said, not a list.
///
/// Deliberately unsplit, because most labels are free text and only some are
/// lists. `Summary:` is a paragraph that may contain commas — splitting it and
/// taking the first piece truncates a work's description mid-sentence, which is
/// how a summary came back as its own opening clause. A label the member
/// rendered as links is joined back with the separator the archive writes
/// between its values.
///
/// `None` is the family's way of writing "this label has no value" —
/// `Series: None`, `Characters: None` — and is reported as absence rather than
/// as a value called `None`. Whether a value that is `None` became a tag is
/// [`split_list`]'s business, not this function's.
fn label_value(labels: &[Label], name: &str) -> Option<String> {
    let wanted = name.to_ascii_lowercase();
    let label = labels.iter().find(|label| label.name == wanted)?;
    if !label.items.is_empty() {
        let joined = label.items.join(", ");
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    let value = label.value.trim();
    (!value.is_empty() && value != "None").then(|| value.to_owned())
}

/// A label's items: the member's link texts when it has them, and its text split
/// on separators when it does not.
///
/// A single item either way, so a caller that wants one value does not have to
/// know which shape the page used.
fn label_items(label: &Label) -> Vec<String> {
    if !label.items.is_empty() {
        return label.items.clone();
    }
    split_list(&label.value)
}

/// A label's value as a number, for the fields the site writes as text.
fn label_number(labels: &[Label], name: &str) -> Option<i64> {
    let raw = label_value(labels, name)?;
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Labels that only ever describe the *archive*, never one work.
///
/// The set exists for one reason: an archive's statistics block and a work's
/// metadata block share label names — `Chapters:`, `Word count:`, `Series:` —
/// so merging a page's blocks is only safe if the names that would collide in
/// the damaging direction are excluded. A work's own `Chapters:` and
/// `Word count:` always come from its own block, which is read first and wins.
const STATISTIC_LABELS: &[&str] = &[
    "authors",
    "challengers",
    "challenges",
    "members",
    "newest member",
    "reviewers",
    "reviews",
    "stories",
];

/// The story block's labels, plus any the work's own block did not carry.
///
/// # Why a page's other blocks are read at all
///
/// Because members disagree about where a work's metadata lives.
/// `tgstorytime.com` writes `Rated:` in a `div.storyinfo` above the block and
/// everything else in the block, so an adapter that read only the block would
/// report no rating for that member and a rating for the next — a difference a
/// reader would never be able to explain.
///
/// # Why this is safe
///
/// Three bounds. Only blocks whose labels sit directly inside them are read, so
/// a value can never run past the end of its own block. Only names the story
/// block did not already carry are taken, and the story block is read first. And
/// a name that means "this archive's total" is never taken from anywhere —
/// [`STATISTIC_LABELS`] — because those are exactly the names a statistics block
/// shares with a work and the ones that would import a whole archive's word
/// count onto one story.
fn merged_labels(document: &Html, primary: &[Label]) -> Vec<Label> {
    let mut merged = primary.to_vec();
    for block in label_blocks(document) {
        for label in block {
            if STATISTIC_LABELS.contains(&label.name.as_str()) {
                continue;
            }
            if merged.iter().any(|existing| existing.name == label.name) {
                continue;
            }
            merged.push(label);
        }
    }
    merged
}

/// Every flat block of labels on the page, in document order.
///
/// A block is an element that directly contains label spans — the story
/// metadata block and, on some members, a ratings or statistics block beside it.
/// Grouping by the label's own parent rather than walking siblings from each
/// label is what keeps a value inside its block: a sibling walk from a label
/// near the end of one block would run on into the next block and read the whole
/// page as that label's value.
fn label_blocks(document: &Html) -> Vec<Vec<Label>> {
    let Ok(selector) = Selector::parse("span.label") else {
        return Vec::new();
    };
    let mut seen = Vec::new();
    let mut blocks = Vec::new();
    for span in document.select(&selector) {
        // `ElementRef::parent()` is inherited from the node it derefs into and
        // yields a node, not an element: a label's parent can be the document
        // itself, which is not a block.
        let Some(parent) = span.parent().and_then(ElementRef::wrap) else {
            continue;
        };
        if seen.contains(&parent.id()) || !labels_are_flat(parent) {
            continue;
        }
        seen.push(parent.id());
        blocks.push(label_values(parent));
    }
    blocks
}

/// The work's summary.
///
/// Two shapes, because the family has two: `tgstorytime.com` puts the summary in
/// its own `div.summarytext` and repeats it, on a chapter page, as a `Summary:`
/// label inside the metadata block; `giantessworld.net` has no `div.summarytext`
/// at all and only ever writes the label. Reading the label first and the
/// element second covers both with one code path — the label is present wherever
/// the element is, which the fixtures confirm.
fn summary_of(document: &Html, labels: &[Label]) -> String {
    label_value(labels, "Summary")
        .map(|value| strip_tags(&value))
        .map(|value| collapse_whitespace(&value))
        .filter(|value| !value.is_empty())
        .or_else(|| {
            text_of_selector(document, "div.summarytext")
                .map(|text| strip_tags(&text))
                .map(|text| collapse_whitespace(&text))
                .filter(|text| !text.is_empty())
        })
        .unwrap_or_default()
}

/// The work's warnings, from whichever spelling the member uses.
fn warnings_of(labels: &[Label]) -> Vec<String> {
    for name in ["Warnings", "Warning"] {
        if let Some(label) = labels
            .iter()
            .find(|label| label.name == name.to_ascii_lowercase())
        {
            let items = label_items(label);
            if !items.is_empty() {
                return items;
            }
        }
    }
    Vec::new()
}

/// Labels that have a field of their own, or hold a count or a date rather than
/// a classification. Everything else in a story block is a tag.
const NON_TAG_LABELS: &[&str] = &[
    "chapters",
    "completed",
    "published",
    "read",
    "summary",
    "updated",
    "word count",
    "rated",
    "rating",
    "warning",
    "warnings",
];

/// The work's tags: every class label's values, as display text.
///
/// Deliberately broad. The family-standard labels are `Categories`, `Genre`,
/// `Characters` and `Pairing`, and each member adds its own story codes — on
/// `giantessworld.net` those include `Turned Into`, `Growth` and `Size Roles`,
/// which are how readers on that archive actually find things. Carrying them all
/// as display text follows the AO3 adapter, and mapping them onto Lorehaven's own
/// taxonomy is Milestone 9's work rather than a scraper's.
fn tags_of(labels: &[Label]) -> Vec<String> {
    let mut tags = Vec::new();
    for label in labels {
        if NON_TAG_LABELS.contains(&label.name.as_str()) {
            continue;
        }
        tags.extend(label_items(label));
    }
    tags.sort();
    tags.dedup();
    tags
}

/// Split a label's rendered text into its items.
///
/// Only for values the member wrote as text rather than as links. A comma is the
/// family's separator; a slash and an `&` are not, and treating them as one
/// splits the archive's own vocabulary down the middle — `Slow/Gradual Change`
/// and `FF/m` are single values on the members that write them, and
/// `Working & Single` is one category, not two.
fn split_list(value: &str) -> Vec<String> {
    value
        .split([',', ';'])
        .map(collapse_whitespace)
        .filter(|item| !item.is_empty() && item != "None")
        .collect()
}

/// The work's chapters, from the page's own chapter links.
///
/// # Why the list comes from links rather than from a container
///
/// Because there is no container both members render. `tgstorytime.com` wraps
/// each chapter in its own `div#chapterlist` — nineteen elements sharing one id
/// — and `giantessworld.net` emits no `#chapterlist` and wraps its entries in
/// `<p>` elements instead. What both share is an anchor carrying
/// `chapter=<number>`, which is also the only thing guaranteed by the script
/// rather than by a skin.
///
/// # Why the three lists are zipped
///
/// Because the entry's facts are not all in one element. The chapter's title is
/// the anchor's text; its stable key is a `chapid` on a *review* link that
/// follows it; its word count is loose text. All three appear in document order,
/// once per chapter, so they are collected in document order and paired
/// positionally — and the pairing is checked against the count the page states,
/// because a positional pairing that silently loses its place produces the
/// failure this whole crate exists to avoid: an import that looks complete.
fn chapter_refs(document: &Html, labels: &[Label]) -> SourceResult<Vec<ChapterRef>> {
    let anchors = Selector::parse("a[href]").expect("a constant selector parses");

    let mut refs: Vec<ChapterRef> = Vec::new();
    for anchor in document.select(&anchors) {
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        if !is_chapter_link(Some(href)) {
            continue;
        }
        let Some(ordinal) = split_query(href)
            .iter()
            .find(|(name, value)| {
                name == "chapter" && !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
            })
            .and_then(|(_, value)| value.parse::<u32>().ok())
        else {
            continue;
        };

        let title = collapse_whitespace(&anchor.text().collect::<String>());
        let entry = entry_facts(&anchor);
        let key = entry
            .chapid
            .clone()
            .unwrap_or_else(|| format!("chapter-{ordinal}"));

        // A duplicate is the same chapter linked twice — a jump menu above the
        // list, most often. The first occurrence wins, because it is the one in
        // the list's own order.
        if refs
            .iter()
            .any(|existing| existing.source_chapter_key == key)
        {
            continue;
        }

        refs.push(ChapterRef {
            ordinal,
            source_chapter_key: key,
            title,
        });
    }

    refs.sort_by_key(|reference| reference.ordinal);

    // The page states how many chapters the work has. When it states a number
    // and the number of links found disagrees, this adapter has not understood
    // the page, and importing what it did find would produce a truncated work
    // that looks whole. Refused loudly, naming both numbers.
    if let Some(stated) = label_number(labels, "Chapters") {
        let stated = usize::try_from(stated).unwrap_or(0);
        if stated != refs.len() {
            return Err(SourceError::Parse(format!(
                "the page states {stated} chapters and {} chapter links were read; \
                 refusing to import a partial work",
                refs.len()
            )));
        }
    }

    // An ordinal that does not start at one means the chapter list is paginated
    // and this page holds part of it, which the count check above cannot detect
    // when a member shows no count at all.
    if let Some(first) = refs.first() {
        if first.ordinal != 1 {
            return Err(SourceError::Parse(format!(
                "the chapter list starts at chapter {} rather than 1, so this page \
                 holds part of a paginated list",
                first.ordinal
            )));
        }
    }

    Ok(refs)
}

/// The facts about a chapter entry that are not the anchor's own text.
#[derive(Debug, Default, Clone)]
struct EntryFacts {
    /// The `chapid` the entry's review links carry.
    chapid: Option<String>,
}

/// Read an entry's `chapid` from the review links around a chapter anchor.
///
/// Walked from the anchor's parent through the following siblings, stopping at
/// the first sibling that contains another chapter link. That boundary is what
/// makes this the *entry* rather than the rest of the page: on
/// `tgstorytime.com` the entry is the anchor's own parent and the next entry is
/// the next sibling, while on `giantessworld.net` the anchor is inside a `<b>`
/// and the `chapid` sits in that `<b>`'s following siblings.
fn entry_facts(anchor: &ElementRef<'_>) -> EntryFacts {
    let mut facts = EntryFacts::default();
    let mut hrefs: Vec<String> = Vec::new();

    if let Some(parent) = anchor.parent().and_then(ElementRef::wrap) {
        gather_hrefs(parent, &mut hrefs);
        for sibling in parent.next_siblings() {
            if let Some(element) = ElementRef::wrap(sibling) {
                if contains_chapter_link(element) {
                    break;
                }
                gather_hrefs(element, &mut hrefs);
            }
        }
    }

    facts.chapid = hrefs
        .iter()
        .find_map(|href| query_value(href, "chapid"))
        .filter(|chapid| !chapid.is_empty());
    facts
}

/// Collect every anchor's `href` in an element, including the element itself.
///
/// `select` searches descendants, so an element that *is* an anchor contributes
/// nothing on its own — and on `giantessworld.net` the entry's `chapid` links are
/// siblings of the chapter link rather than descendants of a shared container,
/// so each of them is reached as an element in its own right. Missing them costs
/// every chapter its stable key, silently: the fallback still produces a key, it
/// is simply the ordinal, and an ordinal is exactly what the key exists not to
/// be.
fn gather_hrefs(element: ElementRef<'_>, into: &mut Vec<String>) {
    if let Some(href) = element.value().attr("href") {
        into.push(href.to_owned());
    }
    for anchor in element.select(&anchor_selector()) {
        if let Some(href) = anchor.value().attr("href") {
            into.push(href.to_owned());
        }
    }
}

/// Whether an element is, or contains, a link to a numbered chapter.
fn contains_chapter_link(element: ElementRef<'_>) -> bool {
    is_chapter_link(element.value().attr("href"))
        || element
            .select(&anchor_selector())
            .any(|anchor| is_chapter_link(anchor.value().attr("href")))
}

/// Whether an href addresses a numbered chapter.
///
/// `chapter=all` is the printable whole-work view rather than a chapter, and a
/// page that offered it would otherwise gain a phantom entry on every import.
fn is_chapter_link(href: Option<&str>) -> bool {
    let Some(href) = href else {
        return false;
    };
    split_query(href).iter().any(|(name, value)| {
        name == "chapter" && !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
    })
}

/// A query string split into pairs, without a URL parser.
///
/// The hrefs in these pages are relative and some are entity-escaped, so parsing
/// them as URLs would fail on exactly the links that matter. Only the presence
/// and shape of a parameter is needed, and that is a string operation.
fn split_query(href: &str) -> Vec<(String, String)> {
    let Some((_, query)) = href.split_once('?') else {
        return Vec::new();
    };
    let query = query.split('#').next().unwrap_or(query);
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| {
            // The HTML parser decodes `&amp;` in an attribute, so a name here is
            // normally clean. A page that double-escaped its own links would
            // arrive with the entity intact, and a `chapid` that read as
            // `amp;chapid` is a chapter key silently lost.
            let name = name.strip_prefix("amp;").unwrap_or(name);
            let value = value.split('&').next().unwrap_or(value);
            (name.to_owned(), value.to_owned())
        })
        .collect()
}

/// One parameter's value from a query string.
fn query_value(href: &str, name: &str) -> Option<String> {
    split_query(href)
        .into_iter()
        .find(|(parameter, _)| parameter == name)
        .map(|(_, value)| value)
}

/// The chapter's prose, from whichever view the page is.
///
/// `div#story` is the reading view's body and `div.chapter` is the print view's;
/// both members render one or the other, and the fixtures confirm the prose is
/// identical between them. Read in that order because the reading view is the one
/// this adapter asks for, and kept in both orders' reach because a member that
/// answered a reading-view URL with its print template would otherwise lose every
/// chapter to a parse failure.
fn chapter_body(document: &Html) -> Option<String> {
    for selector in ["div#story", "div.chapter"] {
        if let Some(html) = inner_html_of(document, selector) {
            if !strip_tags(&html).trim().is_empty() {
                return Some(html);
            }
        }
    }
    None
}

/// Parse a date in any of the shapes this family writes.
///
/// # The numeric shape is ambiguous, and this is the choice
///
/// `tgstorytime.com` writes `08/06/21` and `09/01/26`: day and month, one of
/// them under thirteen, with no way to tell which from the value. Read as
/// day-first it is 8 June 2021; month-first, 6 August 2021. Both are plausible
/// and neither is checkable against the page.
///
/// Day-first is chosen, for three reasons: eFiction's own default display format
/// is `d/m/Y`; both members that write a numeric date are British archives; and
/// the reading is disambiguated when it can be — if exactly one of the two
/// readings is in the future, a publication date cannot be, so the other is
/// taken. That check resolves the case where the two readings straddle today,
/// which is the case where being wrong is most visible.
///
/// A date in a shape not listed becomes `None` rather than a guess. A wrong date
/// on an imported work is worse than an absent one, and the storage layer keeps
/// its own `first_imported_at` for the question "when did this arrive here?".
fn parse_date(raw: &str) -> Option<OffsetDateTime> {
    let text = collapse_whitespace(raw);
    if text.is_empty() {
        return None;
    }

    if let Some((day, month, year)) = numeric_date(&text) {
        return make_date(year, month, day);
    }

    // `January 18 2022`, `January 18, 2022`, `18 January 2022`, `18 Jan 2022`.
    // The three tokens are the same three either way; what tells the shapes
    // apart is which end the number is on, so the check is on the first token
    // rather than on two match arms that bind the same names.
    let cleaned = text.replace(',', " ");
    let parts: Vec<&str> = cleaned.split_whitespace().collect();
    let [first, second, third] = parts.as_slice() else {
        return None;
    };
    let year: i32 = third.parse().ok()?;
    let (day, month) = if first.starts_with(|c: char| c.is_ascii_digit()) {
        (first.parse().ok()?, month_number(second)?)
    } else {
        (second.parse().ok()?, month_number(first)?)
    };
    make_date(year, month, day)
}

/// The day, month and year of an `A/B/C` date, day-first.
///
/// Returns both readings' components so the caller can prefer the one that is
/// not in the future; `None` when the text is not three numbers separated by
/// slashes.
fn numeric_date(text: &str) -> Option<(u8, u8, i32)> {
    let parts: Vec<&str> = text.split('/').collect();
    let [first, second, year] = parts.as_slice() else {
        return None;
    };
    let first: u8 = first.trim().parse().ok()?;
    let second: u8 = second.trim().parse().ok()?;
    let year: i32 = year.trim().parse().ok()?;
    let year = if year < 100 { 2000 + year } else { year };

    // Day-first, unless that reading is in the future and the other is not.
    let day_first = make_date(year, second, first);
    if let Some(candidate) = day_first {
        if candidate > OffsetDateTime::now_utc() {
            if let Some(other) = make_date(year, first, second) {
                if other <= OffsetDateTime::now_utc() {
                    return Some((first, second, year));
                }
            }
        }
    }
    Some((first, second, year))
}

/// Assemble a date, at midnight.
fn make_date(year: i32, month: u8, day: u8) -> Option<OffsetDateTime> {
    let date = Date::from_calendar_date(year, Month::try_from(month).ok()?, day).ok()?;
    Some(PrimitiveDateTime::new(date, Time::MIDNIGHT).assume_utc())
}

/// The month a name or abbreviation refers to.
fn month_number(name: &str) -> Option<u8> {
    let name = name.trim().to_ascii_lowercase();
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    // An exact name, or an abbreviation of at least three characters — which is
    // every abbreviation the family writes (`Jan`, `Sept`) and short enough that
    // `Ja` or `Ma` cannot collide with two months at once.
    MONTHS
        .iter()
        .position(|month| *month == name || (name.len() >= 3 && month.starts_with(&name)))
        .map(|index| u8::try_from(index + 1).unwrap_or(1))
}

/// What a member's `Completed:` value means.
///
/// The recorded vocabulary is `Yes` on `giantessworld.net` and
/// `Story Incomplete` on `tgstorytime.com`, and neither contains the other —
/// which is what makes this a lookup rather than a `contains("yes")`.
fn work_status(value: &str) -> WorkStatus {
    let value = value.trim().to_ascii_lowercase();
    if value.contains("incomplete") || value.contains("no") {
        WorkStatus::Ongoing
    } else if value.contains("hiatus") || value.contains("paused") {
        WorkStatus::Hiatus
    } else if value.contains("abandon") || value.contains("cancel") || value.contains("unfinish") {
        WorkStatus::Cancelled
    } else if value.contains("yes") || value.contains("complete") || value.contains("finished") {
        WorkStatus::Complete
    } else {
        WorkStatus::Unknown
    }
}

/// Resolve an href against a page's own URL.
fn resolve(base: &Url, href: &str) -> Option<Url> {
    base.join(href).ok()
}

/// A URL's host.
fn host_of(url: &str) -> Option<String> {
    Url::parse(url).ok()?.host_str().map(str::to_owned)
}

/// A URL's scheme.
fn scheme_of(url: &str) -> Option<String> {
    Some(Url::parse(url).ok()?.scheme().to_owned())
}

/// The text of the first element matching a selector.
fn text_of_selector(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .next()
        .map(|element| collapse_whitespace(&element.text().collect::<String>()))
        .filter(|text| !text.is_empty())
}

/// The inner HTML of the first element matching a selector.
fn inner_html_of(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document.select(&selector).next().map(|e| e.inner_html())
}

/// The `span.label` selector, built once per call rather than stored.
fn label_selector() -> Selector {
    Selector::parse("span.label").expect("a constant selector parses")
}

/// The `a[href]` selector.
fn anchor_selector() -> Selector {
    Selector::parse("a[href]").expect("a constant selector parses")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_member_host_is_claimed_and_the_table_has_no_duplicate_pair() {
        let adapter = Efiction::new();
        let hosts = adapter.hosts();
        assert_eq!(hosts.len(), 18, "eighteen distinct hosts: {hosts:?}");
        for member in MEMBERS {
            assert!(
                hosts.contains(&member.host.to_owned()),
                "{} must be in the allow-list",
                member.host
            );
        }
    }

    #[test]
    fn a_story_url_on_every_member_is_claimed() {
        let adapter = Efiction::new();
        for member in MEMBERS {
            let raw = format!(
                "https://{}{}/viewstory.php?sid=1",
                member.host, member.archive_path
            );
            let url = Url::parse(&raw).expect("a member URL parses");
            assert!(adapter.can_handle(&url), "{raw} must be claimed");
        }
    }

    #[test]
    fn a_story_url_with_a_www_prefix_is_claimed() {
        let adapter = Efiction::new();
        let url = Url::parse("https://www.tgstorytime.com/viewstory.php?sid=6369").unwrap();
        assert!(adapter.can_handle(&url));
    }

    #[test]
    fn another_script_on_the_same_host_is_not_claimed() {
        let adapter = Efiction::new();
        for raw in [
            "https://www.tgstorytime.com/browse.php?type=name",
            "https://www.tgstorytime.com/viewuser.php?uid=14631",
            "https://example.invalid/viewstory.php?sid=1",
        ] {
            let url = Url::parse(raw).unwrap();
            assert!(!adapter.can_handle(&url), "{raw} must not be claimed");
        }
    }

    #[test]
    fn an_archive_in_a_subdirectory_is_claimed_only_under_its_own_path() {
        let adapter = Efiction::new();
        assert!(adapter.can_handle(
            &Url::parse("https://dark-solace.org/elysian/viewstory.php?sid=1").unwrap()
        ));
        assert!(adapter.can_handle(
            &Url::parse("https://sinfuldreams.com/unicornfic/viewstory.php?sid=1").unwrap()
        ));
        assert!(
            !adapter
                .can_handle(&Url::parse("https://dark-solace.org/viewstory.php?sid=1").unwrap()),
            "the archive is not at the host root"
        );
    }

    #[test]
    fn a_work_url_is_normalised_from_a_chapter_url() {
        let url =
            Url::parse("https://www.tgstorytime.com/viewstory.php?sid=6369&chapter=3").unwrap();
        assert_eq!(
            Efiction::work_url(&url).unwrap().as_str(),
            "https://www.tgstorytime.com/viewstory.php?sid=6369&index=1"
        );
    }

    #[test]
    fn a_chapter_url_carries_the_archive_path_of_a_subdirectory_member() {
        let member = Efiction::member_for(
            &Url::parse("https://dark-solace.org/elysian/viewstory.php?sid=7").unwrap(),
        )
        .unwrap();
        assert_eq!(
            Efiction::chapter_url(member, "dark-solace.org", "https", "7", 2),
            "https://dark-solace.org/elysian/viewstory.php?sid=7&textsize=0&chapter=2"
        );
    }

    // --- dates ------------------------------------------------------------

    #[test]
    fn a_month_name_date_with_and_without_a_comma_is_read() {
        assert_eq!(
            parse_date("January 18, 2022"),
            parse_date("January 18 2022"),
            "the comma is not part of the date"
        );
        let date = parse_date("January 18 2022").expect("the giantessworld shape");
        assert_eq!((date.year(), date.month() as u8, date.day()), (2022, 1, 18));
    }

    #[test]
    fn a_day_first_month_name_date_is_read() {
        let date = parse_date("18 Jan 2022").expect("an abbreviation");
        assert_eq!((date.year(), date.month() as u8, date.day()), (2022, 1, 18));
        let date = parse_date("18 January 2022").expect("a full name");
        assert_eq!((date.year(), date.month() as u8, date.day()), (2022, 1, 18));
    }

    #[test]
    fn a_numeric_date_is_read_day_first() {
        // tgstorytime's own `Published: 08/06/21`, read as 8 June 2021.
        let date = parse_date("08/06/21").expect("the tgstorytime shape");
        assert_eq!((date.year(), date.month() as u8, date.day()), (2021, 6, 8));
    }

    #[test]
    fn a_date_in_an_unknown_shape_is_absent_rather_than_guessed() {
        assert_eq!(parse_date("sometime last year"), None);
        assert_eq!(parse_date(""), None);
        assert_eq!(
            parse_date("2022-13-45"),
            None,
            "an impossible date is not a date"
        );
    }

    // --- status -----------------------------------------------------------

    #[test]
    fn both_members_completed_vocabularies_are_read() {
        assert_eq!(work_status("Yes"), WorkStatus::Complete);
        assert_eq!(work_status("Story Incomplete"), WorkStatus::Ongoing);
        assert_eq!(work_status("No"), WorkStatus::Ongoing);
        assert_eq!(work_status("On Hiatus"), WorkStatus::Hiatus);
        assert_eq!(work_status(""), WorkStatus::Unknown);
    }

    // --- labels -----------------------------------------------------------

    #[test]
    fn a_label_name_is_matched_without_its_colon_or_its_case() {
        let label = Label {
            name: "word count".into(),
            display: "Word count".into(),
            value: "1,234".into(),
            items: Vec::new(),
        };
        assert_eq!(label_number(&[label], "Word count"), Some(1234));
    }

    #[test]
    fn a_member_specific_class_label_becomes_a_tag_and_a_slash_is_not_a_separator() {
        let labels = vec![
            Label {
                name: "categories".into(),
                display: "Categories".into(),
                value: "Drama/Angst".into(),
                items: Vec::new(),
            },
            Label {
                name: "word count".into(),
                display: "Word count".into(),
                value: "1,234".into(),
                items: Vec::new(),
            },
            Label {
                name: "series".into(),
                display: "Series".into(),
                value: "None".into(),
                items: Vec::new(),
            },
        ];
        // `Word count` has a field of its own and `Series: None` is the member
        // writing that it has no value; `Drama/Angst` is one value, because the
        // members that write a slash write it inside a value.
        assert_eq!(tags_of(&labels), vec!["Drama/Angst"]);
    }

    #[test]
    fn a_query_parameter_is_read_without_a_url_parser() {
        // As the HTML parser hands it over, with the entities already decoded.
        assert_eq!(
            query_value("reviews.php?type=ST&item=6369&chapid=32368", "chapid"),
            Some("32368".into())
        );
        // And as a page that double-escaped its own links would hand it over.
        assert_eq!(
            query_value(
                "reviews.php?type=ST&amp;item=6369&amp;chapid=32368",
                "chapid"
            ),
            Some("32368".into())
        );
        assert_eq!(query_value("viewstory.php?sid=1", "chapid"), None);
    }

    #[test]
    fn a_chapter_parameter_that_is_not_a_number_is_not_a_chapter() {
        // `chapter=all` is the printable whole-work view, and treating it as a
        // chapter would add a phantom entry to every import.
        let pairs = split_query("viewstory.php?action=printable&sid=11369&chapter=all");
        assert!(pairs
            .iter()
            .any(|(name, value)| name == "chapter" && value == "all"));
        assert_eq!(
            pairs
                .iter()
                .find(|(name, _)| name == "chapter")
                .map(|(_, value)| value.as_str()),
            Some("all"),
            "`chapter=all` is the whole-work view, not a chapter number"
        );
    }

    #[test]
    fn the_capability_flags_say_what_the_adapter_can_do() {
        let capabilities = Efiction::new().capabilities();
        assert!(capabilities.metadata);
        assert!(capabilities.chapters);
        assert!(capabilities.per_chapter_fetch);
        assert!(!capabilities.bibliography);
        assert!(!capabilities.incremental);
    }

    #[test]
    fn the_key_and_display_name_are_not_the_same_string() {
        let adapter = Efiction::new();
        assert_eq!(adapter.key().as_str(), "efiction");
        assert_eq!(adapter.display_name(), "eFiction archives");
    }
}
