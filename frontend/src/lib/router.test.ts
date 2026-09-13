import { describe, expect, it } from 'vitest';
import { isPlainLeftClick, matchRoute } from './router';

describe('routing', () => {
  it('maps the root to the home view', () => {
    expect(matchRoute('/').id).toBe('home');
    expect(matchRoute('').id).toBe('home');
  });

  it('maps linked-but-unbuilt destinations to an honest placeholder', () => {
    const match = matchRoute('/discover');
    expect(match.id).toBe('planned');
    expect(match.planned?.title).toBe('Discover');
    expect(match.planned?.milestone).toMatch(/^Milestone \d+$/);
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
