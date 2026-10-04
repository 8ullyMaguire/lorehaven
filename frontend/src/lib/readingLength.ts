/**
 * Item 4: reading time as "12k words · ~48 min".
 *
 * Shared by all three reader-surface components rather than defined in each. Three copies
 * of a rounding decision is three places for the audit's 250 wpm to drift, and a reader who
 * sees "~48 min" on one rail and "~1 hour" on the next has been given two answers.
 *
 * Returns `undefined` — and the caller renders nothing — only when the count is ABSENT.
 * `word_count` is required on `SurfaceWork` and COALESCEd to 0 by the server, so 0 is a real
 * value: a work with no chapters says "no words yet" rather than nothing, which tells the
 * reader the work exists but is unwritten. Silently omitting it would make an unwritten
 * work look the same as one the server failed to measure.
 *
 * `Math.ceil`, not round-to-nearest: under-stating by a minute is the error a reader
 * actually experiences, and over-stating costs them nothing.
 */
export function readingLength(wordCount: number | undefined): string | undefined {
  if (wordCount === undefined) return undefined;
  if (wordCount === 0) return 'no words yet';
  const words =
    wordCount < 1000 ? `${wordCount} words` : `${(wordCount / 1000).toFixed(wordCount < 10000 ? 1 : 0)}k words`;
  const minutes = Math.ceil(wordCount / 250);
  return `${words} · ${minutes === 1 ? 'about a minute' : `~${minutes} min`}`;
}
