// Fixtures for the `/operations` E2E suite.
//
// The handler below is a small, faithful stand-in for the backend's ROW-LEVEL
// authorization, not a fixture that simply returns whatever the test asks for.
// That distinction is the whole point of these specs: the browser must never be
// able to widen a scope, and the only way to prove that is to have the fake
// server refuse in exactly the places the real one does.
//
// The four viewers mirror the epic's visibility matrix:
//
// - **alice** — creator of the shared session `sess-shared`.
// - **bob**   — a collaborator on that same session.
// - **erin**  — unrelated to it; she may see neither its runtime nor its rows.
// - **grace** — a deployment global administrator.
//
// Alice and Bob both access one session, and neither may read the other's API
// rows. Erin sees neither. Grace sees everything, including the anonymous and
// orphan records nobody can be attributed.

import type { Page, Route } from '@playwright/test';

export interface Viewer {
  login: string;
  id: number;
  /** Whether the deployment lists this login in `FKST_GLOBAL_ADMINS`. */
  globalAdmin: boolean;
  /** Sessions this viewer passes session-visibility authorization for. */
  sessions: string[];
}

export const ALICE: Viewer = {
  login: 'alice',
  id: 101,
  globalAdmin: false,
  sessions: ['sess-shared'],
};
export const BOB: Viewer = { login: 'bob', id: 102, globalAdmin: false, sessions: ['sess-shared'] };
export const ERIN: Viewer = { login: 'erin', id: 103, globalAdmin: false, sessions: [] };
export const GRACE: Viewer = {
  login: 'grace',
  id: 104,
  globalAdmin: true,
  sessions: ['sess-shared'],
};

const iso = (value: string) => new Date(value).toISOString();

function apiRow(
  eventId: string,
  actor: { id: number | null; login: string | null; kind: string },
  over: Record<string, unknown> = {}
) {
  return {
    record_kind: 'api_request',
    event_id: eventId,
    request_id: `req-${eventId}`,
    started_at: iso('2026-08-01T10:00:00Z'),
    completed_at: iso('2026-08-01T10:00:01Z'),
    method: 'GET',
    route_template: '/api/v1/repos/{owner}/{name}/sessions',
    operation_id: 'canvas_repo_sessions',
    actor: { kind: actor.kind, id: actor.id, login: actor.login },
    principal: { kind: 'github_user_token', id: null },
    arguments: { owner: 'acme', name: 'app' },
    status_code: 200,
    outcome: 'success',
    duration_ms: 87,
    correlation: {
      session_id: 'sess-shared',
      repo_full_name: 'acme/app',
      trigger_issue: 42,
      request_id: `req-${eventId}`,
    },
    delivery_state: 'verified_in_posthog',
    source: 'posthog',
    ...over,
  };
}

const lifecycleRow = {
  record_kind: 'sandbox_lifecycle',
  event_id: 'ev-lifecycle-shared',
  occurred_at: iso('2026-08-01T09:58:00Z'),
  lifecycle_action: 'created',
  actor: { kind: 'system', id: null, login: null },
  principal: { kind: 'reconciler', id: null },
  session_id: 'sess-shared',
  backend: 'kubernetes',
  runtime_id: 'fkst-sess-shared',
  creator: { id: ALICE.id, login: ALICE.login },
  trigger_author: { id: ALICE.id, login: ALICE.login },
  correlation: { session_id: 'sess-shared', repo_full_name: 'acme/app', trigger_issue: 42 },
  created_at: iso('2026-08-01T09:58:00Z'),
  reason_code: null,
  delivery_state: 'queued',
  source: 'relay',
};

/** Every activity record the deployment holds. The handler filters it exactly
 *  the way the backend's viewer predicate does. */
const ALL_ACTIVITY = [
  apiRow('ev-alice-1', { id: ALICE.id, login: ALICE.login, kind: 'github_user' }),
  apiRow('ev-bob-1', { id: BOB.id, login: BOB.login, kind: 'github_user' }),
  apiRow('ev-erin-1', { id: ERIN.id, login: ERIN.login, kind: 'github_user' }),
  apiRow(
    'ev-anon-1',
    { id: null, login: null, kind: 'anonymous' },
    { status_code: 401, outcome: 'rejected', correlation: { request_id: 'req-ev-anon-1' } }
  ),
  lifecycleRow,
];

function sandbox(over: Record<string, unknown> = {}) {
  return {
    backend: 'kubernetes',
    runtime_id: 'fkst-sess-shared',
    runtime_name: 'fkst-sess-shared',
    runtime_uid: 'uid-shared',
    backend_location: 'chronoai-fkst',
    session_id: 'sess-shared',
    managed: true,
    metadata_state: 'complete',
    creator_id: ALICE.id,
    creator_login: ALICE.login,
    trigger_author_id: ALICE.id,
    trigger_author_login: ALICE.login,
    attribution_source: 'launch_metadata',
    repo_full_name: 'acme/app',
    installation_id: 11,
    trigger_issue: 42,
    status: 'running',
    raw_status: 'Running',
    status_reason: null,
    status_message: null,
    created_at: iso('2026-08-01T09:00:00Z'),
    age_seconds: 3600,
    max_lifetime_seconds: 7200,
    expires_at: iso('2026-08-01T11:00:00Z'),
    remaining_seconds: 3600,
    minimum_lifetime_seconds: 600,
    minimum_lifetime_remaining_seconds: null,
    idle_grace_seconds: 900,
    last_pending_at: null,
    idle_for_seconds: null,
    restart_count: 1,
    last_transition_at: iso('2026-08-01T09:01:00Z'),
    deletion_timestamp: null,
    warning_codes: [],
    ...over,
  };
}

