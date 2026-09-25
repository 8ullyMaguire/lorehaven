//! Sanitation of imported chapter bodies (spec §11.4's "sanitation" step).
//!
//! A chapter body is HTML written by a stranger, on a site we do not control,
//! and about to be stored and then rendered to a reader. It is therefore treated
//! as hostile input, and this module is the only thing that turns it into
//! storage-ready markup.
//!
//! # The policy
//!
//! **Allow-list, not deny-list.** A tag survives because it is on the list;
//! everything else is dropped along with its attributes. A deny-list has to
//! name every dangerous thing in a language that grows new ones — `<script>`,
//! `<iframe>`, `onclick=`, `<form action>`, `<meta http-equiv=refresh>`,
//! `<style>` with `position:fixed`, `<svg>` with `<animate>` and a hundred more
//! — while an allow-list fails closed, by construction, on whatever nobody
//! thought of.
//!
//! **Attributes are dropped unless they are on a per-tag list.** That is what
//! removes every `on*` handler without naming one, and every `style` attribute
//! without parsing CSS. The only attribute that survives at all is a link's
//! `href`, and only after its scheme has been checked and it has been made
//! absolute against the page it came from.
//!
//! **Content of a few tags is dropped, not unwrapped.** `<script>alert(1)</script>`
//! must lose the script *text* as well as the tags: unwrapping it would put
//! `alert(1)` into the prose.
//!
//! **Nothing is a network reference.** Images, iframes and other embedded
//! resources are removed rather than pointed at (spec §11.5: "Strip or proxy
//! unsafe embedded resources"). A stripped image is a visible omission; a
//! proxied one is a request from the reader's browser to a third party who now
//! knows what they are reading. Proxy support, when it arrives, must route
//! through this server rather than the reader.

use url::Url;

use crate::html_unescape;

/// Tags whose content is discarded along with the tag.
///
/// Unwrapping these instead of dropping them is the single most common way a
/// sanitiser leaks: the markup goes, the payload stays as text, and a
/// `javascript:` URL or a template expression becomes part of the prose.
const DROP_CONTENT: [&str; 9] = [
    "script", "style", "noscript", "template", "iframe", "object", "embed", "svg", "canvas",
];

/// Tags that survive, with the attributes they may keep.
///
/// Deliberately short. Imported text is prose; anything beyond emphasis,
/// structure and links is a source's chrome that the reader did not ask for.
const ALLOWED: [(&str, &[&str]); 22] = [
    ("p", &[]),
    ("br", &[]),
    ("hr", &[]),
    ("em", &[]),
    ("i", &[]),
    ("strong", &[]),
    ("b", &[]),
    ("u", &[]),
    ("s", &[]),
    ("sup", &[]),
    ("sub", &[]),
    ("blockquote", &[]),
    ("ul", &[]),
    ("ol", &[]),
    ("li", &[]),
    ("h1", &[]),
    ("h2", &[]),
    ("h3", &[]),
    ("h4", &[]),
    ("h5", &[]),
    ("h6", &[]),
    ("a", &["href"]),
];

/// Tags that never have a closing tag.
const VOID_TAGS: [&str; 4] = ["br", "hr", "img", "wbr"];

/// Extract every `src` attribute from `<img>` tags in `html`, resolved against
/// `base`.
///
/// The sanitiser strips `<img>` tags (they are not on the allow-list), so this
/// runs **before** sanitisation to rescue the image URLs for the media-rescue
/// pipeline (spec §32.7.9). Returns the URLs in document order, with
/// duplicates removed (first occurrence wins).
pub fn extract_image_urls(html: &str, base: Option<&Url>) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut rest = html;

    while let Some(lt) = rest.find("<img") {
        let tail = &rest[lt..];
        let Some(gt) = find_tag_end(tail) else {
            rest = &tail[4..];
            continue;
        };
        let tag = &tail[..=gt];
        if let Some(src) = extract_src(tag) {
            let resolved = resolve_image_url(&src, base);
            if !resolved.is_empty() && seen.insert(resolved.clone()) {
                urls.push(resolved);
            }
        }
        rest = &tail[gt + 1..];
    }
    urls
}

