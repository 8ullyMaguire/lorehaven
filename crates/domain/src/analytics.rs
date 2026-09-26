//! Trust-gated analytics: the capability registry and the gate that guards it.
//!
//! # Why a registry and not a dashboard
//!
//! The obvious implementation is a per-role list of metrics and a filter. It
//! fails quietly. A capability that is not in the list is invisible, so a
//! dashboard that forgets a name is merely a missing number — until somebody
//! adds the metric, renders it unconditionally, and the filter is one branch
//! away from not covering it.
//!
//! So the direction is inverted. A dashboard asks the registry what the viewer
//! may see and renders exactly that. The registry is the whole surface, and a
//! test can assert it without reading any UI.
//!
//! # Why the floor is applied at the query
//!
//! §36.12 requires a work with few views to suppress its breakdowns. A display
//! filter that runs after the query has already fetched five individual rows
//! has already had the privacy incident; the rows were in the process. Every
//! `coarsen` call in this module is therefore the last thing that happens to a
//! count, and the DB layer applies it to the aggregate rather than to rows.
//!
//! # Trust is a ceiling, not a score
//!
//! Two things are checked, not one. A capability needs a minimum trust level
//! *and*, for anything operational, a role. A level alone would mean a
//! misconfigured account at TL6 reads the financial dashboard; a role alone
//! would mean the admin panel is one promotion away from every reader.

// Re-exported so a test of the gate needs one import, not two. The ladder
// belongs to governance; a caller reasoning about the gate is reasoning about
// both.
pub use crate::governance::{
    TL_ESTABLISHED, TL_NEW, TL_REGULAR, TL_REVIEWED, TL_SENIOR, TL_STEWARD, TL_TRUSTEE, TRUST_MAX,
};

/// The k-anonymity floor for aggregates about the viewer.
pub const K_SELF: i64 = 5;

/// The k-anonymity floor for aggregates about anyone else.
///
/// §36.12's value, not the source document's 5. A reader whose own dashboard
/// shows a six-person breakdown must not then meet the same six people in an
/// author dashboard — the threshold has to move the *stricter* way as the
/// subject gets less personal, not the looser way.
pub const K_OTHERS: i64 = 10;

/// Who a metric is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    /// About the viewer. Their own reading, their own works.
    Self_,
    /// About anyone else, including the instance as a whole.
    Other,
}

/// Which threshold applies.
pub fn floor_for(subject: Subject) -> i64 {
    match subject {
        Subject::Self_ => K_SELF,
        Subject::Other => K_OTHERS,
    }
}

/// A count after the floor, which is either a number or a statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coarsened {
    /// At or above the floor: the true count.
    Exact(i64),
    /// Below the floor: the true fact, which is that it is below `floor`.
    Below(i64),
}

impl Coarsened {
    /// The numeric value, or `None` when the count is below the floor.
    ///
    /// Returning `Option` rather than a zero matters: a zero is a claim, and a
    /// client that formats `unwrap_or(0)` is how "we suppressed this" becomes
    /// "nobody did this".
    pub fn value(self) -> Option<i64> {
        match self {
            Coarsened::Exact(n) => Some(n),
            Coarsened::Below(_) => None,
        }
    }

    /// Whether this is a real number.
    pub fn is_numeric(self) -> bool {
        self.value().is_some()
    }
}

/// Apply the floor to a count.
///
/// Monotone in the sense that matters: every count below the floor produces the
/// *same* answer, so there is nothing for a reader to average across repeated
/// queries and no range to narrow.
pub fn coarsen(count: i64, floor: i64) -> Coarsened {
    if count >= floor {
        Coarsened::Exact(count)
    } else {
        Coarsened::Below(floor)
    }
}

/// How fresh a number is, and therefore whether it can be cached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// The viewer's own history and credit balance.
    RealTime,
    /// The viewer's own reading counts. Minutes.
    NearRealTime,
    /// Work-level aggregates. Hourly.
    Hourly,
    /// Community dashboards, cohorts, co-occurrence. Daily.
    Daily,
}

impl Freshness {
    pub fn as_str(self) -> &'static str {
        match self {
            Freshness::RealTime => "real-time",
            Freshness::NearRealTime => "near-real-time (minutes)",
            Freshness::Hourly => "hourly",
            Freshness::Daily => "daily",
        }
    }

    /// Whether a response may be cached and keyed without a reader's identity.
    ///
    /// A personal number at near-real-time freshness is the dangerous case: a
    /// shared cache serving one reader's count to another is a disclosure that
    /// no test at the SQL layer would catch.
    pub fn cacheable_without_identity(self) -> bool {
        matches!(self, Freshness::Hourly | Freshness::Daily)
    }
}

/// How a metric is computed — the five things §24.2 requires of every metric.
///
/// A capability whose `definition` does not name what it counts is not a
/// definition, and the registry test rejects anything under twenty characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Method {
    /// The formula, in words, naming its denominator.
    pub definition: &'static str,
    /// How often it is recomputed.
    pub freshness: Freshness,
    /// What is estimated, and by how much, or that it is exact.
    pub approximation: &'static str,
}

