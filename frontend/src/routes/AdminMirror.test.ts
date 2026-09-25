import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, fireEvent, waitFor } from '@testing-library/svelte';
import AdminMirror from './AdminMirror.svelte';
import * as api from '../lib/api';

vi.mock('../lib/api', async () => {
  return {
    ...(await vi.importActual<typeof import('../lib/api')>('../lib/api')),
    reverseMediaSearch: vi.fn(),
    fetchLocalMirrors: vi.fn(),
    addLocalMirror: vi.fn(),
    deactivateLocalMirror: vi.fn(),
    fetchIpfsPins: vi.fn(),
    addIpfsPin: vi.fn(),
  };
});

describe('AdminMirror', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders search form', () => {
    const { getByLabelText } = render(AdminMirror);
    expect(getByLabelText('Perceptual hash')).toBeTruthy();
  });

  it('submits search and shows references', async () => {
    const mockData: api.ReverseSearchView = {
      references: [
        { id: 'ref-1', media_kind: 'image', perceptual_hash: 'abc', content_hash: 'hash1', curator_verified: true, match_kind: 'exact', match_distance: 0, match_confidence: 1, auto_attach: true },
      ],
      works: [],
    };
    (api.reverseMediaSearch as ReturnType<typeof vi.fn>).mockResolvedValueOnce(mockData);
    (api.fetchLocalMirrors as ReturnType<typeof vi.fn>).mockResolvedValueOnce([]);
    (api.fetchIpfsPins as ReturnType<typeof vi.fn>).mockResolvedValueOnce([]);

    const { getByLabelText, getByText, findByText } = render(AdminMirror);
    const input = getByLabelText('Perceptual hash') as HTMLInputElement;
    await fireEvent.input(input, { target: { value: 'abc123' } });
    await fireEvent.click(getByText('Search'));

    await waitFor(() => {
      expect(api.reverseMediaSearch).toHaveBeenCalledWith({ hash: 'abc123' });
      expect(findByText('References found')).toBeTruthy();
    });
  });

  it('manages reference when selected', async () => {
    const mockData: api.ReverseSearchView = {
      references: [
        { id: 'ref-1', media_kind: 'image', perceptual_hash: 'abc', content_hash: 'hash1', curator_verified: false, match_kind: 'perceptual', match_distance: 4, match_confidence: 0.9375, auto_attach: false },
      ],
      works: [],
    };
    (api.reverseMediaSearch as ReturnType<typeof vi.fn>).mockResolvedValueOnce(mockData);
    (api.fetchLocalMirrors as ReturnType<typeof vi.fn>).mockResolvedValueOnce([]);
    (api.fetchIpfsPins as ReturnType<typeof vi.fn>).mockResolvedValueOnce([]);

    const { getByLabelText, getByText, findByText } = render(AdminMirror);
    const input = getByLabelText('Perceptual hash') as HTMLInputElement;
    await fireEvent.input(input, { target: { value: 'abc' } });
    await fireEvent.click(getByText('Search'));

    await waitFor(() => {
      expect(findByText('Manage')).toBeTruthy();
    });

    await fireEvent.click(getByText('Manage'));

    await waitFor(() => {
      expect(api.fetchLocalMirrors).toHaveBeenCalledWith('ref-1');
      expect(api.fetchIpfsPins).toHaveBeenCalledWith('ref-1');
      expect(findByText('Local Mirrors')).toBeTruthy();
      expect(findByText('IPFS Pins')).toBeTruthy();
    });
  });

  it('adds a local mirror', async () => {
    const mockData: api.ReverseSearchView = {
      references: [
        { id: 'ref-1', media_kind: 'image', perceptual_hash: 'abc', content_hash: 'hash1', curator_verified: false, match_kind: 'perceptual', match_distance: 4, match_confidence: 0.9375, auto_attach: false },
      ],
      works: [],
    };
    (api.reverseMediaSearch as ReturnType<typeof vi.fn>).mockResolvedValueOnce(mockData);
    (api.fetchLocalMirrors as ReturnType<typeof vi.fn>).mockResolvedValueOnce([]);
    (api.fetchIpfsPins as ReturnType<typeof vi.fn>).mockResolvedValueOnce([]);

    const { getByLabelText, getByText, findByText } = render(AdminMirror);
    const input = getByLabelText('Perceptual hash') as HTMLInputElement;
    await fireEvent.input(input, { target: { value: 'abc' } });
    await fireEvent.click(getByText('Search'));

    await waitFor(() => expect(findByText('Manage')).toBeTruthy());
    await fireEvent.click(getByText('Manage'));

    await waitFor(() => expect(findByText('Storage path')).toBeTruthy());

    const pathInput = document.querySelector('input[placeholder="/data/media/ref-abc.jpg"]') as HTMLInputElement;
    const urlInput = document.querySelector('input[placeholder="https://..."]') as HTMLInputElement;
    await fireEvent.input(pathInput, { target: { value: '/data/media/ref.jpg' } });
    await fireEvent.input(urlInput, { target: { value: 'https://example.com/img.jpg' } });
    await fireEvent.click(getByText('Register Mirror'));

    await waitFor(() => {
      expect(api.addLocalMirror).toHaveBeenCalledWith({
        media_reference_id: 'ref-1',
        storage_path: '/data/media/ref.jpg',
        original_url: 'https://example.com/img.jpg',
      });
    });
  });

  it('surfaces errors', async () => {
    (api.reverseMediaSearch as ReturnType<typeof vi.fn>).mockRejectedValueOnce(new Error('Search failed'));

    const { getByLabelText, getByText, findByText } = render(AdminMirror);
    const input = getByLabelText('Perceptual hash') as HTMLInputElement;
    await fireEvent.input(input, { target: { value: 'fail' } });
    await fireEvent.click(getByText('Search'));

    await waitFor(() => {
      expect(findByText('Search failed')).toBeTruthy();
    });
  });
});

  it('shows how closely each reference matched, and which matches are exact', async () => {
    const mockData: api.ReverseSearchView = {
      references: [
        {
          id: 'ref-exact',
          media_kind: 'image',
          perceptual_hash: '00ff',
          content_hash: 'hash1',
          curator_verified: true,
          match_kind: 'exact',
          match_distance: 0,
          match_confidence: 1,
          auto_attach: true,
        },
        {
          id: 'ref-near',
          media_kind: 'image',
          perceptual_hash: '00fc',
          content_hash: 'hash2',
          curator_verified: false,
          match_kind: 'perceptual',
          match_distance: 4,
          match_confidence: 0.9375,
          auto_attach: false,
        },
      ],
      works: [],
    };
    (api.reverseMediaSearch as ReturnType<typeof vi.fn>).mockResolvedValueOnce(mockData);
    (api.fetchLocalMirrors as ReturnType<typeof vi.fn>).mockResolvedValueOnce([]);
    (api.fetchIpfsPins as ReturnType<typeof vi.fn>).mockResolvedValueOnce([]);

    const { findByText, getAllByText, getByLabelText, getByText } = render(AdminMirror);
    const input = getByLabelText('Perceptual hash') as HTMLInputElement;
    await fireEvent.input(input, { target: { value: '00ff' } });
    await fireEvent.click(getByText('Search'));

    // An operator deciding whether to merge two media records needs to know
    // whether this is the same image or something that merely looks like it.
    await findByText('Exact match');
    await findByText('94% similar');
    // Both are shown, and the hash each row matched on is the visible handle.
    expect(getAllByText('00ff').length).toBeGreaterThan(0);
    expect(getAllByText('00fc').length).toBeGreaterThan(0);
  });
