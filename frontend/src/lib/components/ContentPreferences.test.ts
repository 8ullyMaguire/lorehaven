import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import ContentPreferences from './ContentPreferences.svelte';

/**
 * N10's guard, tested at the level it was written.
 *
 * `ac22a89` protected the reader's typography panel by refusing a server copy
 * that is not newer than the local one. The two account panels took a different
 * guard — an `edited` flag consulted by the effect that re-seeds the form — and
 * the flag is cleared at the end of that same effect, which reads it as a
 * dependency. So it survives exactly one flush: the run that skips the re-seed
 * clears the flag and queues another run, and that one seeds.
 *
 * The reader's edit is the thing that must survive. If this test is red, the
 * guard is a delay rather than a shield, and the honest fix is to stop clearing
 * the flag inside the effect (a save clears it).
 */
const settings = (version: number, max = 'general') => ({
  version,
  max_rating: max,
  excluded_warnings: [],
  policy_ceiling: 'explicit',
  effective_max_rating: max,
});

describe('ContentPreferences', () => {
  it('keeps an edit that is in flight when the server copy arrives', async () => {
    const onsave = vi.fn().mockResolvedValue(undefined);
    const onreload = vi.fn().mockResolvedValue(undefined);
    const { getByLabelText, rerender } = render(ContentPreferences, {
      props: { settings: settings(1), onsave, onreload },
    });

    const select = getByLabelText('Show me works rated up to') as HTMLSelectElement;
    await fireEvent.change(select, { target: { value: 'mature' } });
    expect(select.value).toBe('mature');

    // The response that was already in flight when the reader moved the control.
    await rerender({ settings: settings(2), onsave, onreload });

    await waitFor(() => expect(select.value).toBe('mature'));
  });
});