/** An orphan legacy runtime: no session, no creator, no restart concept, no
 *  lifetime ceiling. Only a global administrator ever sees it. */
const ORPHAN_SANDBOX = sandbox({
  backend: 'opensandbox',
  runtime_id: 'osb-orphan-1',
  runtime_name: null,
  runtime_uid: null,
  backend_location: 'sandbox.internal',
  session_id: null,
  metadata_state: 'malformed',
  raw_status: 'ACTIVE',
  creator_id: null,
  creator_login: null,
  trigger_author_id: null,
  trigger_author_login: null,
  attribution_source: 'unknown_legacy',
  repo_full_name: null,
  installation_id: null,
  trigger_issue: null,
  max_lifetime_seconds: null,
  expires_at: null,
  remaining_seconds: null,
  restart_count: null,
  warning_codes: ['missing_session_id', 'malformed_identity'],
});

/** A conflicted runtime: two disagreeing attribution sources. Global-admin only. */
const CONFLICT_SANDBOX = sandbox({
  runtime_id: 'fkst-sess-conflict',
  session_id: 'sess-conflict',
  attribution_source: 'conflict',
  status: 'failed',
  raw_status: 'CrashLoopBackOff',
  status_reason: 'ImagePullBackOff',
  restart_count: 7,
  warning_codes: ['attribution_conflict'],
});

const ALL_SANDBOXES = [sandbox(), ORPHAN_SANDBOX, CONFLICT_SANDBOX];

export interface RouteOptions {
  viewer: Viewer;
  /** Make the activity source unavailable; the sandbox feed must be unaffected. */
  activityUnavailable?: boolean;
  /** Answer a partial (but authorized) activity page. */
  activityPartial?: boolean;
  /** Make the runtime backend unavailable; activity must be unaffected. */
  runtimeUnavailable?: boolean;
  /** Answer `503 session_visibility_unavailable` for the personal scope. */
  registryCold?: boolean;
  /** Return zero authorized rows/runtimes — a COMPLETE empty result. */
  empty?: boolean;
  /** Observe the fleet far enough in the past that the snapshot is STALE.
   *  Staleness is measured against the backend's own `observed_at`, so it is
   *  the only thing a fixture has to move. */
  runtimeObservedSecondsAgo?: number;
  /** Pad every authorized answer out to this many rows, by cloning the viewer's
   *  own records under fresh ids. Used by the capacity check: a page that stays
   *  interactive with one row proves nothing about one with a thousand. */
  padAuthorizedRowsTo?: number;
  /** Replace the free-text-ish fields with worst-case long strings. The row
   *  contents are still the viewer's own; only their LENGTH changes, so the
   *  layout claim is tested without weakening the authorization claim. */
  longStrings?: boolean;
}

/** A worst-case value: long, unbroken, and with no space to wrap at. */
const LONG = `${'z'.repeat(180)}-${'\u4f60\u597d'.repeat(40)}`;

/** Clone `rows` until there are `target` of them, giving each a unique id. */
function pad<T extends { event_id?: string; runtime_id?: string }>(rows: T[], target?: number): T[] {
  if (!target || rows.length === 0 || rows.length >= target) return rows;
  const out: T[] = [];
  for (let index = 0; index < target; index += 1) {
    const source = rows[index % rows.length];
    const clone = { ...source } as T;
    if (clone.event_id) clone.event_id = `${source.event_id}-${index}`;
    if (clone.runtime_id) clone.runtime_id = `${source.runtime_id}-${index}`;
    out.push(clone);
  }
  return out;
}

/** Stretch the bounded display fields of a row to their worst case. */
function stretch(row: Record<string, unknown>): Record<string, unknown> {
  const stretched: Record<string, unknown> = { ...row };
  if ('route_template' in stretched) stretched.route_template = `/api/v1/${LONG}`;
  if ('operation_id' in stretched) stretched.operation_id = LONG;
  if ('arguments' in stretched) stretched.arguments = { owner: LONG, name: LONG };
  if ('status_message' in stretched) stretched.status_message = LONG;
  if ('status_reason' in stretched) stretched.status_reason = LONG;
  if ('repo_full_name' in stretched && stretched.repo_full_name) {
    stretched.repo_full_name = `${LONG}/${LONG}`;
  }
  return stretched;
}

function json(route: Route, body: unknown, status = 200) {
  return route.fulfill({
    status,
    contentType: 'application/json',
    body: JSON.stringify(body),
  });
}

