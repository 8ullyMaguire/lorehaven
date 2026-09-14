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
  authors: PublicAuthor[];
  chapters: ChapterSummary[];
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
}

export interface DiscoveryFeed {
  items: DiscoveryItem[];
}

export async function fetchDiscoveryFeed(signal?: AbortSignal): Promise<DiscoveryFeed> {
  return apiFetch<DiscoveryFeed>('/discovery', { signal });
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
  title: string;
  description?: string;
  category?: string;
}

export interface Group {
  id: string;
  name: string;
  description?: string;
  visibility: string;
  member_count?: number;
}

export interface Conversation {
  id: string;
  other_handle: string;
  last_message?: string;
  updated_at?: string;
}

export interface Block {
  id: string;
  handle: string;
  created_at: string;
}

export async function fetchForums(signal?: AbortSignal): Promise<Forum[]> {
  return apiFetch<Forum[]>('/community/forums', { signal });
}

export async function fetchGroups(signal?: AbortSignal): Promise<Group[]> {
  return apiFetch<Group[]>('/community/groups', { signal });
}

export async function fetchConversations(signal?: AbortSignal): Promise<Conversation[]> {
  return apiFetch<Conversation[]>('/community/conversations', { signal });
}

export async function fetchBlocks(signal?: AbortSignal): Promise<Block[]> {
  return apiFetch<Block[]>('/community/blocks', { signal });
}