impl Method {
    const fn new(
        definition: &'static str,
        freshness: Freshness,
        approximation: &'static str,
    ) -> Self {
        Self {
            definition,
            freshness,
            approximation,
        }
    }
}

/// Which grant a capability needs beyond a trust level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authority {
    /// Anyone at the right trust level.
    Trust,
    /// Requires the trustee role as well — fiduciary data.
    Trustee,
    /// Requires the admin role. A role grant, never a score.
    Admin,
}

/// A named analytics capability.
///
/// The name is the stable identifier. It appears in routes, in the admin UI,
/// and in the tests, and it is what makes "did we ship that metric" a question
/// with an answerable yes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    // -- the reader's own reading -------------------------------------------
    /// Words, chapters, streak and session length.
    OwnReadingBasic,
    /// Fandom, tag and mood distribution of own reads.
    OwnReadingDistribution,
    /// Reads per week over time.
    OwnReadingTrend,
    /// Works read more than once.
    OwnReadingReread,
    /// Qualitative resonance label. Never the score.
    OwnResonanceLabel,
    /// Own percentile against the instance median.
    OwnReadingPercentile,
    /// Own imports, translations, fulfilled wishes.
    OwnContributionHistory,

    // -- the reader's own works ---------------------------------------------
    /// Reader count, word count, chapter count.
    OwnWorkBasic,
    /// Quick-reaction counts, labels with at least five uses.
    OwnWorkReactions,
    /// Per-chapter drop-off curve.
    OwnWorkRetention,
    /// Aggregate seconds per chapter.
    OwnWorkTimeOnPage,
    /// Histogram of chapter number at first bookmark.
    OwnWorkBookmarkTiming,
    /// Early-versus-late comment distribution.
    OwnWorkCommentTiming,
    /// "Reached chapter X" counters.
    OwnWorkSegments,
    /// Dominant moods in comments and reactions.
    OwnWorkMood,
    /// Search impressions and feed appearances.
    OwnWorkDiscoverability,
    /// This work against the reader's other works.
    OwnWorkPortfolio,
    /// "Readers of this also enjoyed."
    OwnWorkCoBookmark,
    /// Return-after-weeks percentage.
    OwnWorkLongRetention,
    /// Series completion after part one.
    OwnWorkSeriesCascade,

    // -- public ------------------------------------------------------------
    /// Word and chapter count, publication date.
    PublicWorkBasic,
    /// Mean and count. Never a histogram (§9.4).
    PublicWorkRating,
    /// Public bookmark count.
    PublicWorkBookmarks,
    /// Public reaction aggregate, if the author permits.
    PublicWorkReactions,
    /// Works per fandom.
    PublicFandomCounts,
    /// Instance totals (§24.2).
    PublicInstanceCounts,

    // -- community ---------------------------------------------------------
    /// Which fandoms and tags are trending. A flag, never a magnitude.
    CommunityTrending,
    /// "Others also bookmarked", on the viewer's own bookmarks.
    CommunityBookmarkCf,
    /// Per-fandom new works per day, readers per day.
    CommunityFandomDash,
    /// Which tags co-occur.
    CommunityTagCooccurrence,
    /// Popular and zero-result queries.
    CommunitySearchTrends,
    /// Instance words read today.
    CommunityInstanceActivity,
    /// Growth rate, author count, tag emergence.
    CommunityFandomGrowth,
    /// Reader overlap between fandoms.
    CommunityCrossFandom,
    /// Refinement and abandonment rates.
    CommunityQueryAnalysis,
    /// Engagement for collections the viewer curates.
    CommunityCollectionPerf,

    // -- stewards ----------------------------------------------------------
    /// Case volume by category, resolution times.
    StewardModerationQueue,
    /// Classifier precision and recall.
    StewardPositivityPerf,
    /// Counts per trust level.
    StewardTrustDistribution,
    /// Sanction types and durations, aggregate.
    StewardSanctions,
    /// Duplicate-detection rates.
    StewardDuplication,
    /// Success and failure by source and adapter.
    StewardImportQuality,
    /// Click-through by engine, diversity slots.
    StewardDiscoveryPerf,

    // -- strategy ----------------------------------------------------------
    /// Retention by signup cohort.
    StrategyCohorts,
    /// Credit flows per category, cap distribution.
    StrategyEconomyFlows,
    /// Inbound and outbound announcement volumes.
    StrategyFederation,
    /// Request-to-delivery times.
    StrategyTranslationThroughput,

    // -- trustees ----------------------------------------------------------
    /// Health, storage, backup, migration state.
    TrusteeOperations,
    /// Aggregate revenue, cost, payout.
    TrusteeFinancials,
    /// Aggregate decision and quorum counts.
    TrusteeAuditTrails,
    /// Break-glass frequency and resolution.
    TrusteeBreakglass,

    // -- admin (a role, not a level) ---------------------------------------
    /// §16.2 dimensions, exemplars, anti-examples.
    AdminTasteProfile,
    /// Complete audit trail, with the admin's own access logged.
    AdminModerationAudit,
    /// Explicit lookup, audit-logged.
    AdminUserLookup,
}

