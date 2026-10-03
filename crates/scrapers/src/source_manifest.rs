//! M45-23 — the declarative source-adapter manifest (spec §55.3).
//!
//! A curator submits URLs and CSS selectors. The host interprets them. There is
//! no code here to escape from, which is the entire argument for the declarative
//! path and the reason it can ship before the WASM sandbox of §55.4.
//!
//! **This lives in `lorehaven-scrapers`, not `lorehaven-domain`,** and that is a
//! consequence rather than a preference. Two things it needs are only here:
//! `scraper::Selector` compiles the selectors, and `safety::validate_url` is the
//! §11.5 guard that refuses private, loopback and link-local addresses. Putting
//! the schema in the domain crate would mean either adding a second CSS engine
//! there or reimplementing the address rules — and reimplementing them is exactly
//! the failure §55.3 exists to avoid, since the whole claim is that a declarative
//! adapter inherits §11.5's guards rather than restating them.
//!
//! What *is* domain-level — §21.1's `Manifest` and `Category` — lives in
//! `lorehaven-domain::extension`, because it is shared by every extension kind.

use scraper::Selector;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::safety::{is_forbidden_ip, is_local_hostname, validate_url, FetchPolicy};

/// A §55.3 declarative source adapter.
///
/// `deny_unknown_fields` on every struct here, for the same reason §21.1's
/// `Manifest` has it: the quorum argument is that a steward can read the
/// manifest and see what the adapter does. A silently-ignored key puts a gap
/// between the reviewed text and the running adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceManifest {
    pub source_id: String,
    pub name: String,
    /// Scheme and host, no path. This is also the entire network allowlist —
    /// §55.4.1's lockdown, at the level a declarative manifest can express.
    pub base_url: String,
    /// The adapter's *request* for a pace, in requests per second.
    ///
    /// An upper bound the host may lower, never a floor it honours. §11.5's
    /// `Crawl-delay` and its one-request-per-second floor win on conflict, and a
    /// manifest naming a faster rate is not refused — it is quietly lowered,
    /// because refusing would make an over-eager curator's manifest unusable
    /// rather than safe, and the safe reading of the same number is the host's.
    pub rate_limit_per_second: f64,
    /// `/works/{id}` — the pattern for a work's page.
    pub work_pattern: String,
    /// `/works/{id}/chapters/{num}` — the pattern for one chapter.
    pub chapter_pattern: String,
    pub selectors: Selectors,
    pub pagination: Pagination,
    pub auth: Auth,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selectors {
    pub title: String,
    pub author: String,
    pub summary: String,
    pub body: String,
    pub tags: String,
    pub word_count: String,
    pub date_published: String,
}

/// How the adapter walks from one page to the next.
///
/// `None` is a real value rather than an absent field: a source with 40,000
/// chapters and no pagination needs a single, reachable chapter list, and that is
/// a different thing from an adapter whose pagination selector silently matches
/// nothing. Naming both makes the difference reviewable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Pagination {
    NextLink { selector: String },
    None,
}

/// How the source authenticates, if it does.
///
/// §55.4.2 holds for this path exactly as for the WASM one: the curator declares
/// *that* a login is needed and never supplies or sees the secret. `login_url` is
/// a path, not a credential.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Auth {
    None,
    CookieLogin { login_url: String },
    ApiKey,
    Oauth,
}

/// A manifest whose selectors have been compiled and whose base URL has passed
/// §11.5's validator.
///
/// Holding this rather than `SourceManifest` is what makes §55.3's "compiles at
/// submission" a type-level property: there is no path from a submitted manifest
/// to a running adapter that does not pass through `CompiledSource`.
#[derive(Debug, Clone)]
pub struct CompiledSource {
    pub manifest: SourceManifest,
    /// The single host the adapter may reach. Not a list: a declarative adapter
    /// has one origin, and a second is a different source with its own manifest.
    pub origin: Url,
    pub selectors: CompiledSelectors,
}

#[derive(Debug, Clone)]
pub struct CompiledSelectors {
    pub title: Selector,
    pub author: Selector,
    pub summary: Selector,
    pub body: Selector,
    pub tags: Selector,
    pub word_count: Selector,
    pub date_published: Selector,
}

