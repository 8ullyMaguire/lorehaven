//! M45-57 — §55.5's pre-review check, and the report a reviewer actually reads.
//!
//! Every submission, on both paths, runs this before a reviewer spends attention:
//! fetch three known works, validate against §4.3's shapes, verify robots and
//! pacing, and produce a report.
//!
//! **The report is the feature.** §55.5 says the report is what makes quorum
//! review meaningful — for a declarative adapter the manifest *is* the whole
//! artifact, so a reviewer is looking at CSS selectors and the parsed output
//! those selectors produce. A pass/fail boolean would leave that reviewer asking
//! "did it get the chapters?", which is the question the sample parses answer.
//!
//! Two design commitments, both about honesty rather than coverage:
//!
//! 1. **A check that could not run reports that it could not run.**
//!    `CheckOutcome` carries `Skipped`, and a report's `complete` is false when
//!    any work was skipped. A live check that reached no fixture and returned
//!    `passed: true` is the exact failure §11.7 forbids — a claim of support that
//!    is not evidence.
//! 2. **The report names the manifest it checked**, by `source_id`, base URL and
//!    a digest of the manifest text. A report that does not say which artifact it
//!    describes is a report about nothing, and a reviewer comparing two reports
//!    for the same submission cannot tell whether the manifest changed between
//!    them.

use std::fmt;

use serde_json::json;
use sha2::{Digest, Sha256};

use url::Url;

use crate::declarative::DeclarativeAdapter;
use crate::source_manifest::CompiledSource;
use crate::{Fetcher, SourceAdapter, SourceWork};

/// How many works §55.5 asks a check to fetch.
///
/// Three, and the number is in the type as a constant rather than a parameter
/// because it is part of what the check *means*: "one work parsed" is evidence
/// about one work, and a reviewer reading a report is entitled to know how much
/// evidence stands behind it.
pub const WORKS_PER_CHECK: usize = 3;

/// §4.3's shape requirements, named.
///
/// Not a list of prose. Each entry is a check with a name, because a report that
/// says `false` without saying which rule failed is the same defect as a bare
/// 403 on a trust refusal: the reader is left with no next action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeRule {
    /// The work has a title. §4.3's `work.title` is not nullable, so a parse
    /// yielding an empty title would fail at the INSERT rather than at the check.
    TitlePresent,
    /// The work has a canonical URL.
    UrlPresent,
    /// The work has an identity the import can key on.
    SourceWorkKeyPresent,
    /// The work has at least one chapter. §4.3's chapter list is what an import
    /// iterates; an empty one is indistinguishable from a source that refused us.
    HasChapters,
    /// Chapter ordinals are 1-based and strictly increasing. A reader's progress
    /// and notes map onto these ordinals across a re-import, so a zero-based or
    /// gapped list silently attaches every reader's history to the wrong place.
    OrdinalsAreOneBased,
    /// The summary is plain text with no markup left in it. The import sanitises
    /// bodies, but a summary that is still marked up means the manifest's
    /// selectors are reaching past the summary element.
    SummaryIsPlainText,
}

impl ShapeRule {
    /// Every rule, in a stable order, so a report lists them the same way twice.
    pub const ALL: [Self; 6] = [
        Self::TitlePresent,
        Self::UrlPresent,
        Self::SourceWorkKeyPresent,
        Self::HasChapters,
        Self::OrdinalsAreOneBased,
        Self::SummaryIsPlainText,
    ];

    /// The stored spelling, so a report's rule names are stable text rather than
    /// a `Debug` rendering that changes with a variant rename.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TitlePresent => "title_present",
            Self::UrlPresent => "url_present",
            Self::SourceWorkKeyPresent => "source_work_key_present",
            Self::HasChapters => "has_chapters",
            Self::OrdinalsAreOneBased => "ordinals_are_one_based",
            Self::SummaryIsPlainText => "summary_is_plain_text",
        }
    }

    /// Whether `work` satisfies this rule.
    ///
    /// Returns `Result<(), String>`: the failure carries what was wrong,
    /// because "ordinals_are_one_based: false" tells a curator less than
    /// "ordinals_are_one_based: starts at 0" does.
    pub fn check(self, work: &SourceWork) -> Result<(), String> {
        match self {
            Self::TitlePresent => {
                if work.title.trim().is_empty() {
                    Err("the work has no title".to_owned())
                } else {
                    Ok(())
                }
            }
            Self::UrlPresent => {
                if work.source_url.trim().is_empty() {
                    Err("the work has no canonical URL".to_owned())
                } else {
                    Ok(())
                }
            }
            Self::SourceWorkKeyPresent => {
                if work.source_work_key.trim().is_empty() {
                    Err("the work has no import key".to_owned())
                } else {
                    Ok(())
                }
            }
            Self::HasChapters => {
                if work.chapters.is_empty() {
                    Err("no chapters were parsed".to_owned())
                } else {
                    Ok(())
                }
            }
            Self::OrdinalsAreOneBased => {
                let mut previous: Option<u32> = None;
                for chapter in &work.chapters {
                    match previous {
                        None if chapter.ordinal != 1 => {
                            return Err(format!(
                                "the first chapter is at ordinal {}, not 1",
                                chapter.ordinal
                            ));
                        }
                        Some(prev) if chapter.ordinal != prev + 1 => {
                            return Err(format!(
                                "ordinals jump from {prev} to {}",
                                chapter.ordinal
                            ));
                        }
                        _ => {}
                    }
                    previous = Some(chapter.ordinal);
                }
                Ok(())
            }
            Self::SummaryIsPlainText => {
                // The summary is already derived as plain text by the adapter, so
                // this asserts on what actually reached it rather than on a
                // heuristic. A `<` that survived means the selector reached past
                // the summary element, which is the bug worth catching here.
                if work.summary.contains('<') {
                    Err("the summary still contains markup".to_owned())
                } else {
                    Ok(())
                }
            }
        }
    }
}

