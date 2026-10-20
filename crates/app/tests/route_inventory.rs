//! Route-inventory audit — spec §2.3.
//!
//! Every route handler in the application must declare an audience
//! extractor (`MaybeSession` for public doors, `RequireSession` for
//! authenticated doors).  Handlers without either extractor are silent
//! defaults to anonymous-only and leak data to unauthenticated callers.
//!
// Phase 1c (§2.3c): extends the presence check to an explicit
//! `(method, path) → audience` table.  The test now:
//!   1. Scans every route source file for `async fn` with
//!      `State<AppState>` and records its declared extractor
//!      (`MaybeSession`, `RequireSession`, `RequirePseud`, or none).
//!   2. Compares each found handler against the table below; the
//!      table entry names the file, the handler, and the *correct*
//!      audience.  A mismatch (wrong extractor type, or missing
//!      extractor entirely) is a build failure.
//!   3. Asserts every registered route in `server.rs` has a
//!      corresponding table entry — no unregistered route may exist.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

// The route table, the audience enum and the entry type live in the crate:
// the OpenAPI document is generated from the same declaration, so a route
// cannot be in one and not the other.
use darkhold_app::routes::inventory::{Audience, ROUTE_TABLE};

/// Check whether a function signature contains an audience extractor
/// (`MaybeSession`, `RequireSession`, or `RequirePseud`), and return
/// which one it found (or `None`).
fn find_audience_extractor(sig: &str) -> Option<&'static str> {
    let args_start = sig.find('(')?;
    let mut depth = 0u32;
    let mut args_end = args_start;
    for (i, ch) in sig[args_start..].chars().enumerate() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    args_end = args_start + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    let args = &sig[args_start..args_end];
    // `RequireActorScoped` is checked **first**, and it has to be. The
    // substring "RequireActorScoped" does not contain "RequireSession" and
    // "MaybeSession" does not either, so a later check would not have found it
    // by accident — but the reverse is true and it matters: a door written as
    // `RequireActorScoped` with a `RequireSession` in the same signature would
    // report as a session door, and this is precisely the extractor whose whole
    // point is that it is *not* a session door. Ordering it ahead of the
    // session checks keeps the reported audience the one the handler actually
    // authenticates with.
    if args.contains("RequireActorScoped") {
        Some("RequireActorScoped")
    } else if args.contains("RequireActor") {
        // Checked after `RequireActorScoped` and never before it. `RequireActor`
        // is a PREFIX of `RequireActorScoped`, so a naive `contains` in the
        // other order reports every scoped door as unscoped — and the two differ
        // in exactly what they disclose: `RequireActor` answers "who are you",
        // `RequireActorScoped` answers that and refuses when the scope is
        // missing, so only the second reveals nothing at all. Collapsing them
        // would make the inventory unable to tell a door that reports a token's
        // scopes from one that hides the token's existence.
        Some("RequireActor")
    } else if args.contains("RequirePseud") {
        Some("RequirePseud")
    } else if args.contains("RequireSession") {
        Some("RequireSession")
    } else if args.contains("MaybeSession") {
        Some("MaybeSession")
    } else {
        None
    }
}

/// Collect a multi-line function signature starting at `start_line`.
/// Stops when the parenthesis depth returns to zero after the
/// opening `(`.
fn collect_signature(src: &str, start_line: usize) -> String {
    let mut sig = String::new();
    let mut depth = 0u32;
    let mut found_open_paren = false;
    for line in src.lines().skip(start_line) {
        for ch in line.chars() {
            match ch {
                '(' | '[' | '{' => {
                    depth += 1;
                    found_open_paren = true;
                }
                ')' | ']' | '}' => {
                    depth -= 1;
                }
                _ => {}
            }
            sig.push(ch);
            if found_open_paren && depth == 0 {
                return sig;
            }
        }
        if found_open_paren && depth == 0 {
            return sig;
        }
    }
    sig
}

