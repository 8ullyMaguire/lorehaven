import { describe, expect, it } from 'vitest';
import { AGE_BANDS, describeAgeState, describeRating, describeValue } from './labels';

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
