// One typed failure for both operations endpoints.
//
// The UI never renders the backend's `message`: it renders a localized string
// keyed by the STABLE `error` code, because the message is written for an
// operator reading a log, may name internal machinery, and is not translated.
// The code is the contract; the message stays out of the DOM entirely.
//
// Two codes are minted client-side and are as load-bearing as the server's:
//
// - `scope_mismatch` — the page we asked for and the page we got disagree about
//   `effective_scope`. That is a security-shaped failure, not a display bug: the
//   only honest response is to render NO rows, because we cannot tell whose rows
//   these are.
// - `malformed` — a field the renderer dereferences is missing or the wrong
//   type. Failing loudly here beats a half-drawn table built on guesses.

/** Stable error codes the backend returns for these two routes, plus the two
 *  the client mints. Every one has a localized string in the catalog. */
export const OPERATIONS_ERROR_CODES = [
  'invalid_request',
  'invalid_activity_cursor',
  'unauthorized',
  'forbidden',
  'operations_scope_forbidden',
  'activity_session_not_found',
  'sandbox_not_found',
  'not_found',
  'rate_limited',
  'upstream_error',
  'unavailable',
  'session_visibility_unavailable',
  'audit_query_not_configured',
  'sandbox_inventory_disabled',
  'sandbox_inventory_unavailable',
  'sandbox_inventory_too_large',
  'internal',
  // client-minted
  'scope_mismatch',
  'malformed',
  'network',
] as const;

export type OperationsErrorCode = (typeof OPERATIONS_ERROR_CODES)[number];

/** Map an arbitrary envelope code onto the closed set, so an unrecognized code
 *  from a future backend still renders a stable localized failure. */
export function asErrorCode(value: unknown): OperationsErrorCode {
  return typeof value === 'string' &&
    (OPERATIONS_ERROR_CODES as readonly string[]).includes(value)
    ? (value as OperationsErrorCode)
    : 'internal';
}

/** A failed operations call. `status` is `0` for a transport failure. */
export class OperationsError extends Error {
  readonly code: OperationsErrorCode;
  readonly status: number;
  /** The propagated `X-Request-Id`, when the response exposed one. Kept so a
   *  user can quote it to support; never used for anything else. */
  readonly requestId: string | null;

  constructor(code: OperationsErrorCode, status: number, requestId: string | null = null) {
    // The Error message is a machine string for a stack trace, never UI copy.
    super(`operations request failed: ${code} (${status})`);
    this.name = 'OperationsError';
    this.code = code;
    this.status = status;
    this.requestId = requestId;
  }
}

/** True when this failure means the caller may not have the scope they asked
 *  for, so the page must drop every row and cursor and retry in the scope the
 *  server does allow. */
export function isScopeDenied(error: unknown): boolean {
  return error instanceof OperationsError && error.code === 'operations_scope_forbidden';
}

/** True when the viewer's session is gone. The page hands these to the shared
 *  sign-in/session-expired behavior rather than rendering a failure. */
export function isUnauthenticated(error: unknown): boolean {
  return error instanceof OperationsError && error.status === 401;
}

/** Reduce any thrown value to the pair the UI renders. A transport failure (an
 *  offline browser, a DNS error, a CORS refusal) produced no response at all, so
 *  it is `network` — distinct from every failure the server actually answered. */
export function describeError(error: unknown): {
  code: OperationsErrorCode;
  requestId: string | null;
} {
  if (error instanceof OperationsError) {
    return { code: error.code, requestId: error.requestId };
  }
  return { code: 'network', requestId: null };
}

/** Build the typed error for a non-2xx response, reading only the stable code.
 *  A body that is not the expected envelope degrades to a status-derived code —
 *  it never leaks whatever text was there. */
export async function operationsError(res: Response): Promise<OperationsError> {
  const requestId = res.headers.get('x-request-id');
  let code: OperationsErrorCode = fallbackCode(res.status);
  try {
    const envelope = (await res.json()) as { error?: unknown };
    if (typeof envelope?.error === 'string') {
      code = asErrorCode(envelope.error);
    }
  } catch {
    /* non-JSON error body — keep the status-derived code */
  }
  return new OperationsError(code, res.status, requestId);
}

/** The code to use when the body carried none. */
function fallbackCode(status: number): OperationsErrorCode {
  if (status === 400) return 'invalid_request';
  if (status === 401) return 'unauthorized';
  if (status === 403) return 'forbidden';
  if (status === 404) return 'not_found';
  if (status === 429) return 'rate_limited';
  if (status === 502) return 'upstream_error';
  if (status === 503) return 'unavailable';
  return 'internal';
}