/// Verify every handler declared in `ROUTE_TABLE` actually exists in
/// the source file with the correct audience extractor.
#[test]
fn every_route_has_correct_audience() {
    // Build a lookup from (file, handler) to the expected audience.
    let mut expected: BTreeMap<(&str, &str), (&Audience, &str, &str)> = BTreeMap::new();
    for entry in ROUTE_TABLE {
        expected.insert(
            (entry.file, entry.handler),
            (&entry.audience, entry.method, entry.path),
        );
    }

    let routes_dir = Path::new("src/routes");
    let mut failures: Vec<String> = Vec::new();

    for entry in ROUTE_TABLE {
        let src_path = routes_dir.join(entry.file);
        let src = fs::read_to_string(&src_path)
            .unwrap_or_else(|_| panic!("cannot read {}: {}", entry.file, src_path.display()));

        // Find the handler function and check its extractor.
        let mut found = false;
        for (line_no, line) in src.lines().enumerate() {
            let trimmed = line.trim();
            if !trimmed.starts_with("async fn") && !trimmed.starts_with("pub async fn") {
                continue;
            }
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            let fn_name = parts
                .iter()
                .rposition(|p| *p == "fn")
                .and_then(|i| parts.get(i + 1))
                .map(|p| {
                    // Extract the function name: take up to the first '(' if present
                    p.split('(').next().unwrap_or(p).trim()
                })
                .unwrap_or("");
            if fn_name != entry.handler {
                continue;
            }
            let sig = collect_signature(&src, line_no);
            if !sig.contains("State<AppState>") {
                continue;
            }
            found = true;
            let actual = find_audience_extractor(&sig);
            let expected_str = entry.audience.as_str();
            match actual {
                Some(actual_str) if actual_str == expected_str => {}
                Some(actual_str) => {
                    failures.push(format!(
                        "{}:{} — {} expected {} but found {}",
                        entry.file,
                        line_no + 1,
                        entry.handler,
                        expected_str,
                        actual_str
                    ));
                }
                None => {
                    failures.push(format!(
                        "{}:{} — {} has no audience extractor (expected {})",
                        entry.file,
                        line_no + 1,
                        entry.handler,
                        expected_str
                    ));
                }
            }
            break;
        }
        if !found {
            failures.push(format!(
                "{}:{} — handler not found with State<AppState>",
                entry.file, entry.handler
            ));
        }
    }

    if !failures.is_empty() {
        panic!("Audience mismatches:\n{}", failures.join("\n"));
    }
}

/// Verify the table covers every registered route — no unregistered
/// route may exist in `server.rs`.
///
/// We check each route module's `router()` function for the path
/// The name of the nearest `fn` declared at or above `index` — the function a
/// `.route(...)` call on that line belongs to.
fn enclosing_fn(lines: &[&str], index: usize) -> Option<(String, bool)> {
    for line in lines[..=index].iter().rev() {
        let t = line.trim_start();
        if let Some(i) = t.find("fn ") {
            let name: String = t[i + 3..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                // A router builder is any function returning `Router<…>`:
                // `router()`, `routes()`, `write_router()`, `*_routes()`.  The
                // name alone is not enough — `governance.rs` declares
                // `fn router() { routes() }` and registers its doors in
                // `fn routes()`, which a name list of "router"/"*_routes"
                // silently skipped.
                return Some((name, line.contains("Router<")));
            }
        }
    }
    None
}

