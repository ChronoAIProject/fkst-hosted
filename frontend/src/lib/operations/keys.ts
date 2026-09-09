// Cache keys for the two operations feeds.
//
// A cache key is the identity of a result set. Two results may share a screen
// slot only if their keys are equal, and the polling engine drops last-good data
// the instant the key changes — synchronously, during render, before a single
// stale row can be painted.
//
// The key therefore has to name every input that could change WHOSE rows these
// are or WHICH rows they are:
//
// - the authenticated identity generation (sign-out, sign-in, account switch);
// - the effective scope, including the "server decides" state, which is a
//   genuinely different request from an explicit one;
// - the record kind, because a lifecycle timeline is authorized differently from
//   an API-request one;
// - every filter, because a narrower filter set is a different result set and a
//   cursor issued for one is invalid for the other.
//
// The time PRESET is part of the key; the resolved instants are not. A preset
// window slides with the wall clock by design, and folding `now` into the key
// would invalidate the cache on every tick.

import type { ActivityFilters, SandboxFilters } from './state';

/** Join key parts with a separator no validated value can contain. */
function join(parts: Array<string | number | null>): string {
  return parts.map((part) => (part === null ? '' : String(part))).join('|');
}

/** The identity of one activity result set. */
export function activityCacheKey(
  identityGeneration: number,
  scope: string | null,
  filters: ActivityFilters
): string {
  return join([
    'activity',
    identityGeneration,
    scope,
    filters.recordKind,
    filters.preset,
    filters.preset === 'custom' ? filters.from : null,
    filters.preset === 'custom' ? filters.to : null,
    filters.actorId,
    filters.actorLogin,
    filters.operationId,
    filters.method,
    filters.statusClass,
    filters.statusCode,
    filters.outcome,
    filters.repoFullName,
    filters.triggerIssue,
    filters.sessionId,
    filters.requestId,
  ]);
}

/** The identity of one sandbox snapshot. */
export function sandboxCacheKey(
  identityGeneration: number,
  scope: string | null,
  filters: SandboxFilters
): string {
  return join([
    'sandboxes',
    identityGeneration,
    scope,
    filters.status,
    filters.backend,
    filters.creatorId,
    filters.creatorLogin,
    filters.repoFullName,
    filters.sessionId,
    filters.triggerIssue,
    filters.attributionSource,
  ]);
}
