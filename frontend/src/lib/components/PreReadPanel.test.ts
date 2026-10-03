import { render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import PreReadPanel from './PreReadPanel.svelte';

vi.mock('../api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api')>();
  return {
    ...actual,
    fetchPreread: vi.fn(),
    forgetPrereadProvider: vi.fn(),
  };
});

import { ApiError, fetchPreread, forgetPrereadProvider } from '../api';

const mocked = vi.mocked(fetchPreread);
const mockedForget = vi.mocked(forgetPrereadProvider);

/**
 * What these tests are actually about.
 *
 * This is a **negative feature**: §32.6 says a pre-read report is shown to the author and
 * never on the public work page, and §0.3 says no ranking signal may move on payment. So the
 * assertions that matter are mostly about what is *absent*:
 *
 *   - no composite score anywhere in the rendered markup. §32.6 forbids displaying
 *     composite quality scores publicly, and a component that helpfully computed an
 *     average of the per-dimension values would make a ranking signal out of an assessment.
 *     Nothing here would catch that except a test that looks for a total and requires it not
 *     to be there.
 *   - a 404 renders **nothing at all**, not an error message. The server answers "not
 *     yours" and "does not exist" identically precisely so an outsider cannot confirm a
 *     draft exists; a panel that rendered "not found" on somebody else's work would undo
 *     that by confirming it.
 *   - a partial report says so *above* its numbers. A partial report whose limitation is in
 *     a footer reads as a full one at a glance.
 *   - withdrawal is per provider, with no single "delete everything" control.
 */
describe('PreReadPanel', () => {
  beforeEach(() => {
    mocked.mockReset();
    mockedForget.mockReset();
  });

  function assessed(overrides: Record<string, unknown> = {}) {
    return {
      status: 'assessed' as const,
      providers: ['ollama'],
      report: {
        workId: 'work-1',
        dimensions: [
          { dimension: 'tone', score: 0.82, note: 'even' },
          { dimension: 'pacing', score: 0.5, note: 'steady' },
          { dimension: 'length', score: 0.35, note: 'long' },
        ],
        missing: [],
        complete: true,
        ...overrides,
      },
    };
  }

  it('shows the dimensions and never a composite', async () => {
    mocked.mockResolvedValue(assessed() as never);

    render(PreReadPanel, { workId: 'work-1' });

    await screen.findByText('tone');
    // Worst-first is the server's ordering, used as given: the author's question is "what is
    // weakest", so re-sorting ascending here would answer the one they did not ask.
    const rows = screen.getAllByRole('listitem');
    expect(rows[0].textContent).toContain('tone');
    expect(rows[2].textContent).toContain('length');

    // §32.6 and §0.3. No total, no average, no "overall". An average of these three is
    // 0.5567, and any element rendering it would be a composite quality score.
    const html = document.body.innerHTML;
    expect(html).not.toMatch(/overall|average|total|composite/i);
    expect(html).not.toMatch(/0\.56|0\.55/);
  });

  it('renders nothing at all when the work is not the caller\'s', async () => {
    // The server's 404 is indistinguishable from "no such work", so the panel's job is to
    // stay silent. An error summary here would confirm the draft exists to whoever opened
    // somebody else's editor.
    mocked.mockRejectedValue(Object.assign(new Error('not found'), { status: 404 }));

    render(PreReadPanel, { workId: 'work-2' });

    await waitFor(() => expect(mocked).toHaveBeenCalledWith('work-2'));
    expect(document.body.textContent).toBe('');
    expect(document.body.innerHTML).not.toMatch(/not found|error/i);
  });

  it('reports a real failure rather than hiding it as not-mine', async () => {
    // The complement of the 404 rule: a 500 is not "not yours" and must not be swallowed
    // into silence, or a broken endpoint would look like a caller who owns nothing.
    // Argument order matters and the test could not tell: `ApiError` is
    // (status, code, message, requestId, fieldErrors), so this was constructing
    // status='boom' and message=500, and the assertion on /server exploded/i
    // passed only because the string was sitting in `requestId`. svelte-check
    // caught the type error; the assertion never could.
    mocked.mockRejectedValue(new ApiError(500, 'SERVER_ERROR', 'server exploded'));

    render(PreReadPanel, { workId: 'work-1' });

    await screen.findByText(/server exploded/i);
  });

  it('says so above the numbers when the report is partial', async () => {
    mocked.mockResolvedValue(
      assessed({
        complete: false,
        missing: [
          { dimension: 'tags', reason: 'the provider was not configured' },
          {
            dimension: 'tone',
            reason: 'the provider returned output that could not be validated',
          },
        ],
      }) as never,
    );

    render(PreReadPanel, { workId: 'work-1' });

    // The banner precedes the numbers in the DOM, so the limitation is read before the
    // scores rather than after them.
    const banner = await screen.findByText(/partial/i);
    const firstScore = screen.getByText('0.82');
    expect(
      banner.compareDocumentPosition(firstScore) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();

    // The reasons are shown, not just the absence — that difference is what tells an author
    // whether to trust the scores.
    await screen.findByText(/the provider was not configured/i);
    await screen.findByText(/could not be validated/i);
  });

  it('distinguishes never assessed from a withdrawn provider', async () => {
    mocked.mockResolvedValue({
      status: 'not_assessed',
      providers: [],
      reason: 'no_provider_has_assessed_this_work',
    } as never);

    render(PreReadPanel, { workId: 'work-1' });

    // An empty score list would read as a work that scored nothing on everything, which is a
    // different and much worse claim than "nobody has looked at this yet".
    await screen.findByText(/no provider has assessed this work yet/i);
    expect(screen.queryByRole('list')).toBeNull();
  });

  it('offers a withdrawal per provider and no delete-everything control', async () => {
    // §23.7: opting out is of *specific* providers, so a single "clear all" button would be
    // a withdrawal from providers the author never asked to withdraw from.
    mocked.mockResolvedValue({
      status: 'assessed',
      providers: ['ollama', 'openai-compatible'],
      report: {
        workId: 'work-1',
        dimensions: [{ dimension: 'tone', score: 0.7, note: '' }],
        missing: [],
        complete: true,
      },
    } as never);
    mockedForget.mockResolvedValue({ removed: 1, providers: ['openai-compatible'] });

    render(PreReadPanel, { workId: 'work-1' });

    await screen.findByText('ollama');
    expect(screen.getByText('openai-compatible')).toBeTruthy();
    const buttons = screen.getAllByRole('button', { name: /withdraw/i });
    expect(buttons).toHaveLength(2);

    // Every control is the same per-provider action, and nothing says "all".
    expect(screen.queryByRole('button', { name: /all|everything|clear/i })).toBeNull();
    expect(document.body.innerHTML).not.toMatch(/withdraw all|delete all|clear all/i);
  });
});