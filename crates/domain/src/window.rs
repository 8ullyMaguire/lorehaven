//! Time windows: the one grammar every surface uses for "when".
//!
//! The catalogue stores three clocks per row and they disagree by design — the
//! source's (`source_updated_at`), ours (`last_synced_at`), and the entry's
//! (`created_at`). "Updated last week" is therefore ambiguous unless the query
//! says *whose* clock, and "updated more than a week ago" is not expressible at
//! all with a duration alone.
//!
//! So a window is a small language:
//!
//! ```text
//! 7d 36h 3w 6m 2y          within the last N            [now-N, ∞)
//! >7d                       older than N                 (-∞, now-N)
//! -30d..-7d                 between 30 and 7 days ago
//! 2026-10-01..2026-11-01    absolute range, half-open    [2026-10-01, 2026-11-01)
//! ..-7d                     everything older than 7d ago (-∞, now-7d)
//! @2026-11-01               that calendar day
//! today yesterday this-week last-week this-month last-month this-year last-year
//! any                       no constraint
//! ```
//!
//! # Half-open, because the alternative double-counts
//!
//! Every bound is `[from, to)`. That is what makes `7d` and `>7d` **partition**
//! the catalogue: a row at exactly seven days old is in the first window and not
//! the second, so "how many are new vs old" adds up to the whole table. A `<=` on
//! either end puts the boundary row in both and every such count is wrong by the
//! boundary — silently, because both numbers look plausible.
//!
//! # Calendar words are UTC
//!
//! `this-week` starts on Monday 00:00 **UTC**. The instance has no configured
//! display timezone, and inventing one here would put a second source of truth
//! beside the frontend's. Knowing this is a documented limitation is better than
//! guessing a local offset per process.
//!
//! # Months and years are 30 and 365 days
//!
//! `6m` is 180 days, not "six calendar months". A window is a filter for "recent
//! enough", and the difference matters only at a boundary nobody queries. The
//! calendar words above are the exact form when it does matter.

use serde::{Deserialize, Serialize};
use time::{Date, Duration, OffsetDateTime, UtcOffset};

/// A half-open time window, `[from, to)`. `None` means unbounded on that side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    /// Inclusive lower bound.
    pub from: Option<OffsetDateTime>,
    /// Exclusive upper bound.
    pub to: Option<OffsetDateTime>,
}

impl Bounds {
    /// Unconstrained: every row.
    #[must_use]
    pub fn unbounded() -> Self {
        Self {
            from: None,
            to: None,
        }
    }

    /// True when the window constrains nothing.
    #[must_use]
    pub fn is_unbounded(&self) -> bool {
        self.from.is_none() && self.to.is_none()
    }

    /// Whether an instant (as the catalogue stores it) falls inside.
    ///
    /// Kept here so a caller that already has a parsed timestamp — a test, a
    /// saved-query preview, the CLI's local engine — applies the same rule the
    /// SQL does.
    #[must_use]
    pub fn contains(&self, at: OffsetDateTime) -> bool {
        self.from.is_none_or(|from| at >= from) && self.to.is_none_or(|to| at < to)
    }

    /// Lower bound in the canonical stored shape, `YYYY-MM-DDTHH:MM:SSZ`.
    #[must_use]
    pub fn from_sql(&self) -> Option<String> {
        self.from.as_ref().map(sql_shape)
    }

    /// Upper bound as the canonical stored shape, `YYYY-MM-DDTHH:MM:SSZ`.
    #[must_use]
    pub fn to_sql(&self) -> Option<String> {
        self.to.as_ref().map(sql_shape)
    }
}

/// A bound in the shape the catalogue *stores*, so a window is a plain range comparison
/// against the column and can use an index (migration 0132 made that shape the only one the
/// database accepts).
///
/// `Z`, not `+00:00`: that is what `time`'s Rfc3339 formatter emits for UTC, so it is what
/// every in-tree writer already produces — measured, not assumed (`OffsetDateTime` formatted
/// with `Rfc3339` gives `2026-11-01T12:00:00Z`). Choosing `+00:00` here would have made the
/// database refuse its own writer's output.
///
/// This used to be `YYYY-MM-DD HH:MM:SS` — a shape no row ever had. It was correct only
/// because every comparison wrapped both sides in a normalising function (`datetime()` on
/// SQLite, `AT TIME ZONE 'UTC'` on PostgreSQL), and that function is what cost the index.
fn sql_shape(at: &OffsetDateTime) -> String {
    let utc = at.to_offset(UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        utc.year(),
        utc.month() as u8,
        utc.day(),
        utc.hour(),
        utc.minute(),
        utc.second()
    )
}

