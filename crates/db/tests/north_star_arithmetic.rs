//! Unit tests for the arithmetic in `crates/db/src/north_star.rs`.
//!
//! These are the parts that cannot be reached by seeding a database, because they are
//! about *degenerate inputs* — an empty window, an even-sized sample, an unparseable
//! bound. The integration tests in `crates/app/tests/north_star.rs` cover the query; these
//! cover the decisions, which is where a metric quietly stops being the metric it claims.
//!
//! Every assertion below calls the **real** function. An earlier version of this file
//! re-implemented `median` locally to exercise it, which proved only that the copy was
//! self-consistent — the same mistake `docs/plans/REMAINING-2026-10-03.md` records for
//! `arena_weights_decode.rs`, where a hand-copied query stayed green while the query in
//! `tasting.rs` had been reverted. The functions are `pub` for exactly this reason.

use lorehaven_db::north_star::{days_between, median, months_between, parse_rfc3339};

mod median {
    use super::{days_between, median};

    #[test]
    fn an_empty_sample_is_undefined_rather_than_zero() {
        assert_eq!(
            median(&[]),
            None,
            "§53.5: undefined, not 0 -- a zero would report instant discovery"
        );
    }

    #[test]
    fn a_single_sample_is_itself() {
        assert_eq!(median(&[7.0]), Some(7.0));
    }

    #[test]
    fn two_samples_take_their_mean_not_an_endpoint() {
        assert_eq!(
            median(&[2.0, 8.0]),
            Some(5.0),
            "the mean is the median; returning 2.0 or 8.0 would make the metric depend on \
             whether a window happened to contain an even number of discoveries"
        );
    }

    #[test]
    fn four_samples_take_the_mean_of_the_middle_two() {
        assert_eq!(median(&[1.0, 2.0, 8.0, 9.0]), Some(5.0));
    }

    #[test]
    fn an_odd_sample_takes_the_middle_value() {
        assert_eq!(median(&[1.0, 5.0, 9.0]), Some(5.0));
    }

    #[test]
    fn the_median_is_unaffected_by_outliers_at_the_ends() {
        // Neither 0 nor 1000 is a median candidate, so the answer is 5 -- which is the
        // entire reason the spec asks for a median. The mean of this sample is 146.7.
        assert_eq!(median(&[0.0, 1.0, 2.0, 5.0, 9.0, 10.0, 1000.0]), Some(5.0));
    }

    #[test]
    fn a_window_of_one_day_gaps_reads_as_zero_days_not_one() {
        // A reader who loved a work the same day they saw it took 0 days, not 1. Rounding
        // instead of flooring would move a whole class of fast discoveries across the
        // boundary and shift the median.
        assert_eq!(
            days_between("2026-06-01T00:00:00Z", "2026-06-01T23:00:00Z"),
            Some(0.0)
        );
    }
}

mod months {
    use super::{days_between, months_between, parse_rfc3339};

    #[test]
    fn a_year_is_twelve_months() {
        // 365 / 30.4375 = 11.99..., where a naive 30-day month gives 12.17. The point is
        // that a whole-year window must not report thirteen months.
        let months = months_between("2026-01-01T00:00:00Z", "2027-01-01T00:00:00Z");
        assert!(
            (months - 12.0).abs() < 0.05,
            "a year is about twelve months, got {months}"
        );
    }

    #[test]
    fn an_hour_long_window_is_floored_rather_than_dividing_by_zero() {
        let months = months_between("2026-01-01T00:00:00Z", "2026-01-01T01:00:00Z");
        assert!(
            months >= 1.0 / 20.0,
            "a sub-day window must not divide by zero; the floor is a twentieth of a month"
        );
    }

    #[test]
    fn a_backwards_window_is_floored_too() {
        // `since` after `until` is the operator's mistake, not a reason to divide by a
        // negative number and report a negative rate.
        let months = months_between("2026-12-01T00:00:00Z", "2026-01-01T00:00:00Z");
        assert!(
            months > 0.0,
            "a backwards window must not produce a negative month count, got {months}"
        );
    }

    #[test]
    fn an_unparseable_bound_does_not_claim_a_36_hour_month() {
        // Reading an unparseable bound as zero days would then hit the floor and report a
        // one-twentieth-of-a-month window for a query that actually covered a year,
        // inflating the rate twentyfold. One month is the least-bad reading.
        assert_eq!(
            months_between("not-a-timestamp", "2027-01-01T00:00:00Z"),
            1.0,
            "an unparseable bound reads as one month, not as the floor"
        );
    }

    #[test]
    fn both_spellings_of_utc_parse() {
        // Rows written here use `Z`; rows PostgreSQL returns from `::text` use `+00:00`.
        // A parser that accepted only one would make the median `None` on one engine
        // only, which is exactly the per-dialect difference this module exists to avoid.
        assert!(parse_rfc3339("2026-01-01T00:00:00Z").is_some());
        assert!(parse_rfc3339("2026-01-01T00:00:00+00:00").is_some());
    }

    #[test]
    fn a_null_timestamp_yields_no_gap_rather_than_a_panic() {
        // `MIN()` over no matching slot is NULL, and the store hands that straight to the
        // day arithmetic. It must be `None`, not a panic and not a zero.
        assert_eq!(
            days_between("2026-06-01T00:00:00Z", "not-a-timestamp"),
            None
        );
    }
}
