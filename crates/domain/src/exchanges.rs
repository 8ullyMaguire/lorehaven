//! Gift-exchange matching: who writes for whom, and who cannot be told yet.
//!
//! Spec §18.11. Pure: no database, no clock, no I/O. The store decides what a participant
//! declared; this module decides whether a pairing exists and which one to take.
//!
//! ## Why maximum cardinality comes first
//!
//! A gift exchange is a promise. A pairing that matches "most" people is worse than a
//! refusal, because the people it leaves out are promised nothing and only find out at
//! reveal. So the matcher computes a **maximum** matching over the allowed edges (Kuhn's
//! augmenting paths), and only then improves its total weight. When the maximum leaves
//! anybody out, the answer is [`MatchFailure::Impossible`] naming them, and the store
//! writes nothing.
//!
//! ## Why the weight is integers
//!
//! Two operators running the same exchange on two machines must get the same pairing.
//! Weights are small integers, participants and neighbours are visited in `account` order,
//! and no iteration order comes from a hash map — so the result is a function of the input
//! alone, which is what the determinism test asserts.

use std::collections::{BTreeMap, BTreeSet};

/// What one participant declared. All sets are the raw values the store returned: fandom
/// names are compared exactly as written (the caller normalises), and `excludes` holds
/// account ids.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Participant {
    pub account: String,
    /// Fandoms this person is willing to write.
    pub offers: BTreeSet<String>,
    /// Fandoms this person would like to receive.
    pub wants: BTreeSet<String>,
    /// Accounts this person refuses to be paired with, in either direction.
    pub excludes: BTreeSet<String>,
}

/// One edge of the pairing: `giver` writes for `recipient`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub giver: String,
    pub recipient: String,
    pub weight: i64,
}

