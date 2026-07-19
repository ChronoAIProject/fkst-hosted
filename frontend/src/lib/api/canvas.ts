// Fetch layer for the canvas dashboard endpoints. Every function takes the
// caller's `apiFetch` (the token-bearing fetch from useAuth) as a dependency,
// so components stay testable with a plain stub and this module never imports
// auth state itself.

import type {
  CreateSessionRequest,
  CreateSessionResponse,
  OverviewResponse,
  RepoSessionsResponse,
} from './types';

/** The shape of `useAuth().apiFetch` — injected, never imported. */
export type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

/** Uniform result for mutations: the server's envelope message rides along on
 *  failure so callers can surface it verbatim. */
export type MutationResult<T> = { ok: true; data: T } | { ok: false; message: string | null };

/** Read the `message` out of an error envelope body, or null when the body is
 *  not the expected JSON shape (callers then fall back to a generic string). */
export async function readErrorMessage(res: Response): Promise<string | null> {
  try {
    const envelope = (await res.json()) as { message?: unknown };
    if (typeof envelope?.message === 'string' && envelope.message) return envelope.message;
  } catch {
    /* non-JSON error body */
  }
  return null;
}

/** Minimal boundary validation: the fields the UI dereferences must exist.
 *  A malformed payload fails loudly here instead of deep inside a component.
 *  Exported so the sibling clients (outcomes/logs) share one validation style. */
export function assertShape(cond: boolean, what: string): asserts cond {
  if (!cond) throw new Error(`malformed ${what} response`);
}

/** GET /api/v1/overview — the whole level-0/1 canvas in one call. */
export async function getOverview(apiFetch: ApiFetch): Promise<OverviewResponse> {
  const res = await apiFetch('/api/v1/overview');
  if (!res.ok) throw new Error(`overview failed: ${res.status}`);
  const body = (await res.json()) as OverviewResponse;
  assertShape(Array.isArray(body?.accounts), 'overview');
  assertShape(typeof body?.viewer?.login === 'string', 'overview');
  return body;
}

/** GET /api/v1/repos/{owner}/{name}/sessions — the level-2 detail. */
export async function getRepoSessions(
  apiFetch: ApiFetch,
  owner: string,
  name: string
): Promise<RepoSessionsResponse> {
  const res = await apiFetch(
    `/api/v1/repos/${encodeURIComponent(owner)}/${encodeURIComponent(name)}/sessions`
  );
  if (!res.ok) throw new Error(`repo sessions failed: ${res.status}`);
  const body = (await res.json()) as RepoSessionsResponse;
  assertShape(Array.isArray(body?.sessions), 'repo sessions');
  return body;
}

/** POST /api/v1/repos/{owner}/{name}/sessions — create a trigger issue.
 *  Validation errors (400) surface the parser's message verbatim. */
export async function createTrigger(
  apiFetch: ApiFetch,
  owner: string,
  name: string,
  request: CreateSessionRequest
): Promise<MutationResult<CreateSessionResponse>> {
  const res = await apiFetch(
    `/api/v1/repos/${encodeURIComponent(owner)}/${encodeURIComponent(name)}/sessions`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(request),
    }
  );
  if (res.ok) return { ok: true, data: (await res.json()) as CreateSessionResponse };
  return { ok: false, message: await readErrorMessage(res) };
}

/** DELETE /api/v1/repos/{owner}/{name}/sessions/{issue} — close the trigger
 *  issue, which IS the stop/retire contract. */
export async function stopTrigger(
  apiFetch: ApiFetch,
  owner: string,
  name: string,
  issueNumber: number
): Promise<MutationResult<null>> {
  const res = await apiFetch(
    `/api/v1/repos/${encodeURIComponent(owner)}/${encodeURIComponent(name)}/sessions/${issueNumber}`,
    { method: 'DELETE' }
  );
  if (res.ok) return { ok: true, data: null };
  return { ok: false, message: await readErrorMessage(res) };
}

/** DELETE /api/v1/installations/{owner} — uninstall the GitHub App from an
 *  account. */
export async function uninstallApp(
  apiFetch: ApiFetch,
  owner: string
): Promise<MutationResult<null>> {
  const res = await apiFetch(`/api/v1/installations/${encodeURIComponent(owner)}`, {
    method: 'DELETE',
  });
  if (res.ok) return { ok: true, data: null };
  return { ok: false, message: await readErrorMessage(res) };
}