/// Why a window could not be parsed. Every variant names the offending input, so
/// a 422 can say what was wrong rather than "invalid parameter".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowError {
    /// Empty string.
    Empty,
    /// The unit character is not one of `h d w m y`.
    UnknownUnit,
    /// The number before the unit is missing or not an integer.
    BadNumber,
    /// A `YYYY-MM-DD` that is not a real date.
    BadDate,
    /// A range whose start is after its end.
    Inverted,
    /// A form the grammar does not define.
    Unknown,
}

impl WindowError {
    /// A message a caller can put in front of a person.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Empty => "window is empty",
            Self::UnknownUnit => "window unit must be one of h, d, w, m, y",
            Self::BadNumber => "window needs a whole number before its unit, e.g. 7d",
            Self::BadDate => "window date must be YYYY-MM-DD and a real date",
            Self::Inverted => "window range starts after it ends",
            Self::Unknown => {
                "window must be 7d, >7d, -30d..-7d, 2026-10-01..2026-11-01, @2026-11-01, \
                 a calendar word (today, last-week, this-month, …) or any"
            }
        }
    }
}

impl std::fmt::Display for WindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// Parse a window against `now`.
///
/// `now` is a parameter rather than a call to the system clock so the grammar is
/// testable to the day: a test that reads the wall clock is a test that fails on
/// the last day of a month.
pub fn parse(raw: &str, now: OffsetDateTime) -> Result<Bounds, WindowError> {
    let now = now.to_offset(UtcOffset::UTC);
    let text = raw.trim().to_ascii_lowercase();
    if text.is_empty() {
        return Err(WindowError::Empty);
    }
    if matches!(text.as_str(), "any" | "all" | "*") {
        return Ok(Bounds::unbounded());
    }

    // A calendar word: an exact window whose end is known.
    if let Some(bounds) = calendar(&text, now)? {
        return Ok(bounds);
    }

    // @YYYY-MM-DD — one day.
    if let Some(day) = text.strip_prefix('@') {
        let start = date(day)?;
        let from = start.midnight().assume_utc();
        let to = (start + Duration::days(1)).midnight().assume_utc();
        return Ok(Bounds {
            from: Some(from),
            to: Some(to),
        });
    }

    // A range: `a..b`, either side optional, each side relative or absolute.
    if let Some((left, right)) = text.split_once("..") {
        let from = if left.trim().is_empty() {
            None
        } else {
            Some(boundary(left.trim(), now)?)
        };
        let to = if right.trim().is_empty() {
            None
        } else {
            Some(boundary(right.trim(), now)?)
        };
        if let (Some(from), Some(to)) = (from, to) {
            if from >= to {
                return Err(WindowError::Inverted);
            }
        }
        return Ok(Bounds { from, to });
    }

    // Older than: `>7d` — the half that a duration alone cannot express.
    if let Some(rest) = text.strip_prefix('>') {
        let age = duration(rest.trim())?;
        return Ok(Bounds {
            from: None,
            to: Some(now - age),
        });
    }

    // `<=7d` reads as "within", and is accepted so a caller who writes it gets the
    // window they meant rather than an error about a form that looks reasonable.
    if let Some(rest) = text.strip_prefix("<=") {
        return within(duration(rest.trim())?, now);
    }

    // Within the last N.
    within(duration(&text)?, now)
}

fn within(age: Duration, now: OffsetDateTime) -> Result<Bounds, WindowError> {
    Ok(Bounds {
        from: Some(now - age),
        to: None,
    })
}

/// One side of a range: a relative age (`-30d` or `30d`) or an absolute date.
fn boundary(raw: &str, now: OffsetDateTime) -> Result<OffsetDateTime, WindowError> {
    let text = raw.strip_prefix('-').unwrap_or(raw);
    // A bare `YYYY-MM-DD` is an absolute instant; anything with a unit is an age.
    if text.contains('-') {
        return Ok(date(text)?.midnight().assume_utc());
    }
    Ok(now - duration(text)?)
}

fn duration(raw: &str) -> Result<Duration, WindowError> {
    if raw.is_empty() {
        return Err(WindowError::Empty);
    }
    let (number, unit) = raw.split_at(raw.len() - 1);
    let amount: i64 = number.parse().map_err(|_| WindowError::BadNumber)?;
    if amount < 0 {
        return Err(WindowError::BadNumber);
    }
    let days = match unit {
        "h" => return Ok(Duration::hours(amount)),
        "d" => amount,
        "w" => amount * 7,
        // 30 and 365 rather than "a month" and "a year": documented at the module
        // level, and the calendar words are the exact form.
        "m" => amount * 30,
        "y" => amount * 365,
        _ => return Err(WindowError::UnknownUnit),
    };
    Ok(Duration::days(days))
}