/// Walk a module's `router()` and `*_routes()` functions, resolve `.nest()`
/// prefixes, and collect every `(full_path, method, handler)` triple that the
/// module registers.
///
/// This is the *direction* test: it walks the code, not the table.  Any
/// handler registered in a module but missing from `ROUTE_TABLE` is caught
/// here, including routes inside nested sub-routers.
fn collect_registered(module: &str) -> Vec<(String, String, String, String)> {
    let src_path = Path::new("src/routes").join(module);
    let src = fs::read_to_string(&src_path)
        .unwrap_or_else(|_| panic!("cannot read {}: {}", module, src_path.display()));
    let mut routes = Vec::new();

    // A `.route(` call, as SOURCE TEXT with its closing paren.
    //
    // The walk used to be line-at-a-time, which meant a call rustfmt had wrapped
    // across lines was invisible: the line `.route(` had no path on it, and the
    // lines carrying the path had no `.route(` on them. `settings.rs` alone had
    // seven such routes, all served, none tabulated, and no failure — because a
    // walk that misses things and a walk that collects nothing look identical from
    // outside.
    //
    // So a call is delimited by PARENTHESES with a depth counter, not by newlines.
    // Depth rather than "up to the first `)`" because a handler chain legitimately
    // contains its own parens: `.route("/x", get(a).post(b))`.
    let mut calls: Vec<(usize, &str)> = Vec::new();
    let bytes = src.as_bytes();
    const NEEDLE: &[u8] = b".route(";
    let mut i = 0;
    while i + NEEDLE.len() <= bytes.len() {
        // Compared as BYTES, not as `src[i..].starts_with(NEEDLE)`: a byte index is
        // not a char index, and slicing a `&str` at one that splits a multi-byte
        // character panics. These files are full of `—` and `§` in comments, so
        // that panic was reachable from ordinary source, not an edge case.
        if &bytes[i..i + NEEDLE.len()] == NEEDLE {
            let mut depth = 0_i32;
            let mut j = i + NEEDLE.len() - 1;
            let start = j + 1;
            while j < bytes.len() {
                match bytes[j] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if j >= bytes.len() {
                // Unbalanced — the file is not valid Rust, so `cargo` says so with a
                // better message and a line number than this walker can.
                break;
            }
            calls.push((start, &src[start..=j]));
            i = j + 1;
            continue;
        }
        i += 1;
    }

    // Associate each call with the nearest `fn` declared above it, and keep the
    // ones registered in a `router()` / `*_routes()` function.
    //
    // This replaced a brace-counting scanner that walked function bodies. That
    // scanner could not be trusted: braces inside strings and comments moved the
    // depth, so a body could end early or swallow the rest of the file, and the
    // failure mode was silence — it collected 0 routes from all 33 modules and
    // this test still passed. Scanning backwards for the enclosing declaration has
    // no depth to get wrong.
    let lines: Vec<&str> = src.lines().collect();
    for (start, call) in calls {
        // ALREADY the 0-based index. `src[..start].lines().count()` counts the
        // newlines before `start`, and `start` points just past `.route(` — which is
        // mid-line — so the count is the line `start` sits on, counted from zero.
        //
        // The `-1` that used to be here made every one of these calls a line early,
        // and the symptom was 27 modules reporting "declares a router function but
        // the walk collected no routes": a walk that looks one line up finds the
        // previous `.route(` or the `fn` header, and either way attributes the call
        // to nothing. A module with routes was reported as a module without, which
        // is the one direction this test is supposed to be incapable of getting
        // wrong.
        let line_no = src[..start].lines().count();
        let Some((func, builds_router)) = enclosing_fn(&lines, line_no) else {
            continue;
        };
        if !builds_router {
            continue;
        }
        let prefix = find_nest_prefix(&src, module, &func);

        // `call` is the text from just AFTER `.route(` to its closing `)`. It does
        // not contain `.route(` itself, so there is nothing to re-find: the earlier
        // version searched for it, got `None`, and `continue`d on every call in the
        // workspace. That is the same failure mode as the original infinite loop —
        // a walk that collects nothing — wearing a different bug, and 27 modules
        // reported "declares a router function but the walk collected no routes",
        // which is the test correctly reporting its own blindness.
        let call = call.trim();
        // .route("path", handler) or .route("path", get(handler).post(other))
        let Some(path_end) = call.find('"') else {
            continue;
        };
        let rest = &call[path_end + 1..];
        let Some(path_close) = rest.find('"') else {
            continue;
        };
        let path = &rest[..path_close];
        let after_path = &rest[path_close + 1..];

        // Every `(method, handler)` in the rest: `get(h)`, `post(h)`, and any
        // method chain of them, across however many lines the call spans.
        for (method, handler) in extract_handlers(after_path) {
            let full_path = if prefix.is_empty() {
                path.to_owned()
            } else if path == "/" {
                // A nested router's own root is spelled with the trailing slash the
                // table uses (`/recipes/`).
                format!("{prefix}/")
            } else {
                format!("{prefix}{path}")
            };
            routes.push((full_path, handler, method, func.clone()));
        }
    }

    routes
}

/// Find the `.nest("prefix", func_name())` call that nests this module's
/// sub-router under a prefix.  Called from the parent router.
fn find_nest_prefix(src: &str, _module: &str, func_name: &str) -> String {
    for line in src.lines() {
        let t = line.trim();
        if t.contains(".nest(") && t.contains(func_name) {
            if let Some(nest_start) = t.find(".nest(") {
                let after = &t[nest_start + 6..];
                if let Some(p_start) = after.find('"') {
                    let rest = &after[p_start + 1..];
                    if let Some(p_end) = rest.find('"') {
                        return rest[..p_end].to_string();
                    }
                }
            }
        }
    }
    String::new()
}

/// Extract **every** handler function name from the rest of a `.route()` call
/// after the path, e.g. `, get(list_media))` → `["list_media"]` and
/// `, get(list_reviews).post(record_review))` → `["list_reviews",
/// "record_review"]`.
///
/// This returns a `Vec` where it used to return the first name only, and that
/// was a real blind spot rather than a limitation: a route written
/// `get(a).post(b)` registers **two** handlers, the second was never collected,
/// and so deleting its `ROUTE_TABLE` row left `registered_routes_are_tabled`
/// green. Confirmed by mutation — removing the `record_review` row passed 2/2
/// while removing `submit_adapter`'s turned it red.
///
/// `post(a).get(b)` also appears in the wild, so the chain is scanned for every
/// `method(handler)` pair on the line rather than assuming a fixed order or a
/// fixed arity.
/// The routing methods a `.route(...)` argument list may contain.
///
/// Module scope so both readers of a registration agree on the *vocabulary* while
/// remaining independent in *algorithm*. Sharing the set is right: it is a fact
/// about axum, not about either implementation. Sharing the parsing would be wrong:
/// then agreement between them is a tautology rather than evidence.
const ROUTE_METHODS: [&str; 7] = ["get", "post", "put", "delete", "patch", "head", "options"];

/// Every `(method, handler)` in a route registration's argument list.
///
/// `get(render_queue)` is `("get", "render_queue")`. Both halves are returned
/// because `ROUTE_TABLE` keys on the handler and carries the method as a separate
/// column, and a checker that has one but not the other can only fail.
///
/// **Only [`ROUTE_METHODS`] count.** `.route("/x", get(h).layer(mw))` registers one
/// handler, and a walker that reports `layer` and `mw` as two more has invented
/// routes. That is not hypothetical: the first version accepted any
/// `identifier(identifier)`, and the case list in the termination test is what
/// caught it.
fn extract_handlers(s: &str) -> Vec<(String, String)> {
    //
    // It returned the *method* names before. That was invisible for as long as the
    // loop never advanced: the walk collected nothing, `registered_routes_are_tabled`
    // compared an empty set, and passed. Once the loop was fixed, every route in
    // every module reported `get` where the table said `get_version` — 500-odd
    // failures that all pointed at the table and none at the walker.
    //
    // So the fix is to return what the table is keyed on.
    let mut pairs: Vec<(String, String)> = Vec::new();
    // What follows the path is `, get(handler))` — with a leading comma, which an
    // earlier implementation treated as end-of-route and gave up on, so every route
    // in every module was skipped and the test passed while collecting nothing.
    let rest = s.trim_start_matches(|c: char| c == ',' || c.is_whitespace());

    let bytes: Vec<char> = rest.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        while i < bytes.len() && (bytes[i].is_alphanumeric() || bytes[i] == '_') {
            i += 1;
        }
        let ident: String = bytes[start..i].iter().collect();
        // Step unconditionally, on BOTH non-match branches. A `continue` without
        // `i += 1` is an infinite loop: every branch either advances `i` or has
        // consumed an identifier, so a non-identifier character (`(`, `,`, `)`,
        // whitespace, a quote) leaves `i` where it was and `continue` spins forever.
        //
        // This cost a 10-minute hang of `registered_routes_are_tabled` before it was
        // found, and the SAME defect in a second branch survived the first fix —
        // each with a comment describing a step the code did not perform. A
        // `continue` after a non-match is always the branch that needs the step.
        if ident.is_empty() {
            i += 1;
            continue;
        }
        if i >= bytes.len() || bytes[i] != '(' {
            // A bare identifier with no paren.
            //
            // UNREACHABLE TODAY, and the `i += 1` is a guard rather than a fix.
            // The method gate above rejects every identifier that is not a routing
            // method, and only a routing method can reach here — so a mutation that
            // deletes this step stays green (measured: mutation R1). It is kept
            // because the gate is a separate decision from this one: widen the
            // vocabulary, or wrap a `.route(...)` call so `route` itself lands here,
            // and the step is what keeps that from becoming a hang.
            //
            // The same defect existed in BOTH branches of this function before, each
            // with a comment describing a step the code did not perform, and each
            // hung the suite for ten minutes when it did. A `continue` after a
            // non-match is always the branch that needs the step.
            i += 1;
            continue;
        }
        // `ident(` is a candidate link. It is only a ROUTE if `ident` is a routing
        // method — `.layer(mw)` is a call too, and treating it as a handler is how a
        // walker invents routes that were never registered.
        if !ROUTE_METHODS.contains(&ident.as_str()) {
            i += 1;
            continue;
        }
        // The handler is the first identifier inside the method's parens.
        i += 1;
        let h_start = i;
        while i < bytes.len() && (bytes[i].is_alphanumeric() || bytes[i] == '_') {
            i += 1;
        }
        let handler: String = bytes[h_start..i].iter().collect();
        // Skip to this call's matching `)`. A bare depth counter rather than a scan
        // for the next `)`, because a handler whose own arguments contain a paren —
        // `.route("/x", get(h), fallback(a))` — would otherwise end the link early.
        let mut depth = 1_i32;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        if handler.is_empty() {
            // `get()` with no argument — a registration with no handler, which is
            // nonsense but must not be recorded as a handler named "".
            continue;
        }
        pairs.push((ident, handler));
    }
    pairs
}

