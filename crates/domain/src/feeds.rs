//! M18 — Feed document builders (RSS 2.0 and Atom).

/// Feed kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedKind {
    Rss,
    Atom,
    Podcast,
}

impl FeedKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rss => "rss",
            Self::Atom => "atom",
            Self::Podcast => "podcast",
        }
    }
}

/// A feed item entry.
#[derive(Debug, Clone)]
pub struct FeedItem {
    pub title: String,
    pub link: String,
    pub description: String,
    pub published_at: String,
    pub guid: String,
}

/// Generate a stable feed handle from a subject.
pub fn feed_handle(kind: &str, subject: &str) -> String {
    format!("{}-{}", kind, subject.replace(|c: char| !c.is_alphanumeric(), "-"))
}

/// Escape XML special characters.
pub fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Build an RSS 2.0 feed document.
pub fn build_rss(title: &str, link: &str, description: &str, items: &[FeedItem]) -> String {
    let mut rss = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
<channel>
<title>{}</title>
<link>{}</link>
<description>{}</description>
"#,
        escape_xml(title),
        escape_xml(link),
        escape_xml(description)
    );

    for item in items {
        rss.push_str(&format!(
            r#"<item>
<title>{}</title>
<link>{}</link>
<description>{}</description>
<guid>{}</guid>
<pubDate>{}</pubDate>
</item>
"#,
            escape_xml(&item.title),
            escape_xml(&item.link),
            escape_xml(&item.description),
            escape_xml(&item.guid),
            escape_xml(&item.published_at)
        ));
    }

    rss.push_str("</channel>
</rss>");
    rss
}

/// Build an Atom feed document.
pub fn build_atom(title: &str, link: &str, description: &str, items: &[FeedItem]) -> String {
    let mut atom = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<title>{}</title>
<link href="{}"/>
<subtitle>{}</subtitle>
"#,
        escape_xml(title),
        escape_xml(link),
        escape_xml(description)
    );

    for item in items {
        atom.push_str(&format!(
            r#"<entry>
<title>{}</title>
<link href="{}"/>
<summary>{}</summary>
<id>{}</id>
<updated>{}</updated>
</entry>
"#,
            escape_xml(&item.title),
            escape_xml(&item.link),
            escape_xml(&item.description),
            escape_xml(&item.guid),
            escape_xml(&item.published_at)
        ));
    }

    atom.push_str("</feed>");
    atom
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_handle_generation() {
        let h = feed_handle("work", "my-work-123");
        assert_eq!(h, "work-my-work-123");
    }

    #[test]
    fn feed_handle_sanitizes() {
        let h = feed_handle("work", "my work!@#");
        assert_eq!(h, "work-my-work---");
    }

    #[test]
    fn escape_xml_escapes_special_chars() {
        assert_eq!(escape_xml("a < b & c > d"), "a &lt; b &amp; c &gt; d");
        assert_eq!(escape_xml(r#""quoted""#), "&quot;quoted&quot;");
    }

    #[test]
    fn build_rss_produces_valid_structure() {
        let items = vec![
            FeedItem {
                title: "Test".to_string(),
                link: "https://example.com/1".to_string(),
                description: "A test".to_string(),
                published_at: "2026-09-14T12:00:00Z".to_string(),
                guid: "guid-1".to_string(),
            },
        ];
        let rss = build_rss("My Feed", "https://example.com", "Description", &items);
        assert!(rss.contains(r#"<rss version="2.0">"#));
        assert!(rss.contains("<title>My Feed</title>"));
        assert!(rss.contains("<item>"));
        assert!(rss.contains("<title>Test</title>"));
    }

    #[test]
    fn build_atom_produces_valid_structure() {
        let items = vec![
            FeedItem {
                title: "Test".to_string(),
                link: "https://example.com/1".to_string(),
                description: "A test".to_string(),
                published_at: "2026-09-14T12:00:00Z".to_string(),
                guid: "guid-1".to_string(),
            },
        ];
        let atom = build_atom("My Feed", "https://example.com", "Description", &items);
        assert!(atom.contains("<feed xmlns=\"http://www.w3.org/2005/Atom\">"));
        assert!(atom.contains("<title>My Feed</title>"));
        assert!(atom.contains("<entry>"));
    }
}