impl Scope {
    /// The stable name. Appears in routes, in the admin UI, and in tests.
    pub fn as_str(self) -> &'static str {
        use Scope::*;
        match self {
            OwnReadingBasic => "own.reading.basic",
            OwnReadingDistribution => "own.reading.distribution",
            OwnReadingTrend => "own.reading.trend",
            OwnReadingReread => "own.reading.reread",
            OwnResonanceLabel => "own.resonance.label",
            OwnReadingPercentile => "own.reading.percentile",
            OwnContributionHistory => "own.contribution.history",

            OwnWorkBasic => "own.work.basic",
            OwnWorkReactions => "own.work.reactions",
            OwnWorkRetention => "own.work.retention",
            OwnWorkTimeOnPage => "own.work.time_on_page",
            OwnWorkBookmarkTiming => "own.work.bookmark_timing",
            OwnWorkCommentTiming => "own.work.comment_timing",
            OwnWorkSegments => "own.work.segments",
            OwnWorkMood => "own.work.mood",
            OwnWorkDiscoverability => "own.work.discoverability",
            OwnWorkPortfolio => "own.work.portfolio",
            OwnWorkCoBookmark => "own.work.co_bookmark",
            OwnWorkLongRetention => "own.work.long_retention",
            OwnWorkSeriesCascade => "own.work.series_cascade",

            PublicWorkBasic => "public.work.basic",
            PublicWorkRating => "public.work.rating",
            PublicWorkBookmarks => "public.work.bookmarks",
            PublicWorkReactions => "public.work.reactions",
            PublicFandomCounts => "public.fandom.counts",
            PublicInstanceCounts => "public.instance.counts",

            CommunityTrending => "community.trending",
            CommunityBookmarkCf => "community.bookmark_cf",
            CommunityFandomDash => "community.fandom_dash",
            CommunityTagCooccurrence => "community.tag_cooccurrence",
            CommunitySearchTrends => "community.search_trends",
            CommunityInstanceActivity => "community.instance_activity",
            CommunityFandomGrowth => "community.fandom_growth",
            CommunityCrossFandom => "community.cross_fandom",
            CommunityQueryAnalysis => "community.query_analysis",
            CommunityCollectionPerf => "community.collection_perf",

            StewardModerationQueue => "steward.moderation_queue",
            StewardPositivityPerf => "steward.positivity_perf",
            StewardTrustDistribution => "steward.trust_distribution",
            StewardSanctions => "steward.sanctions",
            StewardDuplication => "steward.duplication",
            StewardImportQuality => "steward.import_quality",
            StewardDiscoveryPerf => "steward.discovery_perf",

            StrategyCohorts => "strategy.cohorts",
            StrategyEconomyFlows => "strategy.economy_flows",
            StrategyFederation => "strategy.federation",
            StrategyTranslationThroughput => "strategy.translation_throughput",

            TrusteeOperations => "trustee.operations",
            TrusteeFinancials => "trustee.financials",
            TrusteeAuditTrails => "trustee.audit_trails",
            TrusteeBreakglass => "trustee.breakglass",

            AdminTasteProfile => "admin.taste_profile",
            AdminModerationAudit => "admin.moderation_audit",
            AdminUserLookup => "admin.user_lookup",
        }
    }

    /// Parse a name. Used by the routes, so a typo is a 404 rather than a
    /// default-allowed capability.
    pub fn parse(name: &str) -> Option<Self> {
        ALL_SCOPES.iter().copied().find(|s| s.as_str() == name)
    }

    /// The minimum trust level, on the §19.1 ladder.
    pub fn minimum_trust(self) -> i64 {
        use Scope::*;
        match self {
            // TL0
            OwnReadingBasic | OwnWorkBasic | PublicWorkBasic | PublicWorkRating
            | PublicWorkBookmarks | PublicWorkReactions | PublicFandomCounts
            | PublicInstanceCounts => TL_NEW,

            // TL1
            OwnReadingDistribution
            | OwnReadingTrend
            | OwnReadingReread
            | OwnResonanceLabel
            | OwnWorkReactions
            | OwnWorkRetention
            | OwnWorkTimeOnPage
            | OwnWorkBookmarkTiming
            | OwnWorkCommentTiming
            | CommunityTrending
            | CommunityBookmarkCf => TL_ESTABLISHED,

            // TL2
            OwnReadingPercentile
            | OwnContributionHistory
            | OwnWorkSegments
            | OwnWorkMood
            | OwnWorkDiscoverability
            | OwnWorkPortfolio
            | CommunityFandomDash
            | CommunityTagCooccurrence
            | CommunitySearchTrends
            | CommunityInstanceActivity => TL_REGULAR,

            // TL3
            OwnWorkCoBookmark
            | OwnWorkLongRetention
            | OwnWorkSeriesCascade
            | CommunityFandomGrowth
            | CommunityCrossFandom
            | CommunityQueryAnalysis
            | CommunityCollectionPerf => TL_REVIEWED,

            // TL4
            StewardModerationQueue
            | StewardPositivityPerf
            | StewardTrustDistribution
            | StewardSanctions
            | StewardDuplication
            | StewardImportQuality
            | StewardDiscoveryPerf => TL_STEWARD,

            // TL5
            StrategyCohorts
            | StrategyEconomyFlows
            | StrategyFederation
            | StrategyTranslationThroughput => TL_SENIOR,

            // TL6
            TrusteeOperations | TrusteeFinancials | TrusteeAuditTrails | TrusteeBreakglass => {
                TL_TRUSTEE
            }

            // Role-gated. A level is not enough; see `Authority`.
            AdminTasteProfile | AdminModerationAudit | AdminUserLookup => TRUST_MAX,
        }
    }

    /// What the capability is about, which decides the floor.
    pub fn subject(self) -> Subject {
        use Scope::*;
        match self {
            OwnReadingBasic
            | OwnReadingDistribution
            | OwnReadingTrend
            | OwnReadingReread
            | OwnResonanceLabel
            | OwnReadingPercentile
            | OwnContributionHistory
            | OwnWorkReactions
            | OwnWorkRetention
            | OwnWorkTimeOnPage
            | OwnWorkBookmarkTiming
            | OwnWorkCommentTiming
            | OwnWorkSegments
            | OwnWorkMood
            | OwnWorkDiscoverability
            | OwnWorkPortfolio
            | OwnWorkCoBookmark
            | OwnWorkLongRetention
            | OwnWorkSeriesCascade => Subject::Self_,

            _ => Subject::Other,
        }
    }

    /// The floor, or `None` when the capability is a single fact rather than a
    /// count over people — a work's word count is not a k-anonymity problem.
    pub fn floor(self) -> Option<i64> {
        use Scope::*;
        match self {
            // Single-entity facts. No population, so no floor.
            PublicWorkBasic | PublicInstanceCounts | PublicFandomCounts | OwnWorkPortfolio
            | OwnReadingReread | OwnWorkSegments | OwnWorkSeriesCascade => None,

            _ => Some(floor_for(self.subject())),
        }
    }

    /// The extra grant, if any.
    pub fn authority(self) -> Authority {
        use Scope::*;
        match self {
            AdminTasteProfile | AdminModerationAudit | AdminUserLookup => Authority::Admin,
            // Fiduciary. §20.10.3 cap distribution and aggregate revenue are
            // trustee business, not a reward for seniority.
            StrategyEconomyFlows
            | StrategyCohorts
            | StrategyFederation
            | StrategyTranslationThroughput
            | TrusteeOperations
            | TrusteeFinancials
            | TrusteeAuditTrails
            | TrusteeBreakglass => Authority::Trustee,
            _ => Authority::Trust,
        }
    }

    /// How the number is computed.
    pub fn method(self) -> Method {
        use Scope::*;
        match self {
            OwnReadingBasic => Method::new(
                "Words and chapters the viewer's own reading events completed, summed over the window; session length is wall-clock between two progress updates on the same chapter, capped at 30 minutes per gap.",
                Freshness::NearRealTime,
                "Session length and pace are estimates; the 30-minute cap per gap is the disclosed approximation.",
            ),
            OwnReadingDistribution => Method::new(
                "Share of the viewer's finished works by fandom, tag and mood, over the window; the denominator is finished works, not opens.",
                Freshness::Hourly,
                "Exact given the reader's own history; taxonomy labels are as stored.",
            ),
            OwnReadingTrend => Method::new(
                "Words and chapters completed per ISO week over the trailing window, divided by the number of weeks in the window.",
                Freshness::Hourly,
                "Exact. A partial current week is shown in full and is not averaged down.",
            ),
            OwnReadingReread => Method::new(
                "Works the viewer opened on two or more distinct dates at least a day apart; the denominator is works started.",
                Freshness::Daily,
                "A re-read interrupted by a day is not counted; that is a deliberate undercount.",
            ),
            OwnResonanceLabel => Method::new(
                "The scalar projection of the viewer's taste vector against the instance taste profile, bucketed: below 0.4 Developing, below 0.7 Moderate, otherwise Strong.",
                Freshness::Daily,
                "The score itself is never returned. The bucket thresholds are the whole disclosure.",
            ),
            OwnReadingPercentile => Method::new(
                "The viewer's own rank against the instance median for the same window, by words read. Never shown to anyone else.",
                Freshness::Daily,
                "Exact rank within the active-reader set; the set is defined by §24.2's 'active contributor'.",
            ),
            OwnContributionHistory => Method::new(
                "Counts of the viewer's own imports, translations, fulfilled wishlist items and reviews over the window.",
                Freshness::Hourly,
                "Exact.",
            ),
            OwnWorkBasic => Method::new(
                "Distinct pseudonyms with a reading event on this work in the window, plus its word and chapter counts.",
                Freshness::Hourly,
                "Reader count is distinct pseudonyms, not sessions and not accounts.",
            ),
            OwnWorkReactions => Method::new(
                "Distinct readers per quick-reaction label; a reader reacting forty times counts once, and labels with fewer than five uses are not returned.",
                Freshness::NearRealTime,
                "Exact. The per-label floor is the disclosure, and it is why a new work shows nothing.",
            ),
            OwnWorkRetention => Method::new(
                "Per chapter, readers who reached it divided by readers who reached chapter one; non-increasing by construction.",
                Freshness::Hourly,
                "Exact from reading events. A reader who opened the work page and left is not a denominator.",
            ),
            OwnWorkTimeOnPage => Method::new(
                "Wall-clock between consecutive progress updates on the same chapter, summed, capped at 30 minutes per gap.",
                Freshness::Hourly,
                "Estimated. A reader who closed the tab without a final update contributes nothing for that chapter.",
            ),
            OwnWorkBookmarkTiming => Method::new(
                "Chapter number at each reader's first bookmark, bucketed by chapter, with at least five readers per bucket.",
                Freshness::Daily,
                "Exact. Buckets below the floor are omitted rather than merged upward.",
            ),
            OwnWorkCommentTiming => Method::new(
                "Publication date to comment date, bucketed into early and late halves, over the window.",
                Freshness::Daily,
                "Exact. 'Early' is relative to this work's own publication, not an instance-wide constant.",
            ),
            OwnWorkSegments => Method::new(
                "Cumulative distinct readers who reached each chapter, over the window, at the self-or-others floor.",
                Freshness::Hourly,
                "Exact.",
            ),
            OwnWorkMood => Method::new(
                "Share of the viewer's readers whose own history is dominated by each mood tag, among readers of this work, at the others floor.",
                Freshness::Daily,
                "Inferred from each reader's own reading history, not from their reactions. A reader with no history is excluded.",
            ),
            OwnWorkDiscoverability => Method::new(
                "Times this work appeared in a search result or a feed slot, over the window, regardless of whether it was clicked.",
                Freshness::Hourly,
                "Impressions are counted, not clicks-through. A search that returns this work twice counts twice.",
            ),
            OwnWorkPortfolio => Method::new(
                "This work's own-work metrics against the viewer's other works, compared on the same window and the same floor.",
                Freshness::Hourly,
                "Exact. The comparison is between a viewer's own works, which §9.6 already lets them see.",
            ),
            OwnWorkCoBookmark => Method::new(
                "Other works appearing in the bookmark sets of at least ten readers who also bookmarked this one, top ten by shared readers.",
                Freshness::Daily,
                "Requires at least ten readers on this work first; below that the whole capability is withheld rather than coarsened.",
            ),
            OwnWorkLongRetention => Method::new(
                "Share of this work's readers who opened it again at least seven days after their first chapter, over the window.",
                Freshness::Daily,
                "Exact. A reader who finished once is counted as retained only on a second distinct session.",
            ),
            OwnWorkSeriesCascade => Method::new(
                "Readers of part one who went on to read the final part of the same series, divided by readers of part one.",
                Freshness::Daily,
                "Exact. Series membership is the taxonomy parent, not a manual list.",
            ),
            PublicWorkBasic => Method::new(
                "The work's own stored word count, chapter count and publication date.",
                Freshness::Daily,
                "Exact. A single-entity fact, so no population floor applies.",
            ),
            PublicWorkRating => Method::new(
                "Mean private rating and the count of raters, shown only where the author has enabled public display.",
                Freshness::Hourly,
                "Mean and count only. A histogram would expose a rater's exact choice (§9.4).",
            ),
            PublicWorkBookmarks => Method::new(
                "Count of bookmarks on this work that are themselves public, over the window.",
                Freshness::Hourly,
                "Private shelves are excluded, so the number understates a popular work's real reach.",
            ),
            PublicWorkReactions => Method::new(
                "Count of quick reactions on this work, shown only where the author permits public aggregates.",
                Freshness::NearRealTime,
                "Distinct readers per label, floored.",
            ),
            PublicFandomCounts => Method::new(
                "Count of public works tagged with each fandom; a per-tag count, not a per-reader one.",
                Freshness::Daily,
                "Exact at the public-works floor. Alias-spelled tags roll up to one canonical fandom.",
            ),
            PublicInstanceCounts => Method::new(
                "Instance totals per §24.2: public works, public chapters and words, active public contributors, translation coverage by language.",
                Freshness::Daily,
                "Counts of public content only. Private imports are never counted as holdings.",
            ),
            CommunityTrending => Method::new(
                "Fandoms and tags whose 7-day rate exceeds two standard deviations of their own trailing 30-day baseline.",
                Freshness::Daily,
                "A boolean, never a magnitude. A magnitude is a leaderboard.",
            ),
            CommunityBookmarkCf => Method::new(
                "For a work the viewer has bookmarked, other works bookmarked by at least five of the same readers.",
                Freshness::Daily,
                "Requires the viewer to have bookmarked it, and at least five co-bookmarkers, or the capability is withheld.",
            ),
            CommunityFandomDash => Method::new(
                "Per fandom per day: new public works, active readers, and the mood tags most applied that week.",
                Freshness::Daily,
                "Active readers is §24.2's definition, not a session count.",
            ),
            CommunityTagCooccurrence => Method::new(
                "Tag pairs appearing together on a public work, ranked by the count of works carrying both.",
                Freshness::Daily,
                "Public data only, so exact for public works. Pairs under the floor are omitted.",
            ),
            CommunitySearchTrends => Method::new(
                "Query terms appearing in at least five searches over the window, and terms returning no results, at the others floor.",
                Freshness::Daily,
                "Raw queries are never retained (§24.3). Terms are normalised to the field or tag they resolve to, and a term that resolves to a single work is not returned at all.",
            ),
            CommunityInstanceActivity => Method::new(
                "Instance words read, chapters completed and works finished today, as rolling sums over reading events.",
                Freshness::Hourly,
                "Exact. A rolling day, not a calendar day, so it does not spike at midnight.",
            ),
            CommunityFandomGrowth => Method::new(
                "Per fandom: public work growth rate over the window, active author count, and tags first seen in it.",
                Freshness::Daily,
                "Growth is a rate over the window, never an all-time total (§9.7.1).",
            ),
            CommunityCrossFandom => Method::new(
                "Pairs of fandoms whose readers overlap above the others floor, as a share of the smaller fandom's active readers.",
                Freshness::Daily,
                "Computed on pseudonyms, so a reader with several pseudonyms counts once per pseud.",
            ),
            CommunityQueryAnalysis => Method::new(
                "Share of searches followed by a refinement, and the share abandoned with no click, over the window, at the others floor.",
                Freshness::Daily,
                "Session-scoped within one request chain; no chain is retained beyond the aggregation.",
            ),
            CommunityCollectionPerf => Method::new(
                "Reads and completions attributable to a collection the viewer curates, over the window.",
                Freshness::Daily,
                "Attribution is by referral category, not by a tracked reader.",
            ),
            StewardModerationQueue => Method::new(
                "Case counts by category and resolution-time distribution over the window, aggregate.",
                Freshness::Hourly,
                "No per-case detail and no reporter identity; a case is counted, never listed.",
            ),
            StewardPositivityPerf => Method::new(
                "Classifier precision and recall against adjudicated outcomes, plus appeal outcomes, over the window.",
                Freshness::Daily,
                "Computed against adjudicated cases only. Un-adjudicated classifier output is not ground truth.",
            ),
            StewardTrustDistribution => Method::new(
                "Count of accounts at each trust level, over the window.",
                Freshness::Daily,
                "Counts only. A level is a governance record, not a statistic about a person.",
            ),
            StewardSanctions => Method::new(
                "Sanction counts by type and duration band, over the window, aggregate.",
                Freshness::Daily,
                "Never who received one. Duration is banded, not exact, so a single case is not identifiable by its unusual length.",
            ),
            StewardDuplication => Method::new(
                "Share of import batches in which a duplicate was identified, by source.",
                Freshness::Daily,
                "Detected-duplicate share, not the duplicate titles.",
            ),
            StewardImportQuality => Method::new(
                "Import success and failure rates by source and adapter version over the window.",
                Freshness::Daily,
                "Exact on outcomes; a source's failure reason is not attributed to a specific importer.",
            ),
            StewardDiscoveryPerf => Method::new(
                "Impression-to-click ratio per discovery engine, and how often a diversity slot is engaged with, over the window.",
                Freshness::Daily,
                "Aggregate per engine. §0.3's admin-taste invisibility means no engine's weighting is shown.",
            ),
            StrategyCohorts => Method::new(
                "Retention by signup cohort, each cohort compared to the cohorts around it. No single cohort's raw total is returned.",
                Freshness::Daily,
                "All-time is not offered (§9.7.1): a cohort's own cumulative number is a leaderboard of signup dates.",
            ),
            StrategyEconomyFlows => Method::new(
                "Credits earned and spent per category over the window, and the count of authors in each cap band (§20.10.3).",
                Freshness::Daily,
                "Flows are aggregate. Individual earnings and balances are never included, including for an author viewing their own.",
            ),
            StrategyFederation => Method::new(
                "Inbound and outbound announcement and activity counts per peer, over the window.",
                Freshness::Daily,
                "Volumes, not content. A peer's traffic is not a reader's.",
            ),
            StrategyTranslationThroughput => Method::new(
                "Time from translation request to delivery, and review depth per request, over the window.",
                Freshness::Daily,
                "Median and quartiles, never a maximum — a maximum is one unusually slow requester.",
            ),
            TrusteeOperations => Method::new(
                "Server health, storage growth, backup recency and applied-migration state, read from the instance's own telemetry.",
                Freshness::RealTime,
                "Live. A stale health number is worse than no health number, so this one is never cached.",
            ),
            TrusteeFinancials => Method::new(
                "Aggregate revenue, aggregate cost and aggregate payout over the window, by category.",
                Freshness::Daily,
                "Aggregates only. No individual transaction, payout or subscriber appears at any trust level.",
            ),
            TrusteeAuditTrails => Method::new(
                "Decision counts, quorum patterns and appeal outcomes over the window, aggregate.",
                Freshness::Daily,
                "Counts, not cases. A per-case trail is §24.4 security telemetry, not analytics.",
            ),
            TrusteeBreakglass => Method::new(
                "Break-glass access events per window: frequency, resolving role, and resolution status, aggregate.",
                Freshness::Daily,
                "Roles are counted, not named. Who used an emergency power is not a statistic a dashboard should keep.",
            ),
            AdminTasteProfile => Method::new(
                "The instance taste profile's dimensions, exemplars and anti-examples as configured (§16.2).",
                Freshness::RealTime,
                "Visible to admin only, and every read of it is audit-logged (§24.3 legal basis).",
            ),
            AdminModerationAudit => Method::new(
                "The complete moderation audit trail, including the admin's own accesses.",
                Freshness::NearRealTime,
                "Complete, so not an aggregate — which is why it is admin-only and access-logged rather than floored.",
            ),
            AdminUserLookup => Method::new(
                "One account's data, retrieved by explicit lookup for a named operational reason, with the lookup itself logged.",
                Freshness::RealTime,
                "Not an aggregate and not floored; the control is the audit log and the requirement for a stated reason.",
            ),
        }
    }

    /// Whether the response must not be cached without a reader identity.
    pub fn requires_ttl(self) -> bool {
        !self.method().freshness.cacheable_without_identity()
    }

    /// Whether this is about the viewer's own data.
    pub fn is_personal(self) -> bool {
        self.subject() == Subject::Self_
    }
}

