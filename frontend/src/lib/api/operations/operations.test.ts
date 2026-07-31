import { describe, expect, it, vi } from 'vitest';
import {
  OperationsError,
  activitySearchParams,
  describeError,
  getActivity,
  getSandboxes,
  isScopeDenied,
  isUnauthenticated,
  sandboxSearchParams,
  validateActivityPage,
  validateSandboxInventory,
} from './index';
import type { ActivityQuery } from './activity';
import type { SandboxQuery } from './sandboxes';

/** A minimal `Response` stand-in — enough for the client's `ok`/`json`/headers. */
function response(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: new Headers(headers),
    json: async () => body,
  } as unknown as Response;
}

const apiRow = {
  record_kind: 'api_request',
  event_id: 'ev-1',
  request_id: 'req-1',
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
  correlation: { request_id: 'req-1' },
  delivery_state: 'verified_in_posthog',
  source: 'posthog',
};

function page(overrides: Record<string, unknown> = {}) {
  return {
    queried_at: '2026-08-01T10:00:02.000Z',
    from: '2026-07-31T10:00:00.000Z',
    to: '2026-08-01T10:00:02.000Z',
    effective_scope: 'mine',
    can_view_all: false,
    items: [apiRow],
    source_status: { posthog: 'healthy', relay: 'not_configured', partial: false },
    ...overrides,
  };
}

const sandboxRow = {
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
  restart_count: null,
  last_transition_at: null,
  deletion_timestamp: null,
  warning_codes: [],
};

function inventory(overrides: Record<string, unknown> = {}) {
  return {
    observed_at: '2026-08-01T10:00:00.000Z',
    backend: 'kubernetes',
    effective_scope: 'accessible',
    can_view_all: false,
    item_count: 1,
    filters_applied: {},
    items: [sandboxRow],
    warning_codes: [],
    ...overrides,
  };
}

const baseActivityQuery: ActivityQuery = { scope: 'mine', recordKind: 'api_request' };
const baseSandboxQuery: SandboxQuery = { scope: 'accessible' };

describe('activitySearchParams', () => {
  it('encodes every stated filter and omits the unstated ones', () => {
    const params = activitySearchParams({
      ...baseActivityQuery,
      recordKind: 'all',
      from: '2026-08-01T00:00:00.000Z',
      to: '2026-08-01T10:00:00.000Z',
      actorId: 7,
      operationId: 'canvas_overview',
      method: 'GET',
      statusCode: 404,
      statusClass: '4xx',
      outcome: 'client_error',
      sessionId: 'sess-1',
      repoFullName: 'acme/app',
      triggerIssue: 42,
      requestId: 'req-1',
      cursor: 'opaque',
      limit: 50,
    });
    expect(params.get('scope')).toBe('mine');
    expect(params.get('record_kind')).toBe('all');
    expect(params.get('actor_id')).toBe('7');
    expect(params.get('status_code')).toBe('404');
    expect(params.get('cursor')).toBe('opaque');
    expect(params.get('limit')).toBe('50');
    expect(params.has('actor_login')).toBe(false);
  });

  it('omits `scope` entirely when the server is meant to choose', () => {
    expect(activitySearchParams({ ...baseActivityQuery, scope: null }).has('scope')).toBe(false);
  });

  it('escapes a value that would otherwise forge a second parameter', () => {
    // The backend would reject this login anyway; the point is that it can never
    // arrive as a SEPARATE parameter.
    const params = activitySearchParams({
      ...baseActivityQuery,
      actorLogin: 'alice&actor_id=99',
    });
    expect(params.get('actor_login')).toBe('alice&actor_id=99');
    expect(params.get('actor_id')).toBeNull();
    expect(params.toString()).toContain('actor_login=alice%26actor_id%3D99');
  });
});

describe('sandboxSearchParams', () => {
  it('encodes stated filters only', () => {
    const params = sandboxSearchParams({
      ...baseSandboxQuery,
      status: 'failed',
      creatorId: 7,
      attributionSource: 'unknown_legacy',
    });
    expect(params.get('scope')).toBe('accessible');
    expect(params.get('status')).toBe('failed');
    expect(params.get('creator_id')).toBe('7');
    expect(params.has('backend')).toBe(false);
  });
});

