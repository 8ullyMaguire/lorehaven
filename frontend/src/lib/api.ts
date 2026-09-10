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
