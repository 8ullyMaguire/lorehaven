//! The draw slot's deal (§16.10a, `docs/spec-amendments/draw-slot.md`).
//!
//! A draw deals a hand of works from a pool the recommender already chose. This
//! module is the whole of the deciding, and it is deliberately a pure function of
//! `(seed, pool length, hand size)`: a deal that needed a database could not be
//! tested, and the properties that matter are properties of the deal itself.
//!
//! **The deal never invents a candidate.** It returns indices into the pool it is
//! given, so a work outside the ranked pool cannot be dealt — which is what makes
//! a draw a presentation of the recommender rather than a second recommender.
//!
//! # Why a seeded deal and not `rand::random`
//!
//! Asking twice with the same nonce on the same day must return the same hand. A
//! reader who refreshes and receives three different works has been asked two
//! different questions and can answer neither, and the instances's own record of
//! what it served (`recommendation_slots`) would describe a hand nobody saw. The
//! nonce is the escape hatch — a *different* hand is a different request, and it
//! is a different seed.

use sha2::{Digest, Sha256};

/// How many cards a hand holds when the client does not say.
///
/// Three, because a hand of two is a comparison (the arena's question) and a hand
/// of five is a browse; three is small enough to be a draw and large enough that
/// the choice carries information.
pub const DEFAULT_HAND: usize = 3;

/// The largest hand a caller may ask for.
///
/// A bound rather than an opinion: every card in a hand is a recorded
/// `recommendation_slot`, and an unbounded `hand` is an unauthenticated writer to
/// the table the north-star attribution reads.
pub const MAX_HAND: usize = 10;

/// The seed for one reader's hand, as the amendment states it:
/// `sha256(account_id ‖ date ‖ nonce)`, hex encoded.
///
/// The pieces are joined with a unit separator rather than nothing, so that
/// `("ab", "c")` and `("a", "bc")` cannot produce one seed. The same reasoning as
/// the secret cipher's associated data, and the same failure if it is skipped:
/// two inputs collapsing into one seed is a hand served to the wrong question.
#[must_use]
pub fn seed(account_id: &str, date: &str, nonce: u64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(account_id.as_bytes());
    hasher.update([0x1f]);
    hasher.update(date.as_bytes());
    hasher.update([0x1f]);
    hasher.update(nonce.to_string().as_bytes());
    hex::encode(hasher.finalize())
}

/// Deal up to `hand` distinct positions out of a pool of `pool_len`.
///
/// A partial Fisher–Yates: the first `hand` positions of a shuffle, without
/// shuffling the rest. Every position in the pool is equally likely to be dealt,
/// and no position is dealt twice.
///
/// Returns fewer than `hand` when the pool is smaller — a short hand is the
/// honest answer to a short pool, and filling it with duplicates would serve the
/// same work twice under two slot ids.
#[must_use]
pub fn deal(seed_text: &str, pool_len: usize, hand: usize) -> Vec<usize> {
    if pool_len == 0 || hand == 0 {
        return Vec::new();
    }
    let hand = hand.min(pool_len).min(MAX_HAND);

    // The seed is hashed rather than parsed: it arrives as text from the route and
    // as a literal from a test, and a deal whose result depended on the seed's
    // spelling would make `seed()` the only legal input.
    let digest = Sha256::digest(seed_text.as_bytes());
    let mut state = u64::from_le_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ]);
    // A zero state is a fixed point of nothing in splitmix64 (the increment is
    // added first), but a seed of all-zero bytes is common enough in tests that it
    // is worth not depending on that detail.
    if state == 0 {
        state = 0x9e37_79b9_7f4a_7c15;
    }

    let mut positions: Vec<usize> = (0..pool_len).collect();
    let mut dealt = Vec::with_capacity(hand);
    for i in 0..hand {
        let remaining = pool_len - i;
        let pick = i + (next(&mut state) % remaining as u64) as usize;
        positions.swap(i, pick);
        dealt.push(positions[i]);
    }
    dealt
}

/// One splitmix64 step — the generator `rand` uses for seeding, in nine lines and
/// no dependency. Statistical quality is not the requirement here (the pool is
/// small and the seed is hashed); uniform-enough and reproducible are.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_question_deals_the_same_hand() {
        // The property the whole design rests on: a refresh is not a re-deal.
        let s = seed("account-1", "2026-10-27", 0);
        assert_eq!(deal(&s, 24, 3), deal(&s, 24, 3));
    }

    #[test]
    fn the_nonce_is_how_a_reader_asks_for_a_different_hand() {
        let a = seed("account-1", "2026-10-27", 0);
        let b = seed("account-1", "2026-10-27", 1);
        assert_ne!(a, b);
        // Not a guarantee about every pair of nonces, but if the deal ignored the
        // seed entirely this would pass on the seed check and fail here.
        assert_ne!(deal(&a, 24, 3), deal(&b, 24, 3));
    }

    #[test]
    fn a_hand_holds_no_duplicates_and_only_real_positions() {
        for nonce in 0..200 {
            let s = seed("account-1", "2026-10-27", nonce);
            let hand = deal(&s, 13, 5);
            assert_eq!(hand.len(), 5);
            let mut sorted = hand.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), 5, "a position was dealt twice");
            assert!(hand.iter().all(|&i| i < 13));
        }
    }

    #[test]
    fn a_short_pool_gives_a_short_hand() {
        let s = seed("a", "2026-10-27", 0);
        assert_eq!(deal(&s, 2, 3).len(), 2);
        assert!(deal(&s, 0, 3).is_empty());
        assert!(deal(&s, 5, 0).is_empty());
    }

    #[test]
    fn the_hand_size_is_capped() {
        // A caller asking for 1,000 cards is asking to write 1,000 slot rows.
        let s = seed("a", "2026-10-27", 0);
        assert_eq!(deal(&s, 100, 1_000).len(), MAX_HAND);
    }

    #[test]
    fn the_deal_spreads_over_the_pool() {
        // Not a distribution test — a smoke test that the deal is not stuck on the
        // first `hand` positions, which a modulo-free bug would look like.
        let mut seen = [0usize; 20];
        for nonce in 0..500 {
            let s = seed("account-1", "2026-10-27", nonce);
            for i in deal(&s, 20, 3) {
                seen[i] += 1;
            }
        }
        assert!(
            seen.iter().all(|&n| n > 0),
            "some positions were never dealt in 500 hands: {seen:?}"
        );
    }

    #[test]
    fn the_seed_separates_its_parts() {
        // ("ab", "c") and ("a", "bc") must not collide.
        assert_ne!(seed("ab", "c", 0), seed("a", "bc", 0));
        assert_ne!(seed("a", "b", 1), seed("a", "b", 11));
    }
}