/// Extract the `src` attribute value from a single `<img ...>` tag string,
/// or `None` if the tag has no `src`.
fn extract_src(tag: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let src_pos = lower.find("src")?;
    let after = &tag[src_pos + 3..];
    let after = after.trim_start();
    let after = after.strip_prefix('=')?;
    let after = after.trim_start();
    if let Some(after) = after.strip_prefix('"') {
        let end = after.find('"')?;
        Some(after[..end].to_string())
    } else if let Some(after) = after.strip_prefix('\'') {
        let end = after.find('\'')?;
        Some(after[..end].to_string())
    } else {
        let end = after
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(after.len());
        Some(after[..end].to_string())
    }
}

/// Resolve an image URL against `base`. Returns an empty string for
/// non-http(s) schemes or unparseable URLs.
fn resolve_image_url(src: &str, base: Option<&Url>) -> String {
    let trimmed = src.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with("data:") || trimmed.starts_with("javascript:") {
        return String::new();
    }
    match Url::parse(trimmed) {
        Ok(url) => {
            if url.scheme() == "http" || url.scheme() == "https" {
                url.as_str().to_string()
            } else {
                String::new()
            }
        }
        Err(_) => {
            let Some(base) = base else {
                return String::new();
            };
            base.join(trimmed)
                .map(|u| {
                    if u.scheme() == "http" || u.scheme() == "https" {
                        u.as_str().to_string()
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default()
        }
    }
}

/// Sanitise a fragment of HTML, making links absolute against `base`.
///
/// `base` is the URL the fragment came from. A relative link in the body
/// (`/works/1`) is meaningless once the body is stored away from its source, so
/// it is resolved here — and a link whose scheme is not http(s) is dropped
/// without being made absolute, which is what stops a stored `javascript:`
/// becoming a live link in a reader's browser.
#[must_use]
pub fn sanitize_fragment(html: &str, base: Option<&Url>) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let mut open: Vec<String> = Vec::new();

    while let Some(lt) = rest.find('<') {
        push_text(&mut out, &rest[..lt]);
        let tail = &rest[lt..];

        // A comment or a doctype: skip to the end of it.
        if tail.starts_with("<!--") {
            match tail.find("-->") {
                Some(end) => rest = &tail[end + 3..],
                None => break,
            }
            continue;
        }
        if tail.starts_with("<!") || tail.starts_with("<?") {
            match tail.find('>') {
                Some(end) => rest = &tail[end + 1..],
                None => break,
            }
            continue;
        }

        let Some(gt) = find_tag_end(tail) else {
            // An unclosed `<` is text, not a tag.
            push_text(&mut out, "<");
            rest = &tail[1..];
            continue;
        };
        let tag = &tail[1..gt];
        rest = &tail[gt + 1..];

        let closing = tag.starts_with('/');
        // Strip the slash *before* looking for the end of the name: otherwise
        // the name of `</em>` is empty, the tag is skipped, and every closing
        // tag in the document is silently dropped.
        let body = if closing { &tag[1..] } else { tag };
        let name_end = body
            .find(|c: char| c.is_whitespace() || c == '/')
            .unwrap_or(body.len());
        let name = body[..name_end].trim().to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }

        if DROP_CONTENT.contains(&name.as_str()) {
            if closing {
                continue;
            }
            // Skip to this tag's closing partner, accounting for nesting of the
            // same tag — a `<div>` full of `<div>`s is not closed by the first
            // `</div>`.
            rest = skip_element(rest, &name);
            continue;
        }

        let Some((_, allowed_attributes)) = ALLOWED
            .iter()
            .find(|(allowed, _)| *allowed == name.as_str())
        else {
            // Not allowed: unwrap it, keeping its text for the next iteration.
            continue;
        };

        if closing {
            // Close only a tag we actually opened, and close the ones nested
            // inside it that the source forgot — an unclosed `<em>` must not
            // swallow the rest of the chapter.
            if let Some(position) = open.iter().rposition(|tag| tag == &name) {
                for stale in open.drain(position..).rev() {
                    out.push_str("</");
                    out.push_str(&stale);
                    out.push('>');
                }
            }
            continue;
        }

        let self_closing = tag.trim_end().ends_with('/') || VOID_TAGS.contains(&name.as_str());
        out.push('<');
        out.push_str(&name);
        push_attributes(&mut out, tag, allowed_attributes, base);
        if self_closing {
            out.push_str(" />");
        } else {
            out.push('>');
            open.push(name);
        }
    }

    push_text(&mut out, rest);
    // Anything left open is closed, so a fragment cannot leak emphasis into
    // whatever the reader renders next to it.
    for stale in open.iter().rev() {
        out.push_str("</");
        out.push_str(stale);
        out.push('>');
    }
    out
}

/// Find the `>` that ends a tag, skipping one inside a quoted attribute value.
fn find_tag_end(tail: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (index, ch) in tail.char_indices() {
        match quote {
            Some(open) if ch == open => quote = None,
            Some(_) => {}
            None => match ch {
                '"' | '\'' => quote = Some(ch),
                '>' => return Some(index),
                _ => {}
            },
        }
    }
    None
}

/// Advance past `</name>` for the element currently open.
///
/// Counts nested opens of the same tag so that `<div><div>x</div></div>` is
/// consumed whole. When the closing tag is missing entirely — which a malformed
/// source will produce — the rest of the document is treated as inside it, and
/// the caller's loop terminates naturally.
fn skip_element<'a>(rest: &'a str, name: &str) -> &'a str {
    let open_tag = format!("<{name}");
    let close_tag = format!("</{name}");
    let mut depth = 1usize;
    let mut cursor = rest;

    loop {
        let next_close = cursor.find(&close_tag);
        let next_open = cursor.find(&open_tag);
        match (next_open, next_close) {
            (_, None) => return "",
            (Some(open), Some(close)) if open < close => {
                let after_open = &cursor[open + open_tag.len()..];
                // `<scriptfoo` is not `<script`.
                if after_open
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_whitespace() || c == '>' || c == '/')
                {
                    depth += 1;
                }
                cursor = &cursor[open + open_tag.len()..];
            }
            (_, Some(close)) => {
                depth -= 1;
                cursor = &cursor[close + close_tag.len()..];
                if depth == 0 {
                    return cursor.strip_prefix('>').unwrap_or(cursor);
                }
            }
        }
    }
}