impl fmt::Display for ShapeRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What happened to one work in the check.
///
/// `Parsed` is boxed. `SourceWork` is ~360 bytes and the other three variants are
/// under 50, so an unboxed `Parsed` makes *every* `CheckOutcome` that size — the
/// report holds one per work, and a three-work report would carry a kilobyte of
/// padding for a struct whose two other states are two `String`s. Boxing is the
/// fix rather than shrinking `SourceWork`, which is the importer's type and not
/// this module's to reshape.
#[derive(Debug, Clone)]
pub enum CheckOutcome {
    /// Fetched and parsed. `failures` names every shape rule it missed.
    Parsed {
        work: Box<SourceWork>,
        failures: Vec<(ShapeRule, String)>,
    },
    /// The fetch or the parse failed. `reason` is what the source said, or what
    /// the adapter could not do with the page.
    Failed { url: String, reason: String },
    /// The check deliberately did not attempt this one, and `why` says so.
    ///
    /// Never conflated with `Passed`: a check with three `Skipped` outcomes has
    /// established nothing, and a report must not present that as a pass.
    Skipped { url: String, why: String },
}

impl CheckOutcome {
    /// Whether this work was fetched and parsed.
    #[must_use]
    pub const fn was_checked(&self) -> bool {
        matches!(self, Self::Parsed { .. })
    }

    /// Whether the check *attempted* this work and has an answer.
    ///
    /// Distinct from [`CheckOutcome::was_checked`]: a fetch that returned an
    /// error is a checked outcome in the sense that matters here — the work was
    /// tried and the attempt produced a result. Only [`CheckOutcome::Skipped`]
    /// means the check never asked.
    ///
    /// Collapsing these two is how "we tried and the source said no" gets
    /// reported as "we never tried", which is the less useful of the two facts
    /// and the one a reviewer cannot act on.
    #[must_use]
    pub const fn was_attempted(&self) -> bool {
        !matches!(self, Self::Skipped { .. })
    }
}

/// One work's line in the report.
#[derive(Debug, Clone)]
pub struct WorkReport {
    pub url: String,
    pub outcome: CheckOutcome,
}

/// What the check's evidence actually is.
///
/// This is a **carried fact, not a derived one.** The obvious implementation is a
/// method asking "did every outcome come from a fetch?", which is true for a
/// `FixtureFetcher` run exactly as it is for a live one — and that makes a check
/// that read three local files report `evidence: live fetch`, which is §11.7's
/// forbidden claim of support stated as a measurement.
///
/// Only the caller knows which transport it handed in, so the caller says so and
/// the report carries it. A fixture-backed check is real evidence about parsing
/// and **no** evidence at all about the live source, and the report has to make
/// that distinction available to the reviewer rather than collapse it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// Pages served from local fixtures. Proves selectors compile and parse;
    /// proves nothing about the source.
    Fixtures,
    /// Pages actually fetched. Proves both.
    Live,
}

impl Evidence {
    /// The word a reviewer reads.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fixtures => "offline fixtures",
            Self::Live => "live fetch",
        }
    }
}

/// §55.5's report.
///
/// The whole point of the type is that it is *renderable by a human*: a reviewer
/// reads `render()` and sees the manifest, the verdict, the per-rule results and
/// the sample parses, without running anything.
#[derive(Debug, Clone)]
pub struct CheckReport {
    /// The manifest this report is about. §55.5's reviewers read the manifest and
    /// the report; a report that does not say which manifest is meaningless.
    pub source_id: String,
    pub base_url: String,
    /// A digest of the manifest text, so two reports for the same submission can
    /// be compared and a changed manifest shows up as a changed digest.
    pub manifest_digest: String,
    /// The URLs the check asked for, in order.
    pub works: Vec<WorkReport>,
    /// What the evidence is, stated by the caller. Never derived.
    pub evidence: Evidence,
    /// §11.5's pacing as applied: the manifest's request versus what the host
    /// enforced. A manifest cannot raise its own rate (§55.8's last runtime
    /// line), so this is the *applied* figure.
    pub applied_rate_per_second: f64,
    /// What the host did about robots.txt, in the words of §11.5's posture.
    pub robots_note: String,
}

impl CheckReport {
    /// Whether every requested work was fetched, parsed and shape-valid.
    ///
    /// `false` if any outcome is `Failed` **or** `Skipped`. This is the property
    /// that stops an offline check from reporting a live result.
    #[must_use]
    pub fn passed(&self) -> bool {
        !self.works.is_empty()
            && self.works.iter().all(|w| {
                matches!(&w.outcome, CheckOutcome::Parsed { failures, .. } if failures.is_empty())
            })
    }

