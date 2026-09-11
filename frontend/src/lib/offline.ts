/**
 * Offline copies of exported files (spec §13.5, §13.6).
 *
 * An export is a file the reader chose to take, and the point of taking it is to
 * have it when the network does not. So the browser keeps its own copy, and the
 * interesting decisions are all about *not* surprising the reader with it:
 *
 *  * The store is bounded. A browser will happily let a site fill the disk, and
 *    a reader who imported a 122-chapter work and exported it four times has a
 *    few hundred megabytes of copies of the same thing. Copies are evicted
 *    oldest-first, and the eviction is computed by a pure function so the rule is
 *    testable without a browser.
 *
 *  * Signing out offers to remove them, and does not remove them silently. Spec
 *    §13.6 is explicit that the offer is made and the answer is the reader's: a
 *    local copy is theirs, and a site that deletes files the user asked for
 *    without saying so is worse than one that leaves them.
 *
 *  * Nothing here is a source of truth. A local copy is a convenience; the
 *    export on the server is the record, and a copy that cannot be found is a
 *    copy that has to be re-downloaded rather than an error.
 */

/** One file the browser has kept. */
export interface OfflineCopy {
  /** The export it came from, which is what the server would call it. */
  exportId: string;
  /** The work or library item it is of, so a reader can find it again. */
  subjectId: string;
  /** What to show: the work's title, as the page that saved it knew it. */
  title: string;
  /** The format, which decides whether this browser can open it at all. */
  format: string;
  mediaType: string;
  /** The bytes. */
  blob: Blob;
  sizeBytes: number;
  /** When it was saved, RFC 3339. */
  savedAt: string;
}

/**
 * How much the store may hold before it evicts.
 *
 * Deliberately modest. An EPUB of a long novel is a few megabytes, so this is
 * "a shelf", not "an archive" — and a reader who wants an archive has the files
 * they downloaded.
 */
export const OFFLINE_LIMIT_BYTES = 512 * 1024 * 1024;

/**
 * Which copies to remove to fit `incomingBytes` into `limitBytes`.
 *
 * Pure, and separate from the storage, because this is the rule that decides
 * whether a reader loses something: oldest first, so the most recently saved
 * copies survive, and the newest copy is never evicted to make room for itself.
 *
 * Returns the ids to evict, in the order to evict them.
 */
export function evictionPlan(
  copies: readonly OfflineCopy[],
  incomingBytes: number,
  limitBytes: number = OFFLINE_LIMIT_BYTES,
): string[] {
  const sorted = [...copies].sort((a, b) => a.savedAt.localeCompare(b.savedAt));
  let used = sorted.reduce((total, copy) => total + copy.sizeBytes, 0);
  const evicted: string[] = [];

  for (const copy of sorted) {
    if (used + incomingBytes <= limitBytes) break;
    evicted.push(copy.exportId);
    used -= copy.sizeBytes;
  }

  // If even an empty store cannot hold it, the caller finds out by comparing the
  // plan's freed bytes against what is needed — this function never claims a
  // file fits when it does not.
  return evicted;
}

/** Whether `incomingBytes` fits after applying the plan. */
export function planFreesEnough(
  copies: readonly OfflineCopy[],
  incomingBytes: number,
  limitBytes: number = OFFLINE_LIMIT_BYTES,
): boolean {
  const plan = evictionPlan(copies, incomingBytes, limitBytes);
  const freed = copies
    .filter((copy) => plan.includes(copy.exportId))
    .reduce((total, copy) => total + copy.sizeBytes, 0);
  const used = copies.reduce((total, copy) => total + copy.sizeBytes, 0);
  return used - freed + incomingBytes <= limitBytes;
}

const DB_NAME = 'lorehaven-offline';
const DB_VERSION = 1;
const STORE = 'exports';

function openDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    if (typeof indexedDB === 'undefined') {
      reject(new Error('this browser cannot keep files offline'));
      return;
    }
    const request = indexedDB.open(DB_NAME, DB_VERSION);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(STORE)) {
        db.createObjectStore(STORE, { keyPath: 'exportId' });
      }
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error('could not open the store'));
  });
}

async function withStore<T>(
  mode: IDBTransactionMode,
  run: (store: IDBObjectStore) => IDBRequest<T>,
): Promise<T> {
  const db = await openDatabase();
  try {
    return await new Promise<T>((resolve, reject) => {
      const transaction = db.transaction(STORE, mode);
      const request = run(transaction.objectStore(STORE));
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error ?? new Error('the store refused'));
    });
  } finally {
    db.close();
  }
}

/** Every copy the browser is holding. */
export async function listCopies(): Promise<OfflineCopy[]> {
  try {
    const copies = await withStore<OfflineCopy[]>('readonly', (store) => store.getAll());
    return copies.sort((a, b) => b.savedAt.localeCompare(a.savedAt));
  } catch {
    // A browser that cannot keep files is not an error: it means the reader
    // downloads to their own device instead, which is what they asked for.
    return [];
  }
}

/** The copy of one export, if this browser has one. */
export async function findCopy(exportId: string): Promise<OfflineCopy | null> {
  try {
    const copy = await withStore<OfflineCopy | undefined>('readonly', (store) =>
      store.get(exportId),
    );
    return copy ?? null;
  } catch {
    return null;
  }
}

/**
 * Keep a file, evicting older copies to make room.
 *
 * Returns the ids that were evicted, so the interface can say what happened
 * rather than the reader finding a file missing later.
 */
export async function saveCopy(copy: OfflineCopy): Promise<string[]> {
  const existing = await listCopies();
  const withoutSame = existing.filter((item) => item.exportId !== copy.exportId);
  const plan = evictionPlan(withoutSame, copy.sizeBytes);
  if (!planFreesEnough(withoutSame, copy.sizeBytes)) {
    throw new Error(
      'this file is larger than the space set aside for offline reading; download it instead',
    );
  }

  for (const exportId of plan) {
    await removeCopy(exportId);
  }
  await withStore('readwrite', (store) => store.put(copy));
  return plan;
}

/** Let go of one copy. */
export async function removeCopy(exportId: string): Promise<void> {
  try {
    await withStore('readwrite', (store) => store.delete(exportId));
  } catch {
    // Nothing to do: the copy is gone or was never there.
  }
}

/** Let go of every copy. Used by the sign-out offer, never on its own. */
export async function clearCopies(): Promise<void> {
  try {
    await withStore('readwrite', (store) => store.clear());
  } catch {
    // As above.
  }
}

/** How much is being held, so the interface can show it. */
export async function copiesUsage(): Promise<{ count: number; bytes: number }> {
  const copies = await listCopies();
  return {
    count: copies.length,
    bytes: copies.reduce((total, copy) => total + copy.sizeBytes, 0),
  };
}

/**
 * Ask the browser not to evict this origin's storage.
 *
 * Asked at the moment the reader saves their first file, which is the moment
 * they have expressed the intent — not on page load, where the prompt is noise.
 * Firefox shows nothing and says no; that is an answer, not a failure.
 */
export async function requestPersistence(): Promise<boolean> {
  if (typeof navigator === 'undefined' || !navigator.storage?.persist) return false;
  try {
    if (await navigator.storage.persisted()) return true;
    return await navigator.storage.persist();
  } catch {
    return false;
  }
}