impl CompiledSelectors {
    /// Every selector, with the field name it belongs to.
    ///
    /// Returned as pairs so a validation error can name the offending field.
    /// Compiling them through one list is also what stops a field being added to
    /// `Selectors` and quietly left out of validation.
    #[must_use]
    pub fn all(&self) -> [(&'static str, &Selector); 7] {
        [
            ("title", &self.title),
            ("author", &self.author),
            ("summary", &self.summary),
            ("body", &self.body),
            ("tags", &self.tags),
            ("word_count", &self.word_count),
            ("date_published", &self.date_published),
        ]
    }

    /// The selector for a named field, or `None` if the name is not one.
    ///
    /// The adapter's parsing paths go through here rather than through the
    /// struct fields, so a field added to `Selectors` is reachable by name from
    /// the same list that validation used — the two cannot drift, which is the
    /// entire reason `all()` exists.
    #[must_use]
    pub fn get(&self, field: &str) -> Option<&Selector> {
        self.all()
            .into_iter()
            .find(|(name, _)| *name == field)
            .map(|(_, sel)| sel)
    }
}

/// Parse and compile a §55.3 manifest in one step.
///
/// The pair is one operation for a caller because both halves must hold before
/// a steward reads the manifest: text that does not parse cannot be reviewed,
/// and selectors that do not compile cannot work. Reporting them separately
/// means two round-trips to discover the same submission is unacceptable.
///
/// Returns every problem at once, which is what `compile` already does.
///
/// Lives here rather than in the HTTP layer so the app crate needs no YAML
/// dependency, and so the offline submission path and the routes enforce exactly
/// the same rule — a second parse-and-check in the route would be a second
/// implementation to keep in step.
pub fn parse_and_compile(text: &str) -> Result<CompiledSource, Vec<String>> {
    let manifest: SourceManifest = match serde_yaml::from_str(text) {
        Ok(m) => m,
        Err(e) => return Err(vec![format!("not valid YAML for a §55.3 manifest: {e}")]),
    };
    manifest.compile()
}

impl SourceManifest {
    /// Compile every selector and validate the base URL.
    ///
    /// **Every** problem is reported, not the first. A reviewer fixing one
    /// selector per round-trip across three submissions is a worse review than a
    /// list, and §55.5's reviewers are the reason this path is cheap.
    pub fn compile(&self) -> Result<CompiledSource, Vec<String>> {
        let mut problems = Vec::new();

        // §11.5's guards are two functions, not one, and using the wrong half is
        // a silent hole. `validate_url` checks the *syntactic* shape — scheme,
        // length, port — and does not resolve anything, so it happily accepts
        // `http://169.254.169.254`. `resolve_public` holds the address rules
        // (§11.5's "address DNS rebinding") and is `async` because it resolves.
        //
        // The literal-address case needs no network and is checked here; the
        // resolved case belongs to `SafeFetcher`, which calls `resolve_public`
        // before every connection and re-checks every redirect. So submission
        // checks everything checkable without a request, and the fetch path
        // owns the rest. Asserting on `validate_url` alone would have made this
        // test pass against a manifest pointing at the cloud metadata service.
        let origin = match validate_url(&self.base_url, &FetchPolicy::default()) {
            Ok(url) => url,
            Err(e) => {
                problems.push(format!(
                    "base_url `{}` is refused by §11.5: {e}",
                    self.base_url
                ));
                return Err(problems);
            }
        };

        if let Some(host) = origin.host_str() {
            // A literal IP in a `base_url` is always either forbidden or a
            // stranger, and a stranger is never the point of a source adapter.
            if let Ok(ip) = host.trim_start_matches('[').trim_end_matches(']').parse() {
                if is_forbidden_ip(ip) {
                    problems.push(format!(
                        "base_url host `{ip}` is not a routable public address (§11.5) — \
                         this includes the cloud metadata service and this instance's own \
                         loopback"
                    ));
                } else {
                    problems.push(format!(
                        "base_url host `{ip}` is a bare address; name the source's host so a \
                         later change of address cannot silently point the adapter elsewhere"
                    ));
                }
            } else if is_local_hostname(host) {
                problems.push(format!(
                    "base_url host `{host}` is a local name; §11.5 refuses it and so does this"
                ));
            }
        }

        if let Some(problem) = base_url_is_too_broad(&origin) {
            problems.push(problem);
        }

        if self.rate_limit_per_second <= 0.0 {
            problems.push(format!(
                "rate_limit_per_second is {}; a rate of zero or less means no requests, \
                 and §11.5's floor of one request per second applies regardless",
                self.rate_limit_per_second
            ));
        }

        if !self.work_pattern.contains("{id}") {
            problems.push("work_pattern must contain `{id}`".to_string());
        }
        if !self.chapter_pattern.contains("{id}") {
            problems.push("chapter_pattern must contain `{id}`".to_string());
        }

        let selectors = match compile_selectors(&self.selectors) {
            Ok(s) => s,
            Err(mut p) => {
                problems.append(&mut p);
                // Without selectors there is no adapter to build, and the
                // remaining checks need a compiled value to be meaningful.
                return Err(problems);
            }
        };

        if let Pagination::NextLink { selector } = &self.pagination {
            if let Err(e) = Selector::parse(selector) {
                problems.push(format!("pagination.selector `{selector}`: {e}"));
            }
        }

        if problems.is_empty() {
            Ok(CompiledSource {
                manifest: self.clone(),
                origin,
                selectors,
            })
        } else {
            Err(problems)
        }
    }