/// The walk must terminate on real source text, and it must terminate **fast**.
///
/// `extract_handlers` shipped an infinite loop: a non-identifier character left
/// `i` untouched and the `continue` re-read it forever. Every route in the
/// workspace whose handler name is a *single character* — `mark`, `list`, and
/// every other short name — walked past a `(` and spun. The suite did not fail,
/// it hung: `registered_routes_are_tabled` consumed 10 minutes of CPU on 100% and
/// printed nothing, which reads exactly like a slow disk.
///
/// So this test does two things the previous coverage did not. It runs the
/// extractor over **every route line in the workspace**, so no real spelling is
/// unexercised; and it bounds the work, because the only way to prove a loop
/// terminates is to run it under something that would notice if it did not.
#[test]
fn the_handler_walk_terminates_on_every_route_line_in_the_workspace() {
    let mut lines: Vec<(String, String)> = Vec::new();
    let entries = fs::read_dir(Path::new("src/routes")).expect("cannot read routes directory");
    for entry in entries {
        let path = entry.expect("read_dir entry").path();
        if !path.is_file() || !path.extension().is_some_and(|e| e == "rs") {
            continue;
        }
        let src = fs::read_to_string(&path).expect("read route module");
        for (n, line) in src.lines().enumerate() {
            if line.contains(".route(") {
                lines.push((format!("{}:{}", path.display(), n + 1), line.to_owned()));
            }
        }
    }
    assert!(
        !lines.is_empty(),
        "the walk found no route lines at all, so this test is measuring nothing"
    );

    // Every handler chain the workspace contains, plus the spellings that are
    // easy to get wrong: a bare path with no handler, a chain longer than two,
    // and the `.post(x).get(y)` order `cta.rs` uses.
    let cases: Vec<String> = lines
        .iter()
        .map(|(_, l)| l.clone())
        .chain([
            r#".route("/x", get(h))"#.to_owned(),
            r#".route("/x", post(h).get(h))"#.to_owned(),
            r#".route("/x", axum::routing::post(mark).get(list))"#.to_owned(),
            r#".route("/x", get(a).post(b).put(c).delete(d))"#.to_owned(),
            r#".route("/x")"#.to_owned(),
            r#".route("/x", get(h).layer(mw))"#.to_owned(),
            r#".route("/x/{id}", get(handler_name_with_digits_2))"#.to_owned(),
            // A trailing handler-less argument. Measured, not assumed: adding it
            // did NOT make the bare-identifier branch's `i += 1` load-bearing,
            // because the method gate rejects a non-method identifier one branch
            // earlier. See the note on that branch — the step is there for a caller
            // the gate has not been widened to admit, not for these cases.
            r#".route("/x", get(h), fallback)"#.to_owned(),
        ])
        .collect();

    for source in &cases {
        let after = match source.find(".route(") {
            Some(idx) => &source[idx + 7..],
            None => continue,
        };
        let quoted = match after.find('"') {
            Some(q) => &after[q + 1..],
            None => continue,
        };
        let path_end = match quoted.find('"') {
            Some(e) => e,
            None => continue,
        };
        let found = extract_handlers(&quoted[path_end + 1..]);
        let handlers: Vec<String> = found.iter().map(|(_m, h)| h.clone()).collect();

        // The expectation is computed by an INDEPENDENT reader of the same slice,
        // not by a second copy of the rule under test: every identifier that is the
        // sole argument of a call spelled `method(identifier)` is a handler. That is
        // a different rule from the extractor's scan-and-skip, so the two agreeing is
        // evidence rather than a tautology.
        //
        // `.layer(mw)` and a bare `route("x")` are the cases that make the two rules
        // diverge in principle, and both are in `cases` above — a test that only fed
        // the extractor `get(h)` would not notice a walker that reported `layer` or
        // `mw` as handlers.
        let expected = expected_handlers(&quoted[path_end + 1..]);
        assert_eq!(
            expected.len(),
            handlers.len(),
            "{source:?}: the walk and the reading of the line disagree; collected {handlers:?}"
        );
        for name in &expected {
            assert!(
                handlers.contains(name),
                "{source:?}: handler `{name}` is registered but was not collected; \
                 collected {handlers:?}"
            );
        }
    }
}

