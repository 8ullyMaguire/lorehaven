/**
 * The user documentation set.
 *
 * Docs are plain markdown files in `src/docs/`, imported here statically so
 * they ship inside the single frontend bundle (spec §5: one artifact, no
 * Node server) and work offline. The registry is the single source of truth
 * for: which docs exist, their titles and sections, and the search index.
 *
 * Writing for the lowest common denominator is the standing rule for these
 * files (see getting-started.md): plain words, numbered steps, no jargon
 * left unexplained.
 */

import gettingStarted from '../docs/getting-started.md?raw';
import findingStories from '../docs/finding-stories.md?raw';
import postingFirstStory from '../docs/posting-your-first-story.md?raw';
import forumExplained from '../docs/the-forum-explained.md?raw';
import keyboardShortcuts from '../docs/keyboard-shortcuts.md?raw';
import accountPrivacy from '../docs/account-and-privacy.md?raw';
import importingStories from '../docs/importing-stories.md?raw';

export interface DocEntry {
  /** URL slug, also the filename without `.md`. */
  slug: string;
  /** Page title, from the first `# ` heading. */
  title: string;
  /** One-line plain-language summary shown under the title. */
  summary: string;
  /** Raw markdown body (the `# ` title line removed). */
  markdown: string;
  /** Section titles (`## ` headings) for search and the on-page contents. */
  sections: string[];
}

export type DocOrder = readonly string[];

/** Reading order for the docs index page. */
export const DOC_ORDER: DocOrder = [
  'getting-started',
  'finding-stories',
  'posting-your-first-story',
  'the-forum-explained',
  'importing-stories',
  'account-and-privacy',
  'keyboard-shortcuts',
] as const;

function titleOf(markdown: string): string {
  const m = markdown.match(/^#\s+(.+)$/m);
  return m ? m[1].trim() : 'Untitled';
}

function bodyOf(markdown: string): string {
  // Everything after the first `# ` line.
  const idx = markdown.search(/^#\s+/m);
  if (idx === -1) return markdown;
  const after = markdown.indexOf('\n', idx);
  return after === -1 ? '' : markdown.slice(after + 1).trim();
}

function sectionsOf(markdown: string): string[] {
  const out: string[] = [];
  for (const m of markdown.matchAll(/^##\s+(.+)$/gm)) {
    out.push(m[1].trim());
  }
  return out;
}

function summaryOf(markdown: string): string {
  // First non-heading, non-empty paragraph, stripped of markdown syntax.
  const firstPara = markdown
    .split(/\n\s*\n/)
    .map((p) => p.trim())
    .find((p) => p.length > 0 && !p.startsWith('#'));
  if (!firstPara) return '';
  return firstPara
    .replace(/\*\*|\*|`|\[|\]\([^)]*\)/g, '')
    .replace(/\s+/g, ' ')
    .slice(0, 140);
}

const SOURCES: Record<string, string> = {
  'getting-started': gettingStarted,
  'finding-stories': findingStories,
  'posting-your-first-story': postingFirstStory,
  'the-forum-explained': forumExplained,
  'keyboard-shortcuts': keyboardShortcuts,
  'account-and-privacy': accountPrivacy,
  'importing-stories': importingStories,
};

/** All docs, in reading order. */
export const DOCS: readonly DocEntry[] = DOC_ORDER.map((slug) => {
  const markdown = SOURCES[slug];
  return {
    slug,
    title: titleOf(markdown),
    summary: summaryOf(markdown),
    markdown: bodyOf(markdown),
    sections: sectionsOf(markdown),
  };
});

export function docBySlug(slug: string): DocEntry | undefined {
  return DOCS.find((d) => d.slug === slug);
}

/**
 * Plain-text index of a doc for search: title, summary and body with
 * markdown syntax stripped, lowercased once at load.
 */
function searchText(doc: DocEntry): string {
  return `${doc.title} ${doc.summary} ${doc.sections.join(' ')} ${doc.markdown}`
    .replace(/\*\*|\*|`|\[|\]\([^)]*\)/g, ' ')
    .replace(/\s+/g, ' ')
    .toLowerCase();
}

const SEARCH_TEXT: readonly string[] = DOCS.map(searchText);

/**
 * Search the docs. Scoring: a title hit is worth 10, a section hit 4, a
 * body hit 1 — so "forum" ranks "The forum, explained" above a doc that
 * merely mentions the word. Results are ordered by score, then by reading
 * order.
 */
export function searchDocs(query: string, limit = 8): DocEntry[] {
  const q = query.trim().toLowerCase();
  if (q.length === 0) return [];
  const terms = q.split(/\s+/);
  const scored = DOCS.map((doc, i) => {
    const text = SEARCH_TEXT[i];
    let score = 0;
    for (const term of terms) {
      const title = doc.title.toLowerCase();
      const sections = doc.sections.map((s) => s.toLowerCase());
      if (title.includes(term)) score += 10;
      for (const s of sections) if (s.includes(term)) score += 4;
      if (text.includes(term)) score += 1;
    }
    return { doc, score };
  });
  return scored
    .filter((s) => s.score > 0)
    .sort((a, b) => b.score - a.score)
    .slice(0, limit)
    .map((s) => s.doc);
}