/// Which algorithm produced the pairing. Stored beside the rows so a later reader can tell
/// a greedy result from an augmented one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Every participant had exactly one allowed partner: the pairing was forced.
    Forced,
    /// Augmenting paths found a maximum matching, and no 2-opt swap improved it.
    Augmented,
    /// Augmenting paths found a maximum matching, then a swap improved the total weight.
    Improved,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Forced => "forced",
            Method::Augmented => "augment",
            Method::Improved => "improved",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pairs {
    pub pairs: Vec<Pair>,
    pub method: Method,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchFailure {
    /// Fewer than two participants: an exchange needs somebody to write for.
    TooFew { participants: usize },
    /// The maximum matching leaves these accounts out, so pairing the rest would promise
    /// gifts the exchange cannot keep. Nothing is written.
    Impossible { unmatched: Vec<String> },
}

/// Weight of one directed pair.
///
/// `3` per shared fandom — `giver.offers ∩ recipient.wants` counts a giver who can write
/// what the recipient asked for, `giver.wants ∩ recipient.offers` counts the reciprocity
/// that makes an exchange feel like one — plus `1` for any overlap at all, so two people
/// who share a fandom outrank two strangers even when neither intersection is exact.
pub fn pair_weight(a: &Participant, b: &Participant) -> i64 {
    let gives_what_they_want = a.offers.intersection(&b.wants).count() as i64;
    let wants_what_they_give = a.wants.intersection(&b.offers).count() as i64;
    let mut weight = 3 * (gives_what_they_want + wants_what_they_give);
    let shares_anything = !a.offers.is_disjoint(&b.offers)
        || !a.offers.is_disjoint(&b.wants)
        || !a.wants.is_disjoint(&b.offers)
        || !a.wants.is_disjoint(&b.wants);
    if shares_anything {
        weight += 1;
    }
    weight
}

/// Whether `giver` may be paired with `recipient` at all.
pub fn edge_allowed(giver: &Participant, recipient: &Participant) -> bool {
    giver.account != recipient.account
        && !giver.excludes.contains(&recipient.account)
        && !recipient.excludes.contains(&giver.account)
}

/// A giver may be told whom they write for only after the exchange is revealed.
pub fn may_see_recipient(revealed: bool, caller_is_giver: bool) -> bool {
    revealed && caller_is_giver
}

/// Pair the participants, or say why nobody can be paired.
pub fn match_participants(participants: &[Participant]) -> Result<Pairs, MatchFailure> {
    if participants.len() < 2 {
        return Err(MatchFailure::TooFew {
            participants: participants.len(),
        });
    }

    // Sorted by account: every later loop walks this order, so ties resolve by id.
    let mut people: Vec<&Participant> = participants.iter().collect();
    people.sort_by(|a, b| a.account.cmp(&b.account));
    let index: BTreeMap<&str, usize> = people
        .iter()
        .enumerate()
        .map(|(i, p)| (p.account.as_str(), i))
        .collect();

    // allowed[giver] = recipient indices, in account order.
    let mut allowed: Vec<Vec<usize>> = vec![Vec::new(); people.len()];
    for (giver_i, giver) in people.iter().enumerate() {
        for (recipient_i, recipient) in people.iter().enumerate() {
            if edge_allowed(giver, recipient) {
                allowed[giver_i].push(recipient_i);
            }
        }
    }

    // Kuhn's algorithm: for each giver in order, try to find an augmenting path.
    // `assigned_to[recipient] = giver` is the current pairing.
    let mut assigned_to: Vec<Option<usize>> = vec![None; people.len()];
    for giver_i in 0..people.len() {
        let mut seen = vec![false; people.len()];
        // Failure here is not the failure condition: a later giver can free a recipient this
        // one could not take. Maximum cardinality is whatever survives the whole pass.
        let _ = augment(giver_i, &allowed, &mut assigned_to, &mut seen);
    }

    let mut unmatched: Vec<String> = Vec::new();
    for (recipient_i, giver) in assigned_to.iter().enumerate() {
        if giver.is_none() {
            unmatched.push(people[recipient_i].account.clone());
        }
    }
    // A maximum matching on a graph where in and out degree can differ leaves recipients
    // unpaired; those recipients are participants who would get nothing.
    if !unmatched.is_empty() {
        unmatched.sort();
        return Err(MatchFailure::Impossible { unmatched });
    }

    let mut pairing: Vec<Pair> = assigned_to
        .iter()
        .enumerate()
        .map(|(recipient_i, giver)| {
            let giver = giver.expect("checked above");
            Pair {
                giver: people[giver].account.clone(),
                recipient: people[recipient_i].account.clone(),
                weight: pair_weight(people[giver], people[recipient_i]),
            }
        })
        .collect();
    pairing.sort_by(|a, b| a.giver.cmp(&b.giver));

    let forced = people
        .iter()
        .all(|p| allowed[index[p.account.as_str()]].len() == 1);
    let (pairs, improved) = improve(pairing, &people, &index);
    let method = if improved {
        Method::Improved
    } else if forced {
        Method::Forced
    } else {
        Method::Augmented
    };

    Ok(Pairs { pairs, method })
}

/// Try to find an augmenting path for `giver`, reassigning along the way.
fn augment(
    giver: usize,
    allowed: &[Vec<usize>],
    assigned_to: &mut [Option<usize>],
    seen: &mut [bool],
) -> bool {
    for &recipient in &allowed[giver] {
        if seen[recipient] {
            continue;
        }
        seen[recipient] = true;
        let free = assigned_to[recipient].is_none();
        if free
            || augment(
                assigned_to[recipient].expect("checked"),
                allowed,
                assigned_to,
                seen,
            )
        {
            assigned_to[recipient] = Some(giver);
            return true;
        }
    }
    false
}

/// Deterministic 2-opt: swap the recipients of two givers when that raises the total weight
/// and stays allowed. Repeated until a full pass changes nothing.
///
/// This is a **local** optimum, not the global one — the global problem is the assignment
/// problem and would need a Hungarian implementation for at most a few points of weight.
/// Said plainly here because the alternative is a comment that overclaims.
fn improve(
    mut pairing: Vec<Pair>,
    people: &[&Participant],
    index: &BTreeMap<&str, usize>,
) -> (Vec<Pair>, bool) {
    let mut improved_any = false;
    loop {
        let mut swapped = false;
        for i in 0..pairing.len() {
            for j in (i + 1)..pairing.len() {
                let gi = index[pairing[i].giver.as_str()];
                let gj = index[pairing[j].giver.as_str()];
                let ri = index[pairing[i].recipient.as_str()];
                let rj = index[pairing[j].recipient.as_str()];
                // Exchange the two recipients. Both new edges have to be allowed, or the swap
                // would trade a worse pairing for an invalid one.
                if !edge_allowed(people[gi], people[rj]) || !edge_allowed(people[gj], people[ri]) {
                    continue;
                }
                let before = pairing[i].weight + pairing[j].weight;
                let weight_i = pair_weight(people[gi], people[rj]);
                let weight_j = pair_weight(people[gj], people[ri]);
                if weight_i + weight_j <= before {
                    continue;
                }
                let recipient_of_i = pairing[i].recipient.clone();
                pairing[i].recipient = pairing[j].recipient.clone();
                pairing[i].weight = weight_i;
                pairing[j].recipient = recipient_of_i;
                pairing[j].weight = weight_j;
                swapped = true;
                improved_any = true;
            }
        }
        if !swapped {
            break;
        }
    }
    pairing.sort_by(|a, b| a.giver.cmp(&b.giver));
    (pairing, improved_any)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(account: &str, offers: &[&str], wants: &[&str], excludes: &[&str]) -> Participant {
        Participant {
            account: account.to_string(),
            offers: offers.iter().map(|s| s.to_string()).collect(),
            wants: wants.iter().map(|s| s.to_string()).collect(),
            excludes: excludes.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn pairs_of(p: &Pairs) -> Vec<(String, String)> {
        p.pairs
            .iter()
            .map(|x| (x.giver.clone(), x.recipient.clone()))
            .collect()
    }

    #[test]
    fn one_giver_per_recipient_and_nobody_for_themselves() {
        let people = vec![
            person("a", &["x"], &["x"], &[]),
            person("b", &["x"], &["x"], &[]),
            person("c", &["y"], &["y"], &[]),
        ];
        let out = match_participants(&people).expect("a pairing exists");
        assert_eq!(out.pairs.len(), 3);
        let mut givers: Vec<_> = out.pairs.iter().map(|p| p.giver.clone()).collect();
        let mut recipients: Vec<_> = out.pairs.iter().map(|p| p.recipient.clone()).collect();
        givers.sort();
        recipients.sort();
        assert_eq!(givers, vec!["a", "b", "c"]);
        assert_eq!(recipients, vec!["a", "b", "c"]);
        for pair in &out.pairs {
            assert_ne!(pair.giver, pair.recipient, "self-pairing");
        }
    }

    #[test]
    fn an_exclusion_is_respected_in_both_directions() {
        // Two people who exclude each other cannot be paired, and with only two people the
        // matcher must refuse rather than pair them.
        let people = vec![person("a", &[], &[], &["b"]), person("b", &[], &[], &[])];
        match match_participants(&people) {
            Err(MatchFailure::Impossible { unmatched }) => {
                assert_eq!(unmatched, vec!["a".to_string(), "b".to_string()]);
            }
            other => panic!("an exclusion must make the pairing impossible: {other:?}"),
        }

        // Four people, one exclusion: a pairing still exists and must not use that edge.
        // Three would not do, and the reason is worth keeping: with a excluding b, the only
        // person who can send to b is c (nobody sends to themselves), so c is spent before a
        // has a sender — the maximum matching is 2 of 3 and the matcher correctly refuses.
        // The arity of the fixture is part of the claim, not an accident.
        let people = vec![
            person("a", &[], &[], &["b"]),
            person("b", &[], &[], &[]),
            person("c", &[], &[], &[]),
            person("d", &[], &[], &[]),
        ];
        let out = match_participants(&people).expect("four people can be paired");
        assert_eq!(out.pairs.len(), 4, "everybody is paired: {out:?}");
        for pair in &out.pairs {
            assert!(
                !(pair.giver == "a" && pair.recipient == "b")
                    && !(pair.giver == "b" && pair.recipient == "a"),
                "the excluded edge was used: {pair:?}"
            );
        }
    }

    #[test]
    fn the_pairing_prefers_shared_fandoms() {
        let people = vec![
            person("giver1", &["dragon"], &[], &[]),
            person("taker1", &[], &["dragon"], &[]),
            person("giver2", &["space"], &[], &[]),
            person("taker2", &[], &["space"], &[]),
        ];
        let out = match_participants(&people).expect("pairing");
        let map: BTreeMap<_, _> = out
            .pairs
            .iter()
            .map(|p| (p.giver.clone(), p.recipient.clone()))
            .collect();
        assert_eq!(map["giver1"], "taker1", "shared fandom first: {map:?}");
        assert_eq!(map["giver2"], "taker2", "shared fandom first: {map:?}");
    }

    #[test]
    fn a_partial_pairing_is_a_refusal_naming_the_people() {
        // `c` excludes everyone and is excluded by everyone: a maximum matching cannot
        // place them, so nobody is paired at all.
        let people = vec![
            person("a", &[], &[], &[]),
            person("b", &[], &[], &[]),
            person("c", &[], &[], &["a", "b"]),
        ];
        match match_participants(&people) {
            Err(MatchFailure::Impossible { unmatched }) => {
                assert!(unmatched.contains(&"c".to_string()), "{unmatched:?}");
            }
            other => panic!("an unmatched participant is a refusal: {other:?}"),
        }
    }

    #[test]
    fn the_result_does_not_depend_on_input_order() {
        let build = |order: &[usize]| {
            let all = [
                person("a", &["x"], &["y"], &[]),
                person("b", &["y"], &["x"], &[]),
                person("c", &["z"], &["z"], &[]),
                person("d", &["x", "z"], &["y", "z"], &[]),
            ];
            order.iter().map(|i| all[*i].clone()).collect::<Vec<_>>()
        };
        let first = match_participants(&build(&[0, 1, 2, 3])).expect("pairing");
        let second = match_participants(&build(&[3, 2, 1, 0])).expect("pairing");
        assert_eq!(pairs_of(&first), pairs_of(&second), "order changed the pairing");
    }

    #[test]
    fn two_people_is_too_few_and_zero_is_named() {
        assert_eq!(
            match_participants(&[]),
            Err(MatchFailure::TooFew { participants: 0 })
        );
        assert_eq!(
            match_participants(&[person("a", &[], &[], &[])]),
            Err(MatchFailure::TooFew { participants: 1 })
        );
    }

    #[test]
    fn a_forced_pairing_says_so() {
        // Two people can only pair with each other, so the method is `forced` — the branch
        // that exists to tell a later reader that no search happened.
        let people = vec![person("a", &[], &[], &[]), person("b", &[], &[], &[])];
        let out = match_participants(&people).expect("pairing");
        assert_eq!(out.method, Method::Forced);
        assert_eq!(out.method.as_str(), "forced");

        // Four strangers still pair, but nothing was forced about it.
        let people = vec![
            person("a", &[], &[], &[]),
            person("b", &[], &[], &[]),
            person("c", &[], &[], &[]),
            person("d", &[], &[], &[]),
        ];
        let out = match_participants(&people).expect("pairing");
        assert_eq!(out.method, Method::Augmented, "{out:?}");
    }

    #[test]
    fn the_recipient_is_secret_until_reveal() {
        assert!(!may_see_recipient(false, true), "before reveal, even a giver");
        assert!(!may_see_recipient(true, false), "after reveal, a non-giver");
        assert!(may_see_recipient(true, true), "after reveal, the giver");
    }

    #[test]
    fn the_weight_counts_both_directions() {
        let giver = person("g", &["x"], &["y"], &[]);
        let taker = person("t", &["y"], &["x"], &[]);
        // giver gives x (taker wants x) and wants y (taker offers y): 3 + 3 + 1.
        assert_eq!(pair_weight(&giver, &taker), 7);
        let stranger = person("s", &["q"], &["q"], &[]);
        assert_eq!(pair_weight(&giver, &stranger), 0);
    }
}
