/**
 * Autosave for the chapter editor (spec §8.3).
 *
 * ```text
 * debounce → local recovery save → versioned server update → visible state
 * ```
 *
 * The states are the point. A writer must be able to tell, at a glance, whether
 * their words are on the server, only in this browser, or not saved at all — so
 * "saving", "saved", "offline" and "conflict" are distinct and none of them is
 * silently collapsed into another.
 *
 * Two rules this module exists to hold:
 *
 * 1. **A rejected save never discards the text.** On a conflict the local copy
 *    is kept (in memory and in storage) and the writer is offered both
 *    versions, because the alternative is losing work to a race.
 * 2. **The debounce is not a queue.** Intermediate keystrokes replace each
 *    other; only the newest document is ever sent. Sending every intermediate
 *    version would fill the revision history with noise.
 *
 * It is deliberately free of Svelte and of the DOM: the storage is injected,
 * the clock is injected, and every transition is a pure function of what
 * happened, so it can be tested without a browser.
 */

import { ApiError } from './api';

/** What the editor can be doing. */
export type SaveState = 'idle' | 'pending' | 'saving' | 'saved' | 'offline' | 'conflict' | 'error';

/** The part of storage this module needs. `window.localStorage` satisfies it. */
export interface Storage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/** What a save attempt needs to send. */
export interface SavePayload {
  /** The editor document. */
  document: unknown;
  /** The version the editor believes it holds. */
  expected_version: number;
}

/** What a successful save returns. */
export interface SaveReceipt {
  /** The new version, which the next save must send back. */
  version: number;
  /** Words in the saved revision, as the server counted them. */
  word_count?: number;
}

export interface AutosaveOptions {
  /** Storage key for the local recovery copy. */
  storageKey: string;
  storage?: Storage | null;
  /** How long to wait after the last keystroke. */
  debounceMs?: number;
  /** How long the "saved" badge stays visible. */
  savedVisibleMs?: number;
  save: (payload: SavePayload) => Promise<SaveReceipt>;
  /** Called on every state change, for the interface to render. */
  onState?: (state: SaveState, detail?: string) => void;
  now?: () => number;
}

/** A recovery copy found in storage. */
export interface RecoveredDraft {
  document: unknown;
  savedAt: number;
}

const DEFAULT_DEBOUNCE_MS = 1200;

export class ChapterAutosave {
  private readonly options: AutosaveOptions;
  private readonly storage: Storage | null;
  private readonly now: () => number;

  private timer: ReturnType<typeof setTimeout> | null = null;
  private savedTimer: ReturnType<typeof setTimeout> | null = null;

  /** The newest document that has not been sent. */
  private pending: unknown | null = null;
  /** The newest document that *has* been sent, for the recovery copy. */
  private latest: unknown | null = null;

  private state: SaveState = 'idle';
  private detail: string | undefined;
  private version: number;

  /** Every state change, in order. Used by tests and by the status line. */
  readonly history: SaveState[] = [];

  constructor(version: number, options: AutosaveOptions) {
    this.version = version;
    this.options = options;
    this.storage = options.storage === undefined ? null : options.storage;
    this.now = options.now ?? (() => Date.now());
  }

  get current(): SaveState {
    return this.state;
  }

  get currentVersion(): number {
    return this.version;
  }

  get message(): string | undefined {
    return this.detail;
  }

  /** Whether anything is waiting to be sent. */
  get hasUnsavedWork(): boolean {
    return this.pending !== null || this.state === 'conflict' || this.state === 'offline';
  }

  /**
   * Accept a document from the editor.
   *
   * Called on every keystroke, so it does as little as possible: it records the
   * document, writes the recovery copy, and restarts the debounce.
   */
  update(document: unknown): void {
    this.pending = document;
    this.latest = document;
    this.writeRecovery();
    this.setState('pending');
    this.schedule();
  }

  /** Change the version the editor believes it holds (after a reload). */
  setVersion(version: number): void {
    this.version = version;
  }

