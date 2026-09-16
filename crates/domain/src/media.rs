//! M22 domain rules for the generalized media model (spec §32, ADR 0019).
//!
//! Pure validation and enumeration — no I/O. The vocabulary here is the
//! contract the repository and route layers read and write; a value outside
//! these sets must be refused at the edge, not stored and ignored later.

use std::fmt;

macro_rules! vocabulary {
    ($name:ident, $as_str:ident, { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($variant,)+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant,)+];

            pub fn $as_str(&self) -> &'static str {
                match self {
                    $($name::$variant => $text,)+
                }
            }
        }

        impl std::str::FromStr for $name {
            type Err = String;

            /// Inverse of [`$name::as_str`]; the tests pin the round trip.
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($text => Ok($name::$variant),)+
                    _ => Err(format!("unknown {}: {value}", stringify!($name))),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.$as_str())
            }
        }
    };
}

vocabulary!(CreatorKind, as_str, {
    LocalPseud => "local_pseud",
    External => "external",
});

vocabulary!(CreatorRole, as_str, {
    Author => "author",
    Narrator => "narrator",
    Editor => "editor",
    Translator => "translator",
    Artist => "artist",
    Other => "other",
});

vocabulary!(DistributorKind, as_str, {
    Platform => "platform",
    Publisher => "publisher",
    Archive => "archive",
    Zine => "zine",
    Self_ => "self",
});

vocabulary!(DistributionRole, as_str, {
    Published => "published",
    Hosted => "hosted",
    Mirrored => "mirrored",
    Preserved => "preserved",
    Narrated => "narrated",
    Translated => "translated",
    Reprinted => "reprinted",
});

vocabulary!(CollectionKind, as_str, {
    Series => "series",
    Anthology => "anthology",
    ReadingList => "reading_list",
    ArchiveCollection => "archive_collection",
    ChallengeAnthology => "challenge_anthology",
    PreservedBatch => "preserved_batch",
});

vocabulary!(EditionKind, as_str, {
    Draft => "draft",
    Revised => "revised",
    Anthology => "anthology",
    Translation => "translation",
    Narration => "narration",
    Printing => "printing",
});

vocabulary!(QualitySignalKind, as_str, {
    EditorialReview => "editorial_review",
    QuorumDistinction => "quorum_distinction",
    Completeness => "completeness",
    Maturity => "maturity",
    ReaderPositivity => "reader_positivity",
});

vocabulary!(MediaFormat, as_str, {
    Prose => "prose",
    Poetry => "poetry",
    Essay => "essay",
    Article => "article",
    Book => "book",
    Fanwork => "fanwork",
    Translation => "translation",
    Podfic => "podfic",
    Audiobook => "audiobook",
    FanFilm => "fan_film",
    Video => "video",
    FanComic => "fan_comic",
    Comic => "comic",
    ZineScan => "zine_scan",
    Image => "image",
    Interactive => "interactive",
    Dataset => "dataset",
    Other => "other",
});

vocabulary!(LendingClass, as_str, {
    None_ => "none",
    Lending => "lending",
    Reference => "reference",
});

/// A creator record must be internally consistent: a local pseud points at
/// a pseud, an external creator points at its source. `verified_at` is set
/// only by quorum (§19) and never by import.
pub fn creator_record_is_consistent(
    kind: CreatorKind,
    pseud_id: Option<()>,
    source_key: Option<&str>,
    source_creator_id: Option<&str>,
) -> bool {
    match kind {
        CreatorKind::LocalPseud => pseud_id.is_some() && source_key.is_none(),
        CreatorKind::External => {
            pseud_id.is_none() && source_key.is_some() && source_creator_id.is_some()
        }
    }
}

/// Quality signal values are normalised to 0..=1000 so a composite can
/// weight them without per-signal scales. Weights are non-negative; the
/// composite is instance configuration (ADR 0019), never purchasable.
pub fn quality_signal_value_is_valid(value: i64) -> bool {
    (0..=1000).contains(&value)
}

pub fn quality_signal_weight_is_valid(weight: i64) -> bool {
    weight >= 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabularies_round_trip() {
        use std::str::FromStr;
        for kind in CreatorKind::ALL {
            assert_eq!(CreatorKind::from_str(kind.as_str()).ok(), Some(*kind));
        }
        for role in CreatorRole::ALL {
            assert_eq!(CreatorRole::from_str(role.as_str()).ok(), Some(*role));
        }
        for kind in DistributorKind::ALL {
            assert_eq!(DistributorKind::from_str(kind.as_str()).ok(), Some(*kind));
        }
        for role in DistributionRole::ALL {
            assert_eq!(DistributionRole::from_str(role.as_str()).ok(), Some(*role));
        }
        for kind in CollectionKind::ALL {
            assert_eq!(CollectionKind::from_str(kind.as_str()).ok(), Some(*kind));
        }
        for kind in EditionKind::ALL {
            assert_eq!(EditionKind::from_str(kind.as_str()).ok(), Some(*kind));
        }
        for kind in QualitySignalKind::ALL {
            assert_eq!(QualitySignalKind::from_str(kind.as_str()).ok(), Some(*kind));
        }
        for format in MediaFormat::ALL {
            assert_eq!(MediaFormat::from_str(format.as_str()).ok(), Some(*format));
        }
        for class in LendingClass::ALL {
            assert_eq!(LendingClass::from_str(class.as_str()).ok(), Some(*class));
        }
        assert!(CreatorKind::from_str("server").is_err());
        assert!(MediaFormat::from_str("prose ").is_err());
    }

    #[test]
    fn creator_records_must_be_internally_consistent() {
        assert!(creator_record_is_consistent(
            CreatorKind::LocalPseud,
            Some(()),
            None,
            None
        ));
        assert!(!creator_record_is_consistent(
            CreatorKind::LocalPseud,
            None,
            None,
            None
        ));
        assert!(!creator_record_is_consistent(
            CreatorKind::LocalPseud,
            Some(()),
            Some("ao3"),
            Some("123")
        ));
        assert!(creator_record_is_consistent(
            CreatorKind::External,
            None,
            Some("ao3"),
            Some("123456")
        ));
        assert!(!creator_record_is_consistent(
            CreatorKind::External,
            Some(()),
            Some("ao3"),
            Some("123456")
        ));
        assert!(!creator_record_is_consistent(
            CreatorKind::External,
            None,
            Some("ao3"),
            None
        ));
    }

    #[test]
    fn quality_signal_bounds() {
        assert!(quality_signal_value_is_valid(0));
        assert!(quality_signal_value_is_valid(1000));
        assert!(!quality_signal_value_is_valid(-1));
        assert!(!quality_signal_value_is_valid(1001));
        assert!(quality_signal_weight_is_valid(0));
        assert!(quality_signal_weight_is_valid(5));
        assert!(!quality_signal_weight_is_valid(-1));
    }
}
