/**
 * A very small client-side router.
 *
 * The server falls back to the SPA shell for any extensionless path, so the
 * client only has to map `location.pathname` to a view and intercept
 * same-origin link clicks. Anything more elaborate can come later; what matters
 * is that routing *works* rather than that it is clever.
 *
 * Routes that are linked from the shell but not yet built resolve to `planned`
 * rather than 404. That is deliberate honesty: the navigation is real, and the
 * page says plainly which milestone will fill it, instead of showing mock data
 * (spec §1.1).
 */

export type RouteId =
  | 'home'
  | 'register'
  | 'sign-in'
  | 'password-reset'
  | 'account'
  | 'pseuds'
  | 'pseud-profile'
  | 'planned'
  | 'not-found';

export interface PlannedRoute {
  title: string;
  /** The milestone in `docs/requirements.csv` that will deliver it. */
  milestone: string;
  /** One-line description of what will be there. */
  summary: string;
}

export const PLANNED_ROUTES: Record<string, PlannedRoute> = {
  '/discover': {
    title: 'Discover',
    milestone: 'Milestone 10',
    summary:
      'Recommendation engines, blind date, and the private taste-influence controls that are off by default per instance.',
  },
  '/search': {
    title: 'Search',
    milestone: 'Milestone 9',
    summary:
      'Structured search over characters, relationships, prominence and attributes, with explicit handling of unknown metadata.',
  },
  '/library': {
    title: 'Library',
    milestone: 'Milestone 8',
    summary: 'Imported works, shelves, private tags, update checking and storage usage.',
  },
  '/write': {
    title: 'Write',
    milestone: 'Milestone 3',
    summary:
      'Drafts, chapters, autosave with conflict detection, publishing, and revision history.',
  },
  '/community': {
    title: 'Community',
    milestone: 'Milestone 11',
    summary: 'Comments, forums, groups and messages, with blocks enforced everywhere.',
  },
  '/notifications': {
    title: 'Notifications',
    milestone: 'Milestone 16',
    summary: 'Outbox-driven in-app, email and push delivery, with lock-screen text kept generic.',
  },
};

/**
 * Fixed paths that resolve to a view.
 *
 * `/pseud` is the owner's own pseuds; `/pseud/<handle>` is somebody's public
 * profile, and is matched separately below.
 */
const FIXED_ROUTES: Record<string, RouteId> = {
  '/': 'home',
  '/register': 'register',
  '/sign-in': 'sign-in',
  '/password-reset': 'password-reset',
  '/account': 'account',
  '/pseud': 'pseuds',
};

export interface RouteMatch {
  id: RouteId;
  /** The path the route was matched from. */
  path: string;
  /** Set when `id` is `planned`. */
  planned?: PlannedRoute;
  /** Path parameters, e.g. the handle for a public profile. */
  params?: Record<string, string>;
}

/** Map a path to a view. */
export function matchRoute(path: string): RouteMatch {
  const normalised = path.replace(/\/+$/, '') || '/';

  const fixed = FIXED_ROUTES[normalised];
  if (fixed) return { id: fixed, path: normalised };

  // `/pseud/<handle>`: a public profile. The handle is decoded because it
  // arrives percent-encoded and the server compares it to the stored handle.
  if (normalised.startsWith('/pseud/')) {
    const handle = decodeURIComponent(normalised.slice('/pseud/'.length));
    if (handle) return { id: 'pseud-profile', path: normalised, params: { handle } };
  }

  const planned = PLANNED_ROUTES[normalised];
  if (planned) return { id: 'planned', path: normalised, planned };

  return { id: 'not-found', path: normalised };
}

/** Whether a click event should be handled by the router. */
export function isPlainLeftClick(event: MouseEvent): boolean {
  return (
    event.button === 0 &&
    !event.defaultPrevented &&
    !event.metaKey &&
    !event.ctrlKey &&
    !event.shiftKey &&
    !event.altKey
  );
}

/** Navigate without a full page load. */
export function navigate(to: string, history: History = window.history): void {
  history.pushState({}, '', to);
  window.dispatchEvent(new PopStateEvent('popstate'));
}

/**
 * Handle a click on an in-app link.
 *
 * Modified clicks (new tab, new window, download) are left to the browser: only
 * a plain left click becomes a client-side navigation.
 */
export function handleLinkClick(event: MouseEvent, href: string): void {
  if (!isPlainLeftClick(event)) return;
  event.preventDefault();
  navigate(href);
}
