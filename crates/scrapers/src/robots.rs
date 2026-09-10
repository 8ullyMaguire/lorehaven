//! `robots.txt`: what a site publishes about how it wants to be read.
//!
//! Two halves, and both are load-bearing:
//!
//! * **`Disallow`** says which paths we must not fetch at all. A page we are
//!   forbidden to read is not a page whose parse failure we should be
//!   debugging, so this is a refusal, not a warning.
//! * **`Crawl-delay`** says how long to wait between requests. It is the site's
//!   own published number, which is strictly better than one we invented: the
//!   operator of the server knows what it can take, and a number in this
//!   repository is a guess that goes stale.
//!
//! # Scope, stated plainly
//!
//! This is a reader, not an implementation of RFC 9309 in full. It handles what
//! real files contain: comment lines, case-insensitive field names, wildcard
//! (`*`) and end-anchor (`$`) patterns, most-specific-rule-wins with `Allow`
//! breaking ties, the `*` group as a fallback, and a `Crawl-delay` in seconds
//! (possibly fractional). It does not implement `Sitemap`, `Request-rate`, or
//! `Noindex`, because nothing here consumes them — an unread directive is not
//! made better by being parsed.
//!
//! # Why the fallback is a second rather than zero
//!
//! A site with no `Crawl-delay` has told us nothing, and "nothing" must not be
//! read as "no limit". The caller combines the result here with its own floor
//! (see `safety.rs`), so an absent directive means the floor applies rather than
//! meaning the site is happy with an unbounded rate.

use std::time::Duration;

/// One `Allow` or `Disallow` line, in the group that applies to us.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Directive {
    allow: bool,
    /// The path pattern, still carrying any `*` and trailing `$`.
    pattern: String,
}

/// What one site's `robots.txt` says to us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RobotsRules {
    directives: Vec<Directive>,
    crawl_delay: Option<Duration>,
    /// Whether a file was found at all. A missing file means "no restrictions",
    /// which is a different fact from "restrictions we could not read" — and the
    /// caller reports them differently.
    found: bool,
}

impl RobotsRules {
    /// Rules for a site that publishes no `robots.txt`.
    ///
    /// No restrictions, and no crawl delay: absence is not permission to
    /// hammer, but it is also not a restriction, and inventing one here would
    /// make the caller's own floor look like the site's opinion.
    #[must_use]
    pub fn unrestricted() -> Self {
        Self {
            directives: Vec::new(),
            crawl_delay: None,
            found: false,
        }
    }

    /// Whether a file was found and read.
    #[must_use]
    pub fn was_found(&self) -> bool {
        self.found
    }

    /// The site's published minimum gap between requests, when it publishes one.
    #[must_use]
    pub fn crawl_delay(&self) -> Option<Duration> {
        self.crawl_delay
    }

    /// Whether we may fetch `path`.
    ///
    /// The most specific matching rule wins, measured by pattern length, and
    /// `Allow` breaks a tie — both as the de-facto standard has it. A path no
    /// rule mentions is allowed.
    #[must_use]
    pub fn allows(&self, path: &str) -> bool {
        let mut best: Option<(usize, bool)> = None;
        for directive in &self.directives {
            if !pattern_matches(&directive.pattern, path) {
                continue;
            }
            let length = directive.pattern.len();
            best = match best {
                None => Some((length, directive.allow)),
                Some((best_length, best_allow)) => {
                    if length > best_length || (length == best_length && directive.allow) {
                        Some((length, directive.allow))
                    } else {
                        Some((best_length, best_allow))
                    }
                }
            };
        }
        best.map(|(_, allow)| allow).unwrap_or(true)
    }

    /// Parse a `robots.txt` body for a crawler whose product token is
    /// `user_agent_token` (for `Lorehaven/0.1.0 (+import)` that is `Lorehaven`).
    #[must_use]
    pub fn parse(body: &str, user_agent_token: &str) -> Self {
        let groups = split_groups(body);
        let token = user_agent_token.to_ascii_lowercase();

        // The most specific group that names us; otherwise the `*` group.
        let mut chosen: Option<&Group> = None;
        let mut chosen_score = 0usize;
        for group in &groups {
            for name in &group.agents {
                let name = name.to_ascii_lowercase();
                if name == "*" || !names_us(&name, &token) {
                    continue;
                }
                if name.len() > chosen_score {
                    chosen = Some(group);
                    chosen_score = name.len();
                }
            }
        }
        if chosen.is_none() {
            chosen = groups
                .iter()
                .find(|group| group.agents.iter().any(|name| name == "*"));
        }

        let Some(group) = chosen else {
            return Self {
                directives: Vec::new(),
                crawl_delay: None,
                found: true,
            };
        };

        let directives = group
            .rules
            .iter()
            .filter_map(|(allow, value)| {
                // An empty value restricts nothing: `Disallow:` with nothing
                // after it is the conventional way of writing "allow all", and
                // keeping it as a zero-length pattern would be harmless but
                // misleading in a log line.
                if value.is_empty() {
                    return None;
                }
                Some(Directive {
                    allow: *allow,
                    pattern: value.clone(),
                })
            })
            .collect();

        Self {
            directives,
            crawl_delay: group.crawl_delay,
            found: true,
        }
    }
}