    /// Whether the check attempted everything it was asked to.
    ///
    /// Keyed on [`CheckOutcome::was_attempted`], not `was_checked`: a work whose
    /// fetch returned an error WAS attempted, and reporting the run as incomplete
    /// for that reason would tell a reviewer "we never asked" — which sends them
    /// looking for a configuration problem instead of reading the source's own
    /// answer, which is right there in the report.
    ///
    /// A report can be a pass and still be incomplete — that is exactly the case
    /// where the check declined to ask about one of the three. The two facts are
    /// separate and the report carries both.
    #[must_use]
    pub fn complete(&self) -> bool {
        !self.works.is_empty() && self.works.iter().all(|w| w.outcome.was_attempted())
    }

    /// Whether this report may be published as live verification.
    ///
    /// §11.16 counts support from verified adapters only, so this is the gate a
    /// registry would ask before crediting a submission with §11.7's evidence.
    /// It is **conjunctive** on purpose: a fixture run that parsed everything is
    /// not live verification, and a live run that could not read one work is not
    /// verification of the other two either.
    #[must_use]
    pub fn verifies_live_source(&self) -> bool {
        self.evidence == Evidence::Live && self.complete() && self.passed()
    }

    /// The report as JSON, for storage on a submission row.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "source_id": self.source_id,
            "base_url": self.base_url,
            "manifest_digest": self.manifest_digest,
            "works_per_check": WORKS_PER_CHECK,
            "evidence": self.evidence.as_str(),
            "passed": self.passed(),
            "complete": self.complete(),
            "verifies_live_source": self.verifies_live_source(),
            "applied_rate_per_second": self.applied_rate_per_second,
            "robots_note": self.robots_note,
            "works": self.works.iter().map(|w| json!({
                "url": w.url,
                "checked": w.outcome.was_checked(),
                "shape_failures": match &w.outcome {
                    CheckOutcome::Parsed { failures, .. } => failures
                        .iter()
                        .map(|(rule, detail)| json!({ "rule": rule.as_str(), "detail": detail }))
                        .collect::<Vec<_>>(),
                    CheckOutcome::Failed { reason, .. }
                    | CheckOutcome::Skipped { why: reason, .. } =>
                        vec![json!({ "rule": "fetch", "detail": reason })],
                },
                "sample": match &w.outcome {
                    CheckOutcome::Parsed { work, .. } => Some(json!({
                        "title": work.title,
                        "author": work.author_text,
                        "summary_excerpt": excerpt(&work.summary),
                        "word_count": work.word_count,
                        "chapters": work.chapters.len(),
                        "chapter_titles": work.chapters.iter().take(5)
                            .map(|c| c.title.clone()).collect::<Vec<_>>(),
                    })),
                    _ => None,
                },
            })).collect::<Vec<_>>(),
        })
    }

    /// The report as text, which is what a reviewer actually reads.
    ///
    /// Sectioned so the three things a reviewer asks are each findable: *does it
    /// pass*, *what did it see*, and *which manifest was this*.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();

        out.push_str(&format!(
            "§55.5 pre-review check — {}\n\
             manifest {}\n\
             digest    sha256:{}\n\
             evidence  {}\n\n",
            self.source_id,
            self.base_url,
            &self.manifest_digest[..16],
            self.evidence.as_str(),
        ));

        let verdict = if self.passed() {
            format!(
                "PASS — {} of {} works parsed and met every §4.3 shape\n\n",
                self.works
                    .iter()
                    .filter(|w| w.outcome.was_checked())
                    .count(),
                self.works.len(),
            )
        } else {
            format!(
                "FAIL — {} of {} works passed\n\n",
                self.works
                    .iter()
                    .filter(|w| w.outcome.was_checked())
                    .count(),
                self.works.len(),
            )
        };
        out.push_str(&verdict);

        for work in &self.works {
            out.push_str(&format!("  {}\n", work.url));
            match &work.outcome {
                CheckOutcome::Parsed { work, failures } => {
                    out.push_str(&format!(
                        "    title    {}\n    author   {}\n    summary  {}\n    chapters {}\n",
                        work.title,
                        if work.author_text.is_empty() {
                            "(none)"
                        } else {
                            &work.author_text
                        },
                        excerpt(&work.summary),
                        work.chapters.len(),
                    ));
                    for (rule, detail) in failures {
                        out.push_str(&format!("    FAILED {rule}: {detail}\n"));
                    }
                }
                CheckOutcome::Failed { reason, .. } => {
                    out.push_str(&format!("    fetch failed: {reason}\n"));
                }
                CheckOutcome::Skipped { why, .. } => {
                    out.push_str(&format!("    not attempted: {why}\n"));
                }
            }
        }

        out.push_str(&format!(
            "\n  pacing    {} req/s applied (the manifest's request, lowered by §11.5 where they conflict)\n  robots    {}\n",
            self.applied_rate_per_second, self.robots_note,
        ));

        out
    }
}

/// A summary excerpt of at most `max` characters, cut on a char boundary.
fn excerpt(text: &str) -> String {
    const MAX: usize = 72;
    let trimmed = text.trim();
    if trimmed.chars().count() <= MAX {
        return trimmed.to_owned();
    }
    let cut: String = trimmed.chars().take(MAX).collect();
    format!("{cut}…")
}

/// A digest of the manifest text, for the report header.
#[must_use]
pub fn manifest_digest(manifest_text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(manifest_text.as_bytes());
    let hex = hasher.finalize();
    hex.iter().map(|b| format!("{b:02x}")).collect()
}

