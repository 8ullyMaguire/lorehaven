//! `lore_metadata` — the shared exchange contract (spec §2.3.1, §11.17).
//!
//! Wire types only: `serde`, no database, no IO, no HTTP client. Two sides
//! compile against these, and a compatible third-party implementation never has
//! to adopt Lorehaven's database. Nothing in `lorehaven-app` or `lorehaven-db`
//! depends on this crate, so it can be published at any point in the build
//! order without waiting for the rest of the architecture.
//!
//! # The field list *is* the privacy policy
//!
//! §0.3 forbids a metadata signal from being a fact about a reader, and §11.17
//! says how that is enforced: *"a field that does not exist cannot be
//! configured into existence."* So the shape of [`WorkSignal`] is the whole
//! mechanism. Reading history, progress, position, reading status, ratings,
//! notes, kudos, library membership, pseud linkage, draft content, source
//! credentials, session identifiers, IP addresses and file paths are not fields
//! on the type, which means a conforming client cannot send them and a
//! non-conforming one is refused by name rather than having its payload quietly
//! trimmed.
//!
//! This is why the struct uses `deny_unknown_fields` on the *batch* types
//! rather than tolerating extras. A lenient deserializer is the loophole that
//! turns "the schema refuses everything else" into "the schema ignores
//! everything else", and a silently-dropped `reader_id` is worse than a
//! rejected one: the sender believes it was transmitted.
//!
//! # Version negotiation
//!
//! [`ExchangeVersion`] is a major version, and negotiation is explicit through
//! [`ExchangeVersion::negotiate`]. A client that cannot agree a version is
//! refused *with the range the server supports*, rather than being allowed to
//! guess and discover the mismatch by losing data. The range is carried in the
//! refusal precisely so the caller can recover without a second round trip.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// The major version of the exchange contract.
///
/// A major version, not a semver string: these types are compiled against, and
/// adding a field is a compatible change while renaming or removing one is
/// not. Callers pin a version they were built against and negotiate.
pub const EXCHANGE_MAJOR: u16 = 1;

/// A version of the exchange contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExchangeVersion(pub u16);

impl ExchangeVersion {
    /// The version this build of the crate speaks.
    pub const CURRENT: Self = Self(EXCHANGE_MAJOR);

    /// The lowest version this build can still read.
    pub const MIN_SUPPORTED: Self = Self(EXCHANGE_MAJOR);

    /// The highest version this build can still read.
    pub const MAX_SUPPORTED: Self = Self(EXCHANGE_MAJOR);

    /// The range this build supports, for the `/exchange/version` endpoint and
    /// for a refusal.
    pub fn supported_range() -> VersionRange {
        VersionRange {
            min: Self::MIN_SUPPORTED,
            max: Self::MAX_SUPPORTED,
        }
    }

    /// Agree a version with a peer that asked for `requested`.
    ///
    /// Returns `Ok` with the version to speak, or `Err` carrying the range this
    /// side supports. The error is the recovery path: a client that cannot agree
    /// is told what *is* possible in the same response, so it can pick another
    /// version or give up without a second round trip.
    ///
    /// Asking for a version below [`Self::MIN_SUPPORTED`] is refused even though
    /// the shapes might happen to parse. A type that parses is not the same as a
    /// contract that is honoured, and quietly accepting an old version is how a
    /// deployment ends up serving a shape it no longer means.
    pub fn negotiate(requested: Self) -> Result<Self, UnsupportedVersion> {
        if requested < Self::MIN_SUPPORTED || requested > Self::MAX_SUPPORTED {
            Err(UnsupportedVersion {
                requested,
                supported: Self::supported_range(),
            })
        } else {
            Ok(requested)
        }
    }
}

impl std::fmt::Display for ExchangeVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An inclusive range of exchange versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionRange {
    /// The lowest supported version, inclusive.
    pub min: ExchangeVersion,
    /// The highest supported version, inclusive.
    pub max: ExchangeVersion,
}

impl VersionRange {
    /// Whether `version` falls inside this range.
    pub fn contains(&self, version: ExchangeVersion) -> bool {
        version >= self.min && version <= self.max
    }
}

impl std::fmt::Display for VersionRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}..={}", self.min, self.max)
    }
}

/// A peer asked for a version this side does not speak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedVersion {
    /// What the peer asked for.
    pub requested: ExchangeVersion,
    /// What this side can speak instead.
    pub supported: VersionRange,
}

impl std::fmt::Display for UnsupportedVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "exchange version {} is not supported; this instance speaks {}",
            self.requested, self.supported
        )
    }
}

impl std::error::Error for UnsupportedVersion {}

/// A reference to a canonical entity — a work, an author, a fandom, a tag, a
/// character or a relationship.
///
/// Open rather than a closed set: the exchange is about lore, and lore has more
/// kinds than any enum written today will list. The `kind` is a string so a
/// third party can add one without a version bump, which is the same reason
/// [`WorkSignal`] uses strings for fandom, tag and character names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityRef {
    /// What sort of thing this is: `work`, `author`, `fandom`, `tag`,
    /// `character`, `relationship`, or a kind a third party defines.
    pub kind: String,
    /// The stable identifier for this entity, scoped to its kind.
    pub id: String,
    /// Human-readable names this entity is also known by.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

/// An identifier a work carries on some source site.
///
/// Keyed by site rather than being a free-form map, so a client cannot smuggle
/// a credential in the value: the key set is closed and the value is an opaque
/// identifier, never a token or a URL with a secret in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteIdentifier {
    /// The site, e.g. `ao3`.
    pub site: String,
    /// The site's own identifier for the work.
    pub id: String,
}