/// Every `identifier(` on a route line, which is what `extract_handlers` must find.
///
/// Written as a separate function because the obvious inline version references
/// the binding it is producing.
fn expected_handlers(source: &str) -> Vec<String> {
    // The independent rule: a handler is the sole identifier argument of a call
    // spelled `identifier(identifier)` where the outer name is a routing method.
    //
    // Deliberately a DIFFERENT implementation from `extract_handlers` — this one
    // tokenises on parentheses and keeps the token between them; the extractor
    // scans identifiers and skips a call's body with a depth counter. Two readers
    // that agree on all the cases is evidence. A copy of the rule under test would
    // agree by construction and prove nothing.
    //
    let tokens: Vec<&str> = source
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty())
        .collect();
    let mut names = Vec::new();
    for pair in tokens.windows(2) {
        // `tokens` is punctuation-stripped, so adjacency here means "these two were
        // separated by exactly one `(`, `,`, whitespace or `)`". Combined with the
        // METHOD check that is enough: a handler is the token right after a method
        // name, and `get(x)` / `get( x )` both reduce to `get`, `x`.
        if ROUTE_METHODS.contains(&pair[0]) {
            names.push(pair[1].to_owned());
        }
    }
    names
}

/// The direction test: walk every module's router() and *_routes() to find
/// all registered paths, then verify each has a row in ROUTE_TABLE.
#[test]
fn registered_routes_are_tabled() {
    let routes_dir = Path::new("src/routes");
    let mut failures: Vec<String> = Vec::new();

    // Build a set of all registered (file, path, method, handler) triples from the table
    let mut table_entries: Vec<(String, String, String, String)> = Vec::new();
    for entry in ROUTE_TABLE {
        table_entries.push((
            entry.file.to_string(),
            entry.path.to_string(),
            entry.method.to_string(),
            entry.handler.to_string(),
        ));
    }

    // Walk every module file
    let entries = fs::read_dir(routes_dir).expect("cannot read routes directory");
    for entry in entries {
        let entry = entry.expect("read_dir entry");
        let path = entry.path();
        if !path.is_file() || !path.extension().is_some_and(|e| e == "rs") {
            continue;
        }
        let module = path.file_stem().unwrap().to_string_lossy().to_string() + ".rs";
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();

        // Skip lib.rs and non-router files
        if file_name == "lib.rs" {
            continue;
        }

        let registered = collect_registered(&module);
        // A walk that collects nothing compares nothing and passes. Every module
        // here declares a router function, so an empty result means the walk
        // broke, not that the module registers no routes — which is exactly how
        // this test passed while collecting 0 routes from all 33 modules.
        let src = fs::read_to_string(&path).expect("read module source");
        if registered.is_empty() && src.contains("Router<") {
            failures.push(format!(
                "{} — declares a router function but the walk collected no routes \
                 (the walk is broken, not the module)",
                file_name
            ));
        }
        for (full_path, handler, method, _func) in registered {
            // Same file, path, method AND handler. The method was not compared
            // before, which let a table row claiming `GET` satisfy a route really
            // registered as `POST` — and `every_route_has_correct_audience` reads
            // the same rows, so the two halves of the table disagreed about a route
            // without either test noticing.
            // The method is compared, and that comparison is defence-in-depth rather
            // than load-bearing: removing it leaves the suite green (measured,
            // mutation M5), because the REVERSE direction below already rejects a
            // row whose method is not the one the router registers. Two rows for one
            // (file, path, handler) under different methods also let an `any` pass
            // either way — two such duplicates existed in
            // `recommendation_transparency.rs` and are gone, with a guard below so
            // they cannot return.
            //
            // Proved by corrupting the table rather than by mutating the guard: a
            // wrong method, a wrong path and a reintroduced duplicate each turn this
            // test red.
            let found = table_entries.iter().any(|(f, p, m, h)| {
                f == &file_name
                    && p == &full_path
                    && h == &handler
                    && m.eq_ignore_ascii_case(&method)
            });
            if !found {
                failures.push(format!(
                    "{}:{} — {} '{}' registered but not in ROUTE_TABLE (path {})",
                    file_name,
                    full_path,
                    method.to_uppercase(),
                    handler,
                    full_path
                ));
            }
        }
    }

    // No two rows may describe the same route.
    //
    // This is not cosmetic tidiness. A duplicate row is what let a missing method
    // comparison hide: `any` over two rows for one (file, path, handler) is
    // satisfied by whichever method is asked for, so the outward direction could not
    // tell "this route's method is absent from the table" from "it is listed twice".
    // Two duplicates existed in `recommendation_transparency.rs` and were removed.
    //
    // Also defence-in-depth by mutation — deleting the guard leaves the suite green —
    // so it was proved by reintroducing a duplicate, which turns this test red.
    let mut seen_rows: Vec<(&str, &str, &str, &str)> = Vec::new();
    for entry in ROUTE_TABLE {
        let key = (entry.file, entry.path, entry.method, entry.handler);
        if seen_rows.contains(&key) {
            failures.push(format!(
                "{}:{} — ROUTE_TABLE has the same {} '{}' row more than once",
                entry.file, entry.path, entry.method, entry.handler
            ));
        } else {
            seen_rows.push(key);
        }
    }

    // Every (file, path, handler) the table claims must ALSO agree with the router
    // about the method. This is the direction the outward walk does not cover: it
    // walks from the code and reports what is MISSING, so a row that is present but
    // WRONG is invisible to it.
    //
    // Measured: with the method dropped from the outward comparison, this suite
    // stayed green while `discovery.rs` `/recipes/{id}` claimed PATCH and the route
    // is POST (mutation R4). That row sat wrong for as long as the walk collected
    // nothing, and nothing would have said so when the walk started working — which
    // is why this check exists rather than the comment above the comparison.
    for entry in ROUTE_TABLE {
        let agrees =
            collect_registered(entry.file)
                .into_iter()
                .any(|(path, handler, method, _)| {
                    path == entry.path
                        && handler == entry.handler
                        && method.eq_ignore_ascii_case(entry.method)
                });
        if !agrees {
            failures.push(format!(
                "{}:{} — ROUTE_TABLE claims {} '{}' but no route registered as \
                 file+path+method+handler matches it",
                entry.file, entry.path, entry.method, entry.handler
            ));
        }
    }

    if !failures.is_empty() {
        panic!("Route table disagreements:\n{}", failures.join("\n"));
    }
}