/// Run §55.5's check over `work_urls`, with the caller stating what the evidence
/// is.
///
/// Takes the transport as an argument and holds none, for the same reason
/// `DeclarativeAdapter` does: the check must be runnable against fixtures with
/// no network, which is what makes it reviewable and deterministic, and what
/// makes the same code the one that runs live.
///
/// `evidence` is a required argument rather than a parameter with a default. The
/// default anyone would write — "assume live", because a live run is the one
/// that counts — is how a fixture run publishes itself as verification. Making
/// the caller say which is what keeps §11.7's discipline at the call site.
///
/// `work_urls` is whatever the operator supplied as the three known works. Fewer
/// than [`WORKS_PER_CHECK`] is not padded — it is reported as the shorter list it
/// is, and `complete()` is about outcomes rather than about the count, so an
/// operator who knows two works are the only ones the source publishes gets an
/// honest report instead of a padded one.
pub async fn run_check(
    source: &CompiledSource,
    manifest_text: &str,
    fetcher: &dyn Fetcher,
    work_urls: &[String],
    evidence: Evidence,
) -> CheckReport {
    run_check_inner(source, manifest_text, fetcher, work_urls, evidence, None).await
}

/// [`run_check`] with a cap on how many works to attempt.
///
/// The cap produces genuine [`CheckOutcome::Skipped`] outcomes rather than
/// silently returning a shorter list, because a check that quietly fetched one
/// work when §55.5 asked for three is indistinguishable in the report from one
/// that fetched three. The remaining URLs are named, so the reviewer sees which
/// works were not looked at.
pub async fn run_check_limited(
    source: &CompiledSource,
    manifest_text: &str,
    fetcher: &dyn Fetcher,
    work_urls: &[String],
    evidence: Evidence,
    limit: usize,
) -> CheckReport {
    run_check_inner(
        source,
        manifest_text,
        fetcher,
        work_urls,
        evidence,
        Some(limit),
    )
    .await
}

async fn run_check_inner(
    source: &CompiledSource,
    manifest_text: &str,
    fetcher: &dyn Fetcher,
    work_urls: &[String],
    evidence: Evidence,
    limit: Option<usize>,
) -> CheckReport {
    let adapter = DeclarativeAdapter::new(source.clone());
    let mut works = Vec::with_capacity(work_urls.len());

    for (index, raw) in work_urls.iter().enumerate() {
        let url = raw.clone();
        if let Some(cap) = limit {
            if index >= cap {
                works.push(WorkReport {
                    url,
                    outcome: CheckOutcome::Skipped {
                        url: raw.clone(),
                        why: format!(
                            "§55.5 asks for {WORKS_PER_CHECK} works and this run was limited to \
                             {cap}; the remaining works were not requested"
                        ),
                    },
                });
                continue;
            }
        }
        match Url::parse(raw) {
            Err(e) => works.push(WorkReport {
                url,
                outcome: CheckOutcome::Failed {
                    url: raw.clone(),
                    reason: format!("not a URL: {e}"),
                },
            }),
            Ok(parsed) => {
                // A URL the adapter will not recognise is a manifest problem, not
                // a fetch problem, so it is reported against the manifest rather
                // than as a network error.
                if !adapter.can_handle(&parsed) {
                    works.push(WorkReport {
                        url,
                        outcome: CheckOutcome::Failed {
                            url: raw.clone(),
                            reason: format!(
                                "the manifest's work_pattern `{}` does not match this URL",
                                source.manifest.work_pattern
                            ),
                        },
                    });
                    continue;
                }

                // No credentials: §55.4.2 holds for this path exactly as for the
                // WASM one, so a `cookie_login` manifest is checked without them
                // and the resulting failure is reported as what it is.
                match adapter.preview(fetcher, &parsed, None).await {
                    Ok(work) => {
                        let failures: Vec<(ShapeRule, String)> = ShapeRule::ALL
                            .into_iter()
                            .filter_map(|rule| rule.check(&work).err().map(|d| (rule, d)))
                            .collect();
                        works.push(WorkReport {
                            url,
                            outcome: CheckOutcome::Parsed {
                                work: Box::new(work),
                                failures,
                            },
                        });
                    }
                    Err(e) => works.push(WorkReport {
                        url,
                        outcome: CheckOutcome::Failed {
                            url: raw.clone(),
                            reason: e.to_string(),
                        },
                    }),
                }
            }
        }
    }

    CheckReport {
        source_id: source.manifest.source_id.clone(),
        base_url: source.manifest.base_url.clone(),
        manifest_digest: manifest_digest(manifest_text),
        works,
        evidence,
        // §11.5's floor is one request per second and `Crawl-delay` may lower it
        // further; the manifest's own request can only ever be an upper bound.
        // Reporting `min(manifest, 1.0)` is the honest figure for a check that
        // has not consulted robots.txt, and `robots_note` says so.
        applied_rate_per_second: source.manifest.rate_limit_per_second.min(1.0),
        robots_note: "not consulted — this check read the manifest and the pages, not robots.txt"
            .to_owned(),
    }
}