describe('validateActivityPage', () => {
  it('accepts a well-formed page for the requested scope', () => {
    const result = validateActivityPage(page(), 'mine');
    expect(result.items).toHaveLength(1);
  });

  it('accepts any scope when the caller deliberately requested none', () => {
    expect(validateActivityPage(page({ effective_scope: 'all', can_view_all: true }), null))
      .toMatchObject({ effective_scope: 'all' });
  });

  it('rejects a page whose scope is not the requested one', () => {
    expect(() => validateActivityPage(page({ effective_scope: 'all', can_view_all: true }), 'mine'))
      .toThrowError(expect.objectContaining({ code: 'scope_mismatch' }));
  });

  it('rejects a global page whose own can_view_all is false', () => {
    expect(() => validateActivityPage(page({ effective_scope: 'all' }), 'all')).toThrowError(
      expect.objectContaining({ code: 'scope_mismatch' })
    );
  });

  it('rejects a row missing a field the renderer dereferences', () => {
    const broken = page({ items: [{ ...apiRow, completed_at: undefined }] });
    expect(() => validateActivityPage(broken, 'mine')).toThrowError(
      expect.objectContaining({ code: 'malformed' })
    );
  });

  it('rejects a lifecycle row wearing an api_request discriminator', () => {
    const broken = page({
      items: [{ ...apiRow, record_kind: 'sandbox_lifecycle' }],
    });
    expect(() => validateActivityPage(broken, 'mine')).toThrowError(
      expect.objectContaining({ code: 'malformed' })
    );
  });

  it('rejects a non-finite numeric that would render as NaN', () => {
    const broken = page({ items: [{ ...apiRow, duration_ms: Number.NaN }] });
    expect(() => validateActivityPage(broken, 'mine')).toThrowError(
      expect.objectContaining({ code: 'malformed' })
    );
  });

  it('rejects an unknown scope word', () => {
    expect(() => validateActivityPage(page({ effective_scope: 'everything' }), null)).toThrowError(
      expect.objectContaining({ code: 'malformed' })
    );
  });
});

describe('validateSandboxInventory', () => {
  it('accepts a well-formed snapshot', () => {
    expect(validateSandboxInventory(inventory(), 'accessible').items).toHaveLength(1);
  });

  it('rejects a scope the caller did not request', () => {
    expect(() =>
      validateSandboxInventory(
        inventory({ effective_scope: 'all', can_view_all: true }),
        'accessible'
      )
    ).toThrowError(expect.objectContaining({ code: 'scope_mismatch' }));
  });

  it('rejects an item_count that disagrees with the rows', () => {
    expect(() => validateSandboxInventory(inventory({ item_count: 5 }), 'accessible')).toThrowError(
      expect.objectContaining({ code: 'malformed' })
    );
  });

  it('accepts a null restart count but rejects a non-numeric one', () => {
    expect(
      validateSandboxInventory(inventory(), 'accessible').items[0]!.restart_count
    ).toBeNull();
    const broken = inventory({ items: [{ ...sandboxRow, restart_count: 'many' }] });
    expect(() => validateSandboxInventory(broken, 'accessible')).toThrowError(
      expect.objectContaining({ code: 'malformed' })
    );
  });
});

describe('getActivity', () => {
  it('calls the operations route with the encoded query', async () => {
    const apiFetch = vi.fn().mockResolvedValue(response(page()));
    await getActivity(apiFetch, baseActivityQuery);
    expect(apiFetch).toHaveBeenCalledTimes(1);
    const [path] = apiFetch.mock.calls[0] as [string];
    expect(path).toContain('/api/v1/operations/activity?');
    expect(path).toContain('scope=mine');
  });

  it('maps a stable envelope code onto a typed error and keeps the request id', async () => {
    const apiFetch = vi
      .fn()
      .mockResolvedValue(
        response({ error: 'operations_scope_forbidden', message: 'nope' }, 403, {
          'x-request-id': 'req-9',
        })
      );
    const error = await getActivity(apiFetch, { ...baseActivityQuery, scope: 'all' }).catch(
      (e: unknown) => e
    );
    expect(error).toBeInstanceOf(OperationsError);
    expect(isScopeDenied(error)).toBe(true);
    expect((error as OperationsError).requestId).toBe('req-9');
    // The backend's own message never becomes UI copy.
    expect(String((error as OperationsError).message)).not.toContain('nope');
  });

  it('derives a code from the status when the body is not an envelope', async () => {
    const apiFetch = vi.fn().mockResolvedValue({
      ok: false,
      status: 401,
      headers: new Headers(),
      json: async () => {
        throw new Error('not json');
      },
    } as unknown as Response);
    const error = await getActivity(apiFetch, baseActivityQuery).catch((e: unknown) => e);
    expect(isUnauthenticated(error)).toBe(true);
    expect((error as OperationsError).code).toBe('unauthorized');
  });
});

describe('getSandboxes', () => {
  it('answers a validated snapshot', async () => {
    const apiFetch = vi.fn().mockResolvedValue(response(inventory()));
    const result = await getSandboxes(apiFetch, baseSandboxQuery);
    expect(result.item_count).toBe(1);
  });

  it('surfaces the stable cold-projection code', async () => {
    const apiFetch = vi
      .fn()
      .mockResolvedValue(response({ error: 'session_visibility_unavailable' }, 503));
    const error = await getSandboxes(apiFetch, baseSandboxQuery).catch((e: unknown) => e);
    expect((error as OperationsError).code).toBe('session_visibility_unavailable');
  });
});

describe('describeError', () => {
  it('reduces a transport failure to the network code', () => {
    expect(describeError(new TypeError('failed to fetch'))).toEqual({
      code: 'network',
      requestId: null,
    });
  });

  it('passes a typed failure through unchanged', () => {
    expect(describeError(new OperationsError('sandbox_not_found', 404, 'r1'))).toEqual({
      code: 'sandbox_not_found',
      requestId: 'r1',
    });
  });
});