/// A fact about a work, offered to the exchange (spec §11.17).
///
/// Every field here is a fact about the *work*. Nothing in this struct is a
/// fact about who holds it, who read it, or who sent it — that is the type
/// doing the work, not a convention. See the module docs.
///
/// `content_rating` is the work's own rating, not a reader's rating of it: a
/// rating left by a reader is reading data and is deliberately absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkSignal {
    /// Identifiers this work carries on other sites.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub site_ids: Vec<SiteIdentifier>,
    /// The work's title, as extracted.
    pub title: String,
    /// Author names on the source. Several, because a work can have several and
    /// because "and" in a single string is not a parseable author list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub author_names: Vec<String>,
    /// The fandom, if the source declared one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fandom: Option<String>,
    /// Tags, unnormalised.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Character names appearing in the work.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub characters: Vec<String>,
    /// Relationship (ship) tags appearing in the work.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<String>,
    /// The work's own content rating, e.g. `general` or `adult`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_rating: Option<String>,
    /// Word count, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_count: Option<u64>,
    /// Chapter count, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_count: Option<u64>,
    /// Whether the work is complete, a work in progress, or abandoned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<Completion>,
    /// A hash of the work's body, for deduplication.
    ///
    /// This is a hash of content, not of a file path and not of a reader's copy,
    /// and §11.17 uses it to make a re-import free: a batch is deduplicated by
    /// content hash before it is stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// The work's language as a BCP 47 tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Where the work was found. A public URL, or absent.
    ///
    /// Free text on purpose rather than a parsed URL: this is a citation, and a
    /// citation that failed to parse is still evidence of where a thing came
    /// from. It is emphatically not a file path — local paths are reading-adjacent
    /// data and are not part of a signal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// When the sender extracted this, RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_at: Option<String>,
}

/// A work's completion state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Completion {
    /// Finished.
    Complete,
    /// Ongoing.
    InProgress,
    /// The author stopped without finishing.
    Abandoned,
}

/// A batch of signals submitted in one call.
///
/// `deny_unknown_fields` is the enforcement point for §11.17's "refused with a
/// named error, not silently truncated": an unknown key is a decode error, so a
/// client that sends `reader_id` is told so rather than having the field
/// dropped while it believes it was sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignalBatch {
    /// The contract version the sender built against.
    pub version: ExchangeVersion,
    /// The signals in this batch.
    pub signals: Vec<WorkSignal>,
}

/// Whether a canonical value passed human quorum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    /// Passed §19.4's human quorum.
    Verified,
    /// Auto-created from signals and not yet reviewed.
    Unverified,
}

/// Community-curated canonical metadata for a work (spec §11.17).
///
/// The response carries no submitter and no holder count. §11.17 is explicit
/// that a count of holders is not computable from what the instance holds, so an
/// endpoint that appeared to offer one would be offering a guess — the type has
/// nowhere to put it.
///
/// `signal_count` is a review priority, and only that. It is never a demand
/// weight (§16.16) and never a count of readers: the first would make metadata
/// submission an attack on ranking, the second would be the holder count this
/// endpoint refuses to serve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalWork {
    /// The canonical work entity.
    pub entity: EntityRef,
    /// The corrected title.
    pub title: String,
    /// The corrected author names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub author_names: Vec<String>,
    /// The corrected fandom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fandom: Option<String>,
    /// The corrected tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// The corrected characters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub characters: Vec<String>,
    /// The corrected relationships.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<String>,
    /// The corrected content rating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_rating: Option<String>,
    /// The corrected word count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_count: Option<u64>,
    /// The corrected completion state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<Completion>,
    /// How many signals contributed. A review priority, nothing else.
    #[serde(default)]
    pub signal_count: u64,
    /// When the value was last curated, RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curated_at: Option<String>,
    /// Whether the value passed human quorum.
    pub review_status: ReviewStatus,
}

/// A batch of canonical works returned in one call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalBatch {
    /// The contract version this response is in.
    pub version: ExchangeVersion,
    /// The canonical values.
    pub works: Vec<CanonicalWork>,
}

impl SignalBatch {
    /// A batch in the version this build speaks.
    pub fn new(signals: Vec<WorkSignal>) -> Self {
        Self {
            version: ExchangeVersion::CURRENT,
            signals,
        }
    }
}

impl CanonicalBatch {
    /// A batch in the version this build speaks.
    pub fn new(works: Vec<CanonicalWork>) -> Self {
        Self {
            version: ExchangeVersion::CURRENT,
            works,
        }
    }
}

impl WorkSignal {
    /// A signal for a work, with only the title set.
    ///
    /// Every other field is optional because a source usually knows some of a
    /// work and not the rest, and a partially-known work is still a fact worth
    /// contributing. The title is the one required field because without it the
    /// signal cannot be matched to anything.
    pub fn titled(title: impl Into<String>) -> Self {
        Self {
            site_ids: Vec::new(),
            title: title.into(),
            author_names: Vec::new(),
            fandom: None,
            tags: Vec::new(),
            characters: Vec::new(),
            relationships: Vec::new(),
            content_rating: None,
            word_count: None,
            chapter_count: None,
            completion: None,
            content_hash: None,
            language: None,
            source_url: None,
            extracted_at: None,
        }
    }

    /// Add a site identifier.
    pub fn on_site(mut self, site: impl Into<String>, id: impl Into<String>) -> Self {
        self.site_ids.push(SiteIdentifier {
            site: site.into(),
            id: id.into(),
        });
        self
    }
}
