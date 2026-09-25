import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, fireEvent, waitFor } from '@testing-library/svelte';
import MediaSearch from './MediaSearch.svelte';
import * as api from '../lib/api';

vi.mock('../lib/api', async () => {
  return {
    ...(await vi.importActual<typeof import('../lib/api')>('../lib/api')),
    reverseMediaSearch: vi.fn(),
  };
});

describe('MediaSearch', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders hash input by default', () => {
    const { getByLabelText } = render(MediaSearch);
    expect(getByLabelText('Perceptual hash')).toBeTruthy();
    expect(() => getByLabelText('URL')).toThrow();
  });

  it('switches to URL mode', async () => {
    const { getByText, getByLabelText } = render(MediaSearch);
    await fireEvent.click(getByText('By URL'));
    expect(getByLabelText('URL')).toBeTruthy();
    expect(() => getByLabelText('Perceptual hash')).toThrow();
  });

  it('submits hash search and shows results', async () => {
    const mockData: api.ReverseSearchView = {
      references: [
        {
          id: 'ref-1',
          media_kind: 'image',
          perceptual_hash: 'abc123',
          content_hash: 'hash1',
          curator_verified: true,
          match_kind: 'exact',
          match_distance: 0,
          match_confidence: 1,
          auto_attach: true,
        },
      ],
      works: [
        {
          reference_id: 'ref-1',
          work_id: 'work-1',
          work_title: 'Story One',
          display_url: 'https://img.example.com/a.png',
        },
        {
          reference_id: 'ref-1',
          work_id: 'work-2',
          work_title: 'Story Two',
          display_url: null,
        },
      ],
    };
    (api.reverseMediaSearch as ReturnType<typeof vi.fn>).mockResolvedValueOnce(mockData);

    const { getByLabelText, getByText, findByText } = render(MediaSearch);
    const hashInput = getByLabelText('Perceptual hash') as HTMLInputElement;
    await fireEvent.input(hashInput, { target: { value: 'abc123' } });
    await fireEvent.click(getByText('Search'));

    await waitFor(() => {
      expect(api.reverseMediaSearch).toHaveBeenCalledWith({ hash: 'abc123' });
      expect(findByText('Found In (2 works)')).toBeTruthy();
      expect(findByText('Story One')).toBeTruthy();
      expect(findByText('Story Two')).toBeTruthy();
    });
  });

  it('shows empty state when no references found', async () => {
    (api.reverseMediaSearch as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      references: [],
      works: [],
    });

    const { getByLabelText, getByText, findByText } = render(MediaSearch);
    const hashInput = getByLabelText('Perceptual hash') as HTMLInputElement;
    await fireEvent.input(hashInput, { target: { value: 'notfound' } });
    await fireEvent.click(getByText('Search'));

    await waitFor(() => {
      expect(findByText('No matching media')).toBeTruthy();
    });
  });

  it('surfaces errors', async () => {
    (api.reverseMediaSearch as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      new Error('Search failed'),
    );

    const { getByLabelText, getByText, findByText } = render(MediaSearch);
    const hashInput = getByLabelText('Perceptual hash') as HTMLInputElement;
    await fireEvent.input(hashInput, { target: { value: 'fail' } });
    await fireEvent.click(getByText('Search'));

    await waitFor(() => {
      expect(findByText('Search failed')).toBeTruthy();
    });
  });
});
