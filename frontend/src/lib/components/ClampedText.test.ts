import { describe, expect, it } from 'vitest';
import { fireEvent, render } from '@testing-library/svelte';
import ClampedText from './ClampedText.svelte';

/**
 * jsdom does not lay anything out: every element reports `scrollHeight` 0 and
 * `clientHeight` 0, so `scrollHeight > clientHeight` is never true. The overflow
 * measurement is therefore stubbed, which is the honest way to test this — the
 * question "does the control appear only when there is something behind it" is
 * about what the component does with a measurement, and that is testable.
 */
function stubOverflow(overflowing: boolean) {
  const proto: Record<string, PropertyDescriptor> = {};
  proto.scrollHeight = { configurable: true, get: () => (overflowing ? 400 : 40) };
  proto.clientHeight = { configurable: true, get: () => 40 };
  for (const [key, descriptor] of Object.entries(proto)) {
    Object.defineProperty(HTMLElement.prototype, key, descriptor);
  }
  return () => {
    for (const key of Object.keys(proto)) {
      delete (HTMLElement.prototype as unknown as Record<string, unknown>)[key];
    }
  };
}

describe('ClampedText', () => {
  it('shows the text', () => {
    const { getByText } = render(ClampedText, { props: { text: 'A summary.' } });
    expect(getByText('A summary.')).toBeTruthy();
  });

  it('offers no control when the text is not being clipped', () => {
    const restore = stubOverflow(false);
    try {
      const { queryByRole } = render(ClampedText, { props: { text: 'Short.' } });
      expect(queryByRole('button')).toBeNull();
    } finally {
      restore();
    }
  });

  it('offers a control when the text is being clipped, and the filter is not needed to find it', () => {
    const restore = stubOverflow(true);
    try {
      const { getByRole } = render(ClampedText, { props: { text: 'Long. '.repeat(200) } });
      expect(getByRole('button').textContent).toContain('Show more');
    } finally {
      restore();
    }
  });

  it('expands and collapses, saying which state it is in', async () => {
    const restore = stubOverflow(true);
    try {
      const { getByRole } = render(ClampedText, { props: { text: 'Long. '.repeat(200) } });
      const button = getByRole('button');
      expect(button.getAttribute('aria-expanded')).toBe('false');

      await fireEvent.click(button);
      const collapsed = getByRole('button');
      expect(collapsed.getAttribute('aria-expanded')).toBe('true');
      expect(collapsed.textContent).toContain('Show less');

      await fireEvent.click(collapsed);
      expect(getByRole('button').getAttribute('aria-expanded')).toBe('false');
    } finally {
      restore();
    }
  });

  it('keeps the reader’s expansion through a text change', async () => {
    const restore = stubOverflow(true);
    try {
      const { getByRole, rerender } = render(ClampedText, {
        props: { text: 'Long. '.repeat(200) },
      });
      await fireEvent.click(getByRole('button'));

      // A refresh replaces the item and therefore the text. The reader opened
      // this on purpose; a re-render is not a reason to close it again.
      await rerender({ text: 'Different. '.repeat(200) });
      expect(getByRole('button').getAttribute('aria-expanded')).toBe('true');
    } finally {
      restore();
    }
  });
});
