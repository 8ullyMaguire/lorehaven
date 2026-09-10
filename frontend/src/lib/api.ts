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