/// The same check, run against fixtures, labelled as such.
///
/// Exists rather than as a flag on [`run_check`] so the labelling cannot be
/// forgotten: a caller who reached for `run_check(.., Evidence::Fixtures)` has to
/// type the word `Fixtures`, and a caller who reaches for this cannot mislabel
/// anything.
pub async fn run_offline_check(
    source: &CompiledSource,
    manifest_text: &str,
    fetcher: &dyn Fetcher,
    work_urls: &[String],
) -> CheckReport {
    run_check(
        source,
        manifest_text,
        fetcher,
        work_urls,
        Evidence::Fixtures,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::source_manifest::SourceManifest;
    use crate::{FixtureFetcher, SourceWork, WorkStatus};

    fn manifest_yaml() -> String {
        r#"
source_id: example-fictions
name: Example Fictions
base_url: https://example-fictions.test
rate_limit_per_second: 1.0
work_pattern: /works/{id}
chapter_pattern: /works/{id}/chapters/{num}
selectors:
  title: "h1.title"
  author: "span.byline"
  summary: "div.summary"
  body: "div#ch-body"
  tags: "ul.tags li"
  word_count: "span.words"
  date_published: "time.published"
pagination:
  type: none
auth:
  type: none
"#
        .to_owned()
    }

    fn work_page(title: &str, chapters: usize) -> String {
        let mut html = format!(
            r#"<html><body>
              <h1 class="title">{title}</h1>
              <span class="byline">A Writer</span>
              <div class="summary">A plain summary with no markup.</div>
              <span class="words">42000</span>
              <time class="published">2020-01-01</time>
              <ul class="tags"><li>tag-one</li></ul>
            "#
        );
        for n in 1..=chapters {
            html.push_str(&format!(
                r#"<li class="chapter"><a href="/works/w1/chapters/{n}">Chapter {n}</a></li>"#
            ));
        }
        html.push_str("</ul></body></html>");
        html
    }

    fn compiled() -> CompiledSource {
        SourceManifest::compile(
            &serde_yaml::from_str::<SourceManifest>(&manifest_yaml()).expect("the manifest parses"),
        )
        .expect("the manifest compiles")
    }

    fn three_urls() -> Vec<String> {
        (1..=WORKS_PER_CHECK)
            .map(|n| format!("https://example-fictions.test/works/w{n}"))
            .collect()
    }

    fn fetcher_for() -> FixtureFetcher {
        let mut f = FixtureFetcher::new();
        for (n, url) in three_urls().iter().enumerate() {
            let _ = n;
            f = f.with_page(url, work_page("A Work", 3));
        }
        f
    }

    fn a_work(chapters: Vec<u32>) -> SourceWork {
        SourceWork {
            source_key: crate::SourceKey("example".to_owned()),
            source_work_key: "w1".to_owned(),
            source_url: "https://example-fictions.test/works/w1".to_owned(),
            title: "A Work".to_owned(),
            author_text: "A Writer".to_owned(),
            author_url: None,
            summary: "plain".to_owned(),
            word_count: Some(42_000),
            language: Some("en".to_owned()),
            status: WorkStatus::Ongoing,
            published_at: None,
            updated_at: None,
            chapters: chapters
                .into_iter()
                .map(|ordinal| crate::ChapterRef {
                    ordinal,
                    source_chapter_key: ordinal.to_string(),
                    title: format!("Chapter {ordinal}"),
                })
                .collect(),
            rating_text: None,
            warning_texts: Vec::new(),
            tags: Vec::new(),
        }
    }

    // -- the report names its artifact --------------------------------------

    #[tokio::test]
    async fn a_check_report_names_the_manifest_version_it_checked() {
        let source = compiled();
        let report =
            run_offline_check(&source, &manifest_yaml(), &fetcher_for(), &three_urls()).await;

        let rendered = report.render();
        assert!(
            rendered.contains("example-fictions"),
            "a report that does not name the source is a report about nothing: {rendered}"
        );
        assert!(
            rendered.contains("sha256:"),
            "§55.5: two reports of the same submission must be comparable, which needs a \
             digest of the manifest text: {rendered}"
        );
        assert_eq!(report.manifest_digest, manifest_digest(&manifest_yaml()));
        assert_eq!(report.base_url, "https://example-fictions.test");
        // §55.5 names pass/fail, the sample parses and the resource usage; the
        // digest is what makes the report about *this* manifest.
        assert!(rendered.contains("PASS"), "{rendered}");
    }

    #[tokio::test]
    async fn a_changed_manifest_produces_a_different_digest() {
        let one = manifest_digest(&manifest_yaml());
        let mut changed = manifest_yaml();
        changed.push_str("\n# a reviewer's note\n");
        assert_ne!(
            one,
            manifest_digest(&changed),
            "a digest that ignores the manifest text cannot detect a changed submission"
        );
    }

    // -- pass and fail -------------------------------------------------------

    #[tokio::test]
    async fn three_parsed_works_meet_every_shape_rule() {
        let source = compiled();
        let report =
            run_offline_check(&source, &manifest_yaml(), &fetcher_for(), &three_urls()).await;

        assert!(
            report.passed(),
            "three well-formed works must pass §55.5: {}",
            report.render()
        );
        assert_eq!(report.works.len(), WORKS_PER_CHECK);
        for work in &report.works {
            assert!(
                matches!(&work.outcome, CheckOutcome::Parsed { failures, .. } if failures.is_empty()),
                "{} should be parsed with no failures",
                work.url
            );
        }
        // The sample parses are in the report, because for a declarative adapter
        // the selectors plus their output are the entire reviewable artifact.
        let json = report.to_json();
        assert_eq!(
            json["works"].as_array().map(Vec::len),
            Some(WORKS_PER_CHECK)
        );
        assert_eq!(json["passed"], json!(true));
        assert_eq!(json["works"][0]["sample"]["title"], json!("A Work"));
    }

    #[tokio::test]
    async fn a_work_missing_its_title_fails_at_the_parse_naming_the_selector() {
        let source = compiled();
        // The title selector is `h1.title` and the page has no such element. The
        // adapter refuses to invent a title, which is §11.6's "a source that
        // changes its HTML must fail loudly" — so the failure arrives as a *fetch*
        // failure carrying the selector name, not as a shape-rule failure on a
        // parsed work. Asserting `title_present` here was asserting an outcome
        // the adapter deliberately cannot produce.
        let fetcher = FixtureFetcher::new().with_page(
            "https://example-fictions.test/works/w1",
            r#"<html><body><h2>Not a title element</h2>
                     <div class="summary">plain</div>
                     <ul class="chapter"><li><a href="/works/w1/chapters/1">One</a></li></ul>
                   </body></html>"#,
        );

        let report = run_offline_check(
            &source,
            &manifest_yaml(),
            &fetcher,
            &["https://example-fictions.test/works/w1".to_owned()],
        )
        .await;

        assert!(!report.passed());
        let rendered = report.render();
        assert!(
            rendered.contains("h1.title"),
            "a parse failure must name the selector that matched nothing, or a curator \
                 cannot tell which of seven to fix: {rendered}"
        );
        assert!(rendered.contains("fetch failed"), "{rendered}");
    }

    #[tokio::test]
    async fn a_shape_rule_failure_is_reported_against_its_name() {
        // The rules are still reachable for works the adapter DID parse. This case
        // exists so the rule names in a report are pinned independently of the
        // parse failures, which is the part §55.5's reviewer reads.
        //
        // The failures are computed from the SAME work that is stored. Building
        // the list from a second, differently-constructed work is the classic
        // fixture bug: the assertion passes for a report describing a work the
        // caller never held.
        let mut parsed = a_work(vec![1, 2]);
        parsed.title = String::new();
        parsed.summary = "<p>markup</p>".to_owned();

        let failures: Vec<(ShapeRule, String)> = ShapeRule::ALL
            .into_iter()
            .filter_map(|rule| rule.check(&parsed).err().map(|detail| (rule, detail)))
            .collect();

        let report = CheckReport {
            source_id: "example-fictions".to_owned(),
            base_url: "https://example-fictions.test".to_owned(),
            manifest_digest: manifest_digest(&manifest_yaml()),
            evidence: Evidence::Fixtures,
            works: vec![WorkReport {
                url: "https://example-fictions.test/works/w1".to_owned(),
                outcome: CheckOutcome::Parsed {
                    work: Box::new(parsed),
                    failures,
                },
            }],
            applied_rate_per_second: 1.0,
            robots_note: "not consulted".to_owned(),
        };

        let rendered = report.render();
        assert!(!report.passed());
        // Assert on the rules that genuinely failed for this work, and on the
        // detail rather than the bare name — a report saying `false` with no
        // reason is the defect this module exists to avoid.
        assert!(rendered.contains("FAILED title_present"), "{rendered}");
        assert!(rendered.contains("no title"), "{rendered}");
        assert!(
            rendered.contains("FAILED summary_is_plain_text"),
            "{rendered}"
        );
        // And a rule the work passed is not printed as a failure.
        assert!(!rendered.contains("FAILED has_chapters"), "{rendered}");
    }

    #[tokio::test]
    async fn a_work_with_no_chapters_fails_rather_than_parsing_as_empty() {
        let source = compiled();
        let fetcher = FixtureFetcher::new().with_page(
            "https://example-fictions.test/works/w1",
            r#"<html><body><h1 class="title">A Work</h1>
                 <div class="summary">plain</div>
               </body></html>"#,
        );

        let report = run_offline_check(
            &source,
            &manifest_yaml(),
            &fetcher,
            &["https://example-fictions.test/works/w1".to_owned()],
        )
        .await;

        assert!(!report.passed());
        assert!(
            report.render().contains("has_chapters"),
            "{}",
            report.render()
        );
    }

    #[tokio::test]
    async fn a_url_the_manifests_pattern_does_not_match_is_a_manifest_failure_not_a_fetch_failure()
    {
        let source = compiled();
        // The fetcher WOULD serve this URL. The manifest's `work_pattern` does
        // not match it, so the check must say the manifest is wrong rather than
        // blaming the network.
        let fetcher =
            fetcher_for().with_page("https://example-fictions.test/novels/n1", work_page("X", 1));

        let report = run_offline_check(
            &source,
            &manifest_yaml(),
            &fetcher,
            &["https://example-fictions.test/novels/n1".to_owned()],
        )
        .await;

        assert!(!report.passed());
        let rendered = report.render();
        assert!(
            rendered.contains("work_pattern"),
            "the operator supplied a URL the manifest cannot read; say that: {rendered}"
        );
    }

    #[tokio::test]
    async fn a_fetch_failure_is_reported_with_the_sources_own_reason() {
        let source = compiled();
        // A refusal is what §11.5's guard produces, and its message is what a
        // reviewer needs to see — a report that renders "failed" with no reason
        // sends them to guess between the manifest, the source and the network.
        let fetcher = fetcher_for().with_failure(
            "/works/",
            crate::SourceError::Parse("no title element matched".to_owned()),
        );

        let report = run_offline_check(&source, &manifest_yaml(), &fetcher, &three_urls()).await;

        assert!(!report.passed(), "every work failed, so the check failed");
        // `complete()` is about *outcomes*, and a fetch that returned an error is
        // a checked outcome — the work was attempted and the attempt has a result.
        // Asserting the opposite (that a failure makes the report incomplete) would
        // conflate "we tried and the source said no" with "we never tried", and the
        // first is the more useful fact for a reviewer.
        assert!(
            report.complete(),
            "every work was attempted and reported a result: {}",
            report.render()
        );
        assert!(
            !report.verifies_live_source(),
            "and it is a fixture run, so it never counts as live verification"
        );
        let rendered = report.render();
        assert!(
            rendered.contains("no title element matched"),
            "§55.5's report is where the adapter's own answer reaches the reviewer: {rendered}"
        );
        assert!(rendered.contains("fetch failed"), "{rendered}");
    }

    // -- the honesty properties ----------------------------------------------

    #[tokio::test]
    async fn a_check_that_attempted_nothing_is_not_a_pass() {
        let source = compiled();
        // A fixture fetcher with nothing recorded returns `Network`, which is a
        // checked outcome: the work was attempted and the answer was "no fixture".
        let report = run_offline_check(
            &source,
            &manifest_yaml(),
            &FixtureFetcher::new(),
            &["not a url at all".to_owned()],
        )
        .await;

        assert!(!report.passed());
        assert!(
            report.complete(),
            "an unparseable URL is attempted-and-answered, not skipped: {}",
            report.render()
        );
        assert!(report.render().contains("not a URL"), "{}", report.render());
    }

    #[tokio::test]
    async fn a_work_the_check_declined_to_request_is_reported_as_skipped_and_named() {
        // The one case that produces `Skipped`, and the reason the variant exists:
        // a limited run must say which works it did not look at rather than
        // returning a shorter list that reads as "that was all of them".
        let source = compiled();
        let report = run_check_limited(
            &source,
            &manifest_yaml(),
            &fetcher_for(),
            &three_urls(),
            Evidence::Live,
            2,
        )
        .await;

        assert!(
            !report.complete(),
            "one work was deliberately not requested"
        );
        assert!(!report.passed());
        let rendered = report.render();
        assert!(rendered.contains("not attempted"), "{rendered}");
        assert!(
            rendered.contains("w3"),
            "the skipped work must be named, or a reviewer cannot tell which one: {rendered}"
        );
        assert!(
            !report.verifies_live_source(),
            "a partial live run verifies nothing"
        );
        // And the two that were checked are still reported, not discarded.
        assert!(rendered.contains("A Work"), "{rendered}");
    }

    #[tokio::test]
    async fn an_empty_work_list_never_reports_a_pass() {
        let source = compiled();
        let report = run_offline_check(&source, &manifest_yaml(), &fetcher_for(), &[]).await;
        assert!(
            !report.passed() && !report.complete(),
            "an empty work list is an empty check, and `works.iter().all()` on an empty \
             slice is vacuously true — which is exactly why the length is asserted too"
        );
    }

    #[tokio::test]
    async fn a_fixture_backed_check_does_not_claim_to_be_live() {
        // §11.7's discipline: a claim of support is evidence or it is nothing. A
        // check that read local fixtures is real evidence about parsing and no
        // evidence at all about the live source, so the report says which it is.
        let source = compiled();
        // Deliberately partial: two of the three works are recorded and the third
        // is not, so `FixtureFetcher` refuses it. `fetcher_for()` would register
        // all three and the report would be a *complete* pass — which is the case
        // `a_live_run_that_parsed_everything_is_the_only_thing_that_verifies`
        // covers. This case is about a check that could not finish.
        let mut fetcher = FixtureFetcher::new();
        fetcher = fetcher
            .with_page(
                "https://example-fictions.test/works/w1",
                work_page("A Work", 3),
            )
            .with_page(
                "https://example-fictions.test/works/w2",
                work_page("A Work", 3),
            );

        let report = run_offline_check(&source, &manifest_yaml(), &fetcher, &three_urls()).await;
        assert!(
            report.complete(),
            "two works parsed and the third was requested and refused — all three were \
             attempted, which is what `complete` measures: {}",
            report.render()
        );
        assert!(
            !report.passed(),
            "two of three is not a pass: {}",
            report.render()
        );
        assert!(
            !report.verifies_live_source(),
            "§11.7's discipline: a check that read local fixtures is evidence about parsing \
             and nothing at all about the live source, so it must never be publishable as \
             verification. The evidence label is carried by the CALLER rather than derived \
             from the outcomes — deriving it is what made a fixture run report 'live fetch'."
        );
        // And the label is visible to the reviewer, not just to the type.
        let rendered = report.render();
        assert!(
            rendered.contains("evidence  offline fixtures"),
            "{rendered}"
        );
        assert!(!rendered.contains("live fetch"), "{rendered}");
    }

    #[tokio::test]
    async fn a_perfect_fixture_run_still_does_not_verify_the_live_source() {
        // The case the derived-evidence design got wrong, and the one that matters
        // most: everything parsed, nothing failed, and the report must STILL not
        // be publishable as verification because the pages came from disk.
        let source = compiled();
        let report =
            run_offline_check(&source, &manifest_yaml(), &fetcher_for(), &three_urls()).await;

        assert!(report.passed(), "{}", report.render());
        assert!(report.complete(), "{}", report.render());
        assert!(
            !report.verifies_live_source(),
            "a fully-parsed fixture run is a pass, and it is still not §11.7's evidence"
        );
        assert_eq!(report.to_json()["passed"], json!(true));
        assert_eq!(report.to_json()["verifies_live_source"], json!(false));
    }

    #[tokio::test]
    async fn a_live_run_that_parsed_everything_is_the_only_thing_that_verifies() {
        // The positive case, so `verifies_live_source` is not merely a rule that
        // always says no — a gate that can never open is indistinguishable from a
        // gate that is simply wrong.
        let source = compiled();
        let report = run_check(
            &source,
            &manifest_yaml(),
            &fetcher_for(),
            &three_urls(),
            Evidence::Live,
        )
        .await;

        assert!(report.passed(), "{}", report.render());
        assert!(report.complete(), "{}", report.render());
        assert!(
            report.verifies_live_source(),
            "a live run over three works that all parsed and met every shape rule is \
             §11.7's evidence"
        );
        assert_eq!(report.to_json()["verifies_live_source"], json!(true));
        assert!(report.render().contains("evidence  live fetch"));
    }

    #[tokio::test]
    async fn a_live_run_missing_one_work_does_not_verify_the_other_two() {
        // Conjunctive on purpose: partial evidence is not evidence of the source.
        let source = compiled();
        let fetcher = FixtureFetcher::new().with_page(
            "https://example-fictions.test/works/w1",
            work_page("A Work", 3),
        );

        let report = run_check(
            &source,
            &manifest_yaml(),
            &fetcher,
            &three_urls(),
            Evidence::Live,
        )
        .await;

        assert!(!report.verifies_live_source(), "{}", report.render());
    }

    #[tokio::test]
    async fn the_applied_rate_is_never_above_the_manifests_request_and_never_above_one_per_second()
    {
        // §55.8's last runtime line: a declarative manifest cannot raise its own
        // rate above the host's applied rate. §11.5's floor is one per second.
        let mut fast = manifest_yaml();
        fast = fast.replace("rate_limit_per_second: 1.0", "rate_limit_per_second: 50.0");
        let source = SourceManifest::compile(
            &serde_yaml::from_str::<SourceManifest>(&fast).expect("parses"),
        )
        .expect("compiles");

        let report = run_offline_check(&source, &fast, &fetcher_for(), &three_urls()).await;
        assert!(
            report.applied_rate_per_second <= 1.0,
            "a manifest asking for 50 req/s must be reported as 1: {}",
            report.render()
        );
        assert!(report.render().contains("pacing"));
    }

    // -- the shape rules, directly ------------------------------------------

    #[test]
    fn an_ordinal_list_starting_at_zero_fails_the_ordinal_rule() {
        let work = a_work(vec![0, 1, 2]);
        // Assert on the verdict, not only on the message. The message is identical
        // for a zero-based list and for one that starts at some other wrong number,
        // so an assertion on the text alone passes a mutation of `!= 1` to `< 1` —
        // measured, not assumed: that mutation kept all 22 tests green.
        assert!(
            ShapeRule::OrdinalsAreOneBased.check(&work).is_err(),
            "a zero-based list must fail the ordinal rule"
        );
        let error = ShapeRule::OrdinalsAreOneBased
            .check(&work)
            .expect_err("a zero-based list must fail");
        assert!(
            error.contains('0'),
            "the failure must say what was wrong: {error}"
        );
    }

    #[test]
    fn an_ordinal_list_starting_at_some_other_number_also_fails() {
        // The second case the message-only assertion cannot distinguish. Together
        // with the zero case, this pins the rule to "the first ordinal is exactly 1"
        // rather than to any weaker property that shares the same message.
        let work = a_work(vec![4, 5, 6]);
        assert!(ShapeRule::OrdinalsAreOneBased.check(&work).is_err());
    }

    #[test]
    fn an_empty_chapter_list_does_not_violate_the_ordinal_rule() {
        // `HasChapters` is the rule that owns "no chapters". If this case also
        // failed the ordinal rule, a report would print two failures for one cause
        // and a curator would go looking for a second problem that is not there.
        assert_eq!(
            ShapeRule::OrdinalsAreOneBased.check(&a_work(vec![])),
            Ok(())
        );
    }

    #[test]
    fn a_gapped_ordinal_list_fails_the_ordinal_rule() {
        let work = a_work(vec![1, 2, 9]);
        let error = ShapeRule::OrdinalsAreOneBased
            .check(&work)
            .expect_err("a gap must fail");
        assert!(error.contains('9'), "{error}");
    }

    #[test]
    fn a_one_based_contiguous_list_passes_the_ordinal_rule() {
        assert_eq!(
            ShapeRule::OrdinalsAreOneBased.check(&a_work(vec![1, 2, 3])),
            Ok(())
        );
    }

    #[test]
    fn a_summary_still_containing_markup_fails_its_rule() {
        let mut work = a_work(vec![1]);
        work.summary = "<p>markup survived</p>".to_owned();
        assert!(ShapeRule::SummaryIsPlainText.check(&work).is_err());
    }

    #[test]
    fn every_rule_name_is_stable_text() {
        // A `Debug` rendering would change with a rename, and these names appear
        // in a stored report a reviewer may read months later.
        for rule in ShapeRule::ALL {
            let name = rule.as_str();
            assert!(!name.is_empty());
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{name} must be a slug"
            );
        }
    }

    #[test]
    fn an_excerpt_cuts_on_a_character_boundary() {
        let long = "é".repeat(200);
        let cut = excerpt(&long);
        assert!(
            cut.chars().count() <= 73,
            "cut at the boundary, not mid-character"
        );
        assert!(cut.ends_with('…'));
    }
}