/// A user-agent group: the names it applies to, and its rules.
#[derive(Debug)]
struct Group {
    agents: Vec<String>,
    /// `(allow, pattern)`, in file order.
    rules: Vec<(bool, String)>,
    crawl_delay: Option<Duration>,
}

/// Whether a `robots.txt` user-agent value names us.
///
/// The de-facto rule is a case-insensitive substring: a group named `Googlebot`
/// applies to a crawler calling itself `Googlebot-Image`. Taken literally that
/// has a trap, and this is where it is closed. Our product token is
/// `Lorehaven`, which *contains* `a`, `e`, `n`, `o`, `r`, `l` and `h` — so a
/// file with a stray `User-agent: a` would capture us and apply whatever rules
/// that group happened to carry, silently. A group name of one or two letters
/// does not name a crawler; it is almost always a placeholder or a typo.
///
/// So: a short name has to be exactly our token to count, and a longer one
/// matches as a substring. `names_us("lorehaven/0.1.0", "lorehaven")` is the
/// other direction and is *not* accepted, because a group naming a versioned
/// agent is naming that version, not ours.
fn names_us(name: &str, token: &str) -> bool {
    if name == token {
        return true;
    }
    // Single letters and two-letter combinations match nearly every crawler by
    // substring, so they are held to an exact match.
    if name.len() < 3 {
        return false;
    }
    token.contains(name)
}