fn date(raw: &str) -> Result<Date, WindowError> {
    let mut parts = raw.split('-');
    let (Some(y), Some(m), Some(d), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(WindowError::BadDate);
    };
    let year: i32 = y.parse().map_err(|_| WindowError::BadDate)?;
    let month: u8 = m.parse().map_err(|_| WindowError::BadDate)?;
    let day: u8 = d.parse().map_err(|_| WindowError::BadDate)?;
    let month = time::Month::try_from(month).map_err(|_| WindowError::BadDate)?;
    Date::from_calendar_date(year, month, day).map_err(|_| WindowError::BadDate)
}

/// Calendar words. `Ok(None)` means the word is not a calendar word.
fn calendar(word: &str, now: OffsetDateTime) -> Result<Option<Bounds>, WindowError> {
    let today = now.date();
    let midnight = |d: Date| d.midnight().assume_utc();
    // Monday of the week `today` falls in.
    let monday = |d: Date| d - Duration::days(i64::from(d.weekday().number_days_from_monday()));
    let first_of_month = |d: Date| {
        Date::from_calendar_date(d.year(), d.month(), 1).expect("the first of a month exists")
    };
    let bounds = match word {
        "today" => Bounds {
            from: Some(midnight(today)),
            to: Some(midnight(today + Duration::days(1))),
        },
        "yesterday" => Bounds {
            from: Some(midnight(today - Duration::days(1))),
            to: Some(midnight(today)),
        },
        "this-week" => Bounds {
            from: Some(midnight(monday(today))),
            to: None,
        },
        "last-week" => Bounds {
            from: Some(midnight(monday(today) - Duration::days(7))),
            to: Some(midnight(monday(today))),
        },
        "this-month" => Bounds {
            from: Some(midnight(first_of_month(today))),
            to: None,
        },
        "last-month" => {
            let this_first = first_of_month(today);
            let prev_last = this_first - Duration::days(1);
            Bounds {
                from: Some(midnight(first_of_month(prev_last))),
                to: Some(midnight(this_first)),
            }
        }
        "this-year" => Bounds {
            from: Some(midnight(
                Date::from_calendar_date(today.year(), time::Month::January, 1)
                    .expect("January 1st exists"),
            )),
            to: None,
        },
        "last-year" => {
            let this_first = Date::from_calendar_date(today.year(), time::Month::January, 1)
                .expect("January 1st exists");
            let prev_first = Date::from_calendar_date(today.year() - 1, time::Month::January, 1)
                .expect("January 1st exists");
            Bounds {
                from: Some(midnight(prev_first)),
                to: Some(midnight(this_first)),
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(bounds))
}

/// The weekday arithmetic above relies on `Weekday::number_days_from_monday`
/// being 0 for Monday; a wrong yule-week bug here would move every `this-week`
/// window by a day, so it is asserted rather than assumed.
#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;
    // `Weekday` is a test-only import: the library code never names it, and an
    // unused import at module scope is a denied warning on the build.
    use time::Weekday;

    fn at(text: &str) -> OffsetDateTime {
        OffsetDateTime::parse(
            text,
            &time::format_description::well_known::Rfc3339,
        )
        .expect("test instant")
    }

    const NOW: fn() -> OffsetDateTime = || datetime!(2026-11-01 12:00:00 UTC);

    #[test]
    fn weekday_index_is_monday_zero() {
        assert_eq!(Weekday::Monday.number_days_from_monday(), 0);
        assert_eq!(Weekday::Sunday.number_days_from_monday(), 6);
    }

    #[test]
    fn a_duration_is_within() {
        let w = parse("7d", NOW()).expect("7d");
        assert_eq!(w.from, Some(datetime!(2026-10-25 12:00:00 UTC)));
        assert_eq!(w.to, None, "within has no upper bound");
    }

    #[test]
    fn older_than_is_the_complement() {
        let within = parse("7d", NOW()).expect("7d");
        let older = parse(">7d", NOW()).expect(">7d");
        assert_eq!(older.from, None);
        assert_eq!(older.to, within.from, ">7d ends exactly where 7d begins");
        // The partition property, stated as the reason the bounds are half-open.
        let boundary = within.from.expect("from");
        assert!(within.contains(boundary), "7d includes its own boundary");
        assert!(!older.contains(boundary), ">7d excludes it, so the two tile");
    }

    #[test]
    fn ranges_are_half_open_and_ordered() {
        let w = parse("-30d..-7d", NOW()).expect("range");
        assert_eq!(w.from, Some(datetime!(2026-10-02 12:00:00 UTC)));
        assert_eq!(w.to, Some(datetime!(2026-10-25 12:00:00 UTC)));
        assert_eq!(
            parse("2026-10-01..2026-11-01", NOW()).expect("absolute").to,
            Some(datetime!(2026-11-01 00:00:00 UTC)),
            "the end date is exclusive"
        );
        assert_eq!(
            parse("..-7d", NOW()).expect("open start").from,
            None,
            "a missing left side is unbounded"
        );
        assert_eq!(
            parse("-30d..-7d", NOW()).err(),
            None,
            "a well-formed range parses"
        );
        assert_eq!(parse("1d..7d", NOW()).err(), Some(WindowError::Inverted));
    }

    #[test]
    fn a_day_is_a_day() {
        let w = parse("@2026-10-31", NOW()).expect("day");
        assert_eq!(w.from, Some(datetime!(2026-10-31 00:00:00 UTC)));
        assert_eq!(w.to, Some(datetime!(2026-11-01 00:00:00 UTC)));
    }

    #[test]
    fn calendar_words_are_utc_and_monday_based() {
        // 2026-11-01 is a Sunday, which is the case that catches an off-by-one.
        assert_eq!(NOW().date().weekday(), Weekday::Sunday);
        let this_week = parse("this-week", NOW()).expect("this-week");
        assert_eq!(this_week.from, Some(datetime!(2026-10-26 00:00:00 UTC)));
        let last_week = parse("last-week", NOW()).expect("last-week");
        assert_eq!(last_week.from, Some(datetime!(2026-10-19 00:00:00 UTC)));
        assert_eq!(last_week.to, Some(datetime!(2026-10-26 00:00:00 UTC)));
        let last_month = parse("last-month", NOW()).expect("last-month");
        assert_eq!(last_month.from, Some(datetime!(2026-10-01 00:00:00 UTC)));
        assert_eq!(last_month.to, Some(datetime!(2026-11-01 00:00:00 UTC)));
        assert_eq!(
            parse("this-year", NOW()).expect("this-year").from,
            Some(datetime!(2026-01-01 00:00:00 UTC))
        );
    }

    #[test]
    fn any_and_star_are_unbounded() {
        for raw in ["any", "ALL", "*", " any "] {
            assert!(parse(raw, NOW()).expect("unbounded").is_unbounded(), "{raw}");
        }
    }

    #[test]
    fn every_error_names_its_cause() {
        assert_eq!(parse("", NOW()).err(), Some(WindowError::Empty));
        assert_eq!(parse("7q", NOW()).err(), Some(WindowError::UnknownUnit));
        assert_eq!(parse("d", NOW()).err(), Some(WindowError::BadNumber));
        assert_eq!(parse("@2026-02-30", NOW()).err(), Some(WindowError::BadDate));
        // No digits at all is a number error; a wrong letter after a number is a
        // unit error. The two are different mistakes and say so.
        assert_eq!(parse("soon", NOW()).err(), Some(WindowError::BadNumber));
        assert_eq!(parse("7q", NOW()).err(), Some(WindowError::UnknownUnit));
        assert_eq!(parse(">", NOW()).err(), Some(WindowError::Empty));
    }

    #[test]
    fn sql_shape_is_the_shape_the_column_stores() {
        // The bound has to be byte-comparable with a stored value, because the window is now a
        // plain range on the column: one shape, explicit +00:00, second precision (0132).
        let w = parse("7d", NOW()).expect("7d");
        assert_eq!(w.from_sql().as_deref(), Some("2026-10-25T12:00:00Z"));
        assert_eq!(w.to_sql(), None);
        // An instant with a non-UTC offset converts rather than truncates.
        let w = parse("@2026-10-31", at("2026-11-01T13:00:00+01:00")).expect("day");
        assert_eq!(w.from_sql().as_deref(), Some("2026-10-31T00:00:00Z"));
        // And it sorts like an instant against every other bound, which is the whole point:
        // '2026-10-31T00:00:00Z' < '2026-10-31T05:00:00Z' as text and as time.
        let bound = w.from_sql().expect("bound");
        assert!(bound.as_str() < "2026-10-31T05:00:00Z");
    }
}