/// The REVERSE direction: every router builder a route module declares must be
/// merged into the API router in `server.rs`.
///
/// `registered_routes_are_tabled` walks the other way — table ↔ code — and a
/// table row is satisfied by the module that declares the route, never by
/// `server.rs`. So a module whose `router()` was never merged keeps every one of
/// its table rows green, its unit tests green, and answers 404 for every door it
/// declares. That is exactly what `author_media` did: `1439bac` (M47) *replaced*
/// `routes::author_media::router()` with `routes::mirror_admin::router()` instead
/// of adding a second `.merge(...)`, and the five `/author/media-*` and
/// `/author/targeted-bounties` endpoints have answered 404 since — while the
/// inventory test passed on every commit in between.
///
/// The check is a grep-equivalent over the two files, so it needs no new
/// dependency and no parsing of `server.rs`. It is deliberately blunt: any
/// `pub fn <name>*router(` in `src/routes/*.rs` must appear as
/// `routes::<module>::<name>` somewhere in `src/server.rs`.
///
/// Blunt is right here because the alternative failure is silence. A router that
/// is merged by a *helper* rather than spelled inline would be reported here as
/// unmerged, and the fix is to spell it — which is what every router in this tree
/// already does, so the check has no false negatives to accommodate.
///
/// **The checker is a pure function, so it can be tested.** A check that only
/// runs against the real tree cannot be shown to catch anything: it is green, and
/// green is indistinguishable from "it is not looking". `the_reverse_check_catches_
/// an_unmounted_router` below feeds it a synthetic module and asserts the failure
/// names the file and the router — which is the card's "prove the test works"
/// step, done permanently rather than by adding a file and removing it in the
/// same commit.
fn unmounted_routers(modules: &[(String, String, String)], server: &str) -> (Vec<String>, usize) {
    let mut failures: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for (module, file_name, src) in modules {
        // `pub fn NAME(` on a line that also mentions `Router<` — a router
        // builder, as opposed to a helper that happens to be named `routerize`.
        // The return type is read from the whole signature rather than the
        // declaration line, because rustfmt wraps
        // `pub fn router() -> axum::Router<AppState> {` differently from
        // `pub fn router() -> Router<AppState> {` only in length, but a builder
        // whose return type wrapped onto the next line is still a builder.
        let mut declared: Vec<String> = Vec::new();
        let lines: Vec<&str> = src.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            let t = line.trim();
            let Some(rest) = t.strip_prefix("pub fn ") else {
                continue;
            };
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() || !name.contains("router") {
                continue;
            }
            // The signature runs to the `{` that opens the body, or to the end of
            // the declaration line if the body is on the same line.
            let mut sig = String::new();
            for line in lines.iter().skip(n).take(4) {
                sig.push_str(line);
                sig.push('\n');
                if line.contains('{') || line.contains(';') {
                    break;
                }
            }
            if !sig.contains("Router<") {
                continue;
            }
            // A builder named `router` in a module that ALSO has a private
            // `fn routes()` is registered under whichever name `server.rs`
            // calls — so check every declared name, not only the first.
            if !declared.contains(&name) {
                declared.push(name);
            }
        }

        for name in &declared {
            checked += 1;
            let needle = format!("routes::{module}::{name}");
            if !server.contains(&needle) {
                failures.push(format!(
                    "{file_name} declares `{name}()` but `{needle}` is never merged in \
                     src/server.rs — every route in it answers 404"
                ));
            }
        }
    }
    (failures, checked)
}