/** Install the two operations routes, enforcing the same authorization the
 *  backend does. Anything else under `/api/v1/**` answers a JSON 404 so a stray
 *  call is visible rather than silent. */
export async function installOperationsRoutes(page: Page, opts: RouteOptions) {
  const { viewer } = opts;
  await page.route('**/api/v1/**', async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname;

    if (path.endsWith('/api/v1/operations/activity')) {
      const scope = url.searchParams.get('scope');
      // A regular caller may neither select the global scope nor filter by
      // another actor. Both are refused BEFORE any source is consulted.
      const crossActor = url.searchParams.has('actor_id') || url.searchParams.has('actor_login');
      if ((scope === 'all' || crossActor) && !viewer.globalAdmin) {
        return json(route, { error: 'operations_scope_forbidden', message: 'denied' }, 403);
      }
      if (opts.activityUnavailable) {
        return json(route, { error: 'audit_query_not_configured', message: 'no query' }, 503);
      }

      const recordKind = url.searchParams.get('record_kind') ?? 'api_request';
      const sessionId = url.searchParams.get('session_id');
      if (scope !== 'all' && recordKind !== 'api_request' && !sessionId) {
        return json(route, { error: 'activity_session_not_found', message: 'no such session' }, 404);
      }
      if (scope !== 'all' && sessionId && !viewer.sessions.includes(sessionId)) {
        return json(route, { error: 'activity_session_not_found', message: 'no such session' }, 404);
      }

      const items = opts.empty
        ? []
        : ALL_ACTIVITY.filter((row) => {
            if (row.record_kind === 'sandbox_lifecycle') {
              if (recordKind === 'api_request') return false;
              return scope === 'all' || viewer.sessions.includes(row.session_id as string);
            }
            if (recordKind === 'sandbox_lifecycle') return false;
            // The viewer predicate: an immutable actor id equal to the caller's.
            // A shared session never widens it.
            if (scope === 'all') return true;
            return row.actor.id === viewer.id;
          });

      const activityItems = pad(
        opts.longStrings ? items.map((row) => stretch(row)) : items,
        opts.padAuthorizedRowsTo
      );

      return json(route, {
        queried_at: iso('2026-08-01T10:00:05Z'),
        from: iso('2026-07-31T10:00:00Z'),
        to: iso('2026-08-01T10:00:05Z'),
        effective_scope: scope === 'all' ? 'all' : 'mine',
        can_view_all: viewer.globalAdmin,
        items: activityItems,
        source_status: opts.activityPartial
          ? {
              posthog: 'unavailable',
              relay: 'healthy',
              partial: true,
              message_code: 'posthog_unavailable',
            }
          : { posthog: 'healthy', relay: 'healthy', partial: false },
        max_range_days: 30,
      });
    }

    if (path.endsWith('/api/v1/operations/sandboxes')) {
      const scope = url.searchParams.get('scope');
      if (scope === 'all' && !viewer.globalAdmin) {
        return json(route, { error: 'operations_scope_forbidden', message: 'denied' }, 403);
      }
      if (scope !== 'all' && opts.registryCold) {
        return json(
          route,
          { error: 'session_visibility_unavailable', message: 'recovering' },
          503
        );
      }
      if (opts.runtimeUnavailable) {
        return json(route, { error: 'sandbox_inventory_unavailable', message: 'down' }, 503);
      }
      const sessionFilter = url.searchParams.get('session_id');
      if (scope !== 'all' && sessionFilter && !viewer.sessions.includes(sessionFilter)) {
        return json(route, { error: 'sandbox_not_found', message: 'no such session' }, 404);
      }

      const items = opts.empty
        ? []
        : ALL_SANDBOXES.filter((item) => {
            if (scope === 'all') return true;
            // Accessible: a validated session id the viewer passes for.
            return item.session_id !== null && viewer.sessions.includes(item.session_id);
          }).filter((item) => !sessionFilter || item.session_id === sessionFilter);

      const sandboxItems = pad(
        opts.longStrings ? items.map((row) => stretch(row)) : items,
        opts.padAuthorizedRowsTo
      );

      return json(route, {
        observed_at: new Date(
          Date.now() - (opts.runtimeObservedSecondsAgo ?? 0) * 1000
        ).toISOString(),
        backend: 'kubernetes',
        effective_scope: scope === 'all' ? 'all' : 'accessible',
        can_view_all: viewer.globalAdmin,
        item_count: sandboxItems.length,
        filters_applied: {},
        items: sandboxItems,
        warning_codes: [],
      });
    }

    if (path.endsWith('/auth/github/refresh')) {
      return json(route, { access_token: 'e2e-refreshed-token' });
    }

    return json(route, { error: 'not_found', message: `no fixture for ${path}` }, 404);
  });
}

/** Seed a token so the SPA renders as an authenticated viewer before any script
 *  runs (the auth context reads localStorage at init). */
export async function seedOperationsAuth(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem('fkst-gh-access', 'e2e-fake-access-token');
  });
}