/// Every capability, in trust order.
///
/// The order is part of the contract: a client renders in this order without
/// sorting, and two clients therefore agree about what a capability list is.
pub const ALL_SCOPES: &[Scope] = &[
    // TL0
    Scope::OwnReadingBasic,
    Scope::OwnWorkBasic,
    Scope::PublicWorkBasic,
    Scope::PublicWorkRating,
    Scope::PublicWorkBookmarks,
    Scope::PublicWorkReactions,
    Scope::PublicFandomCounts,
    Scope::PublicInstanceCounts,
    // TL1
    Scope::OwnReadingDistribution,
    Scope::OwnReadingTrend,
    Scope::OwnReadingReread,
    Scope::OwnResonanceLabel,
    Scope::OwnWorkReactions,
    Scope::OwnWorkRetention,
    Scope::OwnWorkTimeOnPage,
    Scope::OwnWorkBookmarkTiming,
    Scope::OwnWorkCommentTiming,
    Scope::CommunityTrending,
    Scope::CommunityBookmarkCf,
    // TL2
    Scope::OwnReadingPercentile,
    Scope::OwnContributionHistory,
    Scope::OwnWorkSegments,
    Scope::OwnWorkMood,
    Scope::OwnWorkDiscoverability,
    Scope::OwnWorkPortfolio,
    Scope::CommunityFandomDash,
    Scope::CommunityTagCooccurrence,
    Scope::CommunitySearchTrends,
    Scope::CommunityInstanceActivity,
    // TL3
    Scope::OwnWorkCoBookmark,
    Scope::OwnWorkLongRetention,
    Scope::OwnWorkSeriesCascade,
    Scope::CommunityFandomGrowth,
    Scope::CommunityCrossFandom,
    Scope::CommunityQueryAnalysis,
    Scope::CommunityCollectionPerf,
    // TL4
    Scope::StewardModerationQueue,
    Scope::StewardPositivityPerf,
    Scope::StewardTrustDistribution,
    Scope::StewardSanctions,
    Scope::StewardDuplication,
    Scope::StewardImportQuality,
    Scope::StewardDiscoveryPerf,
    // TL5
    Scope::StrategyCohorts,
    Scope::StrategyEconomyFlows,
    Scope::StrategyFederation,
    Scope::StrategyTranslationThroughput,
    // TL6
    Scope::TrusteeOperations,
    Scope::TrusteeFinancials,
    Scope::TrusteeAuditTrails,
    Scope::TrusteeBreakglass,
    // Role-gated
    Scope::AdminTasteProfile,
    Scope::AdminModerationAudit,
    Scope::AdminUserLookup,
];

