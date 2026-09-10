/**
 * Timestamps.
 *
 * The API sends RFC 3339 strings in UTC. They are displayed in the reader's own
 * locale and time zone, and an unparseable value is shown as it arrived rather
 * than as "Invalid Date" — a wrong-looking timestamp is more confusing than an
 * obviously raw one.
 */

/** Format a server timestamp for display. */
export function formatTimestamp(
  value: string,
  locale: string = typeof navigator === 'undefined' ? 'en-GB' : navigator.language,
): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(locale, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(date);
}
