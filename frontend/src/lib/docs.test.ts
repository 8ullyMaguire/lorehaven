import { describe, expect, it } from 'vitest';

import { DOCS, docBySlug, searchDocs } from './docs';

describe('the docs registry', () => {
  it('ships every doc in reading order with a title', () => {
    expect(DOCS.length).toBeGreaterThanOrEqual(7);
    expect(DOCS[0]?.slug).toBe('getting-started');
    for (const doc of DOCS) {
      expect(doc.title.length).toBeGreaterThan(0);
      expect(doc.markdown).not.toMatch(/^#\s/m); // title stripped from body
      expect(doc.summary.length).toBeGreaterThan(0);
    }
  });

  it('looks a doc up by slug and rejects unknown slugs', () => {
    expect(docBySlug('the-forum-explained')?.title).toMatch(/forum/i);
    expect(docBySlug('no-such-doc')).toBeUndefined();
  });
});

describe('searchDocs', () => {
  it('ranks a title hit above a body hit', () => {
    const results = searchDocs('forum');
    expect(results.length).toBeGreaterThan(0);
    expect(results[0]?.slug).toBe('the-forum-explained');
  });

  it('finds pages that only mention the term in the body', () => {
    // "import" is the subject of one page and a word in getting-started.
    const results = searchDocs('import');
    expect(results[0]?.slug).toBe('importing-stories');
  });

  it('returns nothing for an empty or whitespace query', () => {
    expect(searchDocs('')).toEqual([]);
    expect(searchDocs('   ')).toEqual([]);
  });

  it('matches multi-word queries only when every word appears', () => {
    // Both words appear in posting-your-first-story.
    expect(searchDocs('first story')[0]?.slug).toBe('posting-your-first-story');
    // No page contains this pair.
    expect(searchDocs('zorp quux')).toEqual([]);
  });

  it('is case-insensitive', () => {
    expect(searchDocs('FORUM')[0]?.slug).toBe('the-forum-explained');
  });
});
