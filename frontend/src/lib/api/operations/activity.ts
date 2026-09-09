// `GET /api/v1/operations/activity` — the typed client.
//
// Three properties this module is responsible for:
//
// 1. **Every parameter goes through `URLSearchParams`.** No template string ever
//    concatenates a user value into a query, so a login containing `&` cannot
//    smuggle a second parameter (an `actor_id` above all) into the request.
// 2. **The requested scope is remembered and checked.** The value handed to
//    `validateActivityPage` is the scope this call asked for, so a response
//    describing a different one is rejected before any row exists.
// 3. **Cancellation is the caller's.** The `AbortSignal` is passed straight
//    through to `apiFetch`, which forwards it to `fetch`; a superseded request
//    is aborted rather than merely ignored, so a slow page cannot land after the
//    filters moved on.

import type { ApiFetch } from '../canvas';
import { operationsError } from './errors';
import type { ActivityPage, ActivityScope, RecordKindFilter } from './types';
import { validateActivityPage } from './validate';

/** Everything one activity request can state. Every field is already validated
 *  by the URL codec; this type is the boundary between "validated UI state" and
 *  "wire parameters". */
export interface ActivityQuery {
  /** `null` lets the SERVER pick the caller's natural default — used only on a
   *  first load, and adopted from the response afterwards. */
  scope: ActivityScope | null;
  recordKind: RecordKindFilter;
  /** Inclusive RFC3339 UTC lower bound. */
  from?: string;
  /** Exclusive RFC3339 UTC upper bound. */
  to?: string;
  actorId?: number;
  actorLogin?: string;
  operationId?: string;
  method?: string;
  statusCode?: number;
  statusClass?: string;
  outcome?: string;
  sessionId?: string;
  repoFullName?: string;
  triggerIssue?: number;
  requestId?: string;
  cursor?: string;
  limit?: number;
}

/** Encode one query. Exported for its unit test: the encoding IS the contract,
 *  and a silently dropped filter would widen a result set. */
export function activitySearchParams(query: ActivityQuery): URLSearchParams {
  const params = new URLSearchParams();
  if (query.scope !== null) params.set('scope', query.scope);
  params.set('record_kind', query.recordKind);
  const put = (key: string, value: string | number | undefined) => {
    if (value === undefined) return;
    const text = String(value);
    if (text === '') return;
    params.set(key, text);
  };
  put('from', query.from);
  put('to', query.to);
  put('actor_id', query.actorId);
  put('actor_login', query.actorLogin);
  put('operation_id', query.operationId);
  put('method', query.method);
  put('status_code', query.statusCode);
  put('status_class', query.statusClass);
  put('outcome', query.outcome);
  put('session_id', query.sessionId);
  put('repo_full_name', query.repoFullName);
  put('trigger_issue', query.triggerIssue);
  put('request_id', query.requestId);
  put('cursor', query.cursor);
  put('limit', query.limit);
  return params;
}

/** Fetch one keyset page. Throws `OperationsError` for every failure — including
 *  a well-formed body that answers the wrong scope. */
export async function getActivity(
  apiFetch: ApiFetch,
  query: ActivityQuery,
  signal?: AbortSignal
): Promise<ActivityPage> {
  const params = activitySearchParams(query);
  const res = await apiFetch(`/api/v1/operations/activity?${params.toString()}`, { signal });
  if (!res.ok) throw await operationsError(res);
  return validateActivityPage(await res.json(), query.scope);
}