/// Names that are never capabilities, and the clause that forbids each.
///
/// This is the "never shown" list from the design, expressed as data. A scope
/// with one of these names does not exist, so a route referencing one does not
/// compile. Deleting an entry is a deliberate edit to a diff rather than an
/// omission, which is the point of writing it down.
pub const FORBIDDEN_SCOPE_NAMES: &[(&str, &str)] = &[
    ("ab.variant_assignment", "§24.6"),
    ("ab.signal_weights", "§0.3, §9.7.1"),
    ("ab.resonance_numeric", "own-only, label form only"),
    ("ab.pseud_linkage", "§7.2"),
    ("ab.other_reading_history", "§9.5"),
    ("ab.other_credit_balance", "§20.1"),
    ("ab.other_earnings", "aggregate only"),
    ("ab.shadowban_state", "§19.6"),
    ("ab.vanguard_reason", "§16.18"),
    ("ab.session_identifiers", "§24.3"),
    ("ab.feature_flags", "instance config"),
    ("ab.individual_queries", "§24.3"),
    ("ab.comment_scores", "aggregate performance only"),
];

/// An operational role, as distinct from a trust level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// No role. A reader, at whatever trust level.
    Reader,
    /// A fiduciary (§19.1 TL6).
    Trustee,
    /// An instance administrator. Not a trust level.
    Admin,
}

