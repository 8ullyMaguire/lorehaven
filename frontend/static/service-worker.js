/*
 * Lorehaven's service worker (spec §13.5).
 *
 * What it is for, and what it deliberately does not do:
 *
 *  * **The application shell is cached**, so the interface opens without a
 *    connection. Without this, "offline reading" would mean a reader can reach a
 *    file they saved and cannot reach the page that lists it.
 *
 *  * **`/api` is never cached.** Every response there is either the caller's own
 *    data or a capability-addressed download. A shared or stale cache of a
 *    reader's library is a privacy bug, and a cached download URL is a
 *    single-use token that appears to work twice.
 *
 *  * **A chapter that has been read is cached, network-first.** Spec §9 and the
 *    milestone plan ask that a chapter opened before is readable offline, and
 *    that is the one API response this worker keeps. It is network-first so a
 *    reader who is online gets the revision published a moment ago, and it is
 *    dropped on sign-out, because a cached chapter is the reader's own content
 *    and leaving it for the next person to use this browser would be a leak.
 *
 *  * **Exported files are not fetched through here.** A saved copy lives in
 *    IndexedDB and is opened by the page from those bytes, so the worker never
 *    holds a work's text in a cache the reader cannot see or clear. The reader's
 *    own "Kept in this browser" list is the only inventory of what is stored, and
 *    a cache entry the page cannot enumerate would make that list a lie.
 *
 * The build's asset names are hashed, so assets are cache-first: a name that has
 * not changed cannot have changed contents. The HTML shell is network-first with
 * a cached fallback, so a reader who *is* online gets a deploy immediately.
 */

const SHELL_CACHE = 'lorehaven-shell-v1';
/*
 * Chapters a reader has already opened. Separate from the shell because it holds
 * the reader's own content, and because it has to be dropped on sign-out — see
 * the message handler at the bottom.
 */
const READ_CACHE = 'lorehaven-reads-v1';
const SHELL_URLS = ['/', '/index.html', '/manifest.webmanifest', '/icon.svg', '/prepaint.js'];

self.addEventListener('install', (event) => {
  event.waitUntil(
    (async () => {
      const cache = await caches.open(SHELL_CACHE);
      // Individually, so one missing URL does not abandon the whole install —
      // and `/` may legitimately fail on an instance that redirects it.
      await Promise.all(
        SHELL_URLS.map((url) => cache.add(url).catch(() => undefined)),
      );
      await self.skipWaiting();
    })(),
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    (async () => {
      const names = await caches.keys();
      await Promise.all(
        names
          .filter((name) => name !== SHELL_CACHE && name !== READ_CACHE)
          .map((name) => caches.delete(name)),
      );
      await self.clients.claim();
    })(),
  );
});

self.addEventListener('fetch', (event) => {
  const request = event.request;
  if (request.method !== 'GET') return;

  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;

  // One API response is worth keeping: the reader's own chapter. Network-first,
  // so being online means being current.
  if (/^\/api\/v1\/works\/[^/]+\/chapters\/[^/]+$/.test(url.pathname)) {
    event.respondWith(
      (async () => {
        const cache = await caches.open(READ_CACHE);
        try {
          const response = await fetch(request);
          // A response the server marked `no-store` is not this worker's to
          // keep, and a chapter some other origin answered with is not either.
          const directive = response.headers.get('cache-control') ?? '';
          if (response.ok && response.type === 'basic' && !/no-store/i.test(directive)) {
            await cache.put(request, response.clone());
          }
          return response;
        } catch (error) {
          const cached = await cache.match(request);
          if (cached) return cached;
          throw error;
        }
      })(),
    );
    return;
  }

  // Every other API response is the network's business. See the note at the top.
  if (url.pathname.startsWith('/api/') || url.pathname.startsWith('/health')) return;

  // A navigation: the shell, from the network when there is one.
  if (request.mode === 'navigate') {
    event.respondWith(
      (async () => {
        try {
          const response = await fetch(request);
          const cache = await caches.open(SHELL_CACHE);
          await cache.put('/index.html', response.clone());
          return response;
        } catch {
          const cache = await caches.open(SHELL_CACHE);
          const cached = (await cache.match('/index.html')) ?? (await cache.match('/'));
          return (
            cached ??
            new Response('<h1>Offline</h1><p>This page has not been opened before.</p>', {
              status: 503,
              headers: { 'content-type': 'text/html; charset=utf-8' },
            })
          );
        }
      })(),
    );
    return;
  }

  // Assets: cache-first, because a hashed name is a promise about its contents.
  event.respondWith(
    (async () => {
      const cache = await caches.open(SHELL_CACHE);
      const cached = await cache.match(request);
      if (cached) return cached;
      try {
        const response = await fetch(request);
        if (response.ok && response.type === 'basic') {
          await cache.put(request, response.clone());
        }
        return response;
      } catch (error) {
        // Nothing to fall back to for a file that was never fetched.
        throw error;
      }
    })(),
  );
});

/*
 * Sign-out clears the chapter cache.
 *
 * The page sends this when a reader signs out, whether or not they kept any
 * exported files: a cached chapter is the reader's own content, and it is not
 * one of the files they chose to keep, so it goes unconditionally. The exported
 * files are the reader's and are removed only when they say so, which is why
 * that decision is the page's and this one is not.
 */
self.addEventListener('message', (event) => {
  if (event.data?.type !== 'clear-reads') return;
  event.waitUntil(caches.delete(READ_CACHE));
});