/// Emit the attributes a tag is allowed to keep, in a fixed order.
fn push_attributes(out: &mut String, tag: &str, allowed: &[&str], base: Option<&Url>) {
    for name in allowed {
        let Some(value) = attribute_value(tag, name) else {
            continue;
        };
        let value = match *name {
            "href" => match clean_href(&value, base) {
                Some(href) => href,
                None => continue,
            },
            _ => value,
        };
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        escape_attribute(out, &value);
        out.push('"');
    }
}

/// Read one attribute's value out of a tag's inner text.
fn attribute_value(tag: &str, wanted: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut cursor = 0usize;
    while let Some(found) = lower[cursor..].find(wanted) {
        let start = cursor + found;
        // The name must be the whole name: not `data-href`, not `hrefx`.
        let before_ok = start == 0
            || lower[..start]
                .chars()
                .last()
                .is_some_and(|c| c.is_whitespace() || c == '"' || c == '\'');
        let after = &lower[start + wanted.len()..];
        let after_trimmed = after.trim_start();
        if !before_ok || !after_trimmed.starts_with('=') {
            cursor = start + wanted.len();
            continue;
        }
        let value_part = after_trimmed[1..].trim_start();
        let (value, _) = match value_part.chars().next() {
            Some(quote @ ('"' | '\'')) => {
                let inner = &value_part[1..];
                let end = inner.find(quote).unwrap_or(inner.len());
                (inner[..end].to_owned(), ())
            }
            _ => {
                let end = value_part
                    .find(|c: char| c.is_whitespace())
                    .unwrap_or(value_part.len());
                (value_part[..end].to_owned(), ())
            }
        };
        return Some(html_unescape(&value));
    }
    None
}

/// Resolve and vet a link target.
///
/// Returns `None` for anything the reader should not be able to click.
fn clean_href(raw: &str, base: Option<&Url>) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let resolved = match Url::parse(trimmed) {
        Ok(url) => url,
        Err(url::ParseError::RelativeUrlWithoutBase) => base?.join(trimmed).ok()?,
        Err(_) => return None,
    };
    match resolved.scheme() {
        "http" | "https" | "mailto" => Some(resolved.to_string()),
        // `javascript:`, `data:`, `file:`, `vbscript:` and whatever comes next.
        _ => None,
    }
}

/// Write text with its entities resolved and the characters that would
/// otherwise become markup escaped.
///
/// The input is HTML, so `&amp;` in it means an ampersand; escaping without
/// decoding first would store `&amp;amp;` and the reader would see `&amp;`.
fn push_text(out: &mut String, text: &str) {
    for ch in html_unescape(text).chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            _ => out.push(ch),
        }
    }
}

