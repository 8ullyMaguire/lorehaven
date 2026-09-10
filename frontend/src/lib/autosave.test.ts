import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from './api';
import { ChapterAutosave, type Storage } from './autosave';

/**
 * The autosave contract.
 *
 * What is worth testing here is not that a timer fires, but the promises the
 * writer depends on: that only the newest text is sent, that a rejected save
 * never loses it, that a conflict is distinguishable from being offline, and
 * that the local copy is offered rather than applied.
 */

class FakeStorage implements Storage {
  readonly values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }
}

function paragraph(text: string) {
  return { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text }] }] };
}

describe('chapter autosave', () => {
  let storage: FakeStorage;

  beforeEach(() => {
    storage = new FakeStorage();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function build(options: {
    version?: number;
    save: (payload: { document: unknown; expected_version: number }) => Promise<{ version: number }>;
  }) {
    return new ChapterAutosave(options.version ?? 1, {
      storageKey: 'chapter:draft',
      storage,
      debounceMs: 1000,
      savedVisibleMs: 500,
      save: options.save,
    });
  }

  it('sends only the newest document after the debounce', async () => {
    const save = vi.fn(
      async (_payload: { document: unknown; expected_version: number }) => ({ version: 2 }),
    );
    const autosave = build({ save });

    autosave.update(paragraph('one'));
    autosave.update(paragraph('two'));
    autosave.update(paragraph('three'));

    // Nothing is sent while the writer is still typing.
    expect(save).not.toHaveBeenCalled();
    expect(autosave.current).toBe('pending');

    await vi.advanceTimersByTimeAsync(1000);

    expect(save).toHaveBeenCalledTimes(1);
    expect(save.mock.calls[0][0].document).toEqual(paragraph('three'));
    expect(save.mock.calls[0][0].expected_version).toBe(1);
    expect(autosave.current).toBe('saved');
    expect(autosave.currentVersion).toBe(2);
  });

  it('sends the version the server last returned, so a second tab cannot be overwritten', async () => {
    const save = vi.fn(async ({ expected_version }: { expected_version: number }) => ({
      version: expected_version + 1,
    }));
    const autosave = build({ save });

    autosave.update(paragraph('one'));
    await autosave.flush();
    autosave.update(paragraph('two'));
    await autosave.flush();

    expect(save.mock.calls.map((call) => call[0].expected_version)).toEqual([1, 2]);
  });

  it('keeps the text and reports a conflict when the server refuses the version', async () => {
    const save = vi
      .fn()
      .mockRejectedValueOnce(new ApiError(409, 'REVISION_CONFLICT', 'changed'))
      .mockResolvedValueOnce({ version: 7 });
    const autosave = build({ save });

    autosave.update(paragraph('mine'));
    await autosave.flush();

    expect(autosave.current).toBe('conflict');
    expect(autosave.hasUnsavedWork).toBe(true);
    // The recovery copy is deliberately still there: the text is not lost.
    expect(autosave.recover()?.document).toEqual(paragraph('mine'));

    // Retrying sends the same text again.
    await autosave.retry();
    expect(autosave.current).toBe('saved');
    expect(save.mock.calls[1][0].document).toEqual(paragraph('mine'));
  });

  it('distinguishes being offline from a conflict, and keeps the text either way', async () => {
    const save = vi
      .fn()
      .mockRejectedValueOnce(new ApiError(0, 'NETWORK_UNAVAILABLE', 'offline'))
      .mockResolvedValueOnce({ version: 2 });
    const autosave = build({ save });

    autosave.update(paragraph('offline text'));
    await autosave.flush();

    expect(autosave.current).toBe('offline');
    expect(autosave.message).toContain('this browser');

    autosave.markOffline();
    expect(autosave.current).toBe('offline');

    await autosave.retry();
    expect(autosave.current).toBe('saved');
  });

  it('clears the recovery copy once the server has the text', async () => {
    const save = vi.fn(
      async (_payload: { document: unknown; expected_version: number }) => ({ version: 2 }),
    );
    const autosave = build({ save });

    autosave.update(paragraph('saved text'));
    expect(autosave.recover()).not.toBeNull();

    await autosave.flush();
    expect(autosave.recover()).toBeNull();
  });

  it('adopts a server version without touching the stored document', async () => {
    const autosave = build({
      save: vi.fn(async (_payload: { document: unknown; expected_version: number }) => ({
        version: 2,
      })),
    });

    autosave.acceptServerVersion(paragraph('from the server'), 4);

    expect(autosave.currentVersion).toBe(4);
    expect(autosave.current).toBe('saved');
    expect(autosave.hasUnsavedWork).toBe(false);
  });

  it('survives a corrupt recovery copy rather than failing the page', () => {
    const autosave = build({
      save: vi.fn(async (_payload: { document: unknown; expected_version: number }) => ({
        version: 2,
      })),
    });
    storage.setItem('chapter:draft', 'not json');

    expect(autosave.recover()).toBeNull();
    expect(storage.getItem('chapter:draft')).toBeNull();
  });

  it('reports an unexpected failure without pretending it was saved', async () => {
    const save = vi.fn().mockRejectedValue(new ApiError(500, 'INTERNAL', 'boom'));
    const autosave = build({ save });

    autosave.update(paragraph('text'));
    await autosave.flush();

    expect(autosave.current).toBe('error');
    expect(autosave.message).toBe('boom');
  });
});