/// Split a file into groups.
///
/// A group is one or more consecutive `User-agent` lines followed by that
/// group's rules. A `User-agent` line arriving after a rule line starts a new
/// group — which is the detail a naive parser gets wrong, because it then merges
/// two sites' rules into one list and applies the wrong one.
fn split_groups(body: &str) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    let mut current: Option<Group> = None;
    let mut seen_rule = false;

    for raw in body.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let Some((field, value)) = line.split_once(':') else {
            continue;
        };
        let field = field.trim().to_ascii_lowercase();
        let value = value.trim().to_owned();

        match field.as_str() {
            "user-agent" => {
                if seen_rule {
                    if let Some(group) = current.take() {
                        groups.push(group);
                    }
                    seen_rule = false;
                }
                current
                    .get_or_insert_with(|| Group {
                        agents: Vec::new(),
                        rules: Vec::new(),
                        crawl_delay: None,
                    })
                    .agents
                    .push(value);
            }
            "allow" | "disallow" => {
                // A rule before any `User-agent` line belongs to nothing; the
                // standard says such a group applies to no one, and treating it
                // as global would be the unsafe direction.
                let Some(group) = current.as_mut() else {
                    continue;
                };
                seen_rule = true;
                group.rules.push((field == "allow", value));
            }
            "crawl-delay" => {
                let Some(group) = current.as_mut() else {
                    continue;
                };
                seen_rule = true;
                // Fractional delays exist in the wild; a value we cannot read is
                // dropped rather than defaulted, so an unparseable directive
                // cannot silently become "no wait".
                if let Ok(seconds) = value.parse::<f64>() {
                    if seconds.is_finite() && seconds > 0.0 {
                        let delay = Duration::from_secs_f64(seconds);
                        group.crawl_delay = Some(match group.crawl_delay {
                            Some(existing) => existing.max(delay),
                            None => delay,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(group) = current {
        groups.push(group);
    }
    groups
}

/// Everything from the first `#` onwards is a comment.
///
/// Treated as such rather than trimmed: `Disallow: /a#b` is a path containing a
/// hash, and there is no way to tell the two apart other than by the standard,
/// which says a `#` starts a comment. Recorded because it means such a path
/// cannot be expressed at all.
fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(at) => &line[..at],
        None => line,
    }
}

/// Whether a `robots.txt` pattern matches a path.
///
/// A pattern without `$` matches as a prefix; with `$` it must match to the end
/// of the path. `*` matches any run of characters, including none.
fn pattern_matches(pattern: &str, path: &str) -> bool {
    let (pattern, anchored) = match pattern.strip_suffix('$') {
        Some(stripped) => (stripped, true),
        None => (pattern, false),
    };

    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return if anchored {
            path == pattern
        } else {
            path.starts_with(pattern)
        };
    }

    if !path.starts_with(parts[0]) {
        return false;
    }
    let mut position = parts[0].len();

    for (index, part) in parts[1..].iter().enumerate() {
        let is_last = index == parts.len() - 2;
        if is_last && anchored {
            // The final segment has to land on the end of the path.
            return path[position..].ends_with(part);
        }
        match path[position..].find(part) {
            Some(at) => position += at + part.len(),
            None => return false,
        }
    }

    if anchored {
        // Not `true`: an anchored pattern has to consume the whole path, and a
        // trailing `*` is the only thing that lets it finish early.
        pattern.ends_with('*') || position == path.len()
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse with our own token, for brevity in tests.
    fn rules_of(body: &str) -> RobotsRules {
        RobotsRules::parse(body, "Lorehaven")
    }

    const SYOSETU: &str = "User-agent: *\nCrawl-delay: 1\n\nUser-agent: Mediapartners-Google\nDisallow:\n\nUser-agent: BingBot\nCrawl-delay: 3\n\nUser-agent: meta-externalagent/1.1 (+https://developers.facebook.com/docs/sharing/webmasters/crawler)\nDisallow: /\n\nUser-agent: meta-externalagent/1.1\nDisallow: /\n";

    #[test]
    fn a_real_file_is_read_as_the_site_intended() {
        let rules = RobotsRules::parse(SYOSETU, "Lorehaven");

        // The `*` group applies to us, and it publishes a one-second delay.
        assert_eq!(rules.crawl_delay(), Some(Duration::from_secs(1)));
        assert!(rules.was_found());
        // The `*` group disallows nothing.
        assert!(rules.allows("/n2267be/"));
        assert!(rules.allows("/"));
    }

    #[test]
    fn another_crawlers_delay_is_not_ours() {
        // BingBot's three seconds belongs to BingBot. Applying it to us would be
        // slower than the site asked for; ignoring our own group would be
        // faster. Neither is reading the file.
        let rules = RobotsRules::parse(SYOSETU, "BingBot");
        assert_eq!(rules.crawl_delay(), Some(Duration::from_secs(3)));

        let rules = RobotsRules::parse(SYOSETU, "meta-externalagent/1.1");
        assert!(!rules.allows("/anything"));
    }

    #[test]
    fn a_named_group_beats_the_wildcard_one() {
        let body = "User-agent: *\nDisallow: /\n\nUser-agent: Lorehaven\nDisallow: /private\n";
        let rules = RobotsRules::parse(body, "Lorehaven");

        // The `*` group forbids everything; ours forbids only /private. Getting
        // this backwards would make us refuse the whole site, or (worse, the
        // other way) import from a site that forbids us.
        assert!(rules.allows("/public"));
        assert!(!rules.allows("/private"));
        assert!(!rules.allows("/private/thing"));
    }

    #[test]
    fn the_longest_matching_rule_wins() {
        let body = "User-agent: *\nDisallow: /a\nAllow: /a/b\n";
        let rules = rules_of(body);

        assert!(!rules.allows("/a"));
        assert!(rules.allows("/a/b"));
        assert!(rules.allows("/a/bc"));
        // A longer Disallow beats a shorter Allow.
        let body = "User-agent: *\nAllow: /a\nDisallow: /a/b\n";
        assert!(rules_of(body).allows("/a"));
        assert!(!rules_of(body).allows("/a/b"));
    }

    #[test]
    fn a_tie_between_allow_and_disallow_goes_to_allow() {
        let body = "User-agent: *\nDisallow: /same\nAllow: /same\n";
        assert!(rules_of(body).allows("/same"));
    }

    #[test]
    fn a_wildcard_matches_any_run() {
        let body = "User-agent: *\nDisallow: /search\nDisallow: /*.pdf$\nDisallow: /n*/chapter\n";
        let rules = rules_of(body);

        assert!(!rules.allows("/search"));
        assert!(!rules.allows("/search/deep"));
        assert!(!rules.allows("/docs/a.pdf"));
        assert!(rules.allows("/docs/a.pdf.html"));
        assert!(!rules.allows("/n2267be/chapter"));
        assert!(!rules.allows("/n9525ii/chapter"));
        assert!(rules.allows("/other/chapter"));
    }

    #[test]
    fn an_anchor_means_the_end_of_the_path() {
        let body = "User-agent: *\nDisallow: /exact$\n";
        let rules = rules_of(body);

        assert!(!rules.allows("/exact"));
        // Unanchored this would match; anchored it must not.
        assert!(rules.allows("/exact/child"));
    }

    #[test]
    fn an_empty_disallow_restricts_nothing() {
        let body = "User-agent: *\nDisallow:\n";
        let rules = rules_of(body);

        assert!(rules.allows("/"));
        assert!(rules.allows("/anything"));
        // And it is not carried as a pattern, so it cannot appear in a refusal.
        assert_eq!(rules.directives.len(), 0);
    }

    #[test]
    fn a_missing_file_is_no_restrictions_and_no_delay() {
        let rules = RobotsRules::unrestricted();

        assert!(!rules.was_found());
        assert_eq!(rules.crawl_delay(), None);
        assert!(rules.allows("/anything"));
    }

    #[test]
    fn a_file_with_no_group_for_us_restricts_nothing() {
        let body = "User-agent: SomeOtherBot\nDisallow: /\n";
        let rules = RobotsRules::parse(body, "Lorehaven");

        // No `*` group and no group naming us: the file says nothing to us.
        assert!(rules.allows("/n2267be/"));
        assert_eq!(rules.crawl_delay(), None);
        assert!(rules.was_found());
    }

    #[test]
    fn comments_and_whitespace_and_case_are_handled() {
        let body = "# a comment\n  USER-AGENT : *  \n  DISALLOW:   /secret   # trailing\n\n\nCrawl-Delay: 2.5\n";
        let rules = rules_of(body);

        assert!(!rules.allows("/secret"));
        assert!(rules.allows("/other"));
        assert_eq!(rules.crawl_delay(), Some(Duration::from_millis(2_500)));
    }

    #[test]
    fn a_rule_before_any_user_agent_belongs_to_nobody() {
        // Treated as applying to no one rather than to everyone: the other
        // reading would let a malformed file forbid an entire site.
        let rules = RobotsRules::parse("Disallow: /\nUser-agent: *\nDisallow: /x\n", "Lorehaven");
        assert!(rules.allows("/anything"));
        assert!(!rules.allows("/x"));
    }

    #[test]
    fn a_new_group_starts_when_a_rule_has_been_seen() {
        // Without this, the second group's rules merge into the first and a
        // crawler applies a stranger's restrictions to itself — or, depending
        // which group it lands in, misses its own.
        let body =
            "User-agent: Lorehaven\nDisallow: /for-us\nUser-agent: OtherBot\nDisallow: /for-them\n";
        let rules = rules_of(body);

        assert!(!rules.allows("/for-us"), "our own group's rule was lost");
        assert!(
            rules.allows("/for-them"),
            "another bot's rule was applied to us"
        );
    }

    #[test]
    fn a_one_letter_group_does_not_capture_us_by_substring() {
        // `Lorehaven` contains `a`, so the literal substring rule would put us in
        // the group named `a` and apply whatever it carried — silently, and
        // without the site having meant anything by it.
        let body = "User-agent: a\nDisallow: /\n";
        let rules = rules_of(body);

        assert!(rules.allows("/n2267be/"), "a placeholder group captured us");
    }

    #[test]
    fn a_two_letter_group_does_not_capture_us_either() {
        let body = "User-agent: lo\nDisallow: /\n";
        assert!(rules_of(body).allows("/anything"));
    }

    #[test]
    fn an_exact_short_match_is_still_honoured() {
        // The short-name rule is about accidental capture, not about refusing to
        // be addressed: a site that names us exactly is obeyed whatever length
        // the name is.
        let body = "User-agent: lo\nDisallow: /nope\n";
        assert!(!RobotsRules::parse(body, "lo").allows("/nope"));
    }

    #[test]
    fn a_fractional_delay_is_honoured() {
        let body = "User-agent: *\nCrawl-delay: 0.5\n";
        assert_eq!(
            rules_of(body).crawl_delay(),
            Some(Duration::from_millis(500))
        );
    }

    #[test]
    fn an_unreadable_delay_is_dropped_rather_than_defaulted_to_zero() {
        // A delay of zero would be a decision in the unsafe direction made from
        // a value we did not understand.
        let body = "User-agent: *\nCrawl-delay: occasionally\n";
        assert_eq!(rules_of(body).crawl_delay(), None);
    }

    #[test]
    fn a_negative_or_nonsense_delay_is_not_zero() {
        assert_eq!(
            rules_of("User-agent: *\nCrawl-delay: -5\n").crawl_delay(),
            None
        );
        assert_eq!(
            rules_of("User-agent: *\nCrawl-delay: 0\n").crawl_delay(),
            None
        );
        assert_eq!(
            rules_of("User-agent: *\nCrawl-delay: nan\n").crawl_delay(),
            None
        );
    }

    #[test]
    fn the_largest_delay_in_a_group_is_used() {
        let body = "User-agent: *\nCrawl-delay: 1\nCrawl-delay: 4\n";
        assert_eq!(rules_of(body).crawl_delay(), Some(Duration::from_secs(4)));
    }

    #[test]
    fn our_token_matches_a_group_that_names_us_partially() {
        // The standard's matching is "the crawler's product token contains the
        // robots value", so a site naming us as `Lorehaven` applies even though
        // our full agent is `Lorehaven/0.1.0 (+import)`.
        let body = "User-agent: lorehaven\nDisallow: /nope\n";
        assert!(!rules_of(body).allows("/nope"));
    }
}
