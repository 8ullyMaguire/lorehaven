import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import PrivacySettings from './PrivacySettings.svelte';

/**
 * The same guard as `ContentPreferences.test.ts`, on the other panel
 * `fee36d6` changed. Two directions, because a guard that refuses every
 * server copy would keep the edit and freeze the panel:
 *
 *  * an edit in flight survives the copy that lands beside it;
 *  * once that edit is saved, a later copy is adopted as before.
 */
const KEYS = [
  { key: 'messaging_policy', summary: 'Who can message you', values: ['contacts_only', 'nobody'] },
  { key: 'inbox_policy', summary: 'Who can start a conversation', values: ['everyone', 'nobody'] },
];

const values = (messaging: string, inbox: string) => ({
  messaging_policy: messaging,
  inbox_policy: inbox,
});

describe('PrivacySettings', () => {
  it('keeps an edit that is in flight when the server copy arrives', async () => {
    const onsave = vi.fn().mockResolvedValue(undefined);
    const { getByLabelText, rerender } = render(PrivacySettings, {
      props: { keys: KEYS, values: values('contacts_only', 'everyone'), legend: 'Account', onsave },
    });

    const select = getByLabelText('Who can message you') as HTMLSelectElement;
    await fireEvent.change(select, { target: { value: 'nobody' } });
    expect(select.value).toBe('nobody');

    // The full copy lands: it differs elsewhere, which is what changes the
    // signature the effect watches. The reader's own choice must stand.
    await rerender({
      keys: KEYS,
      values: values('contacts_only', 'nobody'),
      legend: 'Account',
      onsave,
    });

    await waitFor(() => expect(select.value).toBe('nobody'));
  });

  it('adopts a copy that arrives after the edit was saved', async () => {
    const onsave = vi.fn().mockResolvedValue(undefined);
    const { getByLabelText, getByRole, rerender } = render(PrivacySettings, {
      props: { keys: KEYS, values: values('contacts_only', 'everyone'), legend: 'Account', onsave },
    });

    const select = getByLabelText('Who can message you') as HTMLSelectElement;
    await fireEvent.change(select, { target: { value: 'nobody' } });
    await fireEvent.click(getByRole('button', { name: 'Save changes' }));
    await waitFor(() => expect(onsave).toHaveBeenCalled());

    // Another device changes the same setting to a value this panel has never
    // seen; with the edit saved, the panel must follow it. Re-queried: a
    // re-render replaces the element, and a detached node reports nothing.
    await rerender({
      keys: KEYS,
      values: values('contacts_only', 'nobody'),
      legend: 'Account',
      onsave,
    });

    await waitFor(() => {
      const fresh = document.querySelector<HTMLSelectElement>('#privacy-messaging_policy');
      expect(fresh?.value).toBe('contacts_only');
    });
  });
});
