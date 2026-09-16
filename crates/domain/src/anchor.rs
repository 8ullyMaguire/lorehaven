/// A comment anchor: a specific position within a text or media unit.
///
/// Anchored comments address a paragraph offset in text (identified by
/// `chapter_id` + `paragraph`) or a timestamp in media (`timestamp`).
/// The full shape is validated by [`validate_anchor`]; routes store the
/// kind/value pair and the chapter id, all nullable for whole-work comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum AnchorKind {
    /// A paragraph offset within a chapter's document.
    Paragraph,
    /// A timestamp within a media unit.
    Timestamp,
}

impl AnchorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Paragraph => "paragraph",
            Self::Timestamp => "timestamp",
        }
    }
}

impl std::str::FromStr for AnchorKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "paragraph" => Ok(Self::Paragraph),
            "timestamp" => Ok(Self::Timestamp),
            _ => Err(format!("unknown anchor kind: {s}")),
        }
    }
}

impl std::fmt::Display for AnchorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Validate a (kind, value, optional chapter_id) triple.
///
/// - `paragraph` anchors require a chapter id and a non-negative integer value.
/// - `timestamp` anchors require a value in `HH:MM:SS(.fff)?` or `SS(.fff)?` form.
pub fn validate_anchor(
    kind: AnchorKind,
    value: &str,
    chapter_id: Option<&str>,
) -> Result<(), String> {
    match kind {
        AnchorKind::Paragraph => {
            let chapter = chapter_id.ok_or("paragraph anchors require a chapter_id")?;
            if uuid::Uuid::parse_str(chapter).is_err() {
                return Err("invalid chapter_id".into());
            }
            let offset: u32 = value
                .parse()
                .map_err(|_| "paragraph anchor must be a non-negative integer".to_string())?;
            let _ = offset;
            Ok(())
        }
        AnchorKind::Timestamp => {
            if chapter_id.is_some() {
                return Err("timestamp anchors must not set a chapter_id".into());
            }
            if !is_valid_timestamp(value) {
                return Err("timestamp anchor must be HH:MM:SS(.fff) or SS(.fff)".into());
            }
            Ok(())
        }
    }
}

/// Validate a timestamp string: `SS(.fff)?` or `HH:MM:SS(.fff)?`.
fn is_valid_timestamp(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.as_slice() {
        [secs] => parse_secs(secs).is_some(),
        [hh, mm, ss] => {
            let Ok(h) = hh.parse::<u32>() else { return false; };
            let Ok(m) = mm.parse::<u8>() else { return false };
            if m >= 60 {
                return false;
            }
            parse_secs(ss).is_some() && h < 24
        }
        _ => false,
    }
}

fn parse_secs(s: &str) -> Option<(u8, Option<u16>)> {
    let parts: Vec<&str> = s.split('.').collect();
    match parts.as_slice() {
        [secs] => secs.parse::<u8>().ok().map(|s| (s, None)),
        [secs, frac] => {
            let secs = secs.parse::<u8>().ok()?;
            let frac = frac.parse::<u16>().ok()?;
            Some((secs, Some(frac)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraph_anchor_requires_chapter() {
        assert!(validate_anchor(AnchorKind::Paragraph, "3", Some("00000000-0000-0000-0000-000000000001")).is_ok());
        assert!(validate_anchor(AnchorKind::Paragraph, "3", None).is_err());
        assert!(validate_anchor(AnchorKind::Paragraph, "abc", Some("00000000-0000-0000-0000-000000000001")).is_err());
    }

    #[test]
    fn timestamp_anchor_formats() {
        assert!(validate_anchor(AnchorKind::Timestamp, "42", None).is_ok());
        assert!(validate_anchor(AnchorKind::Timestamp, "01:23:45", None).is_ok());
        assert!(validate_anchor(AnchorKind::Timestamp, "01:23:45.678", None).is_ok());
        assert!(validate_anchor(AnchorKind::Timestamp, "70", None).is_err()); // secs >= 60
        assert!(validate_anchor(AnchorKind::Timestamp, "42", Some("x")).is_err());
    }

    #[test]
    fn kind_round_trips() {
        for k in [AnchorKind::Paragraph, AnchorKind::Timestamp] {
            assert_eq!(AnchorKind::from_str(k.as_str()).unwrap(), k);
        }
        assert!(AnchorKind::from_str("offset").is_err());
    }
}
