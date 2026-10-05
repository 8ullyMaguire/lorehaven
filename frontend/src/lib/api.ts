/**
 * The API client.
 *
 * Spec §3.3 fixes the error envelope, so the client can do something genuinely
 * useful with failures: surface the stable `code`, attach field errors to the
 * inputs that caused them, and show the `request_id` so a reader can quote it.
 *
 * One rule the whole client depends on: `apiFetch` never navigates. A 401 from
 * a background poll must not throw a signed-in reader out of the page they are
 * reading. Callers decide what to do; the transport stays dumb.
 */

/** The error envelope defined by spec §3.3. */
export interface ErrorEnvelope {
  code: string;
  message: string;
  field_errors?: Record<string, string>;
  request_id?: string;
}

/** A failure the UI can present without guessing. */
export class ApiError extends Error {
  readonly code: string;
  readonly status: number;
  readonly requestId: string | null;
  readonly fieldErrors: Record<string, string>;

  constructor(
    status: number,
    code: string,
    message: string,
    requestId: string | null = null,
    fieldErrors: Record<string, string> = {},
  ) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
    this.requestId = requestId;
    this.fieldErrors = fieldErrors;
  }

  /** Whether re-authenticating would plausibly change the outcome. */
  get isAuthRequired(): boolean {
    return this.code === 'AUTH_REQUIRED';
  }

  /** Whether the caller should wait and retry. */
  get isRetryable(): boolean {
    return this.status >= 500 || this.code === 'RATE_LIMITED';
  }
}

/** How long to wait before giving up on a request. */
const DEFAULT_TIMEOUT_MS = 15_000;

export interface ApiFetchOptions extends Omit<RequestInit, 'signal'> {
  /** Abort after this many milliseconds. */
  timeoutMs?: number;
  /** An external abort signal, combined with the timeout. */
  signal?: AbortSignal;
}

function joinUrl(base: string, path: string): string {
  const trimmedBase = base.replace(/\/+$/, '');
  const trimmedPath = path.startsWith('/') ? path : `/${path}`;
  return `${trimmedBase}${trimmedPath}`;
}

function readCookie(name: string): string | null {
  if (typeof document === 'undefined') return null;
  const match = document.cookie.match(new RegExp(`(?:^|; )${name}=([^;]*)`));
  return match ? decodeURIComponent(match[1]) : null;
}

/**
 * Perform a JSON API call.
 *
 * @param path Path under the API, e.g. `/meta`.
 * @param options Fetch options; `base` defaults to the same origin.
 */
export async function apiFetch<T>(
  path: string,
  options: ApiFetchOptions & { base?: string } = {},
): Promise<T> {
  const { timeoutMs = DEFAULT_TIMEOUT_MS, base = '/api/v1', signal, ...init } = options;

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(new Error('timeout')), timeoutMs);
  if (signal) {
    if (signal.aborted) controller.abort(signal.reason);
    else signal.addEventListener('abort', () => controller.abort(signal.reason), { once: true });
  }

  const headers = new Headers(init.headers);
  if (init.body !== undefined && !headers.has('content-type')) {
    headers.set('content-type', 'application/json');
  }
  headers.set('accept', 'application/json');

  // CSRF: state-changing, cookie-authenticated requests must carry the token
  // issued alongside the session (spec §3.5). A GET carries nothing extra.
  const method = (init.method ?? 'GET').toUpperCase();
  if (!['GET', 'HEAD', 'OPTIONS'].includes(method)) {
    const token = readCookie('lorehaven_csrf');
    if (token) headers.set('x-csrf-token', token);
  }

  let response: Response;
  try {
    response = await fetch(joinUrl(base, path), { ...init, headers, signal: controller.signal });
  } catch (error) {
    clearTimeout(timer);
    if (controller.signal.aborted) {
      throw new ApiError(0, 'REQUEST_ABORTED', 'The request was cancelled.');
    }
    throw new ApiError(
      0,
      'NETWORK_UNAVAILABLE',
      'Could not reach Lorehaven. Check your connection and try again.',
    );
  } finally {
    clearTimeout(timer);
  }

  if (response.status === 204) {
    return undefined as T;
  }

  const text = await response.text();
  const contentType = response.headers.get('content-type') ?? '';
  const isJson = contentType.includes('application/json');

  if (!response.ok) {
    if (isJson && text) {
      try {
        const envelope = JSON.parse(text) as { error?: ErrorEnvelope };
        const body = envelope.error;
        if (body?.code) {
          throw new ApiError(
            response.status,
            body.code,
            body.message || 'Something went wrong.',
            body.request_id ?? null,
            body.field_errors ?? {},
          );
        }
      } catch (error) {
        if (error instanceof ApiError) throw error;
        // Fall through to the generic error below.
      }
    }
    throw new ApiError(
      response.status,
      'INTERNAL',
      `Lorehaven returned an unexpected ${response.status} response.`,
      response.headers.get('x-request-id'),
    );
  }

  if (!text) {
    return undefined as T;
  }
  if (!isJson) {
    throw new ApiError(
      response.status,
      'INTERNAL',
      'Lorehaven returned a response this client does not understand.',
      response.headers.get('x-request-id'),
    );
  }
  return JSON.parse(text) as T;
}

/** Instance metadata, as returned by `/api/v1/meta`. */
export interface InstanceMeta {
  name: string;
  version: string;
  build: string;
  api_version: string;
  environment: string;
  base_url: string;
  policy: {
    anonymous_reading: boolean;
    anonymous_max_rating: string;
    unknown_age_max_rating: string;
    minor_max_rating: string;
    adult_max_rating: string;
    registration_open: boolean;
    csrf_required: boolean;
  };
}

/** Readiness report, as returned by `/health/ready`. */
export interface ReadinessReport {
  status: string;
  build: string;
  checks: Record<string, { ok: boolean; detail: string; remedy?: string }>;
}

/** Liveness report, as returned by `/health/live`. */
export interface LivenessReport {
  status: string;
  version: string;
  build: string;
  environment: string;
  uptime_ms: number;
}

/** Fetch instance metadata. */
export function fetchInstanceMeta(signal?: AbortSignal): Promise<InstanceMeta> {
  return apiFetch<InstanceMeta>('/meta', { signal });
}

/** Fetch the readiness report. */
export function fetchReadiness(signal?: AbortSignal): Promise<ReadinessReport> {
  return apiFetch<ReadinessReport>('/health/ready', { base: '', signal });
}

/** Fetch the liveness report. */
export function fetchLiveness(signal?: AbortSignal): Promise<LivenessReport> {
  return apiFetch<LivenessReport>('/health/live', { base: '', signal });
}

// ---------------------------------------------------------------------------
// Milestone 2 — identity
//
// These mirror the routes in `crates/app/src/routes/{auth,pseuds,settings}.rs`
// exactly. Where the server omits a field (a pseud never carries its owner, a
// session never carries its token) there is no field here to accidentally
// depend on.
// ---------------------------------------------------------------------------

/** What the account may currently do, as decided by the server. */
export interface Capabilities {
  can_read: boolean;
  can_write: boolean;
  can_message: boolean;
  can_be_listed: boolean;
  /** Highest rating this account may be shown, after policy and preference. */
  max_rating: string;
  /** Plain-language explanation when something is unavailable. */
  restriction_note?: string;
}

/** The signed-in account. */
export interface Account {
  id: string;
  email: string;
  age_state: string;
  email_verified: boolean;
  session_expires_at: string;
}

/** A pseud, as its owner sees it. */
export interface Pseud {
  id: string;
  handle: string;
  display_name: string;
  bio: string | null;
}

/** A pseud with the fields only its owner may see. */
export interface OwnPseud extends Pseud {
  /** `listed` or `hidden`. */
  discoverability: string;
  /** Optimistic-concurrency version; the next `PATCH` must send it back. */
  version: number;
  created_at: string;
  /** Whether this is the acting session's active pseud. */
  active: boolean;
}

/** A pseud as the public sees it: no owner, no version, no session. */
export interface PublicPseud {
  id: string;
  handle: string;
  display_name: string;
  bio: string | null;
  created_at: string;
}

/** One signed-in device. */
export interface SessionSummary {
  id: string;
  created_at: string;
  last_seen_at: string;
  expires_at: string;
  device: string;
  /** Whether this is the session making the request. */
  current: boolean;
}

/** A privacy key the server recognises, with the values it accepts. */
export interface PrivacyKeyDescription {
  key: string;
  summary: string;
  values: string[];
}

/**
 * Privacy settings, account-scoped and per pseud, plus the schema they are
 * validated against. The client renders `schema` rather than keeping its own
 * list of keys, so the two cannot drift apart.
 */
export interface PrivacySettings {
  account: Record<string, string>;
  pseuds: Record<string, Record<string, string>>;
  schema: PrivacyKeyDescription[];
}

/** The reader's content preferences, and the ceiling the policy imposes. */
export interface ContentSettings {
  max_rating: string;
  excluded_warnings: string[];
  /** The highest rating the instance will ever show this account. */
  policy_ceiling: string;
  /** The rating actually in force: the lower of preference and ceiling. */
  effective_max_rating: string;
  /** Optimistic-concurrency version; the next `PATCH` must send it back. */
  version: number;
}

/** Registration input. */
export interface RegisterInput {
  email: string;
  password: string;
  /** Handle for the first pseud. */
  handle: string;
  display_name?: string;
  /** `adult`, `minor` or `unknown`. Absent means `unknown`. */
  age_band?: string;
}

/** The account and its capabilities, as returned by register and login. */
export interface AccountResponse {
  account: Account;
  capabilities: Capabilities;
}

/** `/auth/me`: the account, its pseuds, and which pseud is active. */
export interface MeResponse extends AccountResponse {
  pseuds: Pseud[];
  active_pseud_id: string | null;
  /** Trust level (spec §19.1). Used to gate governance actions (§45). */
  trust_level: number;
}

/** The response to a password-reset request. */
export interface PasswordResetStarted {
  message: string;
  /**
   * Present only outside production, and only because no mail transport is
   * configured yet. Never assume it is there.
   */
  development_token?: string;
}

/** Create an account and sign in. */
export function register(input: RegisterInput): Promise<AccountResponse> {
  return apiFetch<AccountResponse>('/auth/register', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

/** Sign in with an address and password. */
export function signIn(email: string, password: string): Promise<AccountResponse> {
  return apiFetch<AccountResponse>('/auth/login', {
    method: 'POST',
    body: JSON.stringify({ email, password }),
  });
}

/** Revoke the current session and clear the cookies. */
export function signOut(): Promise<void> {
  return apiFetch<void>('/auth/logout', { method: 'POST' });
}

/** The signed-in account, its pseuds and its capabilities. */
export function fetchMe(signal?: AbortSignal): Promise<MeResponse> {
  return apiFetch<MeResponse>('/auth/me', { signal });
}

/** Every live session of the account. */
export function fetchSessions(signal?: AbortSignal): Promise<SessionSummary[]> {
  return apiFetch<SessionSummary[]>('/auth/sessions', { signal });
}

/** Revoke one session. */
export function revokeSession(id: string): Promise<void> {
  return apiFetch<void>(`/auth/sessions/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

/** Revoke every session, including the one making the request. */
export function revokeAllSessions(): Promise<void> {
  return apiFetch<void>('/auth/sessions/revoke-all', { method: 'POST' });
}

/** Ask for a reset link. The answer does not reveal whether the address exists. */
export function requestPasswordReset(email: string): Promise<PasswordResetStarted> {
  return apiFetch<PasswordResetStarted>('/auth/password-reset', {
    method: 'POST',
    body: JSON.stringify({ email }),
  });
}

/** Finish a reset. This also ends every existing session. */
export function completePasswordReset(token: string, newPassword: string): Promise<void> {
  return apiFetch<void>('/auth/password-reset/complete', {
    method: 'POST',
    body: JSON.stringify({ token, new_password: newPassword }),
  });
}

/** The pseuds belonging to the signed-in account. */
export function fetchPseuds(signal?: AbortSignal): Promise<OwnPseud[]> {
  return apiFetch<OwnPseud[]>('/pseuds', { signal });
}

/** Create another pseud. */
export function createPseud(input: {
  handle: string;
  display_name?: string;
  bio?: string;
}): Promise<OwnPseud> {
  return apiFetch<OwnPseud>('/pseuds', { method: 'POST', body: JSON.stringify(input) });
}

/**
 * Edit a pseud.
 *
 * `expected_version` is required and is how a stale edit is refused rather
 * than silently overwriting a change made in another tab (spec §3.4).
 */
export function updatePseud(
  id: string,
  patch: {
    expected_version: number;
    display_name?: string;
    bio?: string | null;
    discoverability?: string;
  },
): Promise<OwnPseud> {
  return apiFetch<OwnPseud>(`/pseuds/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify(patch),
  });
}

/** Make a pseud the face this session posts as. */
export function activatePseud(id: string): Promise<void> {
  return apiFetch<void>(`/pseuds/${encodeURIComponent(id)}/activate`, { method: 'POST' });
}

/** A pseud's public profile. Accepts either a handle or an identifier. */
export function fetchPublicPseud(handleOrId: string, signal?: AbortSignal): Promise<PublicPseud> {
  return apiFetch<PublicPseud>(`/pseuds/${encodeURIComponent(handleOrId)}/profile`, { signal });
}

/** Privacy settings for the account and each of its pseuds. */
export function fetchPrivacy(signal?: AbortSignal): Promise<PrivacySettings> {
  return apiFetch<PrivacySettings>('/settings/privacy', { signal });
}

/**
 * Change privacy settings.
 *
 * Omit `pseudId` for account-level keys; supply it for pseud-level ones. Every
 * key must belong to the scope it is sent under, and the whole request is
 * validated before anything is written.
 */
export function patchPrivacy(
  changes: Record<string, string>,
  pseudId?: string,
): Promise<PrivacySettings> {
  const body = pseudId ? { pseud_id: pseudId, changes } : { changes };
  return apiFetch<PrivacySettings>('/settings/privacy', {
    method: 'PATCH',
    body: JSON.stringify(body),
  });
}

/** The reader's content preferences. */
export function fetchContentSettings(signal?: AbortSignal): Promise<ContentSettings> {
  return apiFetch<ContentSettings>('/settings/content', { signal });
}

/** Change content preferences. */
export function patchContentSettings(patch: {
  expected_version: number;
  max_rating?: string;
  excluded_warnings?: string[];
}): Promise<ContentSettings> {
  return apiFetch<ContentSettings>('/settings/content', {
    method: 'PATCH',
    body: JSON.stringify(patch),
  });
}

// ---------------------------------------------------------------------------
// Milestone 3 — works, chapters, revisions and publication
//
// These mirror `crates/app/src/routes/{works,collaborators}.rs`. Where the
// server omits a field — a public work carries no owner, a public chapter
// carries no editor document — there is no field here to depend on.
// ---------------------------------------------------------------------------

/** A work as it appears in the acting pseud's own list. */
export interface WorkSummary {
  id: string;
  title: string;
  lifecycle: string;
  visibility: string;
  completion: string;
  rating: string;
  updated_at: string;
  published_at: string | null;
  version: number;
  chapter_count: number;
  word_count: number;
  role: string;
}

/** A chapter without its text. */
export interface ChapterSummary {
  id: string;
  title: string;
  order_key: number;
  word_count: number;
  revision_count: number;
  version: number;
  updated_at: string;
  created_at: string;
  has_content: boolean;
  current_revision_id: string | null;
}

/** A contributor, as the author's own view shows them. */
export interface Contributor {
  pseud_id: string;
  handle: string;
  display_name: string;
  role: string;
  public_attribution: boolean;
}

/** A work as a contributor sees it: drafts, versions, blockers and all. */
export interface AuthorWork {
  id: string;
  title: string;
  summary: string;
  language: string;
  rating: string;
  visibility: string;
  lifecycle: string;
  completion: string;
  version: number;
  created_at: string;
  updated_at: string;
  published_at: string | null;
  withdrawn_at: string | null;
  show_public_ratings: boolean;
  /** How discussion happens around this work (thread_only/comments_only/both). */
  discussion_mode: string;
  role: string;
  chapters: ChapterSummary[];
  contributors: Contributor[];
  /** Why it cannot be published yet, in the interface's own language. */
  publication_blockers: string[];
}

/** A publicly credited author. */
export interface PublicAuthor {
  handle: string;
  display_name: string;
  role: string;
}

/** A work as the public sees it. */
export interface PublicWork {
  id: string;
  title: string;
  summary: string;
  language: string;
  rating: string;
  visibility: string;
  completion: string;
  published_at: string | null;
  show_public_ratings: boolean;
  /** How discussion happens around this work (thread_only/comments_only/both). */
  discussion_mode: string;
  /** Public engagement counts (null when the owner opted out). */
  metrics: WorkMetricsView | null;
  authors: PublicAuthor[];
  chapters: ChapterSummary[];
}

/** Public engagement counts for a work card. */
export interface WorkMetricsView {
  views: number;
  complete_reads: number;
  reactions: number;
  kudos: number;
  bookmarks: number;
  collection_adds: number;
  reviews: number;
}

/**
 * Whether a work response is the author's view.
 *
 * The server answers the same URL with one of two shapes, so the client must
 * decide which it received rather than casting and hoping: `contributors` only
 * exists on the author's view.
 */
export function isAuthorWork(work: AuthorWork | PublicWork): work is AuthorWork {
  return (work as AuthorWork).contributors !== undefined;
}

/** A chapter's text, and what may be done with it. */
export interface ChapterContent {
  chapter: ChapterSummary;
  document: unknown | null;
  sanitized_html: string;
  plain_text: string;
  word_count: number;
  revision_number: number | null;
  revision_id: string | null;
  editable: boolean;
  previous_chapter_id: string | null;
  next_chapter_id: string | null;
  work: {
    id: string;
    title: string;
    lifecycle: string;
    authors: PublicAuthor[];
  };
}

/** One entry in a chapter's revision history. */
export interface RevisionEntry {
  id: string;
  revision_number: number;
  word_count: number;
  note: string | null;
  created_at: string;
  restored_from_id: string | null;
  author_handle: string;
  current: boolean;
}

/** An invitation, as either side sees it. */
export interface Invitation {
  id: string;
  work_id: string;
  work_title: string;
  invited_handle: string;
  invited_by_handle: string;
  role: string;
  role_label: string;
  status: string;
  message: string | null;
  created_at: string;
  version: number;
}

/** The acting pseud's own works. */
export function fetchWorks(signal?: AbortSignal): Promise<WorkSummary[]> {
  return apiFetch<WorkSummary[]>('/works', { signal });
}

/** Start a draft. */
export function createWork(title: string): Promise<AuthorWork> {
  return apiFetch<AuthorWork>('/works', { method: 'POST', body: JSON.stringify({ title }) });
}

/**
 * Read a work.
 *
 * The same URL answers a contributor with their draft and a visitor with the
 * published work, so the caller must narrow the result.
 */
export function fetchWork(id: string, signal?: AbortSignal): Promise<AuthorWork | PublicWork> {
  return apiFetch<AuthorWork | PublicWork>(`/works/${encodeURIComponent(id)}`, { signal });
}

/** Change a work's metadata. */
export function updateWork(
  id: string,
  patch: {
    expected_version: number;
    title?: string;
    summary?: string;
    language?: string;
    rating?: string;
    visibility?: string;
    completion?: string;
    show_public_ratings?: boolean;
  },
): Promise<AuthorWork> {
  return apiFetch<AuthorWork>(`/works/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify(patch),
  });
}

/**
 * Publish (or republish) a work.
 *
 * `idempotency_key` makes a retry safe: the same key twice publishes and
 * notifies once.
 */
export function publishWork(
  id: string,
  expectedVersion: number,
  idempotencyKey?: string,
): Promise<AuthorWork> {
  return apiFetch<AuthorWork>(`/works/${encodeURIComponent(id)}/publish`, {
    method: 'POST',
    body: JSON.stringify({ expected_version: expectedVersion, idempotency_key: idempotencyKey }),
  });
}

/** Withdraw a published work. */
export function withdrawWork(
  id: string,
  expectedVersion: number,
  idempotencyKey?: string,
): Promise<AuthorWork> {
  return apiFetch<AuthorWork>(`/works/${encodeURIComponent(id)}/withdraw`, {
    method: 'POST',
    body: JSON.stringify({ expected_version: expectedVersion, idempotency_key: idempotencyKey }),
  });
}

/** Append a chapter. */
export function addChapter(workId: string, title: string): Promise<ChapterSummary> {
  return apiFetch<ChapterSummary>(`/works/${encodeURIComponent(workId)}/chapters`, {
    method: 'POST',
    body: JSON.stringify({ title }),
  });
}

/** Reorder the chapters of a work. The list must be the complete one. */
export function reorderChapters(workId: string, chapters: string[]): Promise<ChapterSummary[]> {
  return apiFetch<ChapterSummary[]>(`/works/${encodeURIComponent(workId)}/reorder-chapters`, {
    method: 'POST',
    body: JSON.stringify({ chapters }),
  });
}

/**
 * Rename a chapter and/or save its text.
 *
 * Saving text appends a revision, so `expected_version` is the chapter version
 * the editor last received; a mismatch is a `REVISION_CONFLICT`.
 */
export function updateChapter(
  id: string,
  patch: { expected_version: number; title?: string; document?: unknown; note?: string },
): Promise<ChapterSummary> {
  return apiFetch<ChapterSummary>(`/chapters/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify(patch),
  });
}

