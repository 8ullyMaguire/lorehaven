import { describe, expect, it } from 'vitest';
import { AGE_BANDS, describeAgeState, describeRating, describeValue, formatBytes } from './labels';

describe('labels', () => {
  it('names the values the server accepts in plain words', () => {
    expect(describeValue('private')).toBe('Only me');
    expect(describeValue('contacts_only')).toBe('People I follow');
    expect(describeValue('false')).toBe('Not allowed');
  });

  it('shows an unrecognised value as it arrived rather than hiding it', () => {
    // If the server gains a value first, the interface says so out loud
    // instead of rendering an empty option.
    expect(describeValue('somewhere_new')).toBe('somewhere_new');
    expect(describeRating('graphic')).toBe('graphic');
    expect(describeAgeState('new_state')).toBe('new_state');
  });

  it('describes a self-declaration as unverified', () => {
    // A declared adult is not a verified adult, and the wording must not imply
    // otherwise (spec §7).
    expect(describeAgeState('declared_adult')).toContain('unverified');
  });

  it('offers declining to state an age as a real answer', () => {
    expect(AGE_BANDS.map((band) => band.value)).toEqual(['adult', 'minor', 'unknown']);
  });
});

describe('byte counts', () => {
  it('reads as a size rather than as a number', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(4096)).toBe('4.0 KB');
    expect(formatBytes(3 * 1024 * 1024)).toBe('3.0 MB');
  });

  it('does not claim more precision than a reader can use', () => {
    // A file that is 3.7 MB is not usefully "3.7276611328125 MB".
    expect(formatBytes(3_900_000)).toBe('3.7 MB');
    expect(formatBytes(12.5 * 1024 * 1024)).toBe('13 MB');
  });

  it('says so rather than printing nonsense for a value it cannot read', () => {
    expect(formatBytes(-1)).toBe('unknown size');
    expect(formatBytes(Number.NaN)).toBe('unknown size');
  });
});