/// The instance preset (§0.6). An upper bound on capabilities, never a floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Curated. Public counters and personal stats only.
    Gallery,
    /// The standard set this amendment describes.
    Archive,
    /// Wider aggregate access at lower trust levels. A ceiling, not a floor.
    Commons,
    /// Author analytics and public counts. Minimal community exposure.
    Showcase,
    /// Development. No upper bound.
    Sandbox,
}

impl Preset {
    pub fn as_str(self) -> &'static str {
        match self {
            Preset::Gallery => "gallery",
            Preset::Archive => "archive",
            Preset::Commons => "commons",
            Preset::Showcase => "showcase",
            Preset::Sandbox => "sandbox",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "gallery" => Some(Preset::Gallery),
            "archive" => Some(Preset::Archive),
            "commons" => Some(Preset::Commons),
            "showcase" => Some(Preset::Showcase),
            "sandbox" => Some(Preset::Sandbox),
            _ => None,
        }
    }

    /// The namespace this preset withholds.
    ///
    /// Deliberately coarse: a per-capability preset table would be a second
    /// matrix to keep in step with the first, and the two would drift.
    fn withholds(self, scope: Scope) -> bool {
        let name = scope.as_str();
        match self {
            // Nothing withheld.
            Preset::Sandbox | Preset::Archive => false,
            // A curated instance does not want a trending dashboard. The
            // personal surface stays: a reader's own history is theirs.
            Preset::Gallery => {
                name.starts_with("community.")
                    || name.starts_with("steward.")
                    || name.starts_with("strategy.")
                    || name.starts_with("trustee.")
            }
            // The same, and also the per-fandom activity feed, which is the one
            // community surface an author-facing instance actively wants.
            Preset::Showcase => name.starts_with("community.") || name.starts_with("steward."),
            // Commons widens who sees a *lower* trust level's capabilities. It
            // never raises a ceiling — see `a_preset_never_raises_a_trust_ceiling`.
            Preset::Commons => false,
        }
    }
}

