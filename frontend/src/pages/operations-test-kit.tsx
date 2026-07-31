import { vi } from 'vitest';
import { render } from '@testing-library/react';
import { BrowserRouter } from 'react-router-dom';
import { Operations } from './operations';
import { AuthProvider, useAuth } from '@/lib/auth/github-auth';
import type { ActivityRow, SandboxRow } from '@/lib/api/operations';

// Shared fixtures and fetch stubs for the `/operations` page suites.
//
// The stub answers ONLY the two operations routes and throws on anything else.
// That is the point: a test that starts calling PostHog, a relay, Kubernetes, or
// OpenSandbox from the browser fails loudly instead of quietly passing.

export function jsonResponse(body: unknown, status = 200, headers: Record<string, string> = {}) {
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: new Headers(headers),
    json: async () => body,
  } as Response;
}

export const ALICE_ROW: ActivityRow = {
  record_kind: 'api_request',
  event_id: 'ev-alice-1',
  request_id: 'req-alice-1',
  started_at: '2026-08-01T10:00:00.000Z',
  completed_at: '2026-08-01T10:00:01.000Z',
  method: 'GET',
  route_template: '/api/v1/overview',
  operation_id: 'canvas_overview',
  actor: { kind: 'github_user', id: 7, login: 'alice' },
  principal: { kind: 'github_user_token', id: null },
  arguments: { limit: 20 },
  status_code: 200,
  outcome: 'success',
  duration_ms: 42,
  correlation: { session_id: 'sess-1', repo_full_name: 'acme/app', request_id: 'req-alice-1' },
  delivery_state: 'verified_in_posthog',
  source: 'posthog',
};

export const BOB_ROW: ActivityRow = {
  ...ALICE_ROW,
  event_id: 'ev-bob-1',
  request_id: 'req-bob-1',
  actor: { kind: 'github_user', id: 8, login: 'bob' },
  correlation: { session_id: 'sess-1', repo_full_name: 'acme/app', request_id: 'req-bob-1' },
};

export const LIFECYCLE_ROW: ActivityRow = {
  record_kind: 'sandbox_lifecycle',
  event_id: 'ev-life-1',
  occurred_at: '2026-08-01T09:59:00.000Z',
  lifecycle_action: 'created',
  actor: { kind: 'system', id: null, login: null },
  principal: { kind: 'reconciler', id: null },
  session_id: 'sess-1',
  backend: 'kubernetes',
  runtime_id: 'fkst-sess-1',
  creator: { id: 7, login: 'alice' },
  trigger_author: { id: 7, login: 'alice' },
  correlation: { session_id: 'sess-1', repo_full_name: 'acme/app' },
  created_at: '2026-08-01T09:59:00.000Z',
  reason_code: null,
  delivery_state: 'queued',
  source: 'relay',
};

export const ANON_ROW: ActivityRow = {
  ...ALICE_ROW,
  event_id: 'ev-anon-1',
  request_id: 'req-anon-1',
  actor: { kind: 'anonymous', id: null, login: null },
  status_code: 401,
  outcome: 'rejected',
  correlation: { request_id: 'req-anon-1' },
};

export const RUNNING_SANDBOX: SandboxRow = {
  backend: 'kubernetes',
  runtime_id: 'fkst-sess-1',
  runtime_name: 'fkst-sess-1',
  runtime_uid: 'uid-1',
  backend_location: 'chronoai-fkst',
  session_id: 'sess-1',
  managed: true,
  metadata_state: 'complete',
  creator_id: 7,
  creator_login: 'alice',
  trigger_author_id: 7,
  trigger_author_login: 'alice',
  attribution_source: 'launch_metadata',
  repo_full_name: 'acme/app',
  installation_id: 11,
  trigger_issue: 42,
  status: 'running',
  raw_status: 'Running',
  status_reason: null,
  status_message: null,
  created_at: '2026-08-01T09:00:00.000Z',
  age_seconds: 3600,
  max_lifetime_seconds: 7200,
  expires_at: '2026-08-01T11:00:00.000Z',
  remaining_seconds: 3600,
  minimum_lifetime_seconds: 600,
  minimum_lifetime_remaining_seconds: null,
  idle_grace_seconds: 900,
  last_pending_at: null,
  idle_for_seconds: null,
  restart_count: 2,
  last_transition_at: '2026-08-01T09:01:00.000Z',
  deletion_timestamp: null,
  warning_codes: [],
};