/** A chapter's text. `document` is present only for a contributor. */
export function fetchChapter(
  workId: string,
  chapterId: string,
  signal?: AbortSignal,
): Promise<ChapterContent> {
  return apiFetch<ChapterContent>(
    `/works/${encodeURIComponent(workId)}/chapters/${encodeURIComponent(chapterId)}`,
    { signal },
  );
}

/** A chapter's revision history, newest first. */
export function fetchRevisions(
  chapterId: string,
  signal?: AbortSignal,
): Promise<RevisionEntry[]> {
  return apiFetch<RevisionEntry[]>(`/chapters/${encodeURIComponent(chapterId)}/revisions`, {
    signal,
  });
}

/** Bring an old revision back, as a new revision. */
export function restoreRevision(chapterId: string, revisionId: string): Promise<ChapterSummary> {
  return apiFetch<ChapterSummary>(
    `/chapters/${encodeURIComponent(chapterId)}/restore-revision`,
    { method: 'POST', body: JSON.stringify({ revision_id: revisionId }) },
  );
}

/** Invite a pseud to contribute. */
export function inviteContributor(
  workId: string,
  invite: { handle: string; role: string; message?: string },
): Promise<Invitation> {
  return apiFetch<Invitation>(
    `/works/${encodeURIComponent(workId)}/contributors/invitations`,
    { method: 'POST', body: JSON.stringify(invite) },
  );
}

/** Change a contributor's role or public credit. */
export function updateContributor(
  workId: string,
  pseudId: string,
  patch: { role?: string; public_attribution?: boolean },
): Promise<void> {
  return apiFetch<void>(
    `/works/${encodeURIComponent(workId)}/contributors/${encodeURIComponent(pseudId)}`,
    { method: 'PATCH', body: JSON.stringify(patch) },
  );
}

/** Remove a contributor. */
export function removeContributor(workId: string, pseudId: string): Promise<void> {
  return apiFetch<void>(
    `/works/${encodeURIComponent(workId)}/contributors/${encodeURIComponent(pseudId)}`,
    { method: 'DELETE' },
  );
}

/** Invitations waiting on the acting pseud. */
export function fetchInvitations(signal?: AbortSignal): Promise<Invitation[]> {
  return apiFetch<Invitation[]>('/invitations', { signal });
}

/** Accept or decline an invitation. */
export function respondToInvitation(id: string, accept: boolean): Promise<Invitation> {
  return apiFetch<Invitation>(
    `/invitations/${encodeURIComponent(id)}/${accept ? 'accept' : 'decline'}`,
    { method: 'POST', body: JSON.stringify({}) },
  );
}

/** Revoke a pending invitation. */
export function revokeInvitation(id: string): Promise<void> {
  return apiFetch<void>(`/invitations/${encodeURIComponent(id)}/revoke`, { method: 'POST' });
}

// ---------------------------------------------------------------------------
// Reading (spec §9)
// ---------------------------------------------------------------------------

/** A reading position as stored on the server. */
export interface ServerPosition {
  revision: string | null;
  anchor: string | null;
  position_permille: number;
  device: string | null;
}

/** The resolution of multiple positions for the same subject. */
export interface ProgressResolution {
  kind: 'use_stored' | 'ask_the_reader' | 'no_position';
  position?: ServerPosition;
  mine?: ServerPosition | null;
  other?: ServerPosition | null;
}

/** The full progress view returned by GET /reading/progress. */
export interface ProgressView {
  positions: ServerPosition[];
  resolution: ProgressResolution;
}

/** A history entry, joined with the work's own metadata. */
export interface HistoryItem {
  id: string;
  subject_type: string;
  subject_id: string;
  last_read_at: string;
  title: string;
  authors: string[];
}

/** The history envelope returned by GET /library/history. */
export interface HistoryView {
  items: HistoryItem[];
  next_cursor: string | null;
}

/** A rating view. A rating is private unless `is_public` says otherwise. */
export interface RatingView {
  stars: number;
  is_public: boolean;
  version: number;
}

/** A review. `receipt` is the only sender-visible delivery string (posted vs held). */
export interface ReviewView {
  id: string;
  author_handle: string;
  body: string;
  contains_spoilers: boolean;
  is_public: boolean;
  published_at: string | null;
  version: number;
  receipt?: string | null;
}

/** The envelope every collection answers with (spec §3.3). */
export interface ReviewListView {
  items: ReviewView[];
  next_cursor: string | null;
}

/** A private note view. */
export interface NoteView {
  id: string;
  anchor: string | null;
  body: string;
  created_at: string;
  updated_at: string;
  version: number;
}

/** Typography settings as the server holds them. */
export interface TypographyView {
  font_scale: number;
  line_height: number;
  measure: number;
  reader_theme: string;
  distraction_free: boolean;
  version: number;
}

/** Progress request body for PUT /reading/progress. */
export interface ProgressRequest {
  subject_type: 'work' | 'library_item';
  subject_id: string;
  chapter_id?: string | null;
  /** The revision the reader was looking at, when one is known. */
  content_revision?: string | null;
  paragraph_anchor?: string | null;
  /** Position in the content, in permille (0..=1000). */
  position_permille?: number;
  device_id?: string | null;
}

/** Rating request body. A rating is private unless `is_public` is sent. */
export interface RatingRequest {
  stars: number;
  is_public?: boolean;
  expected_version?: number;
}

/** Review request body. */
export interface ReviewRequest {
  body: string;
  contains_spoilers?: boolean;
  is_public?: boolean;
  expected_version?: number;
}

/** Note request body. */
export interface NoteRequest {
  subject_type: 'work' | 'library_item';
  subject_id: string;
  anchor?: string | null;
  body: string;
}

/** Typography patch request body. Every field but the version is optional. */
export interface TypographyRequest {
  expected_version: number;
  font_scale?: number;
  line_height?: number;
  measure?: number;
  reader_theme?: string;
  distraction_free?: boolean;
}

// ---------------------------------------------------------------------------
// Reading API functions
// ---------------------------------------------------------------------------

/**
 * Save or update the reader's position for a subject.
 *
 * Answers `204`, so there is nothing to return: the position the server holds
 * is read back with `getProgress` when the reader returns.
 */
export function saveProgress(request: ProgressRequest): Promise<void> {
  return apiFetch<void>('/reading/progress', {
    method: 'PUT',
    body: JSON.stringify(request),
  });
}

/** Get positions for a subject (one per device). */
export function getProgress(
  subjectType: 'work' | 'library_item',
  subjectId: string,
  signal?: AbortSignal,
): Promise<ProgressView> {
  const query = `subject_type=${encodeURIComponent(subjectType)}&subject_id=${encodeURIComponent(subjectId)}`;
  return apiFetch<ProgressView>(`/reading/progress?${query}`, { signal });
}

/** Forget this device's position for a subject. */
export function forgetProgress(
  subjectType: 'work' | 'library_item',
  subjectId: string,
): Promise<void> {
  const query = `subject_type=${encodeURIComponent(subjectType)}&subject_id=${encodeURIComponent(subjectId)}`;
  return apiFetch<void>(`/reading/progress?${query}`, { method: 'DELETE' });
}

/** Get the reader's history with optional pagination cursor. */
export function fetchHistory(
  cursor?: string,
  signal?: AbortSignal,
): Promise<HistoryView> {
  const url = cursor
    ? `/library/history?cursor=${encodeURIComponent(cursor)}`
    : '/library/history';
  return apiFetch<HistoryView>(url, { signal });
}

