// `GET /api/v1/operations/sandboxes` — the typed client.
//
// Same three properties as the activity client: `URLSearchParams` for every
// value, the requested scope checked against the answered one, and the caller's
// `AbortSignal` forwarded so a superseded snapshot is cancelled rather than
// merely ignored.
//
// One asymmetry is worth stating: the sandbox response is a COMPLETE snapshot,
// not a page. There is no cursor, so "load older" has no meaning here, and a
// failure leaves the previous snapshot on screen only while the viewer and scope
// are unchanged (the hook owns that rule, not this module).

import type { ApiFetch } from '../canvas';
import { operationsError } from './errors';
import type { SandboxInventory, SandboxScope } from './types';
import { validateSandboxInventory } from './validate';

/** Everything one sandbox request can state. */
export interface SandboxQuery {
  /** `null` lets the SERVER pick the caller's natural default. */
  scope: SandboxScope | null;
  status?: string;
  backend?: string;
  creatorId?: number;
  creatorLogin?: string;
  repoFullName?: string;
  sessionId?: string;
  triggerIssue?: number;
  attributionSource?: string;
}

/** Encode one query. Exported for its unit test. */
export function sandboxSearchParams(query: SandboxQuery): URLSearchParams {
  const params = new URLSearchParams();
  if (query.scope !== null) params.set('scope', query.scope);
  const put = (key: string, value: string | number | undefined) => {
    if (value === undefined) return;
    const text = String(value);
    if (text === '') return;
    params.set(key, text);
  };
  put('status', query.status);
  put('backend', query.backend);
  put('creator_id', query.creatorId);
  put('creator_login', query.creatorLogin);
  put('repo_full_name', query.repoFullName);
  put('session_id', query.sessionId);
  put('trigger_issue', query.triggerIssue);
  put('attribution_source', query.attributionSource);
  return params;
}

/** Fetch one live snapshot. Throws `OperationsError` for every failure. */
export async function getSandboxes(
  apiFetch: ApiFetch,
  query: SandboxQuery,
  signal?: AbortSignal
): Promise<SandboxInventory> {
  const params = sandboxSearchParams(query);
  const suffix = params.toString();
  const res = await apiFetch(
    `/api/v1/operations/sandboxes${suffix ? `?${suffix}` : ''}`,
    { signal }
  );
  if (!res.ok) throw await operationsError(res);
  return validateSandboxInventory(await res.json(), query.scope);
}
