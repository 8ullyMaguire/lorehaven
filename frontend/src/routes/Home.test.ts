import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Home from './Home.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchInstanceMeta: vi.fn(),
    fetchReadiness: vi.fn(),
  };
});

import { fetchInstanceMeta, fetchReadiness } from '../lib/api';

const META = {
  name: 'Test Haven',
  version: '1.0.0',
  build: 'abc123',
  api_version: 'v1',
  environment: 'test',
  base_url: 'http://localhost',
  policy: {
    anonymous_reading: true,
    anonymous_max_rating: 'teen',
    unknown_age_max_rating: 'teen',
    minor_max_rating: 'general',
    adult_max_rating: 'explicit',
    registration_open: true,
    csrf_required: true,
  },
};

const READY = {
  status: 'ok',
  build: 'abc123',
  checks: { database: { ok: true, detail: 'connected' } },
};

beforeEach(() => {
  vi.clearAllMocks();
  (fetchInstanceMeta as any).mockResolvedValue(META);
  (fetchReadiness as any).mockResolvedValue(READY);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('Home page', () => {
  it('renders hero text and instance meta', async () => {
    render(Home);
    expect(screen.getByText(/Read, write, and keep what you love/)).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText('Test Haven')).toBeInTheDocument());
  });

  it('shows the readiness status when the backend is up', async () => {
    render(Home);
    await waitFor(() => expect(screen.getByText('abc123')).toBeInTheDocument());
  });

  it('shows instance policy details', async () => {
    render(Home);
    await waitFor(() => expect(screen.getByText('Registration')).toBeInTheDocument());
    expect(screen.getByText('open')).toBeInTheDocument();
  });

  /**
   * The regression that motivated the redesign: the page opened with instance
   * details and service health, so a visitor's first impression of the place
   * was a build hash. Health must be a FOOTNOTE.
   */
  it('offers real destinations before it offers server health', async () => {
    const { container } = render(Home);
    await waitFor(() => expect(screen.getByText('Test Haven')).toBeInTheDocument());

    const pathHeading = screen
      .getByRole('heading', { name: 'Four ways in' })
      .closest('section') as HTMLElement;
    const instanceHeading = screen
      .getByRole('heading', { name: 'This instance' })
      .closest('section') as HTMLElement;

    // Sections appear in document order, so comparing positions is the claim
    // "what the place offers comes before how the server is".
    expect(
      pathHeading.compareDocumentPosition(instanceHeading) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(container.querySelectorAll('section')[0]).not.toBe(
      instanceHeading,
    );
  });

  it('links each way in to a page that exists', async () => {
    render(Home);
    const grid = screen.getByRole('heading', { name: 'Four ways in' })
      .closest('section') as HTMLElement;
    const hrefs = Array.from(grid.querySelectorAll('a')).map((a) =>
      a.getAttribute('href'),
    );
    expect(hrefs).toEqual(
      expect.arrayContaining(['/discover', '/library', '/search', '/write']),
    );
  });

  /**
   * The clause that is easy to break by editing copy: health is reported as one
   * honest word when everything is up, not as a list of every passing check. A
   * page that lists "database: connected" on the front door is a status page
   * wearing a welcome mat.
   */
  it('summarises healthy service as one line, not a list of passing checks', async () => {
    render(Home);
    await waitFor(() =>
      expect(screen.getByText(/all services healthy/)).toBeInTheDocument(),
    );
    expect(screen.queryByText('connected')).not.toBeInTheDocument();
  });

  it('expands only the failing checks, with the remedy', async () => {
    (fetchReadiness as any).mockResolvedValue({
      status: 'degraded',
      build: 'abc123',
      checks: {
        database: { ok: true, detail: 'connected' },
        worker: { ok: false, detail: 'queue not draining', remedy: 'Restart the worker' },
      },
    });
    render(Home);

    await waitFor(() => expect(screen.getByText('worker')).toBeInTheDocument());
    expect(screen.getByText('Restart the worker')).toBeInTheDocument();
    // The healthy check is NOT listed: naming it would be noise on the front page.
    expect(screen.queryByText('connected')).not.toBeInTheDocument();
    expect(screen.queryByText('database')).not.toBeInTheDocument();
  });

  it('says so plainly when checks are failing', async () => {
    (fetchReadiness as any).mockResolvedValue({
      status: 'degraded',
      build: 'abc123',
      checks: { worker: { ok: false, detail: 'not draining' } },
    });
    render(Home);
    await waitFor(() =>
      expect(screen.getByText(/1 service check needing attention/)).toBeInTheDocument(),
    );
  });

  it('does not promise a signing-up route on a closed instance', async () => {
    (fetchInstanceMeta as any).mockResolvedValue({
      ...META,
      policy: { ...META.policy, registration_open: false },
    });
    render(Home);

    await waitFor(() =>
      expect(screen.getByText(/Registration is closed on this instance/)).toBeInTheDocument(),
    );
    // Reading without an account still has to work, so it stays on offer.
    expect(screen.getByRole('link', { name: /Browse without signing in/ })).toBeInTheDocument();
  });
});
