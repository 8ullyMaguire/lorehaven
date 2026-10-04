import { describe, expect, it, afterEach } from 'vitest';
import { fetchSlotExplanation } from '../api';

/**
 * The client's own 404 mapping, in its own test, because the component above DEPENDS on it.
 *
 * This is the seam between "no explanation" and "try again". `WhyRecommended.test.ts` mocks
 * `fetchSlotExplanation` outright, so nothing in the suite runs its body. If it stopped
 * swallowing 404s and threw everything, the component's catch-all would read a missing slot
 * as a network failure, the component's 404 test would stay GREEN (it mocks the return
 * value, not the throw), and the feature would quietly rot.
 *
 * Its own file, and the reason is mechanical: `WhyRecommended.test.ts` hoists a
 * `vi.mock('../api')`, and that applies to dynamic imports too, so a second suite in the
 * same file importing the real function gets the mock. Splitting the files is not tidiness,
 * it is the only arrangement where this function's own body can be exercised at all.
 */
describe('fetchSlotExplanation error mapping', () => {
  const realFetch = globalThis.fetch;
  afterEach(() => {
    globalThis.fetch = realFetch;
  });

  it('returns null for a 404, and re-throws for a 500', async () => {
    globalThis.fetch = (async () =>
      new Response(JSON.stringify({ error: { code: 'NOT_FOUND', message: 'no slot' } }), {
        status: 404,
        headers: { 'content-type': 'application/json' },
      })) as typeof fetch;
    await expect(fetchSlotExplanation('slot-1')).resolves.toBeNull();

    globalThis.fetch = (async () =>
      new Response(JSON.stringify({ error: { code: 'INTERNAL', message: 'boom' } }), {
        status: 500,
        headers: { 'content-type': 'application/json' },
      })) as typeof fetch;
    await expect(fetchSlotExplanation('slot-1')).rejects.toThrow();
  });
});

