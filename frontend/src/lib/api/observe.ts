// Fetch layer for the live engine observe read-model. Like the other clients
// it takes the caller's `apiFetch` (the token-bearing fetch from useAuth) as a
// dependency, so components stay testable with a plain stub.
//
// The backend returns the engine's own JSON verbatim (spec B5), so this client
// never asserts a shape — it only guarantees an OBJECT comes back. Callers read
// the tolerant `ObserveSnapshot` and render whatever is present.

import type { ApiFetch } from './canvas';
import type { ObserveSnapshot } from './types';

/** Error carrying the HTTP status so a caller can tell 409 (no durable store to
 *  observe) from a transient failure without string-matching. */
export class ObserveError extends Error {
  readonly status: number;
  constructor(status: number) {
    super(`observe failed: ${status}`);
    this.name = 'ObserveError';
    this.status = status;
  }
}

/** GET /api/v1/sessions/{session_id}/observe — the engine read-model snapshot.
 *  This is a SLOW call: it execs into the session pod, so callers should show a
 *  spinner and a "may take up to a minute" note. A non-object body (or a
 *  primitive) degrades to an empty snapshot rather than throwing — the endpoint
 *  is documented as raw engine JSON and we never assume its shape. */
export async function getObserve(
  apiFetch: ApiFetch,
  sessionId: string
): Promise<ObserveSnapshot> {
  const res = await apiFetch(`/api/v1/sessions/${encodeURIComponent(sessionId)}/observe`);
  if (!res.ok) throw new ObserveError(res.status);
  const body = (await res.json()) as unknown;
  if (body == null || typeof body !== 'object' || Array.isArray(body)) return {};
  return body as ObserveSnapshot;
}
