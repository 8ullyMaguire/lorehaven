import { describe, expect, it } from 'vitest';
import { isPlainLeftClick, matchRoute, PLANNED_ROUTES } from './router';

describe('routing', () => {
  it('resolves the media catalogue', () => {
    expect(matchRoute('/media').id).toBe('media');
  });
  it('maps the root to the home view', () => {
    expect(matchRoute('/').id).toBe('home');
    expect(matchRoute('').id).toBe('home');
  });

  it('resolves the pages Milestones 11, 12 and 16 built', () => {
    // These were linked-but-unbuilt placeholders once; a stale placeholder
    // would hide a working surface, exactly as `/library` did before M6.
    expect(matchRoute('/discover').id).toBe('discover');
    expect(matchRoute('/community').id).toBe('community');
    expect(matchRoute('/notifications').id).toBe('notifications');
  });


  it('resolves the forum category and topic pages the community hub links to', () => {
    // The hub linked to `/community/forums/<id>` before any view existed, so
    // every category click dead-ended at the 404 page. These matches are what
    // makes the link honest.
    const category = matchRoute('/community/forums/11111111-1111-1111-1111-111111111111');
    expect(category.id).toBe('forum-category');
    expect(category.params).toEqual({
      categoryId: '11111111-1111-1111-1111-111111111111',
    });

    const topic = matchRoute('/community/topics/22222222-2222-2222-2222-222222222222');
    expect(topic.id).toBe('forum-topic');
    expect(topic.params).toEqual({
      topicId: '22222222-2222-2222-2222-222222222222',
    });

    // Near-misses stay unmatched.
    expect(matchRoute('/community/forums').id).toBe('not-found');
    expect(matchRoute('/community/forums/extra/segments').id).toBe('not-found');
  });

  it('maps a linked-but-unbuilt destination to an honest placeholder', () => {
    // The mechanism stays for the next destination a milestone has not built
    // yet; nothing links to an unbuilt page right now, so drive the branch
    // directly through the (currently empty) planned table.
    expect(Object.keys(PLANNED_ROUTES)).toHaveLength(0);
  });

  it('resolves the library and import pages Milestone 6 built', () => {
    // These were a placeholder until the import machinery existed. They are
    // real pages now, and a stale placeholder would hide a working surface.
    expect(matchRoute('/library').id).toBe('library');
    expect(matchRoute('/exports').id).toBe('exports');
    expect(matchRoute('/import').id).toBe('import');
    expect(matchRoute('/import/').id).toBe('import');
  });

  it('resolves the reader history page', () => {
    expect(matchRoute('/library/history').id).toBe('history');
  });

  it('does not pretend an unknown path exists', () => {
    const match = matchRoute('/nowhere/6f2a9c');
    expect(match.id).toBe('not-found');
    expect(match.path).toBe('/nowhere/6f2a9c');
  });

  it('resolves the writing and reading paths of Milestone 3', () => {
    expect(matchRoute('/write').id).toBe('write');

    const editor = matchRoute('/write/6f2a9c-1');
    expect(editor.id).toBe('work-editor');
    expect(editor.params?.workId).toBe('6f2a9c-1');

    const work = matchRoute('/works/6f2a9c-1');
    expect(work.id).toBe('work-read');
    expect(work.params?.workId).toBe('6f2a9c-1');

    const chapter = matchRoute('/works/6f2a9c-1/chapters/aaa-2');
    expect(chapter.id).toBe('chapter-read');
    expect(chapter.params?.workId).toBe('6f2a9c-1');
    expect(chapter.params?.chapterId).toBe('aaa-2');
  });

  it('decodes percent-encoded identifiers, because a URL is not a request', () => {
    const match = matchRoute('/works/a%20b/chapters/c%2Fd');
    expect(match.params?.workId).toBe('a b');
    expect(match.params?.chapterId).toBe('c/d');
  });

  it('ignores trailing slashes so links and URLs agree', () => {
    expect(matchRoute('/search/').id).toBe('search');
    expect(matchRoute('//').id).toBe('home');
  });

  it('only intercepts plain left clicks, leaving modified clicks to the browser', () => {
    const plain = new MouseEvent('click', { button: 0 });
    expect(isPlainLeftClick(plain)).toBe(true);

    expect(isPlainLeftClick(new MouseEvent('click', { button: 1 }))).toBe(false);
    expect(isPlainLeftClick(new MouseEvent('click', { button: 0, metaKey: true }))).toBe(false);
    expect(isPlainLeftClick(new MouseEvent('click', { button: 0, ctrlKey: true }))).toBe(false);
    expect(isPlainLeftClick(new MouseEvent('click', { button: 0, shiftKey: true }))).toBe(false);

    const prevented = new MouseEvent('click', { button: 0, cancelable: true });
    prevented.preventDefault();
    expect(isPlainLeftClick(prevented)).toBe(false);
  });
});

describe('Milestone 2 routes', () => {
  it('resolves the identity pages the shell now links to', () => {
    expect(matchRoute('/register').id).toBe('register');
    expect(matchRoute('/sign-in').id).toBe('sign-in');
    expect(matchRoute('/password-reset').id).toBe('password-reset');
    expect(matchRoute('/account').id).toBe('account');
  });

  it('resolves the owner’s pseud page, which is no longer a placeholder', () => {
    const match = matchRoute('/pseud');
    expect(match.id).toBe('pseuds');
    expect(match.planned).toBeUndefined();
  });

  it('reads a public profile handle from the path', () => {
    const match = matchRoute('/pseud/Quill');
    expect(match.id).toBe('pseud-profile');
    expect(match.params?.handle).toBe('Quill');
  });

  it('decodes an escaped handle, because handles are compared as text', () => {
    expect(matchRoute('/pseud/Ada%20Lovelace').params?.handle).toBe('Ada Lovelace');
  });

  it('treats a trailing slash as the same page', () => {
    expect(matchRoute('/pseud/').id).toBe('pseuds');
    expect(matchRoute('/account/').id).toBe('account');
  });

  it('does not treat an empty handle as a profile', () => {
    // `/pseud/` is the owner's page; `/pseud//` must not become a profile
    // with an empty handle that would be fetched as a real one.
    expect(matchRoute('/pseud//').id).toBe('pseuds');
  });
});

describe('docs routes', () => {
  it('resolves the docs index', () => {
    const match = matchRoute('/docs');
    expect(match.id).toBe('docs');
  });

  it('resolves one doc page with its slug as a param', () => {
    const match = matchRoute('/docs/keyboard-shortcuts');
    expect(match.id).toBe('doc-page');
    expect(match.params?.slug).toBe('keyboard-shortcuts');
  });

  it('decodes an escaped doc slug', () => {
    expect(matchRoute('/docs/the-forum-explained').params?.slug).toBe('the-forum-explained');
  });

  it('does not treat an empty slug as a doc page', () => {
    expect(matchRoute('/docs/').id).toBe('docs');
    expect(matchRoute('/docs//').id).toBe('docs');
  });
  it('resolves the analytics page rather than the not-found view', () => {
    // The page existed with twenty unit tests and no route to it, so every one
    // of those tests passed while no reader could reach it. A component that
    // is not in FIXED_ROUTES is invisible, and nothing else in the suite
    // notices.
    expect(matchRoute('/analytics').id).toBe('analytics');
  });
});