  /** Save now, without waiting for the debounce. */
  async flush(): Promise<void> {
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    await this.save();
  }

  /** Try again after a conflict or a network failure. */
  async retry(): Promise<void> {
    if (this.latest !== null) {
      this.pending = this.latest;
    }
    await this.flush();
  }

  /**
   * Move to the offline state, keeping the text.
   *
   * Called when the browser reports that it is offline. The text is not lost:
   * it is in the recovery copy, and `retry` sends it when there is a network.
   */
  markOffline(): void {
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    this.setState('offline', 'Offline. Your text is safe in this browser.');
  }

  /** Adopt the server's newer document, discarding the local one. */
  acceptServerVersion(document: unknown, version: number): void {
    this.pending = null;
    this.latest = document;
    this.version = version;
    this.clearRecovery();
    this.setState('saved');
  }

  /** Read a recovery copy left behind by a previous visit. */
  recover(): RecoveredDraft | null {
    if (!this.storage) return null;
    const raw = this.storage.getItem(this.options.storageKey);
    if (!raw) return null;
    try {
      const parsed = JSON.parse(raw) as { document?: unknown; savedAt?: number };
      if (parsed.document === undefined) return null;
      return { document: parsed.document, savedAt: parsed.savedAt ?? 0 };
    } catch {
      // A corrupt recovery copy is not worth failing a page load over.
      this.storage.removeItem(this.options.storageKey);
      return null;
    }
  }

  /** Drop the recovery copy, after the writer accepted or discarded it. */
  discardRecovery(): void {
    this.clearRecovery();
    this.setState('idle');
  }

  // -------------------------------------------------------------------------

  private schedule(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    const delay = this.options.debounceMs ?? DEFAULT_DEBOUNCE_MS;
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.save();
    }, delay);
  }

  private async save(): Promise<void> {
    if (this.pending === null) return;
    const document = this.pending;
    this.pending = null;
    this.setState('saving');

    try {
      const receipt = await this.options.save({
        document,
        expected_version: this.version,
      });
      this.version = receipt.version;
      this.clearRecovery();
      this.setState('saved');
      this.hideSavedLater();
    } catch (failure) {
      // Whatever went wrong, the text stays here: it is in `latest`, and the
      // recovery copy in storage is deliberately *not* cleared.
      this.pending = document;
      if (failure instanceof ApiError && failure.code === 'REVISION_CONFLICT') {
        this.setState(
          'conflict',
          'This chapter changed somewhere else. Your text is kept — reload the other version or save yours over it.',
        );
      } else if (
        failure instanceof ApiError &&
        (failure.code === 'NETWORK_UNAVAILABLE' || failure.code === 'REQUEST_ABORTED')
      ) {
        this.setState('offline', 'Could not reach Lorehaven. Your text is kept in this browser.');
      } else {
        const message =
          failure instanceof ApiError ? failure.message : 'The save failed unexpectedly.';
        this.setState('error', message);
      }
    }
  }

  private hideSavedLater(): void {
    if (this.savedTimer !== null) clearTimeout(this.savedTimer);
    const delay = this.options.savedVisibleMs ?? 2500;
    this.savedTimer = setTimeout(() => {
      this.savedTimer = null;
      if (this.state === 'saved') this.setState('idle');
    }, delay);
  }

  private setState(state: SaveState, detail?: string): void {
    this.state = state;
    this.detail = detail;
    this.history.push(state);
    this.options.onState?.(state, detail);
  }

  private writeRecovery(): void {
    if (!this.storage || this.latest === null) return;
    try {
      this.storage.setItem(
        this.options.storageKey,
        JSON.stringify({ document: this.latest, savedAt: this.now() }),
      );
    } catch {
      // Out of quota or private mode: the in-memory copy still holds the text,
      // and losing the recovery copy is not worth interrupting a writer.
    }
  }

  private clearRecovery(): void {
    if (!this.storage) return;
    try {
      this.storage.removeItem(this.options.storageKey);
    } catch {
      /* nothing to do */
    }
  }
}