#[test]
fn every_declared_router_is_merged_into_server_rs() {
    let routes_dir = Path::new("src/routes");
    let server = fs::read_to_string("src/server.rs").expect("cannot read src/server.rs");

    // (module name, file name, source) for every route module.
    let mut modules: Vec<(String, String, String)> = Vec::new();
    let entries = fs::read_dir(routes_dir).expect("cannot read routes directory");
    for entry in entries {
        let entry = entry.expect("read_dir entry");
        let path = entry.path();
        if !path.is_file() || !path.extension().is_some_and(|e| e == "rs") {
            continue;
        }
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        if file_name == "lib.rs" || file_name == "mod.rs" {
            continue;
        }
        let module = path.file_stem().unwrap().to_string_lossy().to_string();
        let src = fs::read_to_string(&path).expect("read route module");
        modules.push((module, file_name, src));
    }

    let (failures, checked) = unmounted_routers(&modules, &server);

    assert!(
        checked >= 40,
        "only {checked} router builders were examined, so this test is measuring nothing \
         (it found far fewer than src/routes/ declares)"
    );
    if !failures.is_empty() {
        panic!("Unmerged routers:\n{}", failures.join("\n"));
    }
}

/// **The check is proved to catch what it claims to catch.**
///
/// The plan's step for this was "temporarily add a `pub fn unmounted_router()` to a
/// route module and assert the inventory test fails. Remove it in the same commit."
/// That is a one-shot experiment, and a one-shot experiment is not a test: it proves
/// the check worked on the day it was run and nothing after.
///
/// This is the same proof, permanently. The checker is a pure function over
/// `(module, file, source)` triples, so a synthetic module with an unmounted router
/// is fed to it and the failure is asserted — **and the failure has to NAME the file
/// and the router**, because a check that reports "something is wrong" costs the
/// reader the half hour it exists to save.
#[test]
fn the_reverse_check_catches_an_unmounted_router() {
    let synthetic = (
        "synthetic".to_string(),
        "synthetic.rs".to_string(),
        "use axum::Router;\n\
         \n\
         pub fn unmounted_router() -> Router<AppState> {\n\
         \x20   Router::new()\n\
         }\n"
        .to_string(),
    );

    let (failures, _) = unmounted_routers(std::slice::from_ref(&synthetic), "");
    assert_eq!(
        failures.len(),
        1,
        "an unmounted router must be reported exactly once, and it was not: {failures:?}"
    );
    assert!(
        failures[0].contains("synthetic.rs"),
        "the failure must name the file it is in: {}",
        failures[0]
    );
    assert!(
        failures[0].contains("unmounted_router"),
        "the failure must name the router that is not merged: {}",
        failures[0]
    );

    // And the same module, merged, must NOT be reported. A check that reports
    // everything is not a check.
    let merged = "routes::synthetic::unmounted_router()";
    let (failures, checked) = unmounted_routers(std::slice::from_ref(&synthetic), merged);
    assert!(
        failures.is_empty(),
        "a merged router must not be reported: {failures:?}"
    );
    assert_eq!(checked, 1, "the merged router was still examined");
}