/** A legacy OpenSandbox runtime: no creator, no restart concept, no ceiling. */
export const LEGACY_SANDBOX: SandboxRow = {
  ...RUNNING_SANDBOX,
  backend: 'opensandbox',
  runtime_id: 'osb-legacy-1',
  runtime_name: null,
  runtime_uid: null,
  backend_location: 'https://sandbox.example/v1/sandboxes?token=secret',
  session_id: 'sess-legacy',
  metadata_state: 'malformed',
  // A backend-native state whose spelling differs from the normalized label, so
  // a test can prove BOTH are rendered rather than one standing in for the other.
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
};

export function activityPage(over: Record<string, unknown> = {}) {
  return {
    queried_at: '2026-08-01T10:00:02.000Z',
    from: '2026-07-31T10:00:00.000Z',
    to: '2026-08-01T10:00:02.000Z',
    effective_scope: 'mine',
    can_view_all: false,
    items: [ALICE_ROW],
    source_status: { posthog: 'healthy', relay: 'healthy', partial: false },
    ...over,
  };
}

export function sandboxSnapshot(over: Record<string, unknown> = {}) {
  const items = (over.items as SandboxRow[]) ?? [RUNNING_SANDBOX];
  return {
    observed_at: '2026-08-01T10:00:00.000Z',
    backend: 'kubernetes',
    effective_scope: 'accessible',
    can_view_all: false,
    filters_applied: {},
    warning_codes: [],
    ...over,
    items,
    item_count: items.length,
  };
}

/** One recorded request the stub answered. */
export interface StubCall {
  path: string;
  params: URLSearchParams;
}

export interface StubHandlers {
  /** Answer the activity route. Return a `Response`. */
  activity?: (params: URLSearchParams, call: number) => Response;
  /** Answer the sandbox route. */
  sandboxes?: (params: URLSearchParams, call: number) => Response;
}

/** Stub `fetch` for the two operations routes only. */
export function stubOperations(handlers: StubHandlers) {
  const calls: StubCall[] = [];
  let activityCalls = 0;
  let sandboxCalls = 0;
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), 'http://localhost');
    calls.push({ path: url.pathname, params: url.searchParams });
    if (url.pathname.endsWith('/api/v1/operations/activity')) {
      if (!handlers.activity) throw new Error('unexpected activity call');
      return handlers.activity(url.searchParams, activityCalls++);
    }
    if (url.pathname.endsWith('/api/v1/operations/sandboxes')) {
      if (!handlers.sandboxes) throw new Error('unexpected sandbox call');
      return handlers.sandboxes(url.searchParams, sandboxCalls++);
    }
    // Any other host or path is a bug: the browser must never talk to PostHog,
    // the relay, Kubernetes, or OpenSandbox.
    throw new Error(`unexpected fetch: ${String(input)}`);
  });
  vi.stubGlobal('fetch', fetchMock);
  return { fetchMock, calls };
}

/** Seed a token so `useAuth()` renders as an authenticated viewer. */
export function seedAuth() {
  window.localStorage.setItem('fkst-gh-access', 'test-access-token');
}

/** Point the browser at an `/operations` URL before rendering. */
export function seedUrl(search = '') {
  window.history.replaceState(null, '', `/operations${search}`);
}

export function currentSearch(): URLSearchParams {
  return new URLSearchParams(window.location.search);
}

/** Exposes the auth actions a test needs to drive an identity change from the
 *  outside, exactly as the shell's own controls would. */
function AuthProbe() {
  const { signOut } = useAuth();
  return (
    <button type="button" onClick={signOut}>
      probe-sign-out
    </button>
  );
}

export function renderOperations({ authControls = false } = {}) {
  return render(
    <AuthProvider>
      {authControls && <AuthProbe />}
      {/* BrowserRouter, not MemoryRouter: the page's whole state lives in the
          query string, and only a browser router puts it somewhere a test can
          assert the way a user would read it. */}
      <BrowserRouter>
        <Operations />
      </BrowserRouter>
    </AuthProvider>
  );
}