/// Whether a viewer may see a capability, under the standard preset.
pub fn allowed(scope: Scope, trust_level: i64) -> bool {
    allowed_with_role(scope, trust_level, Role::Reader, Preset::Archive)
}

/// Whether a viewer may see a capability, given a role.
pub fn allowed_with_role(scope: Scope, trust_level: i64, role: Role, preset: Preset) -> bool {
    allowed_under(scope, trust_level, role, preset)
}

/// The gate.
///
/// Four checks, in the order that fails cheapest: the preset's upper bound,
/// the role grant, the trust floor, and finally the admin convenience that an
/// admin sees everything a reader does. The last one is a subset of the first
/// three by construction — an admin is at `TRUST_MAX` — so it adds nothing; it
/// is a `|| is_admin` in one place rather than in every route.
pub fn allowed_under(scope: Scope, trust_level: i64, role: Role, preset: Preset) -> bool {
    // Admin still respects the trust floor. The convenience is that an admin
    // sits at TRUST_MAX in practice, not that the check is skipped.
    if preset.withholds(scope) {
        return false;
    }
    match scope.authority() {
        Authority::Admin => role == Role::Admin && trust_level >= scope.minimum_trust(),
        Authority::Trustee => {
            matches!(role, Role::Trustee | Role::Admin) && trust_level >= scope.minimum_trust()
        }
        Authority::Trust => trust_level >= scope.minimum_trust(),
    }
}

/// The capabilities a viewer may see, in registry order.
///
/// This is the function a dashboard calls. Iterating *this* rather than
/// iterating all scopes and filtering is what makes an unimplemented
/// capability invisible instead of an unfiltered leak.
pub fn visible_to(trust_level: i64, role: Role, preset: Preset) -> Vec<Scope> {
    ALL_SCOPES
        .iter()
        .copied()
        .filter(|s| allowed_under(*s, trust_level, role, preset))
        .collect()
}

/// A count, ready for a response, with its suppression already applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(untagged)]
pub enum Reported {
    /// At or above the floor.
    Exact(i64),
    /// Below the floor. The client renders "fewer than N", never a number.
    BelowFloor { fewer_than: i64 },
}

impl Reported {
    /// Apply the floor and produce the wire form.
    pub fn of(count: i64, scope: Scope) -> Self {
        match scope.floor() {
            // No population, so no floor: a word count of 3 is a true fact.
            None => Reported::Exact(count),
            Some(floor) => match coarsen(count, floor) {
                Coarsened::Exact(n) => Reported::Exact(n),
                Coarsened::Below(f) => Reported::BelowFloor { fewer_than: f },
            },
        }
    }
}