    /// Does this URL belong to the declared source?
    ///
    /// §55.4.1's rule in the form a declarative manifest can enforce: same
    /// scheme, same host. A redirect elsewhere is caught by `SafeFetcher`, which
    /// re-validates every hop; this is the adapter-side statement of the same
    /// boundary, and it is deliberately not a network call.
    #[must_use]
    pub fn covers(&self, origin: &Url, url: &Url) -> bool {
        url.scheme() == origin.scheme() && url.host_str() == origin.host_str()
    }
}

/// A `base_url` that is valid but wider than a single source.
///
/// The dangerous version is not `https://example.org/works/1` — that is merely
/// specific. It is a *host* the adapter does not own, which is what a public
/// wildcard or a bare registrable domain invites.
fn base_url_is_too_broad(origin: &Url) -> Option<String> {
    let host = origin.host_str()?;
    // A bare registrable domain with no subdomain is refused: an adapter for
    // `example.org` can reach every other site that domain happens to serve,
    // including ones its curator does not administer and did not review.
    // `www.example.org` and `archive.example.org` are the normal shapes and
    // pass.
    if !host.contains('.') {
        return Some(format!(
            "base_url host `{host}` has no subdomain; an adapter must name one specific host"
        ));
    }
    None
}

fn compile_selectors(s: &Selectors) -> Result<CompiledSelectors, Vec<String>> {
    let mut problems = Vec::new();

    // `Option` per field, collected first and only unwrapped once every selector
    // has been attempted. `Selector` is not `Display` and has no cheap
    // "is this still valid" query, so the honest shape is: try all of them,
    // report all the failures, and construct the struct only when there are none.
    macro_rules! pick {
        ($field:ident) => {
            match Selector::parse(&s.$field) {
                Ok(sel) => Some(sel),
                Err(e) => {
                    problems.push(format!(
                        "selectors.{} `{}`: {e}",
                        stringify!($field),
                        s.$field
                    ));
                    None
                }
            }
        };
    }

    let title = pick!(title);
    let author = pick!(author);
    let summary = pick!(summary);
    let body = pick!(body);
    let tags = pick!(tags);
    let word_count = pick!(word_count);
    let date_published = pick!(date_published);

    if !problems.is_empty() {
        return Err(problems);
    }

    // Every `Option` is `Some` because `problems` is empty: `pick!` pushes a
    // problem for every `None` it returns, and nothing else consumes them.
    Ok(CompiledSelectors {
        title: title.expect("a None would have pushed a problem"),
        author: author.expect("a None would have pushed a problem"),
        summary: summary.expect("a None would have pushed a problem"),
        body: body.expect("a None would have pushed a problem"),
        tags: tags.expect("a None would have pushed a problem"),
        word_count: word_count.expect("a None would have pushed a problem"),
        date_published: date_published.expect("a None would have pushed a problem"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The manifest from spec §55.3, verbatim.
    ///
    /// If the schema changes and this stops parsing, someone has to decide
    /// whether the spec or the schema is wrong. That is the point: the spec's
    /// example is the one thing a curator will copy, so it cannot be allowed to
    /// rot into fiction.
    pub const SPEC_EXAMPLE: &str = r#"
source_id: "example-archive"
name: "Example Archive"
base_url: "https://example-archive.org"
rate_limit_per_second: 2
work_pattern: "/works/{id}"
chapter_pattern: "/works/{id}/chapters/{num}"
selectors:
  title: "h1.work-title"
  author: ".byline a"
  summary: ".summary blockquote"
  body: ".chapter-content"
  tags: ".tags li"
  word_count: ".stats .words"
  date_published: "meta[property='article:published_time']"
pagination:
  type: next_link
  selector: "a[rel='next']"
auth:
  type: none
"#;

    fn manifest() -> SourceManifest {
        serde_yaml::from_str(SPEC_EXAMPLE).expect("the spec's example parses")
    }

    #[test]
    fn the_specs_own_example_manifest_parses() {
        let m = manifest();
        assert_eq!(m.source_id, "example-archive");
        assert_eq!(m.selectors.body, ".chapter-content");
        assert_eq!(m.auth, Auth::None);
        assert_eq!(
            m.pagination,
            Pagination::NextLink {
                selector: "a[rel='next']".to_string()
            }
        );
        assert!(m.compile().is_ok(), "and it compiles");
    }

    #[test]
    fn a_manifest_whose_selector_does_not_compile_is_refused_naming_it() {
        let mut m = manifest();
        m.selectors.body = "div[[[unclosed".to_string();
        let problems = m.compile().unwrap_err();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("selectors.body"),
            "the error names the field: {problems:?}"
        );
        assert!(problems[0].contains("div[[[unclosed"), "and the selector");
    }

    /// §55.3's "reports every problem" — a reviewer fixing one per round trip is
    /// a worse review than a list.
    #[test]
    fn a_manifest_reports_every_problem_not_just_the_first() {
        let mut m = manifest();
        m.selectors.title = "a[[".to_string();
        m.selectors.body = "b[[".to_string();
        m.rate_limit_per_second = 0.0;
        m.work_pattern = "/no-placeholder".to_string();

        let problems = m.compile().unwrap_err();
        assert!(problems.len() >= 4, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("selectors.title")));
        assert!(problems.iter().any(|p| p.contains("selectors.body")));
        assert!(problems.iter().any(|p| p.contains("rate_limit_per_second")));
        assert!(problems.iter().any(|p| p.contains("work_pattern")));
    }

    #[test]
    fn an_unknown_selector_key_is_refused() {
        let text = SPEC_EXAMPLE.replace(
            "  body: \".chapter-content\"",
            "  body: \".chapter-content\"\n  bodyy: \".nope\"",
        );
        let err = serde_yaml::from_str::<SourceManifest>(&text).unwrap_err();
        assert!(err.to_string().contains("bodyy"), "{err}");
    }

    #[test]
    fn a_manifest_with_a_wrongly_typed_field_is_refused() {
        // `rate_limit_per_second` is a number. A quoted one is a curator's typo,
        // and accepting it as zero would silently mean "no requests".
        let text =
            SPEC_EXAMPLE.replace("rate_limit_per_second: 2", "rate_limit_per_second: \"two\"");
        assert!(serde_yaml::from_str::<SourceManifest>(&text).is_err());
    }

    /// §55.8's second acceptance line: a private address is refused at
    /// **submission**, before any fetch exists to leak.
    ///
    /// Asserted through the public predicate rather than a hand-written list, so
    /// the test cannot pass while an address the guard also refuses is missing
    /// from it. `169.254.169.254` is the cloud metadata service and is the one
    /// that matters most; the `::ffff:` form is the same address in a hat, which
    /// `is_forbidden_ip` unwraps deliberately.
    #[test]
    fn a_base_url_pointing_at_a_private_address_is_refused_as_private() {
        for bad in [
            "http://169.254.169.254",
            "http://127.0.0.1:5432",
            "http://10.0.0.1",
            "http://192.168.1.1",
            "http://[::1]",
            "http://[::ffff:169.254.169.254]",
        ] {
            let mut m = manifest();
            m.base_url = bad.to_string();
            let problems = m.compile().unwrap_err();
            // Assert the *reason*, not merely that something objected. An earlier
            // version of this test asserted only "a problem mentioning base_url",
            // and it stayed green after the `is_forbidden_ip` guard was deleted --
            // because the "bare address" branch also names `base_url` and caught
            // the same URL. A test that passes with the guard it guards removed is
            // the same non-evidence as a green test on unfixed code.
            assert!(
                problems
                    .iter()
                    .any(|p| p.contains("not a routable public address")),
                "{bad} was not refused for being private: {problems:?}"
            );
        }
    }

    #[test]
    fn a_base_url_naming_a_local_host_is_refused_as_local() {
        for bad in ["http://localhost", "http://localhost:5432"] {
            let mut m = manifest();
            m.base_url = bad.to_string();
            let problems = m.compile().unwrap_err();
            assert!(
                problems.iter().any(|p| p.contains("is a local name")),
                "{bad} was not refused for being local: {problems:?}"
            );
        }
    }

    #[test]
    fn a_bare_registrable_domain_is_refused_as_too_broad() {
        let mut m = manifest();
        m.base_url = "https://example".to_string();
        let problems = m.compile().unwrap_err();
        assert!(
            problems.iter().any(|p| p.contains("no subdomain")),
            "{problems:?}"
        );

        // And the normal shapes pass.
        for good in ["https://archive.example.org", "https://www.example.org"] {
            let mut m = manifest();
            m.base_url = good.to_string();
            assert!(m.compile().is_ok(), "{good} should be accepted");
        }
    }

    /// The boundary the whole path rests on: an adapter reads its own source and
    /// nothing else. This is the property Path B has no declarative equivalent
    /// of, and the reason §55.6's gate is about the same guard.
    #[test]
    fn covers_accepts_only_the_declared_origin() {
        let m = manifest();
        let origin = Url::parse("https://example-archive.org").unwrap();

        assert!(m.covers(
            &origin,
            &Url::parse("https://example-archive.org/works/1").unwrap()
        ));
        assert!(!m.covers(
            &origin,
            &Url::parse("https://elsewhere.example/works/1").unwrap()
        ));
        assert!(!m.covers(
            &origin,
            &Url::parse("http://example-archive.org/works/1").unwrap()
        ));
        // The metadata service, and this instance's own database.
        assert!(!m.covers(
            &origin,
            &Url::parse("http://169.254.169.254/latest").unwrap()
        ));
        assert!(!m.covers(&origin, &Url::parse("http://127.0.0.1:5432").unwrap()));
    }

    #[test]
    fn a_manifest_with_no_pagination_compiles() {
        let mut m = manifest();
        m.pagination = Pagination::None;
        assert!(m.compile().is_ok());
    }

    #[test]
    fn a_cookie_login_names_a_path_not_a_credential() {
        let text = SPEC_EXAMPLE.replace(
            "auth:\n  type: none",
            "auth:\n  type: cookie_login\n  login_url: \"/login\"",
        );
        let m: SourceManifest = serde_yaml::from_str(&text).unwrap();
        assert_eq!(
            m.auth,
            Auth::CookieLogin {
                login_url: "/login".to_string()
            }
        );
        // A password in the manifest is not a field that exists.
        let leaked = text.replace(
            "login_url: \"/login\"",
            "login_url: \"/login\"\n  password: \"hunter2\"",
        );
        assert!(
            serde_yaml::from_str::<SourceManifest>(&leaked).is_err(),
            "a manifest has nowhere to put a password, and must refuse one"
        );
    }

    #[test]
    fn the_json_and_yaml_encodings_agree() {
        let from_yaml: SourceManifest = serde_yaml::from_str(SPEC_EXAMPLE).unwrap();
        let json = serde_json::to_string(&from_yaml).unwrap();
        assert_eq!(
            serde_json::from_str::<SourceManifest>(&json).unwrap(),
            from_yaml,
            "the two encodings must not drift"
        );
        // And back again, so a stored JSON manifest can be shown to a reviewer
        // as the YAML they submitted.
        let back: SourceManifest =
            serde_yaml::from_str(&serde_yaml::to_string(&from_yaml).unwrap()).unwrap();
        assert_eq!(back, from_yaml);
    }

    #[test]
    fn a_manifest_compiles_once_and_caches_its_selectors() {
        let compiled = manifest().compile().expect("compiles");
        // The point is structural: `CompiledSource` holds `Selector` values, not
        // the strings they came from, so the hot path has nothing left to parse.
        // `Selector` is not `Display`, so the evidence is the type: to read a
        // selector back as text a caller must go to `manifest`, which is the
        // uncompiled original.
        let _: &Selector = &compiled.selectors.body;
        assert_eq!(compiled.selectors.all().len(), 7, "all seven are present");
        assert_eq!(compiled.selectors.all()[3].0, "body", "in field order");
    }

    /// `all()` and `get()` read the same list, so a field cannot be compiled but
    /// unreachable by name — the drift that would let a `Selectors` field exist,
    /// validate, and then never be parsed.
    ///
    /// Asserted against the *names*, because that is the thing that can drift:
    /// the count and the lookup both come from `Selectors`' own fields.
    #[test]
    fn every_selector_field_is_reachable_by_name() {
        let compiled = manifest().compile().expect("compiles");

        for (name, _) in compiled.selectors.all() {
            assert!(
                compiled.selectors.get(name).is_some(),
                "{name} compiled but is not reachable by name"
            );
        }
        // And the whole surface is exactly `Selectors`' fields — seven, with no
        // more and no fewer. A new field added to the struct without adding it
        // here fails this count.
        assert_eq!(compiled.selectors.all().len(), 7);
        assert_eq!(
            compiled.selectors.all().map(|(n, _)| n),
            [
                "title",
                "author",
                "summary",
                "body",
                "tags",
                "word_count",
                "date_published"
            ]
        );
        assert!(compiled.selectors.get("nope").is_none());
    }

    #[test]
    fn json_round_trip_of_the_whole_manifest() {
        let m = manifest();
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["source_id"], json!("example-archive"));
        assert_eq!(serde_json::from_value::<SourceManifest>(v).unwrap(), m);
    }
}
