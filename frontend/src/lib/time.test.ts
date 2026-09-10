import { describe, expect, it } from 'vitest';
import { formatTimestamp } from './time';

describe('timestamps', () => {
  it('formats a server timestamp for a human', () => {
    const formatted = formatTimestamp('2026-09-10T12:34:00Z', 'en-GB');
    expect(formatted).toMatch(/2026/);
    expect(formatted).not.toBe('Invalid Date');
  });

  it('shows an unparseable value as it arrived', () => {
    // A raw timestamp is more informative than "Invalid Date", and hides
    // nothing about what the server actually said.
    expect(formatTimestamp('', 'en-GB')).toBe('');
    expect(formatTimestamp('not a date', 'en-GB')).toBe('not a date');
  });
});
