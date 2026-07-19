// Fetch layer for the named-environment REST API
// (`/api/v1/users/me/environment-profiles`, backend `routes/environments.rs`).
// Like the canvas client, every function takes the caller's `apiFetch` (the
// token-bearing fetch from useAuth) as a dependency, so components stay testable
// with a plain stub and this module never imports auth state itself.

import { assertShape, readErrorMessage, type ApiFetch, type MutationResult } from './canvas';
import type {
  EnvironmentProfileSpec,
  EnvironmentProfileSummary,
  EnvironmentProfileView,
  InstallValidationError,
} from './types';

const BASE = '/api/v1/users/me/environment-profiles';

/** Result of `putEnvironmentProfile`. The PUT runs the install commands in an
 *  isolated pod, so a failure is not a single string: a `422` carrying the
 *  detailed install-validation report surfaces as `{ validation }` (verbatim,
 *  never a thrown string), while every OTHER non-2xx (a pre-validation `422`
 *  rendered as a plain envelope, `401/403/429/503`) surfaces as `{ message }`.
 *  Callers narrow the failure with `'validation' in result`. */
export type PutEnvironmentProfileResult =
  | { ok: true; data: EnvironmentProfileView }
  | { ok: false; validation: InstallValidationError }
  | { ok: false; message: string | null };

/** Parse a response body as JSON, or `null` when the body is not JSON. The PUT
 *  path must inspect the body itself (to tell an install-validation `422` from a
 *  plain envelope), so it cannot delegate the whole read to `readErrorMessage`. */
async function parseJsonBody(res: Response): Promise<unknown> {
  try {
    return await res.json();
  } catch {
    // Non-JSON error body — treated as "no structured detail".
    return null;
  }
}

/** True when `body` is the detailed install-validation `422` report rather than
 *  a plain `ErrorEnvelope` (both are HTTP 422). The fixed machine code
 *  `install_validation_failed` is the discriminator; the remaining fields are
 *  shape-checked so a malformed body falls through to the envelope path. */
function isInstallValidationError(body: unknown): body is InstallValidationError {
  if (typeof body !== 'object' || body === null) return false;
  const b = body as Record<string, unknown>;
  return (
    b.error === 'install_validation_failed' &&
    typeof b.message === 'string' &&
    typeof b.failed_command_index === 'number' &&
    typeof b.failed_command === 'string' &&
    typeof b.exit_code === 'number' &&
    typeof b.timed_out === 'boolean' &&
    typeof b.stderr_tail === 'string'
  );
}

/** Pull a `message` out of an already-parsed body, or null when absent. */
function envelopeMessage(body: unknown): string | null {
  if (typeof body === 'object' && body !== null) {
    const message = (body as { message?: unknown }).message;
    if (typeof message === 'string' && message) return message;
  }
  return null;
}

/** GET /api/v1/users/me/environment-profiles — the caller's environments as
 *  compact summaries. */
export async function listEnvironmentProfiles(
  apiFetch: ApiFetch
): Promise<EnvironmentProfileSummary[]> {
  const res = await apiFetch(BASE);
  if (!res.ok) throw new Error(`environment profiles failed: ${res.status}`);
  const body = (await res.json()) as { environment_profiles?: EnvironmentProfileSummary[] };
  assertShape(Array.isArray(body?.environment_profiles), 'environment profiles');
  return body.environment_profiles as EnvironmentProfileSummary[];
}

/** GET /api/v1/users/me/environment-profiles/{name} — one environment (secret
 *  values omitted; only `secret_keys`). */
export async function getEnvironmentProfile(
  apiFetch: ApiFetch,
  name: string
): Promise<EnvironmentProfileView> {
  const res = await apiFetch(`${BASE}/${encodeURIComponent(name)}`);
  if (!res.ok) throw new Error(`environment profile failed: ${res.status}`);
  const body = (await res.json()) as EnvironmentProfileView;
  assertShape(
    typeof body?.name === 'string' && Array.isArray(body?.secret_keys),
    'environment profile'
  );
  return body;
}

/** PUT /api/v1/users/me/environment-profiles/{name} — replace (or create) an
 *  environment. SLOW: the backend runs the install commands in a throwaway
 *  validation pod before persisting. On a `422` install-validation failure the
 *  detailed report is returned as a typed `{ validation }` result (never
 *  thrown); any other failure carries the envelope message. */
export async function putEnvironmentProfile(
  apiFetch: ApiFetch,
  name: string,
  spec: EnvironmentProfileSpec
): Promise<PutEnvironmentProfileResult> {
  const res = await apiFetch(`${BASE}/${encodeURIComponent(name)}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(spec),
  });
  if (res.ok) return { ok: true, data: (await res.json()) as EnvironmentProfileView };
  const body = await parseJsonBody(res);
  if (isInstallValidationError(body)) return { ok: false, validation: body };
  return { ok: false, message: envelopeMessage(body) };
}

/** DELETE /api/v1/users/me/environment-profiles/{name} — remove an environment.
 *  Idempotent on the backend (`204` whether or not it existed). */
export async function deleteEnvironmentProfile(
  apiFetch: ApiFetch,
  name: string
): Promise<MutationResult<null>> {
  const res = await apiFetch(`${BASE}/${encodeURIComponent(name)}`, { method: 'DELETE' });
  if (res.ok) return { ok: true, data: null };
  return { ok: false, message: await readErrorMessage(res) };
}
