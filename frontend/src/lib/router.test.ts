import { describe, expect, it } from 'vitest';
import { isPlainLeftClick, matchRoute } from './router';

describe('routing', () => {
  it('maps the root to the home view', () => {
    expect(matchRoute('/').id).toBe('home');
    expect(matchRoute('').id).toBe('home');
  });

  it('maps linked-but-unbuilt destinations to an honest placeholder', () => {
    const match = matchRoute('/library');
    expect(match.id).toBe('planned');
    expect(match.planned?.title).toBe('Library');
    expect(match.planned?.milestone).toMatch(/^Milestone \d+$/);
  });

  it('does not pretend an unknown path exists', () => {
    const match = matchRoute('/works/6f2a9c');
    expect(match.id).toBe('not-found');
    expect(match.path).toBe('/works/6f2a9c');
  });

  it('ignores trailing slashes so links and URLs agree', () => {
    expect(matchRoute('/search/').id).toBe('planned');
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
