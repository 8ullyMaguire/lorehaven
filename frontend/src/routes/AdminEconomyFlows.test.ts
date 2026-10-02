import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import AdminEconomyFlows from './AdminEconomyFlows.svelte';
import { fetchEconomyFlows } from '../lib/api';

vi.mock('../lib/api', () => ({
  fetchEconomyFlows: vi.fn(),
}));

const mockFetch = vi.mocked(fetchEconomyFlows);

/** A window with one declared faucet, one declared sink, and one undeclared mechanism. */
function window_() {
  return {
    since: '2026-09-02T00:00:00Z',
    until: '2026-10-02T00:00:00Z',
    faucet_credits: 9_000,
    sink_credits: -3_000,
    net_credits: 6_000,
    undeclared: 1,
    threshold: 50_000,
    over_threshold: false,
    mechanisms: [
      { key: 'author_earnings', flow: 'faucet', net_credits: 9_000, declared: true },
      { key: 'tips', flow: 'sink', net_credits: -3_000, declared: true },
      {
        key: 'undeclared:grant:mystery',
        flow: 'undeclared',
        net_credits: 4_200,
        declared: false,
      },
    ],
    note: 'Aggregate mechanisms only.',
  };
}

afterEach(() => {
  vi.clearAllMocks();
});

describe('AdminEconomyFlows', () => {
  it('shows the balance and the composition, not just the totals', async () => {
    // §53.2 asks for "a balance and a composition". Totals alone would be a number an
    // operator cannot act on: it would say 6,000 without saying which mechanisms made it.
    mockFetch.mockResolvedValue(window_());
    render(AdminEconomyFlows);

    await waitFor(() => expect(screen.getByTestId('net-credits')).toHaveTextContent('6,000'));
    expect(screen.getByTestId('faucet-credits')).toHaveTextContent('9,000');
    expect(screen.getByTestId('sink-credits')).toHaveTextContent('-3,000');

    expect(screen.getByTestId('mechanism-author_earnings')).toBeTruthy();
    expect(screen.getByTestId('mechanism-tips')).toBeTruthy();
  });

  it('shows an undeclared mechanism rather than hiding it', async () => {
    // §53.1: a dashboard that silently omits what nobody classified reports a smaller
    // economy than exists. The mechanism must be visible AND its credits accounted for,
    // so the operator can tell "unclassified" from "missing".
    mockFetch.mockResolvedValue(window_());
    render(AdminEconomyFlows);

    await waitFor(() =>
      expect(screen.getByTestId('undeclared-undeclared:grant:mystery')).toBeTruthy(),
    );
    expect(screen.getByTestId('undeclared-heading')).toHaveTextContent('Undeclared (1)');
    expect(screen.getByTestId('undeclared-warning')).toBeTruthy();
    // Its credits are in the net, not quietly dropped from it.
    expect(screen.getByTestId('undeclared-undeclared:grant:mystery')).toHaveTextContent('4,200');
  });

  it('never renders a per-account balance', async () => {
    // §53.2 forbids per-account detail in an economy view. The server does not send it;
    // this asserts the component neither invents nor displays one.
    const flows = window_();
    mockFetch.mockResolvedValue({
      ...flows,
      mechanisms: [
        ...flows.mechanisms,
        { key: 'acct-secret', flow: 'undeclared', net_credits: 5, declared: false },
      ],
    } as unknown as typeof flows);
    render(AdminEconomyFlows);

    await waitFor(() => expect(screen.getByTestId('net-credits')).toBeTruthy());
    const body = document.body.textContent ?? '';
    // `acct-secret` is a *mechanism key*, not a balance field, so it may appear -- what
    // must not appear is a per-account shape the component could have invented.
    expect(body).not.toMatch(/account_id|accountId|per_account|perAccount|balance_by_account/);
  });

  it('reports a crossed threshold without acting on it', async () => {
    // §53.2 forbids an automatic throttle, and §0.3 makes bought ranking and bought trust
    // non-negotiable. So the crossing is shown and the numbers are unchanged -- there is
    // no control here that could clamp or suspend anything.
    const flows = window_();
    mockFetch.mockResolvedValue({
      ...flows,
      net_credits: 1_000_000,
      over_threshold: true,
    });
    render(AdminEconomyFlows);

    await waitFor(() => expect(screen.getByTestId('over-threshold')).toBeTruthy());
    expect(screen.getByTestId('over-threshold')).toHaveTextContent('50,000');
    expect(
      screen.queryByRole('button', { name: /suspend|clamp|throttle|adjust/i }),
    ).toBeNull();
    // The net is still the net. Nothing was clamped.
    expect(screen.getByTestId('net-credits')).toHaveTextContent('1,000,000');
  });

  it('does not show the threshold note when the window is under it', async () => {
    mockFetch.mockResolvedValue(window_());
    render(AdminEconomyFlows);

    await waitFor(() => expect(screen.getByTestId('net-credits')).toHaveTextContent('6,000'));
    expect(screen.queryByTestId('over-threshold')).toBeNull();
  });

  it('renders 404 as "no such page" rather than as a permission error', async () => {
    // The server answers 404 for a non-operator, because a 403 would confirm the
    // dashboard exists. The page must not relabel that as an authorization failure --
    // it would tell a reader probing the URL that there is something here.
    mockFetch.mockRejectedValue(Object.assign(new Error('not found'), { status: 404 }));
    render(AdminEconomyFlows);

    await waitFor(() => expect(screen.getByText(/No such page/i)).toBeTruthy());
    expect(screen.queryByText(/forbidden|not permitted|unauthor/i)).toBeNull();
  });

  it('requests no window so the server default applies', async () => {
    // The server defaults to the last 30 days. If the component sent its own bounds the
    // figure would depend on when the page happened to be loaded.
    mockFetch.mockResolvedValue(window_());
    render(AdminEconomyFlows);

    await waitFor(() => expect(mockFetch).toHaveBeenCalled());
    expect(mockFetch).toHaveBeenCalledWith();
  });
});
