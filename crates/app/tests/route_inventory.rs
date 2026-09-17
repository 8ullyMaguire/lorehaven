//! Route-inventory audit — spec §2.3.
//!
//! Every route handler in the application must declare an audience
//! extractor (`MaybeSession` for public doors, `RequireSession` for
//! authenticated doors).  Handlers without either extractor are silent
//! defaults to anonymous-only and leak data to unauthenticated callers.
//! This test scans every route source file and fails the build if any
//! `async fn` handler lacks a declared audience.

use std::fs;
use std::path::Path;

/// Check whether a function signature contains an audience extractor
/// (`MaybeSession`, `RequireSession`, or `RequirePseud`).
fn has_audience_extractor(sig: &str) -> bool {
    let args_start = match sig.find('(') {
        Some(i) => i,
        None => return false,
    };
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
    args.contains("MaybeSession")
        || args.contains("RequireSession")
        || args.contains("RequirePseud")
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

#[test]
fn every_route_has_declared_audience() {
    // Cargo runs integration tests with cwd = crate root (crates/app/).
    let routes_dir = Path::new("src/routes");
    let mut failures: Vec<String> = Vec::new();
    for entry in fs::read_dir(routes_dir).expect("read routes dir") {
        let entry = entry.expect("read dir entry");
        let path = entry.path();
        if !path.extension().is_some_and(|e| e == "rs") {
            continue;
        }
        let fname = path.file_name().unwrap().to_string_lossy().to_string();
        if fname == "mod.rs" || fname == "lib.rs" {
            continue;
        }
        let src = fs::read_to_string(&path).expect("read route source");
        for (line_no, line) in src.lines().enumerate() {
            let trimmed = line.trim();
            // Only check `pub async fn` — these are registered route
            // handlers.  Helper functions are `async fn` without `pub`.
            if !trimmed.starts_with("pub async fn") {
                continue;
            }
            let sig = collect_signature(&src, line_no);
            // Only check functions that have `State<AppState>` in
            // their signature — these are registered route handlers.
            // Utility functions (like `check_storage`) are `pub
            // async fn` too but have no `State` extractor.
            if !sig.contains("State<AppState>") && !sig.contains("State(app") {
                continue;
            }
            if !has_audience_extractor(&sig) {
                let fn_name = trimmed
                    .split_whitespace()
                    .nth(2)
                    .unwrap_or("unknown")
                    .trim_end_matches('(');
                failures.push(format!(
                    "{}:{} — {} has no audience extractor",
                    fname,
                    line_no + 1,
                    fn_name
                ));
            }
        }
    }
    if !failures.is_empty() {
        panic!("Routes without declared audience:\n{}", failures.join("\n"));
    }
}