/// Escape an attribute value for a double-quoted attribute.
fn escape_attribute(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("&quot;"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

/// Whether sanitised output contains no markup at all.
///
/// Used by the import to notice a chapter whose body a source's own "chapter is
/// empty" placeholder was, so an import does not store a blank chapter as
/// though it were content.
#[must_use]
pub fn is_blank(html: &str) -> bool {
    let mut stripped = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        stripped.push_str(&rest[..lt]);
        match rest[lt..].find('>') {
            Some(gt) => {
                stripped.push(' ');
                rest = &rest[lt + gt + 1..];
            }
            None => {
                rest = "";
                break;
            }
        }
    }
    stripped.push_str(rest);
    stripped.split_whitespace().next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://example.com/works/1").unwrap()
    }

    #[test]
    fn prose_survives() {
        let out = sanitize_fragment(
            "<p>Hello <em>world</em>, this is <strong>fine</strong>.</p><br />",
            None,
        );
        assert_eq!(
            out,
            "<p>Hello <em>world</em>, this is <strong>fine</strong>.</p><br />"
        );
    }

    #[test]
    fn script_tags_lose_their_content_not_just_their_tags() {
        let out = sanitize_fragment("before<script>alert('xss')</script>after", None);
        assert_eq!(out, "beforeafter");
        assert!(!out.contains("alert"));
    }

    #[test]
    fn a_nested_dropped_element_is_consumed_whole() {
        let out = sanitize_fragment("a<style><style>x</style>y</style>b", None);
        assert_eq!(out, "ab");
    }

    #[test]
    fn event_handlers_are_removed_with_every_other_attribute() {
        let out = sanitize_fragment(
            r#"<p onclick="steal()" style="position:fixed" id="x" class="y">text</p>"#,
            None,
        );
        assert_eq!(out, "<p>text</p>");
    }

    #[test]
    fn a_javascript_link_is_dropped_while_its_text_survives() {
        let out = sanitize_fragment(r#"<a href="javascript:alert(1)">click</a>"#, Some(&base()));
        assert_eq!(out, "<a>click</a>");
        let out = sanitize_fragment(r#"<a href=" data:text/html,<b>">x</a>"#, Some(&base()));
        assert_eq!(out, "<a>x</a>");
    }

    #[test]
    fn a_relative_link_is_made_absolute_and_a_good_one_is_kept() {
        let out = sanitize_fragment(r#"<a href="/works/2">next</a>"#, Some(&base()));
        assert_eq!(out, r#"<a href="https://example.com/works/2">next</a>"#);
        let out = sanitize_fragment(r#"<a href="https://other.example/x">o</a>"#, Some(&base()));
        assert_eq!(out, r#"<a href="https://other.example/x">o</a>"#);
    }

    #[test]
    fn an_image_is_stripped_rather_than_pointed_at() {
        let out = sanitize_fragment(
            r#"<p>a <img src="https://tracker.example/p.gif">b</p>"#,
            None,
        );
        assert_eq!(out, "<p>a b</p>");
        assert!(!out.contains("tracker"));
    }

    #[test]
    fn iframe_and_object_are_consumed() {
        let out = sanitize_fragment(
            r#"x<iframe src="https://evil.example/"></iframe>y<object data="d"></object>z"#,
            None,
        );
        assert_eq!(out, "xyz");
    }

    #[test]
    fn an_unclosed_emphasis_does_not_swallow_the_rest() {
        let out = sanitize_fragment("<p>a <em>b</p>", None);
        assert_eq!(out, "<p>a <em>b</em></p>");
    }

    #[test]
    fn a_mismatched_close_is_dropped_rather_than_emitted() {
        let out = sanitize_fragment("a</div>b", None);
        assert_eq!(out, "ab");
    }

    #[test]
    fn entities_and_bare_markup_are_escaped_in_text() {
        assert_eq!(sanitize_fragment("a & b", None), "a &amp; b");
        assert_eq!(sanitize_fragment("x < y", None), "x &lt; y");
        // A `&` that is already an entity is left as an entity, not doubled.
        assert_eq!(sanitize_fragment("&amp;", None), "&amp;");
    }

    #[test]
    fn comments_and_doctypes_disappear() {
        let out = sanitize_fragment("a<!-- note -->b<!DOCTYPE html>c", None);
        assert_eq!(out, "abc");
    }

    #[test]
    fn a_gt_inside_an_attribute_value_does_not_end_the_tag() {
        let out = sanitize_fragment(
            r#"<a href="https://e.example/?a=1>2" title="x">t</a>"#,
            None,
        );
        assert_eq!(out, r#"<a href="https://e.example/?a=1%3E2">t</a>"#);
    }

    #[test]
    fn an_attribute_name_must_match_completely() {
        let out = sanitize_fragment(r#"<a data-href="/x" href="/y">t</a>"#, Some(&base()));
        assert_eq!(out, r#"<a href="https://example.com/y">t</a>"#);
    }

    #[test]
    fn a_quote_in_a_link_target_cannot_escape_the_attribute() {
        let out = sanitize_fragment(r#"<a href='https://e.example/"><script>'>t</a>"#, None);
        assert!(!out.contains("<script"));
        assert!(out.contains("&quot;") || out.contains("&#") || !out.contains("=\"\"><"));
    }

    #[test]
    fn blank_detection_ignores_markup_and_whitespace() {
        assert!(is_blank(""));
        assert!(is_blank("<p></p>"));
        assert!(is_blank("<p>  \n </p><br />"));
        assert!(!is_blank("<p>text</p>"));
        assert!(!is_blank("x"));
    }

    #[test]
    fn a_realistic_chapter_survives_intact() {
        let fragment = "<p>In the shadowed annals of the world&rsquo;s monsters, few ambitions \
             burned as fiercely.</p>\n<p>He <em>waited</em>.</p>\n<blockquote><p>Quoted.</p>\
             </blockquote>\n<hr />\n<p>Fin.</p>";
        let out = sanitize_fragment(fragment, None);
        assert!(out.starts_with("<p>In the shadowed annals"));
        assert!(out.contains("<em>waited</em>"));
        assert!(out.contains("<blockquote><p>Quoted.</p></blockquote>"));
        assert!(out.contains("<hr />"));
        assert!(out.ends_with("<p>Fin.</p>"));
    }

    #[test]
    fn extract_image_urls_finds_absolute_urls() {
        let html = r#"<p>text</p><img src="https://example.com/a.png"><img src="https://other.example/b.jpg">"#;
        let urls = extract_image_urls(html, None);
        assert_eq!(
            urls,
            vec!["https://example.com/a.png", "https://other.example/b.jpg",]
        );
    }

    #[test]
    fn extract_image_urls_resolves_relative_urls_against_base() {
        let base = Url::parse("https://archive.example/work/123").unwrap();
        let html = r#"<img src="/uploads/1.png"><img src="https://other.example/2.png">"#;
        let urls = extract_image_urls(html, Some(&base));
        assert_eq!(
            urls,
            vec![
                "https://archive.example/uploads/1.png",
                "https://other.example/2.png",
            ]
        );
    }

    #[test]
    fn extract_image_urls_deduplicates() {
        let html = r#"<img src="https://example.com/a.png"><img src="https://example.com/a.png">"#;
        let urls = extract_image_urls(html, None);
        assert_eq!(urls, vec!["https://example.com/a.png"]);
    }

    #[test]
    fn extract_image_urls_skips_data_and_javascript() {
        let html = r#"<img src="data:image/png;base64,AAAA"><img src="javascript:void(0)">"#;
        assert!(extract_image_urls(html, None).is_empty());
    }

    #[test]
    fn extract_image_urls_skips_bad_scheme() {
        let html = r#"<img src="ftp://example.com/a.png">"#;
        assert!(extract_image_urls(html, None).is_empty());
    }

    #[test]
    fn extract_image_urls_returns_empty_for_no_images() {
        assert!(extract_image_urls("<p>just text</p>", None).is_empty());
    }

    #[test]
    fn extract_image_urls_relative_without_base_is_dropped() {
        let html = r#"<img src="/relative.png">"#;
        assert!(extract_image_urls(html, None).is_empty());
    }

    #[test]
    fn extract_image_urls_ignores_other_tags() {
        let html = r#"<a href="https://example.com">link</a><video src="https://v.example/m.mp4"></video>"#;
        assert!(extract_image_urls(html, None).is_empty());
    }

    #[test]
    fn extract_image_urls_unquoted_src() {
        let html = r#"<img src=https://example.com/a.png>"#;
        let urls = extract_image_urls(html, None);
        assert_eq!(urls, vec!["https://example.com/a.png"]);
    }
}