/** Delete a single history entry. */
export function deleteHistoryEntry(id: string): Promise<void> {
  return apiFetch<void>(`/library/history/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

/** Clear the entire history. */
export function clearHistory(): Promise<void> {
  return apiFetch<void>('/library/history/clear', { method: 'POST' });
}

/** Get the caller's own rating for a work, or null when there is none. */
export function fetchRating(workId: string, signal?: AbortSignal): Promise<RatingView | null> {
  return apiFetch<RatingView | null>(`/works/${encodeURIComponent(workId)}/rating`, { signal });
}

/** Submit or update a rating for a work. */
export function upsertRating(
  workId: string,
  request: RatingRequest,
): Promise<RatingView> {
  return apiFetch<RatingView>(`/works/${encodeURIComponent(workId)}/rating`, {
    method: 'PUT',
    body: JSON.stringify(request),
  });
}

/** Delete a rating for a work. */
export function deleteRating(workId: string): Promise<void> {
  return apiFetch<void>(`/works/${encodeURIComponent(workId)}/rating`, { method: 'DELETE' });
}

/** Get public reviews for a work. */
export function fetchReviews(workId: string, signal?: AbortSignal): Promise<ReviewListView> {
  return apiFetch<ReviewListView>(`/works/${encodeURIComponent(workId)}/reviews`, { signal });
}

/** Create or update the caller's review for a work. */
export function upsertReview(
  workId: string,
  request: ReviewRequest,
): Promise<ReviewView> {
  return apiFetch<ReviewView>(`/works/${encodeURIComponent(workId)}/reviews`, {
    method: 'PUT',
    body: JSON.stringify(request),
  });
}

/** Withdraw the caller's review of a work. */
export function deleteReview(workId: string): Promise<void> {
  return apiFetch<void>(`/works/${encodeURIComponent(workId)}/reviews`, { method: 'DELETE' });
}

/** How discussion happens around a work. */
export interface DiscussionModeResponse {
  mode: string;
  comments_enabled: boolean;
  thread_enabled: boolean;
}

/** Get the effective discussion mode for a work. */
export function fetchDiscussionMode(workId: string, signal?: AbortSignal): Promise<DiscussionModeResponse> {
  return apiFetch<DiscussionModeResponse>(`/works/${encodeURIComponent(workId)}/discussion-mode`, { signal });
}

/** Set the discussion mode (author only). */
export function setDiscussionMode(workId: string, mode: string): Promise<{ mode: string }> {
  return apiFetch<{ mode: string }>(`/works/${encodeURIComponent(workId)}/discussion-mode`, {
    method: 'PUT',
    body: JSON.stringify({ mode }),
  });
}

/** The typed-vote reaction bar for a work. */
export interface ReactionsResponse {
  counts: Record<string, number>;
  mine: string | null;
  types: string[];
}

/** Get reaction counts and the caller's own vote. */
export function fetchReactions(workId: string, signal?: AbortSignal): Promise<ReactionsResponse> {
  return apiFetch<ReactionsResponse>(`/works/${encodeURIComponent(workId)}/reactions`, { signal });
}

/** Cast, change, or retract a reaction. */
export function postReaction(
  workId: string,
  voteType: string | null,
): Promise<{ outcome: string }> {
  return apiFetch<{ outcome: string }>(`/works/${encodeURIComponent(workId)}/reactions`, {
    method: 'POST',
    body: JSON.stringify({ vote_type: voteType }),
  });
}

/** Toggle kudos for the signed-in account on a work (spec §9.4). */
export function toggleKudos(workId: string): Promise<{ kudoed: boolean }> {
  return apiFetch<{ kudoed: boolean }>(`/works/${encodeURIComponent(workId)}/kudos`, {
    method: 'POST',
  });
}

// ---------------------------------------------------------------------------
// Reading status on a work (spec §9.6)
// ---------------------------------------------------------------------------
//
// The reader's own record of how far they got. Separate from a rating, a review
// and kudos, and deliberately so: those are statements about the work, this is a
// statement about the reader, which is why it needs no trust level and why it
// lives under `/works/{id}/reading-status` rather than under the library.
//
// Not to be confused with `PUT /library/items/{id}/status`, which keys on a
// *library item* -- a private copy made by an import. Both are named
// `/…/{id}/…status` and they key on different rows; the library door was
// accepting a work id and storing a row against a subject that did not exist.
//
// Reuses the `ReadingStatus` union declared further down this file, which is
// kebab-case to match the server's `as_str`. An earlier version of this comment
// block declared its own four-state union with `on_hold`, which `svelte-check`
// caught as a duplicate identifier and which would have sent the server a
// vocabulary it does not parse -- and would have dropped `want-to-read`, which
// is a real state. The server's `ReadingStatus::parse` is the authority; the
// client mirrors it and nothing else.

export interface WorkReadingStatus {
  status: ReadingStatus;
  started_at: string | null;
  finished_at: string | null;
  updated_at: string;
  version: number;
}

/** The reader's status for this work, or null when they have not recorded one. */
export function fetchWorkReadingStatus(workId: string): Promise<WorkReadingStatus | null> {
  return apiFetch<WorkReadingStatus | null>(
    `/works/${encodeURIComponent(workId)}/reading-status`,
  );
}

export function setWorkReadingStatus(
  workId: string,
  status: ReadingStatus,
): Promise<WorkReadingStatus> {
  return apiFetch<WorkReadingStatus>(`/works/${encodeURIComponent(workId)}/reading-status`, {
    method: 'PUT',
    body: JSON.stringify({ status }),
  });
}

export function clearWorkReadingStatus(workId: string): Promise<void> {
  return apiFetch<void>(`/works/${encodeURIComponent(workId)}/reading-status`, {
    method: 'DELETE',
  });
}

// M43 — Shared browse sort (spec §43.2, §43.4)

/**
 * The browse sort vocabulary, in the server's own words.
 *
 * `Sort::parse` in `crates/domain/src/browse.rs` is the authority and this list
 * matches `Sort::ALL` exactly, in the same order. It is spelled out here rather
 * than fetched so the control renders its options on first paint, before any
 * request resolves -- a reader should not watch an empty select fill itself in.
 */
export const SORT_VALUES = [
  'for-you',
  'new',
  'updated',
  'top',
  'trending',
  'best-match',
  'az',
] as const;

export type SortValue = (typeof SORT_VALUES)[number];

/** Reader-facing labels. The wire value is never shown raw. */
export const SORT_LABELS: Record<SortValue, string> = {
  'for-you': 'For you',
  new: 'Newest',
  updated: 'Recently updated',
  top: 'Top rated',
  trending: 'Trending',
  'best-match': 'Best match',
  az: 'A–Z',
};

export interface SortState {
  surface: string;
  sort: string;
  /** `preference` when the reader chose it, `default` when the surface decided. */
  source: 'preference' | 'default';
}

/** The effective sort for a surface, and where that value came from. */
export function fetchSort(surface: string): Promise<SortState> {
  return apiFetch<SortState>(`/browse/sort/${encodeURIComponent(surface)}`);
}

/**
 * Remember a sort choice for this pseud on this surface (§43.4).
 *
 * PUT rather than a query parameter, because the point is stickiness: the
 * reader chooses once and every later visit to the surface uses it.
 */
export function setSort(surface: string, sort: SortValue): Promise<SortState> {
  return apiFetch<SortState>(`/browse/sort/${encodeURIComponent(surface)}`, {
    method: 'PUT',
    body: JSON.stringify({ sort }),
  });
}

/** Return the surface to its own default. */
export function clearSort(surface: string): Promise<void> {
  return apiFetch<void>(`/browse/sort/${encodeURIComponent(surface)}`, { method: 'DELETE' });
}

// M32 — Typed votes, budgets, meta-moderation, karma (spec §35.2)

export interface CategoryVoteType {
  id: string;
  label: string;
  weight: number;
  cost: number;
  is_negative: boolean;
}

export interface VoteCounts {
  vote_type: string;
  count: number;
}

export interface PostVotesResponse {
  counts: VoteCounts[];
  total: number;
  weighted_bp: number;
  transparency: 'aggregate' | 'authors_only' | 'individual_votes';
  author_opted_in: boolean;
  mine: string | null;
  votes: { id: string; pseud: string; vote_type: string; created_at: string }[] | null;
}

export interface VoteBudgetResponse {
  trust: number;
  window_hours: number;
  limit: number;
  spent: number;
  remaining: number;
  exhausted: boolean;
  resets_at: string | null;
}

export interface KarmaResponse {
  pseud: string;
  karma_bp: number;
  karma: number;
  votes_received: number;
  weighted_received_bp: number;
  updated_at: string | null;
}

/** The taxonomy a category offers. */
export async function getCategoryVoteTypes(
  categoryId: string,
  signal?: AbortSignal,
): Promise<{ category_id: string; items: CategoryVoteType[] }> {
  return apiFetch<{ category_id: string; items: CategoryVoteType[] }>(
    `/forum/categories/${encodeURIComponent(categoryId)}/vote-types`,
    { signal },
  );
}

/** The votes on a post: aggregates to anyone, names per transparency tier. */
export async function getPostVotes(
  postId: string,
  signal?: AbortSignal,
): Promise<PostVotesResponse> {
  return apiFetch<PostVotesResponse>(`/forum/posts/${encodeURIComponent(postId)}/votes`, {
    signal,
  });
}

/** Cast or change a typed vote. Omit voteType (null) to retract. */
export async function castVote(postId: string, voteType: string | null): Promise<{
  outcome: string;
  vote_type: string;
  weight_bp: number;
  budget: { limit: number; spent: number; remaining: number };
}> {
  if (voteType === null) {
    return apiFetch(`/forum/posts/${encodeURIComponent(postId)}/vote`, {
      method: 'DELETE',
    });
  }
  return apiFetch(`/forum/posts/${encodeURIComponent(postId)}/vote`, {
    method: 'POST',
    body: JSON.stringify({ vote_type: voteType }),
  });
}

/** Retract the caller's vote on a post (idempotent). */
export async function retractVote(
  postId: string,
): Promise<{ outcome: string; removed: boolean }> {
  return apiFetch(`/forum/posts/${encodeURIComponent(postId)}/vote`, {
    method: 'DELETE',
  });
}

/** The author's opt-in to revealing who voted. */
export async function setVoteVisibility(postId: string, visible: boolean): Promise<void> {
  await apiFetch(`/forum/posts/${encodeURIComponent(postId)}/vote-visibility`, {
    method: 'PUT',
    body: JSON.stringify({ visible }),
  });
}

/** Meta-moderation: a TL4+ verdict on a vote. */
export async function postMetaVote(
  voteId: string,
  fair: boolean,
): Promise<{ outcome: string }> {
  return apiFetch(`/forum/votes/${encodeURIComponent(voteId)}/meta`, {
    method: 'POST',
    body: JSON.stringify({ fair }),
  });
}

/** The caller's rolling vote allowance. */
export async function getVoteBudget(signal?: AbortSignal): Promise<VoteBudgetResponse> {
  return apiFetch<VoteBudgetResponse>('/me/vote-budget', { signal });
}

/** The caller's own karma. */
export async function getOwnKarma(signal?: AbortSignal): Promise<KarmaResponse> {
  return apiFetch<KarmaResponse>('/forum/karma', { signal });
}

/** A pseud's public karma. */
export async function getKarma(
  pseud: string,
  signal?: AbortSignal,
): Promise<KarmaResponse> {
  return apiFetch<KarmaResponse>(`/forum/karma/${encodeURIComponent(pseud)}`, { signal });
}

/** The work's linked discussion topic. */
export interface ThreadResponse {
  topic_id: string;
  chapter_id: string | null;
}

/** Get the linked forum topic for a work. */
export function fetchThread(workId: string, signal?: AbortSignal): Promise<ThreadResponse> {
  return apiFetch<ThreadResponse>(`/works/${encodeURIComponent(workId)}/thread`, { signal });
}

/** Migrate inline comments to a linked forum topic (author only). */
export function migrateComments(workId: string): Promise<{ topic_id: string; moved: number }> {
  return apiFetch<{ topic_id: string; moved: number }>(
    `/works/${encodeURIComponent(workId)}/migrate-comments`,
    { method: 'POST' },
  );
}

/** The work linked to a topic (backlink card). */
export interface LinkedWorkResponse {
  id: string;
  title: string;
  author_handles: string[];
}

/** Get the work linked to a topic, if any. */
export async function fetchLinkedWork(topicId: string, signal?: AbortSignal): Promise<LinkedWorkResponse> {
  return apiFetch<LinkedWorkResponse>(`/topics/${encodeURIComponent(topicId)}/work`, { signal });
}

// M33 — Thread modes (spec §35.3)

export interface ScheduleSection {
  position: number;
  title: string;
  chapter_start: number;
  chapter_end: number;
  unlocks_at: string;
  created_at: string;
}

export async function getSchedule(topicId: string, signal?: AbortSignal): Promise<ScheduleSection[]> {
  const page = await apiFetch<{ sections: ScheduleSection[] }>(
    `/topics/${encodeURIComponent(topicId)}/schedule`,
    { signal },
  );
  return page.sections;
}

export async function addScheduleSection(
  topicId: string,
  input: { position: number; title: string; chapter_start: number; chapter_end: number; unlocks_at: string },
): Promise<void> {
  await apiFetch(`/topics/${encodeURIComponent(topicId)}/schedule`, {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export async function setTopicMode(topicId: string, mode: string): Promise<void> {
  await apiFetch(`/topics/${encodeURIComponent(topicId)}/mode`, {
    method: 'PUT',
    body: JSON.stringify({ mode }),
  });
}

export interface WikiPin {
  post_id: string;
  body: string;
  author_pseud: string;
  created_at: string;
}

export async function getWikiPin(topicId: string, signal?: AbortSignal): Promise<WikiPin | null> {
  const page = await apiFetch<{ wiki_pin: WikiPin | null }>(
    `/topics/${encodeURIComponent(topicId)}/wiki-pin`,
    { signal },
  );
  return page.wiki_pin;
}

export async function postWikiPin(topicId: string, body: string): Promise<void> {
  await apiFetch(`/topics/${encodeURIComponent(topicId)}/wiki-pin`, {
    method: 'POST',
    body: JSON.stringify({ body }),
  });
}

export async function approveWikiPin(topicId: string, postId: string): Promise<void> {
  await apiFetch(`/topics/${encodeURIComponent(topicId)}/wiki-pin`, {
    method: 'PUT',
    body: JSON.stringify({ post_id: postId }),
  });
}

export async function joinCritique(topicId: string): Promise<number> {
  const page = await apiFetch<{ position: number }>(
    `/topics/${encodeURIComponent(topicId)}/critique/join`,
    { method: 'POST' },
  );
  return page.position;
}

export interface CritiqueEntry {
  /** The waiting critic's pseud. */
  pseud: string;
  /** Queue order within the topic. */
  position: number;
  /** The excerpt queued for critique, if one was recorded. */
  excerpt: string | null;
}

export async function getCritiqueQueue(topicId: string, signal?: AbortSignal): Promise<CritiqueEntry[]> {
  const page = await apiFetch<{ queue: CritiqueEntry[] }>(
    `/topics/${encodeURIComponent(topicId)}/critique/queue`,
    { signal },
  );
  return page.queue;
}

// M34 — Spoilers & readability (spec §35.4)

export interface ContentWarning {
  warning_type: string;
  severity: number;
  custom_text?: string;
}
export async function getContentWarnings(postId: string, signal?: AbortSignal): Promise<ContentWarning[]> {
  const page = await apiFetch<{ warnings: ContentWarning[] }>(
    `/posts/${encodeURIComponent(postId)}/warnings`,
    { signal },
  );
  return page.warnings;
}

export async function addContentWarning(
  postId: string,
  input: { warning_type: string; severity?: number; custom_text?: string },
): Promise<void> {
  await apiFetch(`/posts/${encodeURIComponent(postId)}/warnings`, {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export async function getReaderProgress(workId: string, signal?: AbortSignal): Promise<number> {
  const page = await apiFetch<{ last_chapter: number }>(
    `/works/${encodeURIComponent(workId)}/progress`,
    { signal },
  );
  return page.last_chapter;
}

export async function setReaderProgress(workId: string, lastChapter: number): Promise<void> {
  await apiFetch(`/works/${encodeURIComponent(workId)}/progress`, {
    method: 'PUT',
    body: JSON.stringify({ last_chapter: lastChapter }),
  });
}

export interface WarningPref {
  warning_type: string;
  action: 'blur' | 'show';
}

export async function getWarningPrefs(signal?: AbortSignal): Promise<WarningPref[]> {
  const page = await apiFetch<{ prefs: WarningPref[] }>('/me/warning-prefs', { signal });
  return page.prefs;
}

export async function setWarningPref(input: { warning_type: string; action: 'blur' | 'show' }): Promise<void> {
  await apiFetch('/me/warning-prefs', {
    method: 'PUT',
    body: JSON.stringify(input),
  });
}

// M35 — Moderation & community health (spec §35.5)

export interface Sanction {
  level: 'verbal_warning' | 'post_throttle' | 'read_only' | 'forum_ban' | 'site_ban';
  expires_at?: string;
}

export async function applySanction(input: {
  account: string;
  category_id?: string;
  level: string;
  reason: string;
  expires_at?: string;
}): Promise<string> {
  const page = await apiFetch<{ id: string }>('/mod/sanctions', {
    method: 'POST',
    body: JSON.stringify(input),
  });
  return page.id;
}

export async function checkSanction(account: string, categoryId?: string): Promise<Sanction | null> {
  const q = new URLSearchParams({ account });
  if (categoryId) q.set('category_id', categoryId);
  return apiFetch<Sanction | null>(`/mod/sanctions/check?${q}`);
}

export async function setSlowMode(topicId: string, seconds: number): Promise<void> {
  await apiFetch(`/topics/${encodeURIComponent(topicId)}/slow-mode`, {
    method: 'PUT',
    body: JSON.stringify({ seconds }),
  });
}

export async function setFederationScope(topicId: string, scope: 'public' | 'local' | 'unlisted'): Promise<void> {
  await apiFetch(`/topics/${encodeURIComponent(topicId)}/federation-scope`, {
    method: 'PUT',
    body: JSON.stringify({ scope }),
  });
}

export async function featurePost(postId: string): Promise<void> {
  await apiFetch(`/posts/${encodeURIComponent(postId)}/feature`, { method: 'POST' });
}

/** Get the acting pseud's private notes for a subject. */
export function fetchNotes(
  subjectType: 'work' | 'library_item',
  subjectId: string,
  signal?: AbortSignal,
): Promise<NoteView[]> {
  const query = `subject_type=${encodeURIComponent(subjectType)}&subject_id=${encodeURIComponent(subjectId)}`;
  return apiFetch<NoteView[]>(`/notes?${query}`, { signal });
}

/** Create or update a note. */
export function saveNote(request: NoteRequest): Promise<NoteView> {
  return apiFetch<NoteView>('/notes', { method: 'PUT', body: JSON.stringify(request) });
}

/** Delete a note. */
export function deleteNote(id: string): Promise<void> {
  return apiFetch<void>(`/notes/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

/** Get effective typography settings plus defaults. */
export function fetchTypography(signal?: AbortSignal): Promise<TypographyView> {
  return apiFetch<TypographyView>('/settings/typography', { signal });
}

/** Update typography settings. Returns 409 on stale version. */
export function patchTypography(request: TypographyRequest): Promise<TypographyView> {
  return apiFetch<TypographyView>('/settings/typography', {
    method: 'PATCH',
    body: JSON.stringify(request),
  });
}

// ---------------------------------------------------------------------------
// Positivity filter and feedback delivery (spec §12)
// ---------------------------------------------------------------------------

/** The author's own feedback defaults, plus the one-line effective policy. */
export interface FeedbackPreferencesView {
  accept_constructive: boolean;
  ambiguous_auto: boolean;
  comments_enabled: boolean;
  version: number;
  effective_policy: string;
}

/** The policy in force for one work. */
export interface WorkPolicyView {
  accept_constructive: boolean;
  ambiguous_auto: boolean;
  comments_enabled: boolean;
  effective_policy: string;
}

/** One delivered review on the author's own works. */
export interface FeedbackInboxItem {
  review_id: string;
  work_id: string;
  work_title: string;
  author_handle: string;
  body: string;
  class: string;
  published_at: string | null;
}

/** The author's inbox: delivered items, a held count (never held content). */
export interface FeedbackInbox {
  items: FeedbackInboxItem[];
  held_count: number;
  next_cursor: string | null;
}

/** Read the author's own feedback defaults. */
export function fetchFeedbackPreferences(signal?: AbortSignal): Promise<FeedbackPreferencesView> {
  return apiFetch<FeedbackPreferencesView>('/feedback/preferences', { signal });
}

/** Update the author's own feedback defaults. Returns 409 on stale version. */
export function putFeedbackPreferences(request: {
  accept_constructive?: boolean;
  ambiguous_auto?: boolean;
  comments_enabled?: boolean;
  expected_version?: number;
}): Promise<FeedbackPreferencesView> {
  return apiFetch<FeedbackPreferencesView>('/feedback/preferences', {
    method: 'PUT',
    body: JSON.stringify(request),
  });
}

/** Read the effective policy for one work. */
export function fetchWorkPolicy(workId: string, signal?: AbortSignal): Promise<WorkPolicyView> {
  return apiFetch<WorkPolicyView>(`/feedback/preferences/works/${encodeURIComponent(workId)}`, {
    signal,
  });
}

/** The author's delivered feedback, plus the held count. */
export function fetchFeedbackInbox(signal?: AbortSignal): Promise<FeedbackInbox> {
  return apiFetch<FeedbackInbox>('/feedback/inbox', { signal });
}

// ---------------------------------------------------------------------------
// Search, taxonomy, and tagging (spec §15)
// ---------------------------------------------------------------------------

/** A single search result. */
export interface SearchResult {
  work_id: string;
  title: string;
  author_handle: string;
  word_count: number;
  score: number;
}

/** The search result envelope. */
export interface SearchResultList {
  items: SearchResult[];
}

/** Search works using the AST query language. */
export function searchWorks(
  query: string,
  limit = 20,
  signal?: AbortSignal,
): Promise<SearchResultList> {
  const params = new URLSearchParams({ q: query, limit: String(limit) });
  return apiFetch<SearchResultList>(`/search?${params}`, { signal });
}

/** Search within a single work. */
export function searchInWork(
  workId: string,
  needle: string,
  signal?: AbortSignal,
): Promise<{ pos: number; snippet: string }[]> {
  const params = new URLSearchParams({ q: needle });
  return apiFetch<{ pos: number; snippet: string }[]>(
    `/search/in-work/${encodeURIComponent(workId)}?${params}`,
    { signal },
  );
}

/** A taxonomy node for autocomplete. */
export interface TaxonomyNode {
  id: string;
  kind: string;
  canonical: string;
  norm: string;
  created_at: string;
}

/** Autocomplete taxonomy nodes. */
export function autocompleteTaxonomy(
  kind: string,
  prefix: string,
  limit = 20,
  signal?: AbortSignal,
): Promise<{ items: TaxonomyNode[] }> {
  const params = new URLSearchParams({ kind, prefix, limit: String(limit) });
  return apiFetch<{ items: TaxonomyNode[] }>(`/taxonomy?${params}`, { signal });
}

/** Tag a work with a taxonomy node. */
export function tagWork(
  workId: string,
  nodeId: string,
  weight = 0,
): Promise<unknown> {
  return apiFetch(`/works/${encodeURIComponent(workId)}/tags`, {
    method: 'POST',
    body: JSON.stringify({ node_id: nodeId, weight }),
  });
}

// ---------------------------------------------------------------------------
// The job queue (spec §10.1)

/** One job, as the server describes it. */
export interface Job {
  id: string;
  kind: string;
  state: string;
  payload: string;
  progress_permille: number;
  checkpoint: string | null;
  last_error: string | null;
  attempts: number;
  max_attempts: number;
  available_at: string;
  created_at: string;
  updated_at: string;
  version: number;
  /** The account that asked for the job, when one did. */
  requested_by: string | null;
  /**
   * Whether the server will accept a cancel. Computed server-side so the button
   * is not offered where the answer would be a refusal.
   */
  cancellable: boolean;
}

/** The job envelope returned by `GET /jobs` and `GET /admin/jobs`. */
export interface JobList {
  items: Job[];
  next_cursor: string | null;
}

/** The caller's own queue. */
export function fetchJobs(cursor?: string, signal?: AbortSignal): Promise<JobList> {
  const url = cursor ? `/jobs?cursor=${encodeURIComponent(cursor)}` : '/jobs';
  return apiFetch<JobList>(url, { signal });
}

/** Cancel one of the caller's own jobs. */
export function cancelJob(id: string): Promise<Job> {
  return apiFetch<Job>(`/jobs/${encodeURIComponent(id)}/cancel`, { method: 'POST' });
}

/**
 * Start a job from a request.
 *
 * Development only, and the server refuses anything but the diagnostic
 * maintenance job: the endpoints that need a queue arrive in Milestone 6.
 */
export function startJob(payload: Record<string, unknown> = { task: 'probe' }): Promise<Job> {
  return apiFetch<Job>('/jobs', {
    method: 'POST',
    body: JSON.stringify({ kind: 'maintenance', payload }),
  });
}

/** Every job on the instance. Operators only; everyone else gets a 404. */
export function fetchAllJobs(
  options: { state?: string; cursor?: string } = {},
  signal?: AbortSignal,
): Promise<JobList> {
  const query = new URLSearchParams();
  if (options.state) query.set('state', options.state);
  if (options.cursor) query.set('cursor', options.cursor);
  const suffix = query.toString();
  return apiFetch<JobList>(`/admin/jobs${suffix ? `?${suffix}` : ''}`, { signal });
}

/** Queue a fresh attempt at a job that has finished failing. Operators only. */
export function retryJob(id: string): Promise<Job> {
  return apiFetch<Job>(`/admin/jobs/${encodeURIComponent(id)}/retry`, { method: 'POST' });
}

// ---------------------------------------------------------------------------
// Imports and the library (spec §11)
//
// These mirror `crates/app/src/routes/imports.rs`. Two rules shape them:
//
//  * A preview is not a lesser fetch. It runs the same guard, the same adapter
//    and the same planner the worker will, so what the reader confirms is what
//    will happen.
//  * A credential is never returned. There is no field here for a secret, so
//    there is no way for a page to depend on one.
// ---------------------------------------------------------------------------

/** What an adapter can do, as the catalogue reports it. */
export interface SourceCapabilities {
  /** False for a source this build has no adapter for. */
  known: boolean;
  metadata?: boolean;
  chapters?: boolean;
  /** Whether a single chapter can be re-read without the whole work. */
  per_chapter_fetch?: boolean;
  bibliography?: boolean;
  incremental?: boolean;
  /** `none`, `token`, `password` or `session_cookie`. */
  authentication?: string;
}

/** One source, as `GET /imports/sources` describes it. */
export interface ImportSource {
  key: string;
  display_name: string;
  adapter_version: string;
  enabled: boolean;
  disabled_reason: string | null;
  /** `ok`, `degraded`, `unavailable`, `paused` or `unknown`. */
  health: string;
  last_checked_at: string | null;
  capabilities: SourceCapabilities;
  /** The terms this instance reads sources on. Instance-wide, so identical
   *  across entries — repeated rather than hoisted because the source list is
   *  where an operator looks when one archive refuses. */
  robots?: RobotsTerms;
}

/** How this instance treats a source's own `robots.txt`. */
export interface RobotsTerms {
  /** Whether a path a source forbids is refused. Off is an operator's choice. */
  honour_disallow: boolean;
  /** Whether the pace the same file publishes is still enforced. Always true. */
  honour_crawl_delay: boolean;
  /** What happens to a forbidden path, in the instance's own words. */
  note: string;
}

/** What a preview decided the import would do. */
export interface PlanView {
  /** `create`, `update` or `no_change`. */
  plan: string;
  added: number;
  removed: number;
  reordered: number;
  retitled: number;
  changes: unknown[];
}

/** One chapter as a preview lists it: identity, and no text. */
export interface PreviewChapter {
  ordinal: number;
  source_chapter_key: string;
  title: string;
}

/** The answer to a preview: what was read, and what confirming would do. */
export interface PreviewView {
  source_key: string;
  source_work_key: string;
  source_url: string;
  title: string;
  author_text: string;
  author_url: string | null;
  summary: string;
  language: string | null;
  word_count: number | null;
  status: string;
  chapter_count: number;
  chapters: PreviewChapter[];
  plan: PlanView;
  is_new: boolean;
  /** An apparent copy already held under another source. A warning, not a refusal. */
  duplicate_warning: string | null;
}

/** What starting an import answered with. */
export interface ImportStarted {
  import_id: string;
  job_id: string;
  source_key: string;
  destination: string;
  dry_run: boolean;
  state: string;
  created_at: string;
}

/** One import, as a list or a detail view describes it. */
export interface ImportJobView {
  id: string;
  source_key: string;
  source_url: string;
  destination: string;
  state: string;
  dry_run: boolean;
  library_item_id: string | null;
  report: Record<string, unknown> | null;
  created_at: string;
  updated_at: string;
  cancellable: boolean;
}

/** One stored chapter of an import, as the detail view lists it. */
export interface ImportChapterView {
  ordinal: number;
  source_chapter_key: string;
  title: string;
  state: string;
  checksum: string | null;
  note: string | null;
}

/** An import plus its chapters. */
export interface ImportDetail extends ImportJobView {
  chapters: ImportChapterView[];
}

/** An imported work in the reader's library, with its provenance. */
export interface LibraryItem {
  id: string;
  source_key: string;
  source_work_key: string;
  source_url: string;
  title: string;
  author_text: string;
  author_url: string | null;
  summary: string;
  language: string | null;
  word_count: number | null;
  /**
   * How many chapters of this work are stored and readable.
   *
   * Counted by the server from what it holds, not copied from the source, so a
   * partial import reads as partial rather than as a complete copy of a work the
   * source describes in full.
   */
  chapter_count: number;
  /** What the source is called, for a reader. Falls back to the key. */
  source_display_name: string;
  status: string;
  source_updated_at: string | null;
  last_synced_at: string | null;
  created_at: string;
  updated_at: string;
}

/** An envelope for the import and library listings. */
export interface ImportList<T> {
  items: T[];
  next_cursor: string | null;
}

/** The sources this instance can import from, and what each one can do. */
export function fetchImportSources(signal?: AbortSignal): Promise<{ items: ImportSource[] }> {
  return apiFetch<{ items: ImportSource[] }>('/imports/sources', { signal });
}

/**
 * Ask what an import would do, without doing any of it.
 *
 * A preview writes nothing. That is what makes the confirmation screen honest
 * rather than decorative, and it is why this is a `POST`: the URL is data, and
 * a URL in a query string ends up in access logs.
 */
export function previewImport(url: string, pseudId?: string): Promise<PreviewView> {
  return apiFetch<PreviewView>('/imports/preview', {
    method: 'POST',
    body: JSON.stringify(pseudId ? { url, pseud_id: pseudId } : { url }),
    // Longer than the default: a preview waits on a foreign site, and the
    // server's own clock for one is 20 seconds.
    timeoutMs: 30_000,
  });
}

/**
 * Start an import.
 *
 * `confirmed_plan` is the plan the reader was shown. The server re-derives it
 * and refuses the import if it has changed, so a confirmation cannot be
 * applied to something other than what it described.
 */
export function startImport(request: {
  url: string;
  destination?: string;
  dry_run?: boolean;
  confirmed_plan?: string;
  pseud_id?: string;
}): Promise<ImportStarted> {
  return apiFetch<ImportStarted>('/imports', {
    method: 'POST',
    body: JSON.stringify(request),
  });
}

/** The caller's imports, newest first. */
export function fetchImports(
  options: { state?: string; cursor?: string } = {},
  signal?: AbortSignal,
): Promise<ImportList<ImportJobView>> {
  const query = new URLSearchParams();
  if (options.state) query.set('state', options.state);
  if (options.cursor) query.set('cursor', options.cursor);
  const suffix = query.toString();
  return apiFetch<ImportList<ImportJobView>>(`/imports${suffix ? `?${suffix}` : ''}`, { signal });
}

/** One import, with the state of each of its chapters. */
export function fetchImport(id: string, signal?: AbortSignal): Promise<ImportDetail> {
  return apiFetch<ImportDetail>(`/imports/${encodeURIComponent(id)}`, { signal });
}

/** Stop an import. Cancelling a finished one is not an error. */
export function cancelImport(id: string): Promise<{ import_id: string; state: string }> {
  return apiFetch<{ import_id: string; state: string }>(
    `/imports/${encodeURIComponent(id)}/cancel`,
    { method: 'POST' },
  );
}

/** Queue another attempt at the chapters that failed, and only those. */
export function retryFailedChapters(
  id: string,
): Promise<{ import_id: string; job_id: string; state: string; failed_chapters: number }> {
  return apiFetch<{ import_id: string; job_id: string; state: string; failed_chapters: number }>(
    `/imports/${encodeURIComponent(id)}/retry-failed-chapters`,
    { method: 'POST' },
  );
}

/** Imported works held by the caller, with the source each came from. */
// ---------------------------------------------------------------------------
// Exports (spec §13)
// ---------------------------------------------------------------------------

/** One format this instance can produce, and what it would need to produce it. */
export interface ExportFormatView {
  format: string;
  label: string;
  media_type: string;
  extension: string;
  /** True for the formats this instance renders itself. */
  builtin: boolean;
  available: boolean;
  /** What an operator would install, when it is not available. */
  requires: string | null;
  converter: string | null;
  converter_version: string | null;
}

/** The format catalogue, and the notice a reader has to acknowledge. */
export interface ExportFormatCatalogue {
  formats: ExportFormatView[];
  /**
   * The privacy notice. Returned by the server rather than written in the
   * interface, so the text a reader reads and the text the server enforces
   * cannot drift apart.
   */
  privacy_notice: string;
  retention_days: number;
}

export type ExportState = 'queued' | 'running' | 'ready' | 'failed' | 'cancelled';

/** One export: a request and its outcome. */
export interface ExportJob {
  id: string;
  job_id: string;
  subject_type: 'work' | 'library_item';
  subject_id: string;
  format: string;
  label: string;
  state: ExportState;
  output_bytes: number | null;
  /** True when there is a file to fetch right now. */
  downloadable: boolean;
  error: { code: string; message: string } | null;
  privacy_acknowledged: boolean;
  created_at: string;
  updated_at: string;
}

export interface ExportList {
  exports: ExportJob[];
}

export interface RequestExportInput {
  subjectType: 'work' | 'library_item';
  subjectId: string;
  format: string;
  /** Required. The server refuses an export that has not been told. */
  acknowledgePrivacy: boolean;
  options?: {
    title_page?: boolean;
    chapter_headings?: boolean;
    font_family?: string | null;
    font_size_pt?: number | null;
  };
}

/** What this instance can produce, and what to install for what it cannot. */
export function fetchExportFormats(signal?: AbortSignal): Promise<ExportFormatCatalogue> {
  return apiFetch<ExportFormatCatalogue>('/exports/formats', { signal });
}

/** The caller's own exports, newest first. */
export function fetchExports(signal?: AbortSignal): Promise<ExportList> {
  return apiFetch<ExportList>('/exports', { signal });
}

/** One export, for polling its state. */
export function fetchExport(id: string, signal?: AbortSignal): Promise<ExportJob> {
  return apiFetch<ExportJob>(`/exports/${encodeURIComponent(id)}`, { signal });
}

/**
 * Ask for an export.
 *
 * Answers `202` with a job: the file does not exist yet, and the caller watches
 * the state rather than waiting on this promise for a rendered EPUB.
 */
export function requestExport(input: RequestExportInput): Promise<ExportJob> {
  return apiFetch<ExportJob>('/exports', {
    method: 'POST',
    body: JSON.stringify({
      subject_type: input.subjectType,
      subject_id: input.subjectId,
      format: input.format,
      acknowledge_privacy: input.acknowledgePrivacy,
      options: input.options,
    }),
  });
}

/** Forget an export and delete its file. */
export function deleteExport(id: string): Promise<{ removed: boolean }> {
  return apiFetch<{ removed: boolean }>(`/exports/${encodeURIComponent(id)}`, {
    method: 'DELETE',
  });
}

/** The address of an export's file, for a signed-in download. */
export function exportDownloadUrl(id: string): string {
  return `/api/v1/exports/${encodeURIComponent(id)}/download`;
}

/**
 * The filters a library listing accepts.
 *
 * Filter values travel comma-separated rather than as repeated keys, because a
 * repeated key is not something a query string parser can promise to preserve.
 * Tags are safe to join with a comma: the server collapses whitespace inside a
 * tag and a comma is not a character it keeps.
 */
export interface LibraryQueryParams {
  shelves?: string[];
  tags?: string[];
  statuses?: ReadingStatus[];
  source?: string;
  /** An RFC 3339 instant: only items the source changed at or after it. */
  updatedSince?: string;
  sort?: LibrarySort;
  limit?: number;
}

function libraryQueryString(query: LibraryQueryParams, cursor?: string): string {
  const params = new URLSearchParams();
  if (query.shelves?.length) params.set('shelves', query.shelves.join(','));
  if (query.tags?.length) params.set('tags', query.tags.join(','));
  if (query.statuses?.length) params.set('statuses', query.statuses.join(','));
  if (query.source) params.set('source', query.source);
  if (query.updatedSince) params.set('updated_since', query.updatedSince);
  if (query.sort) params.set('sort', query.sort);
  if (query.limit !== undefined) params.set('limit', String(query.limit));
  if (cursor) params.set('cursor', cursor);
  const suffix = params.toString();
  return suffix ? `?${suffix}` : '';
}

/** One page of the reader's library, filtered and sorted. */
export function fetchLibraryItems(
  query: LibraryQueryParams = {},
  cursor?: string,
  signal?: AbortSignal,
): Promise<LibraryPage> {
  return apiFetch<LibraryPage>(`/library/items${libraryQueryString(query, cursor)}`, { signal });
}

/** A page of the library, with the total the filter matched. */
export interface LibraryPage {
  items: LibraryItem[];
  total: number;
  next_cursor: string | null;
}

/** How a listing is ordered. */
export type LibrarySort = 'recent' | 'title' | 'updated' | 'words' | 'position';

/** What a reader has done with a work. */
export type ReadingStatus = 'want-to-read' | 'reading' | 'on-hold' | 'dropped' | 'finished';

/** A shelf, as the reader made it. */
export interface Shelf {
  id: string;
  name: string;
  description: string;
  is_public: boolean;
  position: number;
  item_count: number | null;
  created_at: string;
  updated_at: string;
  version: number;
}

/** The reader's shelves, in sidebar order. */
export function fetchShelves(signal?: AbortSignal): Promise<ImportList<Shelf>> {
  return apiFetch<ImportList<Shelf>>('/shelves', { signal });
}

/** Make a shelf. */
export function createShelf(input: {
  name: string;
  description?: string;
  is_public?: boolean;
}): Promise<Shelf> {
  return apiFetch<Shelf>('/shelves', { method: 'POST', body: JSON.stringify(input) });
}

/** Rename, describe, share or reorder a shelf. */
export function updateShelf(
  id: string,
  patch: {
    name?: string;
    description?: string;
    is_public?: boolean;
    position?: number;
    expected_version: number;
  },
): Promise<void> {
  return apiFetch<void>(`/shelves/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify(patch),
  });
}

/** Delete a shelf. The works on it stay in the library. */
export function deleteShelf(id: string): Promise<void> {
  return apiFetch<void>(`/shelves/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

/** Put a library item on a shelf. */
export function addToShelf(shelfId: string, libraryItemId: string): Promise<void> {
  return apiFetch<void>(
    `/shelves/${encodeURIComponent(shelfId)}/items/${encodeURIComponent(libraryItemId)}`,
    { method: 'POST', body: JSON.stringify({}) },
  );
}

/** Take a library item off a shelf. */
export function removeFromShelf(shelfId: string, libraryItemId: string): Promise<void> {
  return apiFetch<void>(
    `/shelves/${encodeURIComponent(shelfId)}/items/${encodeURIComponent(libraryItemId)}`,
    { method: 'DELETE' },
  );
}

/** One shelf, with the items on it. */
export function fetchShelf(
  id: string,
  signal?: AbortSignal,
): Promise<{ shelf: Shelf; library_item_ids: string[] }> {
  return apiFetch<{ shelf: Shelf; library_item_ids: string[] }>(
    `/shelves/${encodeURIComponent(id)}`,
    { signal },
  );
}

/** A bookmark: a note about a place in a work. */
export interface Bookmark {
  id: string;
  subject_type: string;
  subject_id: string;
  chapter_id: string | null;
  position_permille: number | null;
  note: string;
  is_public: boolean;
  created_at: string;
  updated_at: string;
  version: number;
}

export function fetchBookmarks(
  subject?: { type: string; id: string },
  signal?: AbortSignal,
): Promise<ImportList<Bookmark>> {
  const params = new URLSearchParams();
  if (subject) {
    params.set('subject_type', subject.type);
    params.set('subject_id', subject.id);
  }
  const suffix = params.toString();
  return apiFetch<ImportList<Bookmark>>(`/bookmarks${suffix ? `?${suffix}` : ''}`, { signal });
}

export function createBookmark(input: {
  subjectType: string;
  subjectId: string;
  chapterId?: string;
  positionPermille?: number;
  note?: string;
  isPublic?: boolean;
}): Promise<Bookmark> {
  return apiFetch<Bookmark>('/bookmarks', {
    method: 'POST',
    body: JSON.stringify({
      subject_type: input.subjectType,
      subject_id: input.subjectId,
      chapter_id: input.chapterId ?? null,
      position_permille: input.positionPermille ?? null,
      note: input.note ?? '',
      // Private unless asked otherwise, and the server says the same.
      is_public: input.isPublic ?? false,
    }),
  });
}

export function updateBookmark(
  id: string,
  patch: { note?: string; positionPermille?: number; isPublic?: boolean; expectedVersion: number },
): Promise<void> {
  return apiFetch<void>(`/bookmarks/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify({
      note: patch.note,
      position_permille: patch.positionPermille,
      is_public: patch.isPublic,
      expected_version: patch.expectedVersion,
    }),
  });
}

export function deleteBookmark(id: string): Promise<void> {
  return apiFetch<void>(`/bookmarks/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

/** The reader's own tags on one item. */
export function fetchItemTags(id: string, signal?: AbortSignal): Promise<ImportList<string>> {
  return apiFetch<ImportList<string>>(
    `/library/items/${encodeURIComponent(id)}/tags`,
    { signal },
  );
}

export function addItemTag(id: string, tag: string): Promise<void> {
  return apiFetch<void>(
    `/library/items/${encodeURIComponent(id)}/tags/${encodeURIComponent(tag)}`,
    { method: 'PUT', body: JSON.stringify({}) },
  );
}

export function removeItemTag(id: string, tag: string): Promise<void> {
  return apiFetch<void>(
    `/library/items/${encodeURIComponent(id)}/tags/${encodeURIComponent(tag)}`,
    { method: 'DELETE' },
  );
}

export function setReadingStatus(id: string, status: ReadingStatus): Promise<unknown> {
  return apiFetch<unknown>(`/library/items/${encodeURIComponent(id)}/status`, {
    method: 'PUT',
    body: JSON.stringify({ status }),
  });
}

export function clearReadingStatus(id: string): Promise<void> {
  return apiFetch<void>(`/library/items/${encodeURIComponent(id)}/status`, { method: 'DELETE' });
}

/** A stored query. */
export interface SavedView {
  id: string;
  name: string;
  query: Record<string, unknown> | null;
  needs_repair: boolean;
  query_version: number;
  sort: LibrarySort;
  scope: string;
  pinned: boolean;
  is_public: boolean;
  created_at: string;
  updated_at: string;
  version: number;
}

export function fetchSavedViews(signal?: AbortSignal): Promise<ImportList<SavedView>> {
  return apiFetch<ImportList<SavedView>>('/saved-views', { signal });
}

export function deleteSavedView(id: string): Promise<void> {
  return apiFetch<void>(`/saved-views/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

/** What the reader's library occupies. */
export interface StorageUsage {
  imported_bytes: number;
  export_bytes: number;
  total_bytes: number;
  item_count: number;
  blob_count: number;
  counts: string;
}

export function fetchStorageUsage(signal?: AbortSignal): Promise<StorageUsage> {
  return apiFetch<StorageUsage>('/library/storage', { signal });
}

/** One item's outcome in a batch. */
export interface BatchFailure {
  id: string;
  code: string;
  message?: string;
}

/** What a batch did, per item, with the sentence the interface shows. */
export interface BatchResult {
  succeeded: string[];
  failed: BatchFailure[];
  summary: string;
  freed_bytes: number;
  delete_copy: boolean;
  removed: number;
}

/**
 * Remove several items at once.
 *
 * `deleteCopy` is the difference between taking an item off the shelf and
 * throwing the copy away, and the answer covers both cases.
 */
export function batchRemoveItems(ids: string[], deleteCopy: boolean): Promise<BatchResult> {
  return apiFetch<BatchResult>('/library/items/batch', {
    method: 'POST',
    body: JSON.stringify({ ids, delete_copy: deleteCopy }),
  });
}

/** Ask the server to check the library against its sources. Answers a job. */
export function startUpdateCheck(): Promise<{ job_id: string; items: number }> {
  return apiFetch<{ job_id: string; items: number }>('/library/updates/check', {
    method: 'POST',
    body: JSON.stringify({}),
  });
}

// ---------------------------------------------------------------------------
// Discovery and taste profile (spec §16)
// ---------------------------------------------------------------------------

export interface DiscoveryItem {
  work_id: string;
  title?: string;
  author_handle?: string;
  word_count?: number;
  /**
   * Item 9: the id of the recorded recommendation slot this item was served as.
   *
   * OPTIONAL, and that asymmetry with `word_count` above is deliberate. The server only
   * sets this when it successfully recorded the response, and it warns-and-continues when
   * that write fails (`crates/app/src/routes/discovery.rs`, the `Err` arm of
   * `record_response`). So a slot id is genuinely absent on a healthy server under some
   * conditions, unlike a word count, which is either computed or the item is not here.
   *
   * A missing slot id means "no explanation available", and the UI shows nothing rather
   * than a disabled control: a "why?" button that never works is worse than no button.
   */
  slot_id?: string;
}

/**
 * Item 9: why a recommendation appears.
 *
 * `reasons` is the server's closed vocabulary (`SlotReason`), so the wire forms are the
 * lower-snake enum names: `taste_tags`, `popular`, `media_reference_collaborative`,
 * `strategy`, `reading_history`, `saved_search`. Unknown values are passed through rather
 * than dropped, because the server normalises and the set may grow.
 *
 * This is a READ of the recorded slot, never a recomputation — see
 * `lorehaven_db::recommendation_slots`'s header. The reason a replay would be wrong is
 * that `time_decay_strategy` reads the clock inside its scoring query, so it explains a
 * different ranking than the one the reader actually received.
 */
export interface SlotExplanation {
  slot_id: string;
  work_id: string;
  position: number;
  reasons: string[];
  blend_score?: number | null;
  served_at?: string | null;
}

/** Human wording for one reason code. */
const SLOT_REASON_LABEL: Record<string, string> = {
  taste_tags: 'matches tags your reading has weighted',
  popular: 'popular on this instance',
  media_reference_collaborative: 'shares media with something you saved',
  strategy: 'a recommendation strategy chose this',
  reading_history: 'followed from something you bookmarked',
  saved_search: 'matches a search you saved',
};

export interface DiscoveryFeed {
  items: DiscoveryItem[];
}

// ---------------------------------------------------------------------------
// M45-22 — spec §54, the personal concierge.
//
// Three shapes, not one, because the server distinguishes them and a UI that
// collapses them is a UI that lies (spec §54.6):
//
//   * `items: []` with `explained_empty` set   — the selector matched nothing.
//   * `items: []` with `truncated_at === 0`    — the budget matched nothing.
//   * `items` truncated at `truncated_at`      — the budget cut the tail.
//
// The first two are both an empty list and a page that renders one "nothing here"
// message for both would be indistinguishable from a fallback to the unfiltered
// queue — which is the exact defect §54.6 forbids. So `explained_empty` and
// `truncated_at` are separate fields here, not a union with a `kind`.
// ---------------------------------------------------------------------------

export interface ConciergeItem {
  work_id: string;
  title?: string;
  author_handle?: string;
  /** The §54.4 estimate for this work. Null when nothing knows its length. */
  estimated_minutes: number | null;
  /**
   * Why this item is in the queue — §54.6's transparency.
   *
   * `kind: 'blend'` means the mood filter did not select it, which a reader who
   * asked for a mood is entitled to see rather than have smoothed over.
   */
  reason: ConciergeReason;
}

export type ConciergeReason =
  | { kind: 'blend' }
  | { kind: 'mood'; mood: string }
  | { kind: 'budget' }
  | { kind: 'unknown_length' };

export interface ConciergeQueue {
  session_id: string;
  items: ConciergeItem[];
  /**
   * The sum of what was RETURNED, never the budget that was asked for.
   *
   * §54.4 charges the queue for the works it actually serves, so a reader who asks
   * for 100 minutes and gets 80 minutes of work has been served 80 minutes. Showing
   * the request instead would make the server look like it overran.
   */
  estimated_minutes: number;
  /**
   * The index the budget cut at, or null when nothing was cut.
   *
   * Null and 0 are different facts and both are reachable: null means the selector
   * matched nothing (§54.6's explained empty), 0 means items existed and none of
   * them fit. The server distinguishes them and so does this type.
   */
  truncated_at: number | null;
  /**
   * §54.4's rate provenance: `observed` when it came from the reader's own
   * progress, `default` when it did not. Stated so a reader comparing two queues
   * built on different rates is not left guessing at the difference.
   */
  rate_source: 'observed' | 'default';
  /** §54.6's explanation, present only when the queue is empty. */
  explained_empty?: string | null;
}

export interface ConciergeSession {
  id: string;
  mood: string | null;
  budget_minutes: number | null;
  estimated_minutes: number;
  truncated_at: number | null;
  rate_source: 'observed' | 'default';
  work_ids: string[];
  created_at: string;
}

export interface ConciergeWatch {
  work_id: string;
  pending: boolean;
  created_at: string;
}

/**
 * Render a queue for this reader (spec §54.4).
 *
 * `mood` and `minutes` are omitted rather than sent as `null`/`0` when unset: the
 * server reads a present `minutes=0` as "I have no time" and answers with an
 * explained empty queue, which is a different response from the unfiltered queue.
 * Building the query by hand means the absent case is actually absent.
 */
export async function fetchConciergeQueue(
  selector: { mood?: string; minutes?: number } = {},
  signal?: AbortSignal,
): Promise<ConciergeQueue> {
  const parts: string[] = [];
  if (selector.mood !== undefined && selector.mood !== '') {
    parts.push(`mood=${encodeURIComponent(selector.mood)}`);
  }
  if (selector.minutes !== undefined) {
    parts.push(`minutes=${encodeURIComponent(String(selector.minutes))}`);
  }
  const query = parts.length ? `?${parts.join('&')}` : '';
  return apiFetch<ConciergeQueue>(`/me/concierge${query}`, { signal });
}

/** This reader's own render history (spec §54.3). */
export async function fetchConciergeSessions(signal?: AbortSignal): Promise<ConciergeSession[]> {
  const body = await apiFetch<{ sessions: ConciergeSession[] }>('/me/concierge/sessions', { signal });
  return body.sessions ?? [];
}

/** This reader's watches (spec §54.5). */
export async function fetchConciergeWatches(signal?: AbortSignal): Promise<ConciergeWatch[]> {
  const body = await apiFetch<{ watches: ConciergeWatch[] }>('/me/watches', { signal });
  return body.watches ?? [];
}

/**
 * Watch a work and be told when it finishes (spec §54.5).
 *
 * The response is the WATCH, not a bare 200, because the immediate-notify case is
 * decided server-side: a work that is already complete notifies on this call and
 * one that is not will notify later. `notified` is how the page knows which
 * happened, and reading it back from the watch list instead would need a second
 * round trip to say the same thing.
 */
export async function watchWork(workId: string): Promise<ConciergeWatch & { notified: boolean }> {
  return apiFetch<ConciergeWatch & { notified: boolean }>(
    `/me/watches/${encodeURIComponent(workId)}`,
    // `{}` and not no body: the route takes no selector, but it is a PUT through a
    // CSRF-bound path, and every other mutating call in this file sends a JSON
    // body so the header is set the same way.
    { method: 'PUT', body: JSON.stringify({}) },
  );
}

/**
 * Withdraw a watch (spec §54.5).
 *
 * Idempotent by design: withdrawing is silent and leaves no tombstone, so a
 * retried DELETE must succeed rather than 404 on a state the caller already has.
 * `removed` reports whether anything went, so a caller can tell a real withdrawal
 * from a repeat.
 */
export async function unwatchWork(workId: string): Promise<{ removed: boolean }> {
  return apiFetch<{ removed: boolean }>(`/me/watches/${encodeURIComponent(workId)}`, {
    method: 'DELETE',
  });
}

/**
 * The discovery feed, optionally in a chosen order (spec §43.2).
 *
 * `sort` is passed through only when the reader has picked one. Omitting it
 * entirely is what lets the server apply its own resolution -- query param >
 * stored preference > surface default -- and that ordering is the requirement.
 * Sending an explicit default instead would override a stored preference with
 * the surface default, silently undoing §43.4.
 */
export async function fetchDiscoveryFeed(
  sort?: SortValue,
  signal?: AbortSignal,
): Promise<DiscoveryFeed> {
  const query = sort ? `?sort=${encodeURIComponent(sort)}` : '';
  return apiFetch<DiscoveryFeed>(`/discovery${query}`, { signal });
}

/**
 * Blind Date: one work for this reader today (gap B, spec §16.1a).
 *
 * `workId` is `null` rather than absent when there is nothing to offer — the endpoint
 * returns 200 for an empty catalogue, and an empty catalogue is a quiet surface rather
 * than an error. Modelling that as `null` instead of throwing is what lets the page
 * render an empty state instead of a failure.
 *
 * There is deliberately no `date` parameter. The server derives the day itself, so a
 * client cannot enumerate forward and turn a once-a-day surface into a catalogue
 * browser — which is the entire reason the pick is deterministic in (account, day).
 */
export interface BlindDateResponse {
  /** YYYY-MM-DD, the server's own date for this pick. */
  date: string;
  workId: string | null;
}

export async function fetchBlindDate(signal?: AbortSignal): Promise<BlindDateResponse> {
  const body = await apiFetch<{ date: string; work_id: string | null }>(
    '/discovery/blind-date',
    { signal },
  );
  return { date: body.date, workId: body.work_id };
}

/**
 * Surprise Me: one work from OUTSIDE the reader's taste profile (item 7, §16.10).
 *
 * Not Blind Date with a different seed. Blind Date leaves the profile by ignoring it;
 * this goes specifically away from it, which is the whole feature. The server excludes work
 * sharing any tag the profile weights, and `profileEmpty` says whether the profile it went
 * away from had anything in it.
 *
 * **`profileEmpty` is load-bearing and is the reason this type is not just `BlindDateResponse`.**
 * The endpoint returns 200 with `work: null` for two quite different situations: an empty
 * public catalogue, and a reader whose profile covers the whole catalogue. A UI that treats
 * both as "nothing to show" tells a reader with strong taste that the button is broken. So
 * the flag travels with the work and the empty state is worded from it.
 *
 * Like Blind Date there is no way to ask for another pick: the endpoint takes no parameters
 * and re-rolls per request, so the surface stays a small deliberate departure rather than a
 * catalogue browser.
 */
export interface SurpriseMeResponse {
  work: {
    workId: string;
    title: string;
    summary: string;
  } | null;
  /** True when the reader has no taste profile at all, so the pick was unconstrained. */
  profileEmpty: boolean;
}

export async function fetchSurpriseMe(signal?: AbortSignal): Promise<SurpriseMeResponse> {
  const body = await apiFetch<{
    work: { work_id: string; title: string; summary: string } | null;
    profile_empty: boolean;
  }>('/discovery/surprise-me', { signal });
  return {
    work: body.work
      ? { workId: body.work.work_id, title: body.work.title, summary: body.work.summary }
      : null,
    profileEmpty: body.profile_empty,
  };
}

/**
 * A pre-read report, as the author-facing endpoint returns it (gap C, §32.6).
 *
 * **There is deliberately no `score`, no `average`, and no way to compute one.** §32.6
 * forbids displaying composite quality scores publicly and §0.3 forbids credit, payment
 * or trust level moving any ranking signal, so a composite in this type would be one
 * template expression away from being rendered. What exists is the per-dimension
 * breakdown, worst-first, plus the dimensions that never came back and why.
 *
 * The same rule applies to `Dimension`: a `score` is fine there because it is one
 * configured dimension's own value, not an average of several.
 */
export interface PreReadDimension {
  dimension: string;
  score: number;
  note: string;
}

/**
 * A configured dimension that has no score, and why.
 *
 * The `reason` is required rather than optional. A report where every dimension came back
 * and one where half the provider's output was unparseable are the same shape without it,
 * and that difference is exactly what tells an author whether to trust the numbers.
 */
export interface PreReadMissing {
  dimension: string;
  reason: string;
}

export interface PreReadReport {
  workId: string;
  /** Worst first. The author's question is "what is weakest". */
  dimensions: PreReadDimension[];
  missing: PreReadMissing[];
  /** Whether every configured dimension came back. */
  complete: boolean;
}

/**
 * The report, or the reason there is not one.
 *
 * A discriminated union rather than `report: PreReadReport | null` so a caller cannot
 * forget to branch: `status: 'assessed'` always carries a report and `status` of anything
 * else always carries a readable reason. The server distinguishes "nothing has assessed
 * this work" from "a provider is listed but its row is gone", and a UI that collapsed
 * them would show an author an empty report for a work that was never assessed.
 */
export type PreReadResponse =
  | { status: 'assessed'; report: PreReadReport; providers: string[] }
  | {
      status: 'not_assessed' | 'provider_absent';
      providers: string[];
      reason: string;
    };

/**
 * Fetch the current pre-read report for a work the caller owns.
 *
 * The server returns 404 for both "no such work" and "not yours" — a pre-read report is an
 * assessment of a draft, so any difference would confirm to an outsider that the draft
 * exists and that its author ran an AI tool on it. That 404 arrives here as a thrown
 * error, and the caller must treat it as "this panel is not for you" rather than as an
 * error worth showing the user.
 */
export async function fetchPreread(
  workId: string,
  signal?: AbortSignal,
): Promise<PreReadResponse> {
  const body = await apiFetch<{
    report: {
      work_id: string;
      dimensions: Array<{ dimension: string; score: number; note: string }>;
      missing: Array<{ dimension: string; reason: string }>;
      complete: boolean;
    } | null;
    providers: string[];
    reason?: string;
  }>(`/works/${encodeURIComponent(workId)}/preread`, { signal });

  if (body.report === null) {
    // `provider_listed_but_report_absent` is only reachable through a concurrent
    // withdrawal, and it is kept distinct because it means something is wrong rather than
    // merely un-assessed.
    return {
      status: body.reason === 'provider_listed_but_report_absent' ? 'provider_absent' : 'not_assessed',
      providers: body.providers,
      reason: body.reason ?? 'no_provider_has_assessed_this_work',
    };
  }
  return {
    status: 'assessed',
    providers: body.providers,
    report: {
      workId: body.report.work_id,
      dimensions: body.report.dimensions,
      missing: body.report.missing,
      complete: body.report.complete,
    },
  };
}

/**
 * Withdraw one provider's output for this work (§23.7's opt-out of *specific* providers).
 *
 * Scoped to a single provider rather than the whole work, so this never discards another
 * provider's report. The returned count is what lets the UI tell a withdrawal that happened
 * from one that did not, instead of re-rendering the same screen twice and calling it
 * success.
 */
export async function forgetPrereadProvider(
  workId: string,
  provider: string,
): Promise<{ removed: number; providers: string[] }> {
  const body = await apiFetch<{ removed: number; providers: string[] }>(
    `/works/${encodeURIComponent(workId)}/preread/${encodeURIComponent(provider)}`,
    { method: 'DELETE' },
  );
  return { removed: body.removed, providers: body.providers };
}

export async function recomputeTasteProfile(): Promise<void> {
  await apiFetch('/discovery/taste-profile/recompute', { method: 'POST' });
}

export async function clearTasteProfile(): Promise<void> {
  await apiFetch('/discovery/taste-profile/clear', { method: 'POST' });
}

// ---------------------------------------------------------------------------
// Notifications (spec §23)
// ---------------------------------------------------------------------------

export interface NotificationItem {
  id: string;
  kind: string;
  title: string;
  body: string;
  read: boolean;
  created_at: string;
  work_id?: string;
}

export interface NotificationList {
  items: NotificationItem[];
  unread_count: number;
}

export async function fetchNotifications(signal?: AbortSignal): Promise<NotificationList> {
  return apiFetch<NotificationList>('/notifications', { signal });
}

export async function markAllNotificationsRead(): Promise<void> {
  await apiFetch('/notifications/read-all', { method: 'POST' });
}

export async function markNotificationRead(id: string): Promise<void> {
  await apiFetch(`/notifications/${encodeURIComponent(id)}/read`, { method: 'POST' });
}

// ---------------------------------------------------------------------------
// Community — forums, groups, messages, blocks (spec §17)
// ---------------------------------------------------------------------------

export interface Forum {
  id: string;
  name: string;
  description?: string;
  category?: string;
}

export interface Group {
  id: string;
  name: string;
  description?: string;
  privacy: string;
  member_count?: number;
  owner?: string;
  created_at?: string;
}

export interface Conversation {
  id: string;
  other_handle: string;
  last_message?: string;
  updated_at?: string;
}

export interface Block {
  blocked: string;
  scope: string;
  created_at: string;
}

export async function fetchForums(signal?: AbortSignal): Promise<Forum[]> {
  const page = await apiFetch<{ items: Forum[] }>('/forums', { signal });
  return page.items;
}

/** A topic inside a forum category. */
export interface ForumTopic {
  id: string;
  category_id: string;
  title: string;
  author_pseud: string;
  /** The author's handle, resolved server-side; falls back to author_pseud. */
  author_handle?: string;
  created_at: string;
  locked: boolean;
  /** Thread mode: 'plain', 'reading_group', 'critique_circle', 'wiki_pin', 'prompt'. */
  mode: string;
  /** Chapter to which spoiler scope is limited (null = whole topic). */
  spoiler_scope_chapter?: number | null;
  /** Seconds between posts (slow mode). */
  slow_mode_seconds?: number;
  /** Federation scope: 'public', 'local', 'unlisted'. */
  federation_scope?: string;
}

/** One reply inside a topic thread. */
export interface ForumPost {
  id: string;
  topic_id: string;
  author_pseud: string;
  /** The author's handle, resolved server-side; falls back to author_pseud. */
  author_handle?: string;
  body: string;
  created_at: string;
}

export async function fetchTopics(categoryId: string, signal?: AbortSignal): Promise<ForumTopic[]> {
  const page = await apiFetch<{ items: ForumTopic[] }>(
    `/forums/${encodeURIComponent(categoryId)}/topics`,
    { signal },
  );
  return page.items;
}

export async function createTopic(
  categoryId: string,
  title: string,
): Promise<ForumTopic> {
  return apiFetch<ForumTopic>(`/forums/${encodeURIComponent(categoryId)}/topics`, {
    method: 'POST',
    body: JSON.stringify({ title }),
  });
}

export async function createTopicWithMode(
  categoryId: string,
  title: string,
  mode: string,
): Promise<ForumTopic> {
  return apiFetch<ForumTopic>(`/forums/${encodeURIComponent(categoryId)}/topics`, {
    method: 'POST',
    body: JSON.stringify({ title, mode }),
  });
}

export async function fetchTopic(topicId: string, signal?: AbortSignal): Promise<ForumTopic> {
  const page = await apiFetch<{ topic: ForumTopic }>(`/topics/${encodeURIComponent(topicId)}`, {
    signal,
  });
  return page.topic;
}

export async function fetchPosts(topicId: string, signal?: AbortSignal): Promise<ForumPost[]> {
  const page = await apiFetch<{ items: ForumPost[] }>(
    `/topics/${encodeURIComponent(topicId)}/replies`,
    { signal },
  );
  return page.items;
}

export async function createReply(topicId: string, body: string): Promise<ForumPost> {
  return apiFetch<ForumPost>(`/topics/${encodeURIComponent(topicId)}/replies`, {
    method: 'POST',
    body: JSON.stringify({ body }),
  });
}

export interface ForumSearchResult {
  kind: string;
  id: string;
  title: string;
  snippet: string;
  author_pseud: string;
  created_at: string;
  score: number;
}

/**
 * Search forum posts and topics (spec §17.4).
 *
 * `q` is a query-language query and the server parses it, so a filter can be
 * typed straight into the box (`replies:>50 category:meta`) as well as passed
 * as a structured parameter. The structured parameters are translated into
 * query terms server-side and conjoined with `q`, so a reader who sets both
 * means both.
 *
 * `category` is a category *name*, which is what the dropdown holds.
 */
export async function searchForum(
  params: {
    q: string;
    category?: string;
    author?: string;
    from?: string;
    to?: string;
    /** Minimum reply count. Omitted when unset rather than sent as 0. */
    min_replies?: number;
    limit?: number;
  },
  signal?: AbortSignal,
): Promise<ForumSearchResult[]> {
  const sp = new URLSearchParams();
  sp.set('q', params.q);
  if (params.category) sp.set('category', params.category);
  if (params.author) sp.set('author', params.author);
  if (params.from) sp.set('from', params.from);
  if (params.to) sp.set('to', params.to);
  if (params.min_replies !== undefined) {
    sp.set('min_replies', String(params.min_replies));
  }
  if (params.limit) sp.set('limit', String(params.limit));
  const page = await apiFetch<{ items: ForumSearchResult[] }>(
    `/search?${sp.toString()}`,
    { signal },
  );
  return page.items;
}


/** One pseudonym matching a user search. */
export interface UserSearchResult {
  pseud_id: string;
  handle: string;
  display_name: string | null;
  joined_at: string | null;
}

/**
 * Search pseudonyms with the shared query language.
 *
 * The query is built by the caller from the filter boxes, using the same
 * syntax the works and forum searches take. The server refuses a field that
 * belongs to another surface with a 422 that names where it does belong, so
 * there is no structured-parameter translation here: every filter is already
 * a term a reader can see and edit in the box.
 */
export async function searchUsers(
  q: string,
  limit = 20,
  signal?: AbortSignal,
): Promise<{ total: number; items: UserSearchResult[] }> {
  const sp = new URLSearchParams();
  sp.set('q', q);
  if (limit) sp.set('limit', String(limit));
  return apiFetch<{ total: number; items: UserSearchResult[] }>(
    `/users/search?${sp.toString()}`,
    { signal },
  );
}

export async function fetchGroups(signal?: AbortSignal): Promise<Group[]> {
  const page = await apiFetch<{ items: Group[] }>('/groups', { signal });
  return page.items;
}

export async function fetchConversations(signal?: AbortSignal): Promise<Conversation[]> {
  const page = await apiFetch<{ items: Conversation[] }>('/conversations', { signal });
  return page.items;
}

export async function fetchBlocks(signal?: AbortSignal): Promise<Block[]> {
  const page = await apiFetch<{ items: Block[] }>('/me/blocks', { signal });
  return page.items;
}

// ---------------------------------------------------------------------------
// Monetization (spec §20.9)
// ---------------------------------------------------------------------------

/** Pricing model for a work. */
export interface PricingRow {
  id: string;
  model: string;          // "purchase" | "tips"
  price_minor: number;
  currency: string;
  public_at_offset: number | null;
  enabled: boolean;
  created_at: string;
  updated_at: string;
  version: number;
}

/** An entitlement the acting account holds. */
export interface EntitlementRow {
  id: string;
  work_id: string;
  kind: string;
  source_payment_id: string | null;
  granted_at: string;
  expires_at: string | null;
}

/** An earnings entry on the author's ledger. */
export interface EarningsRow {
  id: string;
  amount_minor: number;
  currency: string;
  kind: string;
  payment_id: string | null;
  idempotency_key: string | null;
  created_at: string;
}

/** Public pricing info for a work (anonymous-readable). */
export interface PublicPricing {
  model: string;
  price_minor: number;
  currency: string;
  public_at_offset: number | null;
}

export interface PublicPricingResponse {
  pricing: PublicPricing[];
}

/** Purchase result. */
export interface PurchaseResult {
  entitlement_id: string;
  status: string;
  amount_minor: number;
  currency: string;
}

/** Fetch public pricing for a work (anonymous-readable). */
export function fetchWorkPricing(
  workId: string,
  signal?: AbortSignal,
): Promise<PublicPricingResponse> {
  return apiFetch<PublicPricingResponse>(
    `/works/${encodeURIComponent(workId)}/pricing`,
    { signal },
  );
}

/** Purchase a work. Requires a signed-in session with a pseud. */
export function purchaseWork(workId: string): Promise<PurchaseResult> {
  return apiFetch<PurchaseResult>(
    `/works/${encodeURIComponent(workId)}/purchase`,
    { method: 'POST' },
  );
}

/** Set pricing on a work (author only). */
export function setWorkPricing(
  workId: string,
  pricing: { model: string; price_minor: number; currency: string; public_at_offset: number | null },
): Promise<PricingRow> {
  return apiFetch<PricingRow>(
    `/works/${encodeURIComponent(workId)}/pricing`,
    { method: 'POST', body: JSON.stringify(pricing) },
  );
}

/** Remove pricing from a work (author only). */
export function deleteWorkPricing(workId: string): Promise<void> {
  return apiFetch<void>(
    `/works/${encodeURIComponent(workId)}/pricing`,
    { method: 'DELETE' },
  );
}

/** Fetch the acting account's entitlements. */
export function fetchMyEntitlements(signal?: AbortSignal): Promise<EntitlementRow[]> {
  return apiFetch<{ entitlements: EntitlementRow[] }>('/me/entitlements', { signal }).then(
    (page) => page.entitlements,
  );
}

/** Fetch the acting account's earnings. */
export function fetchMyEarnings(signal?: AbortSignal): Promise<EarningsRow[]> {
  return apiFetch<{ earnings: EarningsRow[] }>('/me/earnings', { signal }).then(
    (page) => page.earnings,
  );
}

// ---------------------------------------------------------------------------
// Vanguard — role status and pinning (spec §16.18)
// ---------------------------------------------------------------------------

export interface VanguardStatus {
  is_vanguard: boolean;
}

export interface Pin {
  id: string;
  account_id: string;
  work_id: string;
  pin_reason: string;
  message: string | null;
  created_at: string;
}

/** Check whether the acting account holds the Vanguard role. */
export function fetchVanguardStatus(): Promise<VanguardStatus> {
  return apiFetch<VanguardStatus>('/vanguard/status');
}

/** Fetch pins for a work (public). */
export function fetchPinsForWork(workId: string, signal?: AbortSignal): Promise<{ pins: Pin[] }> {
  return apiFetch<{ pins: Pin[] }>(`/vanguard/pins/${encodeURIComponent(workId)}`, { signal });
}

/** Pin a work to the Vanguard Picks shelf (vanguard only). */
export function pinWork(
  workId: string,
  body: { pin_reason: string; message?: string },
): Promise<{ id: string }> {
  return apiFetch<{ id: string }>(`/vanguard/pins/${encodeURIComponent(workId)}`, {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

/** Unpin a work (vanguard only). */
export function unpinWork(workId: string): Promise<void> {
  return apiFetch<void>(`/vanguard/pins/${encodeURIComponent(workId)}`, { method: 'DELETE' });
}

/** List all vanguards (admin only). */
export function fetchVanguards(signal?: AbortSignal): Promise<{ vanguards: string[] }> {
  return apiFetch<{ vanguards: string[] }>('/vanguards', { signal });
}

// ---------------------------------------------------------------------------
// Quiz — onboarding taste quiz (spec §0.4.2)
// ---------------------------------------------------------------------------

export interface QuizAnswer {
  picked: string[];
  rejected: string[];
}

export interface QuizSkipResponse {
  status: string;
}

/** Fetch quiz works for onboarding. */
/**
 * Item 9: why this item was recommended.
 *
 * A slot id the reader does not own is a 404, not a 403 — §3.3 prefers 404 so the id cannot
 * be probed for the existence of someone else's recommendation. So a 404 here means either
 * "no such slot" or "not yours", and the caller cannot tell them apart or should not try.
 *
 * Returns `null` for a 404 rather than throwing: "no explanation" is a normal outcome for
 * a feed item whose slot write failed, and it must not become an error toast.
 */
export async function fetchSlotExplanation(
  slotId: string,
  signal?: AbortSignal,
): Promise<SlotExplanation | null> {
  try {
    return await apiFetch<SlotExplanation>(`/discovery/slots/${slotId}/explanation`, { signal });
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) return null;
    throw e;
  }
}

/** The label for one reason code, or the code itself if the vocabulary has grown. */
export function slotReasonLabel(reason: string): string {
  return SLOT_REASON_LABEL[reason] ?? reason;
}

export function fetchQuizWorks(signal?: AbortSignal): Promise<{ works: DiscoveryItem[] }> {
  return apiFetch<{ works: DiscoveryItem[] }>('/quiz/works', { signal });
}

/** Save quiz answers. */
export function saveQuizAnswers(body: { picked: string[]; rejected?: string[] }): Promise<{
  status: string;
  vector_dimensions: number;
}> {
  return apiFetch('/quiz/answers', { method: 'POST', body: JSON.stringify(body) });
}

/** Fetch the signed-in user's quiz answers. */
export function fetchMyQuizAnswers(signal?: AbortSignal): Promise<QuizAnswer> {
  return apiFetch<QuizAnswer>('/quiz/answers', { signal });
}

/** Skip the quiz. */
export function skipQuiz(): Promise<QuizSkipResponse> {
  return apiFetch<QuizSkipResponse>('/quiz/skip', { method: 'POST' });
}

/** Check the signed-in user's reading streak (spec §9.7.1). */
export function fetchMyStreak(signal?: AbortSignal): Promise<{
  current_streak: number;
  longest_streak: number;
  last_read_date: string | null;
}> {
  return apiFetch('/me/streak', { signal });
}

// ---------------------------------------------------------------------------
// Taste Calibration Arena (spec §0.4.2a)
// ---------------------------------------------------------------------------

export interface ArenaCard {
  work_id: string;
  title: string;
  fandom: string;
  tags: string[];
  word_count: number;
  excerpt: string;
  target_dimension: string;
}

export interface ArenaRound {
  cards: ArenaCard[];
}

export interface DimensionSummary {
  key: string;
  label: string;
  matches_played: number;
}

export interface ArenaNextResponse {
  /**
   * `null` when there is no round for this reader right now.
   *
   * An ordinary state, not a failure: a reader who has rated nothing, on an
   * instance whose works do not yet share a fandom in fours, has no comparison to
   * be offered. The server used to answer 500 here, which opened `/arena` with a
   * red "That did not work" for every new account.
   */
  round: ArenaRound | null;
  /** Why there is no round. Absent whenever `round` is present. */
  explained_empty?: string | null;
  dimensions: DimensionSummary[];
}

export interface ArenaVoteRequest {
  best_work_id: string;
  worst_work_id: string;
  reason_tags: string[];
}

export interface ArenaVoteResponse {
  success: boolean;
  message: string;
  next_round: ArenaRound | null;
}

export interface DimensionWeightSummary {
  key: string;
  label: string;
  influence: string;
}

export interface ArenaWeightsResponse {
  dimensions: DimensionWeightSummary[];
}

/** Fetch the next arena round. */
export function fetchArenaNext(signal?: AbortSignal): Promise<ArenaNextResponse> {
  return apiFetch<ArenaNextResponse>('/arena/next', { signal });
}

/** Submit an arena ballot. */
export function submitArenaVote(body: ArenaVoteRequest): Promise<ArenaVoteResponse> {
  return apiFetch<ArenaVoteResponse>('/arena/vote', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

/** Dismiss the arena. */
export function dismissArena(): Promise<{ success: boolean; message: string }> {
  return apiFetch('/arena/dismiss', { method: 'POST' });
}

/** Fetch the signed-in user's arena weight summary. */
export function fetchArenaWeights(signal?: AbortSignal): Promise<ArenaWeightsResponse> {
  return apiFetch<ArenaWeightsResponse>('/arena/weights', { signal });
}

// ---------------------------------------------------------------------------
// Resource Directory (spec §39)
// ---------------------------------------------------------------------------

export interface DirectoryList {
  id: string;
  slug: string;
  title: string;
  description: string;
  kind: string;
}

export interface DirectoryEntry {
  id: string;
  title: string;
  url: string | null;
  description: string;
  category: string;
  tags: string[];
  score: number;
  my_vote: number | null;
  submitted_by: string;
  approved_by: string | null;
  /**
   * Absent on older responses and on endpoints that do not compute it, so it
   * is optional rather than nullable: "the server did not say" and "the
   * server said there is nothing to say" are different, and the UI shows
   * nothing in both.
   */
  decay?: DirectoryVoteDecay;
}

export interface DirectoryCategory {
  category: string;
  approved_count: number;
}

/** Fetch all directory lists. */
export function fetchDirectoryLists(signal?: AbortSignal): Promise<{ items: DirectoryList[] }> {
  return apiFetch<{ items: DirectoryList[] }>('/directory/lists', { signal });
}

/** Fetch entries in a directory list (public, ranked). */
export function fetchDirectoryEntries(
  params: {
    list?: string;
    category?: string;
    q?: string;
    sort?: 'top' | 'new';
    limit?: number;
    offset?: number;
  },
  signal?: AbortSignal,
): Promise<{ items: DirectoryEntry[] }> {
  const search = new URLSearchParams();
  if (params.list) search.set('list', params.list);
  if (params.category) search.set('category', params.category);
  if (params.q) search.set('q', params.q);
  if (params.sort) search.set('sort', params.sort);
  if (params.limit) search.set('limit', String(params.limit));
  if (params.offset) search.set('offset', String(params.offset));
  const query = search.toString();
  return apiFetch<{ items: DirectoryEntry[] }>(`/directory/entries${query ? `?${query}` : ''}`, {
    signal,
  });
}

/** Fetch operator-configured categories. */
export function fetchDirectoryCategories(
  signal?: AbortSignal,
): Promise<{ items: DirectoryCategory[] }> {
  return apiFetch<{ items: DirectoryCategory[] }>('/directory/categories', { signal });
}

// --- Category governance (§45) --------------------------------------------

export interface GovernanceCategory {
  slug: string;
  label: string;
  state: 'active' | 'deprecated' | 'merged';
  source: 'seed' | 'config' | 'community';
  merged_into: string | null;
  open_proposals: number;
}

export interface GovernanceState {
  frozen: boolean;
  max_active_categories: number;
  items: GovernanceCategory[];
}

export interface Proposal {
  id: string;
  category_slug: string;
  action: string;
  payload: Record<string, unknown>;
  status: string;
  yes_votes: number;
  no_votes: number;
  quorum_needed: number;
  closes_at: string;
  created_by: string;
  created_at: string;
  decided_by: string | null;
  decision_reason: string | null;
  decided_at: string | null;
}

export interface ChangelogEntry {
  id: string;
  category_slug: string;
  event: string;
  actor: string;
  document: string;
  created_at: string;
}

/** Fetch governance state for all categories. */
export function fetchGovernanceState(
  signal?: AbortSignal,
): Promise<GovernanceState> {
  return apiFetch<GovernanceState>('/directory/categories/governance', { signal });
}

/** Create a category proposal (rename/merge/deprecate/create). */
export function createCategoryProposal(body: {
  category_slug: string;
  action: string;
  payload: Record<string, unknown>;
}): Promise<{ id: string; status: string; quorum_needed: number; closes_at: string }> {
  return apiFetch('/directory/categories/governance/proposals', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

/** Get a single proposal. */
export function fetchProposal(
  proposalId: string,
  signal?: AbortSignal,
): Promise<Proposal> {
  return apiFetch<Proposal>(
    `/directory/categories/governance/proposals/${encodeURIComponent(proposalId)}`,
    { signal },
  );
}

/** Vote on a proposal. */
export function voteProposal(
  proposalId: string,
  value: 'yes' | 'no',
): Promise<{ status: string; passed: boolean | null }> {
  return apiFetch(
    `/directory/categories/governance/proposals/${encodeURIComponent(proposalId)}/vote`,
    {
      method: 'POST',
      body: JSON.stringify({ value }),
    },
  );
}

/** Veto a proposal (operator only). */
export function vetoProposal(
  proposalId: string,
  reason: string,
): Promise<{ status: string }> {
  return apiFetch(
    `/directory/categories/governance/proposals/${encodeURIComponent(proposalId)}/veto`,
    {
      method: 'POST',
      body: JSON.stringify({ reason }),
    },
  );
}

/** List changelog entries for a category. */
export function fetchChangelog(
  slug: string,
  signal?: AbortSignal,
): Promise<{ items: ChangelogEntry[] }> {
  return apiFetch<{ items: ChangelogEntry[] }>(
    `/directory/categories/governance/changelog/${encodeURIComponent(slug)}`,
    { signal },
  );
}

/** Toggle governance freeze (operator only). */
export function toggleFreeze(): Promise<{ frozen: boolean }> {
  return apiFetch('/directory/categories/governance/freeze', { method: 'POST' });
}

/** Propose entry moderation (move/remove). */
export function proposeEntryMod(
  entryId: string,
  action: 'move' | 'remove',
  targetCategory?: string,
): Promise<{ id: string; status: string; quorum_needed: number; closes_at: string }> {
  return apiFetch(
    `/directory/entries/${encodeURIComponent(entryId)}/moderation`,
    {
      method: 'POST',
      body: JSON.stringify({
        action,
        target_category: targetCategory,
      }),
    },
  );
}

/** Vote on entry moderation. */
export function voteEntryMod(
  entryId: string,
  value: 'yes' | 'no',
): Promise<{ status: string; passed: boolean | null }> {
  return apiFetch(
    `/directory/entries/${encodeURIComponent(entryId)}/moderation/vote?value=${value}`,
    { method: 'POST' },
  );
}

/** Submit a directory entry (signed-in users). */
export function submitDirectoryEntry(body: {
  list: string;
  kind: 'external' | 'internal';
  category: string;
  title: string;
  url?: string;
  description?: string;
  ref_id?: string;
  tags?: string[];
}): Promise<{ entry: DirectoryEntry }> {
  return apiFetch<{ entry: DirectoryEntry }>('/directory/entries', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

/**
 * How votes on a directory entry are ageing, if at all.
 *
 * `applies_to_this_entry` is per entry, not per instance: below the
 * activation threshold a vote is permanent however old it gets, and a voter
 * is entitled to know which case they are in before deciding whether coming
 * back is worth anything.
 */
export interface DirectoryVoteDecay {
  enabled: boolean;
  cutoff_days: number;
  applies_to_this_entry: boolean;
}

/** What a directory vote returns. */
export interface DirectoryVoteResult {
  score: number;
  my_vote: number | null;
  decay: DirectoryVoteDecay;
}

/**
 * Vote on a directory entry (signed-in users, one vote per account).
 *
 * `value` is `1` or `-1` to vote in a direction, or `0` to withdraw. Voting
 * the same direction twice *refreshes* the vote — it does not undo it — so
 * withdrawing is a distinct, explicit act.
 */
export function voteDirectoryEntry(
  entryId: string,
  value: 1 | 0 | -1,
): Promise<DirectoryVoteResult> {
  return apiFetch<DirectoryVoteResult>(
    `/directory/entries/${encodeURIComponent(entryId)}/vote`,
    {
      method: 'POST',
      body: JSON.stringify({ value }),
    },
  );
}

/** Fetch the operator moderation queue. */
export function fetchModerationQueue(
  signal?: AbortSignal,
): Promise<{ items: DirectoryEntry[] }> {
  return apiFetch<{ items: DirectoryEntry[] }>('/directory/moderation', { signal });
}

/** Approve a pending directory entry (operator only). */
export function approveDirectoryEntry(entryId: string): Promise<{ entry: DirectoryEntry }> {
  return apiFetch<{ entry: DirectoryEntry }>(
    `/directory/entries/${encodeURIComponent(entryId)}/approve`,
    { method: 'POST' },
  );
}

/** Remove a directory entry (operator only). */
export function removeDirectoryEntry(entryId: string): Promise<{ removed: boolean }> {
  return apiFetch<{ removed: boolean }>(`/directory/entries/${encodeURIComponent(entryId)}`, {
    method: 'DELETE',
  });
}

/** Create a new directory list (operator only). */
export function createDirectoryList(body: {
  slug: string;
  title: string;
  description?: string;
  kind: 'external' | 'internal';
}): Promise<{ list: DirectoryList }> {
  return apiFetch<{ list: DirectoryList }>('/directory/lists', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

// ---------------------------------------------------------------------------
// Admin media health dashboard (spec §32.7.11)
// ---------------------------------------------------------------------------

export interface MediaHealthOverview {
  total_references: number;
  well_mirrored: number;
  below_threshold: number;
  health_pct: number;
  min_healthy_threshold: number;
  recent_rescues_7d: number;
  one_week_ago: string;
}

export interface LinkRotReport {
  since: string;
  total_rot: number;
  by_provider: Array<{ provider: string; dead: number }>;
}

export interface CuratorLeader {
  account_id: string;
  total_rewards: number;
  actions: number;
}

export interface BountyStatus {
  active_bounties: number;
  total_available: number;
}

export interface StorageStatus {
  local_mirrors: number;
  total_bytes: number;
  ipfs_pins: number;
}

export interface ProviderReliability {
  provider: string;
  healthy: number;
  total: number;
  health_rate: number;
}

export function fetchMediaHealthOverview(): Promise<MediaHealthOverview> {
  return apiFetch<MediaHealthOverview>('/admin/media-health/overview');
}

export function fetchLinkRotReport(since?: string): Promise<LinkRotReport> {
  const qs = since ? `?since=${encodeURIComponent(since)}` : '';
  return apiFetch<LinkRotReport>(`/admin/media-health/link-rot${qs}`);
}

export function fetchCuratorLeaderboard(limit = 10): Promise<{ curators: CuratorLeader[] }> {
  return apiFetch<{ curators: CuratorLeader[] }>(`/admin/media-health/curator-leaderboard?limit=${limit}`);
}

export function fetchBountyStatus(): Promise<BountyStatus> {
  return apiFetch<BountyStatus>('/admin/media-health/bounty-status');
}

export function fetchStorageStatus(): Promise<StorageStatus> {
  return apiFetch<StorageStatus>('/admin/media-health/storage');
}

export function fetchProviderReliability(): Promise<{ providers: ProviderReliability[] }> {
  return apiFetch<{ providers: ProviderReliability[] }>('/admin/media-health/providers');
}

// ---------------------------------------------------------------------------
// Reader media references (spec §32.7.7)
// ---------------------------------------------------------------------------

export interface WorkMediaReferenceView {
  id: string;
  work_id: string;
  chapter_id: string | null;
  context: string;
  display_url: string;
  author_note: string | null;
  inserted_at: string;
  healthy_links: number;
  total_links: number;
  best_url: string | null;
}

export function fetchWorkMediaReferences(workId: string): Promise<{ items: WorkMediaReferenceView[] }> {
  return apiFetch<{ items: WorkMediaReferenceView[] }>(`/works/${encodeURIComponent(workId)}/media`);
}

export function reportBrokenLink(referenceId: string, reason?: string): Promise<{ status: string }> {
  return apiFetch<{ status: string }>(
    `/media/references/${encodeURIComponent(referenceId)}/report-broken`,
    { method: 'POST', body: JSON.stringify({ reason }) },
  );
}

// ---------------------------------------------------------------------------
// Author media (spec §32.7.8)
// ---------------------------------------------------------------------------

export interface AuthorMediaHealthRow {
  work_id: string;
  work_title: string;
  total_references: number;
  healthy_references: number;
  at_risk_references: number;
  broken_references: number;
}

export interface MediaPreferences {
  account_id: string;
  auto_submit_to_archive: boolean;
  prefer_curator_verified: boolean;
  broken_link_notifications: string;
  allow_curator_edits: boolean;
  minimum_healthy_links: number;
}

export interface AddMediaReferenceBody {
  work_id: string;
  url: string;
  context?: string;
  chapter_id?: string;
  author_note?: string;
}

export interface PostTargetedBountyBody {
  work_id: string;
  chapter_id?: string;
  media_reference_id?: string;
  reward: number;
  description?: string;
}

export function fetchAuthorMediaHealth(): Promise<{ items: AuthorMediaHealthRow[] }> {
  return apiFetch<{ items: AuthorMediaHealthRow[] }>('/author/media-health');
}

export function fetchMediaPreferences(): Promise<MediaPreferences> {
  return apiFetch<MediaPreferences>('/author/media-preferences');
}

export function putMediaPreferences(body: Partial<MediaPreferences>): Promise<{ status: string }> {
  return apiFetch<{ status: string }>('/author/media-preferences', {
    method: 'PUT',
    body: JSON.stringify(body),
  });
}

export function postMediaReference(body: AddMediaReferenceBody): Promise<{ id: string; link_id: string; status: string }> {
  return apiFetch<{ id: string; link_id: string; status: string }>(
    `/works/${encodeURIComponent(body.work_id)}/media`,
    { method: 'POST', body: JSON.stringify(body) },
  );
}

export function postTargetedBounty(body: PostTargetedBountyBody): Promise<{ id: string }> {
  return apiFetch<{ id: string }>('/author/targeted-bounties', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

// ---------------------------------------------------------------------------
// Reverse media search (spec §32.7.3)
// ---------------------------------------------------------------------------

export interface ReverseSearchBody {
  hash?: string;
  url?: string;
  /**
   * Which fingerprint the hash was computed with. Informational: the server
   * reports an uncomputable comparison as no match rather than scoring it, so a
   * wrong algorithm surfaces as no results instead of a wrong confident one.
   */
  algorithm?: string;
}

export interface ReverseSearchReference {
  id: string;
  media_kind: string;
  perceptual_hash: string | null;
  content_hash: string;
  curator_verified: boolean;
  /** "exact" when the content hash matched too, otherwise a perceptual match. */
  match_kind: 'exact' | 'perceptual';
  /**
   * Bits of difference between the queried perceptual hash and this one.
   * Null when the two hashes were not comparable - a missing or non-hex hash
   * is not a similarity score, so the server omits both this and the
   * confidence rather than reporting a large distance that reads as a verdict.
   */
  match_distance: number | null;
  /**
   * Hash similarity in 0..1, descending with match_distance. This measures the
   * fingerprints, not the artworks: it says how alike two hashes are, not that
   * two different pictures are the same image. A curator confirms the linkage.
   */
  match_confidence: number | null;
  /** True only for a distance-0 match, the one case that may attach silently. */
  auto_attach: boolean;
}

export interface ReverseSearchWork {
  reference_id: string;
  work_id: string;
  work_title: string;
  display_url: string | null;
}

export interface ReverseSearchView {
  references: ReverseSearchReference[];
  works: ReverseSearchWork[];
}

export function reverseMediaSearch(body: ReverseSearchBody): Promise<ReverseSearchView> {
  return apiFetch<ReverseSearchView>('/media/reverse-search', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

// ---------------------------------------------------------------------------
// Local mirror management (spec §32.7.6)
// ---------------------------------------------------------------------------

export interface LocalMirrorView {
  id: string;
  media_reference_id: string;
  storage_path: string;
  original_url: string;
  file_size_bytes: number | null;
  checksum_sha256: string | null;
  status: string;
  last_verified_at: string | null;
  created_at: string;
}

export interface AddLocalMirrorBody {
  media_reference_id: string;
  storage_path: string;
  original_url: string;
  file_size_bytes?: number;
  checksum_sha256?: string;
  content_type?: string;
}

export function fetchLocalMirrors(referenceId: string): Promise<LocalMirrorView[]> {
  return apiFetch<LocalMirrorView[]>(`/media/references/${encodeURIComponent(referenceId)}/mirrors`);
}

export function addLocalMirror(body: AddLocalMirrorBody): Promise<{ id: string }> {
  return apiFetch<{ id: string }>('/admin/local-mirrors', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

export function deactivateLocalMirror(mirrorId: string): Promise<{ status: string }> {
  return apiFetch<{ status: string }>(`/admin/local-mirrors/${encodeURIComponent(mirrorId)}`, {
    method: 'DELETE',
  });
}

// ---------------------------------------------------------------------------
// IPFS pin management (spec §32.7.6)
// ---------------------------------------------------------------------------

export interface IpfsPinView {
  id: string;
  media_reference_id: string;
  cid: string;
  pin_service: string;
  status: string;
  file_size_bytes: number;
  pinned_at: string;
}

export interface AddIpfsPinBody {
  media_reference_id: string;
  cid: string;
  pin_service: string;
  file_size_bytes: number;
}

export function fetchIpfsPins(referenceId: string): Promise<IpfsPinView[]> {
  return apiFetch<IpfsPinView[]>(`/media/references/${encodeURIComponent(referenceId)}/ipfs-pins`);
}

export function addIpfsPin(body: AddIpfsPinBody): Promise<{ id: string }> {
  return apiFetch<{ id: string }>(`/media/references/${encodeURIComponent(body.media_reference_id)}/ipfs-pins`, {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

// ---------------------------------------------------------------------------
// M45 — Roadmap consensus (spec §44, ADR 0023)
// ---------------------------------------------------------------------------

export interface RoadmapCard {
  id: string;
  title: string;
  /**
   * §44.1: what the feature is and why it exists, on the order of a page.
   *
   * The board LISTS the title only; this is the prose behind it, carried on
   * the board payload so opening a card costs no second request, and shown on
   * the detail view. Never null — the column is `NOT NULL DEFAULT ''` — so an
   * empty string is a real value meaning "no description yet", not a missing
   * field. Render a placeholder for it rather than a blank region.
   */
  body: string;
  category: string;
  stage: string;
  elo_rating: number;
  matches_played: number;
  times_best: number;
  times_worst: number;
  created_at: string;
  updated_at: string;
}

export interface RoadmapBoard {
  board: Record<string, RoadmapCard[]>;
  total_cards: number;
}

export interface ArenaBallot {
  ballot_id: string;
  cards: RoadmapCard[];
}

export interface RoadmapMove {
  id: string;
  card_id: string;
  card_title: string;
  from_stage: string;
  to_stage: string;
  reason: string | null;
  actor_account_id: string;
  created_at: string;
}

export interface RoadmapChangelog {
  moves: RoadmapMove[];
}

export interface ArenaVoteBody {
  ballot_id: string;
  best_id: string;
  worst_id: string;
}

export interface SuggestBody {
  title: string;
  category?: string;
  /**
   * An optional description of what is being suggested. Bounded at 8,000
   * characters server-side (§44.1) and refused rather than truncated.
   */
  body?: string;
}

export interface RoadmapCardDetail {
  card: RoadmapCard;
}

export function fetchRoadmapBoard(signal?: AbortSignal): Promise<RoadmapBoard> {
  return apiFetch<RoadmapBoard>('/roadmap', { signal });
}

/**
 * One card in full, with its body. Public — no session, same as the board.
 *
 * `encodeURIComponent` is not decoration. Card ids are uuids today, but the
 * suggest endpoint and any future seeder choose them, and an unescaped
 * interpolation into a path is an injection surface the moment an id contains
 * a slash or a `?`.
 */
export function fetchRoadmapCard(
  cardId: string,
  signal?: AbortSignal,
): Promise<RoadmapCardDetail> {
  return apiFetch<RoadmapCardDetail>(`/roadmap/cards/${encodeURIComponent(cardId)}`, {
    signal,
  });
}

export function fetchArenaBallot(): Promise<ArenaBallot> {
  return apiFetch<ArenaBallot>('/roadmap/arena');
}

export function submitRoadmapVote(body: ArenaVoteBody): Promise<{ status: string }> {
  return apiFetch<{ status: string }>('/roadmap/arena', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

export function suggestFeature(body: SuggestBody): Promise<{ id: string; status: string }> {
  return apiFetch<{ id: string; status: string }>('/roadmap/suggest', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

export function fetchRoadmapChangelog(signal?: AbortSignal): Promise<RoadmapChangelog> {
  return apiFetch<RoadmapChangelog>('/roadmap/changelog', { signal });
}

export function moveRoadmapCard(body: { card_id: string; stage: string; reason?: string }): Promise<{ status: string }> {
  return apiFetch<{ status: string }>('/admin/roadmap/move', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

// ---------------------------------------------------------------------------
// M47 — User Configuration (spec §46)
// ---------------------------------------------------------------------------

export interface SettingSchema {
  key: string;
  summary: string;
}

export interface ResolvedSetting {
  key: string;
  value: unknown;
  source: string;
  summary: string;
}

export interface SearchSettingsView {
  pseud_id: string;
  settings: ResolvedSetting[];
  schema: SettingSchema[];
}

export interface ContentFilterView {
  filter_type: string;
  value: string;
}

export interface ContentFilterListView {
  pseud_id: string;
  filters: ContentFilterView[];
}

export interface NotificationRouteView {
  event_type: string;
  channel: string;
  enabled: boolean;
}

export interface NotificationRoutesView {
  account_id: string;
  routes: NotificationRouteView[];
}

export interface SettingsExport {
  version: string;
  namespaces: Record<string, ResolvedSetting[]>;
}

export interface ImportReport {
  accepted: string[];
  rejected: { key: string; reason: string }[];
}

/**
 * How the reader's recorded engine choice resolved (spec §16.1b).
 *
 * `unavailable` is the case that matters: the reader chose an engine the
 * operator has since disabled. They are told which choice stopped applying
 * and what is in effect instead, rather than being handed another engine's
 * results with no explanation. `engine` and `using` are only present on the
 * variants that report a substitution.
 */
export type RecEngineChoice =
  | { state: 'instance_default'; using: string[] }
  | { state: 'honored'; engine: string }
  | { state: 'unavailable'; engine: string; using: string[] };

export interface RecEngineView {
  pseud_id: string;
  /** The stored choice, or null when the reader has never chosen. */
  engine: string | null;
  choice: RecEngineChoice;
  /** The operator's enabled set — the only things that can be picked. */
  available: string[];
}

export function fetchRecEngine(signal?: AbortSignal): Promise<RecEngineView> {
  return apiFetch<RecEngineView>('/settings/recommendations', { signal });
}

/**
 * Set or clear the reader's engine choice. `''` clears back to the instance
 * default. Rejects with the accepted values on 422, so callers should show the
 * message rather than a generic failure.
 */
export function patchRecEngine(engine: string): Promise<RecEngineView> {
  return apiFetch<RecEngineView>('/settings/recommendations', {
    method: 'PATCH',
    body: JSON.stringify({ engine }),
  });
}

export function fetchSearchSettings(signal?: AbortSignal): Promise<SearchSettingsView> {
  return apiFetch<SearchSettingsView>('/settings/search', { signal });
}

export function patchSearchSettings(changes: { key: string; value: unknown }[]): Promise<SearchSettingsView> {
  return apiFetch<SearchSettingsView>('/settings/search', {
    method: 'PATCH',
    body: JSON.stringify({ changes }),
  });
}

export function deleteSearchSetting(key: string): Promise<{ removed: boolean; key: string }> {
  return apiFetch<{ removed: boolean; key: string }>(`/settings/search/${key}`, {
    method: 'DELETE',
  });
}

export function fetchContentFilters(signal?: AbortSignal): Promise<ContentFilterListView> {
  return apiFetch<ContentFilterListView>('/settings/content-filters', { signal });
}

export function addContentFilter(filter_type: string, value: string): Promise<ContentFilterView> {
  return apiFetch<ContentFilterView>('/settings/content-filters', {
    method: 'POST',
    body: JSON.stringify({ filter_type, value }),
  });
}

export function deleteContentFilter(filter_type: string, value: string): Promise<{ removed: boolean }> {
  return apiFetch<{ removed: boolean }>(`/settings/content-filters/${filter_type}/${value}`, {
    method: 'DELETE',
  });
}

export function fetchNotificationRoutes(signal?: AbortSignal): Promise<NotificationRoutesView> {
  return apiFetch<NotificationRoutesView>('/settings/notifications', { signal });
}

export function patchNotificationRoutes(changes: { event_type: string; channel: string; enabled: boolean }[]): Promise<NotificationRoutesView> {
  return apiFetch<NotificationRoutesView>('/settings/notifications', {
    method: 'PATCH',
    body: JSON.stringify({ changes }),
  });
}

export function deleteNotificationRoute(event_type: string): Promise<{ removed: boolean }> {
  return apiFetch<{ removed: boolean }>(`/settings/notifications/${event_type}`, {
    method: 'DELETE',
  });
}

export function exportSettings(signal?: AbortSignal): Promise<SettingsExport> {
  return apiFetch<SettingsExport>('/settings/export', { signal });
}

export function importSettings(data: SettingsExport): Promise<ImportReport> {
  return apiFetch<ImportReport>('/settings/import', {
    method: 'POST',
    body: JSON.stringify({ data }),
  });
}

// --- Analytics (docs/spec-amendments/trust-gated-analytics.md) ------------
//
// The server decides what a reader may see and sends the list. The client
// holds no ladder, no floor and no capability list of its own, so there is
// nothing here that could disagree with the registry.

/** §24.2's method block, as the registry renders it. */
export interface AnalyticsMeta {
  name: string;
  definition: string;
  freshness: string;
  approximation: string;
  minimum_trust_level: number;
  subject: 'self' | 'other';
  /** The k-anonymity floor, or null for a single-entity fact. */
  floor: number | null;
}

/**
 * A count, exactly as the server reports it.
 *
 * `count` is absent rather than zero when the true number is below the floor,
 * and that distinction is the whole point: zero says "nobody did this", and
 * for a new work the true statement is "too few people to tell you". So the
 * type has no `count: number | undefined` to accidentally `?? 0`.
 */
export interface AnalyticsCount {
  count?: number;
  fewer_than?: number;
}

/**
 * §9.6's own reading totals, as `own.reading.basic` answers.
 *
 * These are the first capability with named fields rather than one count, and
 * the difference is not cosmetic: a band is for a count of *other people*, and
 * these are facts about the reader asking. A zero here is a true zero, so the
 * fields are plain numbers and never optional -- `number | undefined` on
 * `finished_works` would be a `?? 0` waiting to happen, and `?? 0` on a
 * suppressed count is how "we hid this" becomes "nobody did this".
 */
export interface ReadingTotals {
  finished_works: number;
  chapters_read: number;
  words_read: number;
  /** An estimate: capped wall-clock between progress updates. */
  reading_seconds: number;
}

export interface AnalyticsValue extends AnalyticsCount {
  status?: string;
  note?: string;
  /** Present when the capability answers with named fields. */
  reading?: ReadingTotals;
}

export interface AnalyticsList {
  viewer: { trust_level: number; role: string; preset: string };
  capabilities: AnalyticsMeta[];
}

export interface AnalyticsDetail {
  capability: string;
  implemented: boolean;
  meta: AnalyticsMeta;
  value: AnalyticsValue;
}

/** What this reader may see. The client renders this list and nothing else. */
export async function fetchAnalytics(signal?: AbortSignal): Promise<AnalyticsList> {
  return apiFetch<AnalyticsList>('/me/analytics', { signal });
}

/** One capability, already authorised server-side. */
export async function fetchCapability(
  name: string,
  signal?: AbortSignal,
): Promise<AnalyticsDetail> {
  return apiFetch<AnalyticsDetail>(`/me/analytics/${encodeURIComponent(name)}`, { signal });
}

// ---------------------------------------------------------------------------
// API tokens (spec §23.1, §46.5)
// ---------------------------------------------------------------------------

/**
 * One API token, as `GET /me/tokens` describes it.
 *
 * Note there is no `token` field: the raw value is returned exactly once, by
 * the issue call, and never again. Everything here is metadata about a
 * credential the server will not repeat. `scopes` arrives as the raw stored
 * string rather than an array — see {@link parseTokenScopes} — and
 * `acting_pseud_id` is null for a token that was issued without one, which
 * this build refuses to act with.
 */
export interface ApiToken {
  id: string;
  account_id: string;
  name: string;
  /** `user` for one a person made, `bot` for one bound to a bot. */
  kind: string;
  /** A space-separated list, as stored. */
  scopes: string;
  acting_pseud_id: string | null;
  created_at: string;
  last_used_at: string | null;
  expires_at: string | null;
  revoked_at: string | null;
}

/** The scopes this build knows, in the order the settings surface shows them. */
export const KNOWN_TOKEN_SCOPES = [
  'content.read',
  'content.write',
  'library.read',
  'moderation.write',
  'identity.read',
] as const;

export type KnownTokenScope = (typeof KNOWN_TOKEN_SCOPES)[number];

/**
 * Split a stored `scopes` string into a list.
 *
 * The server stores scopes space-separated and validates every one against the
 * known set at issue time, so an unrecognised entry cannot reach here from the
 * issue route — but it can reach here from a row written before a scope was
 * removed, or by hand. It is shown verbatim rather than dropped: a scope the
 * reader cannot see is a scope they cannot reason about, and a token list that
 * quietly hides one is worse than one that shows something odd.
 */
export function parseTokenScopes(raw: string | null | undefined): string[] {
  if (!raw) return [];
  return raw
    .split(/\s+/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

/**
 * Whether a scope is one this build recognises.
 *
 * Used by the create form to warn, not to refuse — see the note on
 * {@link parseTokenScopes}.
 */
export function isKnownTokenScope(scope: string): scope is KnownTokenScope {
  return (KNOWN_TOKEN_SCOPES as readonly string[]).includes(scope);
}

/** List the caller's live tokens. Revoked ones are filtered server-side. */
export async function fetchTokens(signal?: AbortSignal): Promise<ApiToken[]> {
  const body = await apiFetch<{ tokens: ApiToken[] }>('/me/tokens', { signal });
  return body.tokens;
}

/**
 * Issue a token. The raw value is in the response and nowhere else, ever.
 *
 * `acting_pseud_id` is deliberately not a parameter. §23.1's tokens act as an
 * explicit pseud, and this surface is for tokens a *person* made for their own
 * use; the bot flow mints its tokens with an acting pseud through the link
 * handshake, which is a different door. A reader who could pick an arbitrary
 * pseud here could post as any face on the instance.
 */
export async function createToken(
  name: string,
  scopes: string[],
): Promise<{ token: string; id: string }> {
  return apiFetch<{ token: string; id: string }>('/me/tokens', {
    method: 'POST',
    body: JSON.stringify({ name, scopes: scopes.join(' ') }),
  });
}

/** Revoke a token. After this the raw value stops working immediately. */
export async function revokeToken(id: string): Promise<void> {
  await apiFetch(`/me/tokens/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

// ---------------------------------------------------------------------------
// §53 — the operator's faucet/sink view.
// ---------------------------------------------------------------------------

/** One mechanism as the dashboard renders it. */
export interface FlowMechanism {
  key: string;
  /** 'faucet' | 'sink' | 'neutral' | 'undeclared' -- `Flow` in the domain. */
  flow: string;
  net_credits: number;
  /**
   * False when nobody has declared which side this mechanism is on.
   *
   * The field exists so the component can warn rather than silently render. An
   * undeclared mechanism's credits are still in `net_credits`; dropping the row
   * would understate the economy, which is the failure §53.1 forbids.
   */
  declared: boolean;
}

export interface EconomyFlows {
  since: string;
  until: string;
  faucet_credits: number;
  sink_credits: number;
  net_credits: number;
  undeclared: number;
  threshold: number;
  over_threshold: boolean;
  mechanisms: FlowMechanism[];
  note: string;
}

/**
 * Read the economy as faucets and sinks.
 *
 * The window is optional: the server defaults to the last 30 days rather than all
 * time, so an operator asking for "now" gets a figure that means the same thing on
 * every day.
 */
export function fetchEconomyFlows(params: { since?: string; until?: string } = {}): Promise<EconomyFlows> {
  const qs = new URLSearchParams();
  if (params.since) qs.set('since', params.since);
  if (params.until) qs.set('until', params.until);
  const suffix = qs.toString() ? `?${qs}` : '';
  return apiFetch<EconomyFlows>(`/admin/economy/flows${suffix}`);
}

// -------------------------------------------------------------------------------------
// Reader surface — items 14, 27, 33 of the 100-idea audit (spec §57.7 and the audit)
// -------------------------------------------------------------------------------------

/**
 * One work as the reader-surface sections receive it.
 *
 * `recent_bookmarks` and `similarity` are OPTIONAL and both are omitted by the server
 * when they do not apply: `recent_bookmarks` only on the leaderboard, `similarity` only
 * on the rail. They are optional here for the same reason — a client that renders
 * `0` for a missing `similarity` is showing a score of zero for a work the server never
 * scored, which reads as "no similarity found" instead of "not scored".
 */
export interface SurfaceWork {
  id: string;
  title: string;
  summary: string;
  completion: string;
  published_at?: string | null;
  /**
   * Item 4: total words across the work's CURRENT chapter revisions.
   *
   * Required, not optional, because the server COALESCEs the aggregate to 0: a work with
   * no chapters is 0 words, which is a true answer. Making it optional here would put
   * `undefined` in front of every caller for a work that simply has no prose yet, and a
   * card that hides the count for "absent" would then also hide it for 0.
   */
  word_count: number;
  /** Item 27: distinct PUBLIC bookmarkers inside the window. */
  recent_bookmarks?: number;
  /** Item 33: weighted-Jaccard score, always within [0,1] when present. */
  similarity?: number;
}

/**
 * Fetch the newest works in the fandoms this reader has PUBLICLY bookmarked (item 14).
 *
 * REQUIRES a session — it answers "new to you", so an anonymous call is refused by the
 * server rather than answered with everyone else's.
 *
 * Resolves to `[]` when the reader has no public bookmarks. It does NOT fall back to
 * all recent works, and a caller must not add that fallback either: a section that
 * changes subject when it has no data is a section nobody can learn to read.
 */
export function fetchNewInYourFandoms(signal?: AbortSignal): Promise<SurfaceWork[]> {
  return apiFetch<{ works: SurfaceWork[] }>('/discovery/new-in-your-fandoms', { signal }).then(
    (page) => page.works,
  );
}

/**
 * The public weekly bookmark leaderboard (item 27).
 *
 * No session needed. Counts distinct PUBLIC bookmarkers only — the server-side
 * `is_public` predicate is the privacy rule and it is why this door is public.
 *
 * `windowDays` defaults to the server's seven. A value outside 1..365 is refused with
 * a 400 rather than clamped, because a clamped window returns a leaderboard that is not
 * the one the caller asked about.
 */
export function fetchMostBookmarked(windowDays?: number, signal?: AbortSignal): Promise<SurfaceWork[]> {
  const qs = windowDays === undefined ? '' : `?window_days=${encodeURIComponent(windowDays)}`;
  return apiFetch<{ works: SurfaceWork[] }>(`/discovery/most-bookmarked${qs}`, { signal }).then(
    (page) => page.works,
  );
}

/**
 * Works most similar to this one (item 33), best first, with the score that ordered them.
 *
 * No session needed. Resolves to `[]` when the work has fewer than two tags or nothing
 * clears the server's honesty floor — and `[]` is the correct answer, not a failure. An
 * empty rail headed "Similar works" is worse than no rail.
 */
export function fetchSimilarWorks(workId: string, signal?: AbortSignal): Promise<SurfaceWork[]> {
  return apiFetch<{ works: SurfaceWork[] }>(
    `/works/${encodeURIComponent(workId)}/similar`,
    { signal },
  ).then((page) => page.works);
}

// ---------------------------------------------------------------------------
// DNF — item 11 of the 100-idea audit
//
// The whole server side shipped in M45-21 (0073_dnf_reasons.sql, crates/db/src/dnf.rs,
// three routes, crates/db/tests/m45_21_dnf.rs) and nothing in the frontend referenced
// it. These are the first callers.
//
// `isPublic` is the reader's own choice and defaults to FALSE, because a DNF mark is a
// reader's private disposition toward a work and 0073 made that the default for a
// reason. `note` is the private free text; the structured `reason` is the part that can
// ever be aggregated, and only when the author has set `allow_dnf_feedback`.
export type DnfReason =
  | 'not_my_taste'
  | 'triggering'
  | 'slow_pacing'
  | 'abandoned_by_author'
  | 'dropped_other'
  | 'other';

/**
 * The six reasons, with the labels `DnfReason::label()` in crates/domain/src/dnf.rs.
 *
 * Duplicated here because the server does not expose a reason catalogue and adding one
 * for six static strings would be a round trip to learn nothing. The labels are
 * asserted against the Rust source in `DnfPanel.test.ts`, so a rename on either side
 * turns a test red rather than silently changing what a reader sees.
 */
export const DNF_REASONS: ReadonlyArray<{ value: DnfReason; label: string; hint: string }> = [
  { value: 'not_my_taste', label: 'Not my taste', hint: 'The premise or the voice was not for you.' },
  { value: 'triggering', label: 'Triggering content', hint: 'Something in it hit too close.' },
  { value: 'slow_pacing', label: 'Too slow', hint: 'It lost you before it got going.' },
  {
    value: 'abandoned_by_author',
    label: 'Author abandoned it',
    hint: 'The author stopped updating.',
  },
  { value: 'dropped_other', label: 'Dropped for another reason', hint: 'None of the others fit.' },
  { value: 'other', label: 'Other', hint: 'Tell yourself why, in the note below.' },
];

export interface DnfRecord {
  id: string;
  account_id: string;
  pseud_id: string;
  work_id: string;
  reason: DnfReason;
  note: string | null;
  is_public: boolean;
  created_at: string;
  updated_at: string;
}

/** One row of the work's public aggregate, from `GET /works/{id}/dnf/reasons`. */
export interface DnfReasonCount {
  reason: DnfReason;
  count: number;
}

/**
 * Whether an error is a 404 from the server, as opposed to a transport failure or a
 * 5xx.
 *
 * `ApiError.status` is `0` for `REQUEST_ABORTED` and `NETWORK_UNAVAILABLE`, so a
 * truthiness check on the field is wrong: `if (err.status)` is true for 0's negation
 * being false only by accident, and `err.status && ...` reads as "has a status". Both
 * a `0` and a real `404` must be distinguishable, so this compares exactly.
 */
function isNotFound(err: unknown): boolean {
  return err instanceof ApiError && err.status === 404;
}

/**
 * The caller's own DNF record for a work, or `null` when they have not marked it.
 *
 * `null` is a real state here and not an error: the work page needs to distinguish
 * "this reader has not marked it" from "the request failed". So this resolves to
 * `null` on 404 rather than throwing, while every other status still throws — a reader
 * who sees "you have not marked this" when the server was down has been told a lie.
 */
export async function fetchMyDnf(workId: string, signal?: AbortSignal): Promise<DnfRecord | null> {
  try {
    return await apiFetch<DnfRecord>(`/works/${encodeURIComponent(workId)}/dnf`, { signal });
  } catch (err) {
    if (isNotFound(err)) return null;
    throw err;
  }
}

export function setMyDnf(
  workId: string,
  reason: DnfReason,
  options: { note?: string | null; isPublic?: boolean } = {},
): Promise<DnfRecord> {
  return apiFetch<DnfRecord>(`/works/${encodeURIComponent(workId)}/dnf`, {
    method: 'PUT',
    body: JSON.stringify({
      reason,
      note: options.note ?? null,
      is_public: options.isPublic ?? false,
    }),
  });
}

/**
 * One "Continue Reading" banner row: where this reader stopped.
 *
 * Both `positionPermille` and `percent` are present because they are not redundant. The
 * server owns the rounding and the clamping, and a caller that needs the raw value — "page
 * 12 of 340", or a slider — should not re-derive it from a number that has already lost
 * precision. Sending only the percentage would make the raw value unrecoverable.
 */
export interface ContinueReading {
  workId: string;
  title: string;
  /** 0..1000, unrounded. NOT clamped on the wire — see `percent`. */
  positionPermille: number;
  /** Whole percent, already clamped to 0..=100 by the server. */
  percent: number;
  chapterId: string | null;
  chapterTitle: string | null;
  /** RFC 3339. When this reader last wrote this row. */
  updatedAt: string;
}

/**
 * The reader's most recently touched unfinished work, or `null` when there is none.
 *
 * `null` is a real state, not an error: a reader who finished everything, or never opened
 * anything, should see no banner. So a 404 resolves to `null` rather than throwing, while
 * every other status still throws — the alternative is a reader being shown "nothing to
 * continue" because the server was down, which is a lie told by a timeout.
 *
 * A 401 is NOT swallowed. "You are not logged in" and "you have nothing to continue" both
 * mean "no banner", but only one of them should skip the request, and only the client knows
 * whether it has a session.
 */
export async function fetchContinueReading(signal?: AbortSignal): Promise<ContinueReading | null> {
  try {
    const raw = await apiFetch<ContinueReadingWire>("/continue-reading", { signal });
    return {
      workId: raw.work_id,
      title: raw.title,
      positionPermille: raw.position_permille,
      percent: raw.percent,
      chapterId: raw.chapter_id,
      chapterTitle: raw.chapter_title,
      updatedAt: raw.updated_at,
    };
  } catch (err) {
    if (isNotFound(err)) return null;
    throw err;
  }
}

/** The snake_case shape the route actually emits. Never leaves this file. */
interface ContinueReadingWire {
  work_id: string;
  title: string;
  position_permille: number;
  percent: number;
  chapter_id: string | null;
  chapter_title: string | null;
  updated_at: string;
}

export function clearMyDnf(workId: string): Promise<void> {
  return apiFetch<void>(`/works/${encodeURIComponent(workId)}/dnf`, { method: 'DELETE' });
}

/**
 * The work's PUBLIC aggregate. Empty when the author has not enabled DNF feedback or
 * nobody has marked it, and empty is the correct answer in both cases.
 */
export function fetchDnfReasons(
  workId: string,
  signal?: AbortSignal,
): Promise<DnfReasonCount[]> {
  return apiFetch<{ work_id: string; reasons: DnfReasonCount[] }>(
    `/works/${encodeURIComponent(workId)}/dnf/reasons`,
    { signal },
  ).then((page) => page.reasons);
}

/** Everything the caller's own pseud has marked. */
export function fetchMyDnfList(signal?: AbortSignal): Promise<DnfRecord[]> {
  return apiFetch<DnfRecord[]>('/me/dnf', { signal });
}
