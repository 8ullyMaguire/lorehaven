import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Home from './Home.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchInstanceMeta: vi.fn(),
    fetchReadiness: vi.fn(),
    fetchContinueReading: vi.fn(),
  };
});

import { fetchContinueReading, fetchInstanceMeta, fetchReadiness } from '../lib/api';

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
  // Defaults to "nothing to continue", which is what an anonymous visitor gets: this
  // suite renders Home with no session, so the banner must be inert and must NOT turn
  // these page-level tests into tests of the retention feature.
  (fetchContinueReading as any).mockResolvedValue(null);
});

afterEach(() => {
  vi.restoreAllMocks();
});

/**
 * The session is module state, so a test that signs in and does not sign out changes
 * every test after it. This is the same class of bug as a suite that only passes on a
 * dirtied database: order-dependent green.
 */
let sessionRef: { status: string } | null = null;
beforeEach(async () => {
  const { session } = await import('../lib/session.svelte');
  session.status = 'anonymous';
  sessionRef = session as unknown as { status: string };
});
afterEach(() => {
  if (sessionRef) sessionRef.status = 'anonymous';
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

  /**
   * WIRING, not behaviour.
   *
   * `ContinueReadingBanner.test.ts` proves the component renders a row. It cannot prove
   * anything about whether Home MOUNTS it, and this project has shipped a feature that
   * passed every component test while being unreachable: `Concierge.svelte` shipped with
   * eleven green tests and no route. The same failure one layer up is a component with
   * green tests and no parent.
   *
   * So: assert the component is IN THE TREE. The negative case below is what makes the
   * positive case meaningful — if the banner were simply never mounted, the "is in the
   * tree" assertion would fail, which is the point.
   */
  it('mounts the continue-reading banner for a signed-in reader', async () => {
    const { session } = await import('../lib/session.svelte');
    session.status = 'signed-in';
    (fetchContinueReading as any).mockResolvedValue({
      workId: 'work-9',
      title: 'Stars Fall Softly',
      positionPermille: 420,
      percent: 42,
      chapterId: 'chapter-7',
      chapterTitle: 'Chapter 7: The Turn',
      updatedAt: '2026-10-03T10:00:00Z',
    });

    render(Home);
    await waitFor(() => expect(screen.getByText('Stars Fall Softly')).toBeInTheDocument());
    expect(screen.getByTestId('continue-reading-percent').textContent).toContain('42%');
  });

  /**
   * The negative case, and it is the one that needed fixing.
   *
   * The first version waited for the page's own async work (`Test Haven` appearing) and
   * then asserted the loader had not been called. That passes whether or not the banner is
   * mounted, because the loader is called from `onMount` — synchronously on mount — but
   * the assertion can run BEFORE any mount effect has fired at all. Mutation testing found
   * it: deleting `signedIn={session.isSignedIn}` from Home left this test green.
   *
   * The fix is to wait for the thing that is only true if the component mounted at all.
   * `hidden` is on the banner node in EVERY state, so its presence proves the component
   * rendered; and once it is present, `onMount` has already run. That makes "not called"
   * a statement about a mounted component rather than a race against one.
   */
  it('does not even ask for continue-reading when signed out', async () => {
    const { session } = await import('../lib/session.svelte');
    session.status = 'anonymous';

    render(Home);
    // The banner node exists only if the component is mounted. This is the count that
    // gives the absence assertion below its subject.
    const banner = await waitFor(() => {
      const node = screen.getByTestId('continue-reading-banner');
      expect(node).toBeInTheDocument();
      return node;
    });
    expect(banner.hasAttribute('hidden')).toBe(true);
    expect(fetchContinueReading).not.toHaveBeenCalled();
  });
});