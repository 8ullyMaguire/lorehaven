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
});
