import { describe, expect, it } from 'vitest';
import {
  clearCrossActorFilters,
  decodeState,
  encodeState,
  personalScope,
  scopeWords,
} from './url';
import {
  DAY_MS,
  DEFAULT_ACTIVITY_FILTERS,
  DEFAULT_SANDBOX_FILTERS,
  hasUsableWindow,
  needsSessionId,
  windowProblem,
  parseLogin,
  parsePositiveInt,
  parseRepoFullName,
  parseRequestId,
  parseSessionId,
  parseStatusCode,
  resolveWindow,
} from './state';

const decode = (query: string) => decodeState(new URLSearchParams(query));

describe('value grammars', () => {
  it('accepts and normalizes a login, with or without the leading @', () => {
    expect(parseLogin('@alice')).toBe('alice');
    expect(parseLogin(' bob-1 ')).toBe('bob-1');
    expect(parseLogin('has space')).toBeNull();
    expect(parseLogin('a'.repeat(40))).toBeNull();
  });

  it('requires both halves of an owner/name pair', () => {
    expect(parseRepoFullName('acme/app')).toBe('acme/app');
    expect(parseRepoFullName('acme')).toBeNull();
    expect(parseRepoFullName('/app')).toBeNull();
    expect(parseRepoFullName('acme/ap p')).toBeNull();
  });

  it('bounds session and request ids to the audit contract grammars', () => {
    expect(parseSessionId('a1b2-c3.d4_e5')).toBe('a1b2-c3.d4_e5');
    expect(parseSessionId('a/b')).toBeNull();
    // A request id additionally allows the `:` that trace ids carry.
    expect(parseRequestId('w3c:trace.id-1')).toBe('w3c:trace.id-1');
    expect(parseRequestId('has space')).toBeNull();
  });

  it('rejects every numeric spelling that would not round-trip', () => {
    expect(parsePositiveInt('42')).toBe(42);
    expect(parsePositiveInt('0')).toBeNull();
    expect(parsePositiveInt('1.5')).toBeNull();
    expect(parsePositiveInt('1e3')).toBeNull();
    expect(parsePositiveInt('-1')).toBeNull();
  });

  it('bounds a status code to 100..599', () => {
    expect(parseStatusCode('404')).toBe(404);
    expect(parseStatusCode('99')).toBeNull();
    expect(parseStatusCode('600')).toBeNull();
  });
});

describe('time windows', () => {
  const NOW = Date.parse('2026-08-01T12:00:00.000Z');
  const custom = (from: number | null, to: number | null) => ({
    ...DEFAULT_ACTIVITY_FILTERS,
    preset: 'custom' as const,
    from,
    to,
  });

  it('resolves a preset relative to the given instant', () => {
    expect(resolveWindow({ ...DEFAULT_ACTIVITY_FILTERS, preset: '1h' }, NOW)).toEqual({
      from: NOW - 3_600_000,
      to: NOW,
    });
  });

  it('names each reason a window cannot be queried', () => {
    const from = NOW - 2 * DAY_MS;
    expect(windowProblem(custom(null, NOW), undefined, NOW)).toBe('incomplete');
    expect(windowProblem(custom(from, null), undefined, NOW)).toBe('incomplete');
    expect(windowProblem(custom(NOW, from), undefined, NOW)).toBe('unordered');
    expect(windowProblem(custom(from, from), undefined, NOW)).toBe('unordered');
    expect(windowProblem(custom(NOW - 31 * DAY_MS, NOW), undefined, NOW)).toBe('too_wide');
    // The backend refuses a window that STARTS in the future (`check_range`);
    // one that merely ends there is how a live view is written.
    expect(windowProblem(custom(NOW + DAY_MS, NOW + 2 * DAY_MS), undefined, NOW)).toBe('future');
    expect(windowProblem(custom(from, NOW + DAY_MS), undefined, NOW)).toBeNull();
    expect(windowProblem(custom(from, NOW), undefined, NOW)).toBeNull();
  });

  it('measures width against the DEPLOYMENT ceiling, not a client constant', () => {
    const from = NOW - 10 * DAY_MS;
    // Narrower than the 30-day default: a preset the client would otherwise
    // allow becomes unqueryable.
    expect(windowProblem(custom(from, NOW), 7 * DAY_MS, NOW)).toBe('too_wide');
    expect(windowProblem({ ...DEFAULT_ACTIVITY_FILTERS, preset: '30d' }, 7 * DAY_MS, NOW)).toBe(
      'too_wide'
    );
    // Wider than it: a window the default would have refused is accepted.
    expect(windowProblem(custom(NOW - 45 * DAY_MS, NOW), 90 * DAY_MS, NOW)).toBeNull();
  });

  it('withholds the request for any unusable window', () => {
    expect(hasUsableWindow(custom(null, null), undefined, NOW)).toBe(false);
    expect(hasUsableWindow(DEFAULT_ACTIVITY_FILTERS, undefined, NOW)).toBe(true);
    expect(resolveWindow(custom(NOW + DAY_MS, NOW + 2 * DAY_MS), NOW)).toBeNull();
  });
});

describe('needsSessionId', () => {
  it('requires an exact session for a personal lifecycle query', () => {
    expect(
      needsSessionId({ ...DEFAULT_ACTIVITY_FILTERS, recordKind: 'sandbox_lifecycle' }, 'mine')
    ).toBe(true);
    expect(needsSessionId({ ...DEFAULT_ACTIVITY_FILTERS, recordKind: 'all' }, 'mine')).toBe(true);
  });

  it('is satisfied once a session is named', () => {
    expect(
      needsSessionId(
        { ...DEFAULT_ACTIVITY_FILTERS, recordKind: 'all', sessionId: 'sess-1' },
        'mine'
      )
    ).toBe(false);
  });

  it('never blocks an api_request query, and never blocks the global scope', () => {
    expect(needsSessionId(DEFAULT_ACTIVITY_FILTERS, 'mine')).toBe(false);
    expect(needsSessionId({ ...DEFAULT_ACTIVITY_FILTERS, recordKind: 'all' }, 'all')).toBe(false);
  });
});

describe('decodeState', () => {
  it('defaults to the activity tab with the server choosing the scope', () => {
    const { state, ignored } = decode('');
    expect(state.tab).toBe('activity');
    expect(state.scope).toBeNull();
    expect(state.activity).toEqual(DEFAULT_ACTIVITY_FILTERS);
    expect(ignored).toEqual([]);
  });

  it('decodes a full activity link', () => {
    const { state } = decode(
      'tab=activity&scope=mine&range=7d&status_class=5xx&session_id=sess-1&record_kind=all'
    );
    expect(state.scope).toBe('mine');
    expect(state.activity.preset).toBe('7d');
    expect(state.activity.statusClass).toBe('5xx');
    expect(state.activity.sessionId).toBe('sess-1');
    expect(state.activity.recordKind).toBe('all');
  });

  it('decodes a full sandbox link and leaves the activity filters at defaults', () => {
    const { state } = decode('tab=sandboxes&scope=accessible&status=failed&creator_id=7');
    expect(state.tab).toBe('sandboxes');
    expect(state.sandbox.status).toBe('failed');
    expect(state.sandbox.creatorId).toBe(7);
    expect(state.activity).toEqual(DEFAULT_ACTIVITY_FILTERS);
  });

  it('carries a crafted global scope as a REQUEST, never as an authorization', () => {
    // The decoder's job is to say what was asked for. Nothing here grants it —
    // the server's answer does, and the page keys its cache on the scope so
    // there is no global page to flash.
    const { state, ignored } = decode('tab=activity&scope=all&actor_id=99');
    expect(state.scope).toBe('all');
    expect(state.activity.actorId).toBe(99);
    expect(ignored).toEqual([]);
  });

  it('drops every unusable value, names it, and never issues it', () => {
    const { state, ignored } = decode(
      'tab=nonsense&scope=everything&method=TRACE&status_code=999&session_id=a/b&operation_id=nope&actor_login=@bad login'
    );
    expect(state.tab).toBe('activity');
    expect(state.scope).toBeNull();
    expect(state.activity.method).toBeNull();
    expect(state.activity.statusCode).toBeNull();
    expect(state.activity.sessionId).toBeNull();
    expect(state.activity.operationId).toBeNull();
    expect(ignored).toEqual(
      expect.arrayContaining([
        'tab',
        'scope',
        'method',
        'status_code',
        'session_id',
        'operation_id',
        'actor_login',
      ])
    );
  });

  it('rejects a scope word belonging to the OTHER tab', () => {
    expect(decode('tab=sandboxes&scope=mine').state.scope).toBeNull();
    expect(decode('tab=activity&scope=accessible').state.scope).toBeNull();
  });

  it('keeps an explicit window only under the custom preset', () => {
    const withCustom = decode(
      'range=custom&from=2026-08-01T00:00:00Z&to=2026-08-01T06:00:00Z'
    ).state.activity;
    expect(withCustom.from).toBe(Date.parse('2026-08-01T00:00:00Z'));
    expect(withCustom.to).toBe(Date.parse('2026-08-01T06:00:00Z'));

    // A stale `from=` behind a preset would silently narrow the window.
    const withPreset = decode('range=7d&from=2026-08-01T00:00:00Z').state.activity;
    expect(withPreset.from).toBeNull();
  });

  it('normalizes a method to upper case', () => {
    expect(decode('method=get').state.activity.method).toBe('GET');
  });
});

describe('encodeState', () => {
  it('round-trips a non-default activity state', () => {
    const original = decode(
      'tab=activity&scope=all&range=7d&record_kind=all&actor_id=7&session_id=sess-1'
    ).state;
    const round = decodeState(encodeState(original)).state;
    expect(round).toEqual(original);
  });

  it('round-trips a non-default sandbox state', () => {
    const original = decode(
      'tab=sandboxes&scope=all&status=failed&backend=opensandbox&attribution_source=unknown_legacy'
    ).state;
    expect(decodeState(encodeState(original)).state).toEqual(original);
  });

  it('writes a short URL for the default view', () => {
    const params = encodeState({
      tab: 'activity',
      scope: 'mine',
      activity: DEFAULT_ACTIVITY_FILTERS,
      sandbox: DEFAULT_SANDBOX_FILTERS,
    });
    expect(params.toString()).toBe('tab=activity&scope=mine');
  });

  it('writes only the ACTIVE tab filters, so shared keys cannot collide', () => {
    const params = encodeState({
      tab: 'sandboxes',
      scope: 'accessible',
      activity: { ...DEFAULT_ACTIVITY_FILTERS, sessionId: 'from-activity' },
      sandbox: { ...DEFAULT_SANDBOX_FILTERS, sessionId: 'from-sandbox' },
    });
    expect(params.get('session_id')).toBe('from-sandbox');
  });
});

describe('scope helpers', () => {
  it('knows the scope vocabulary of each tab', () => {
    expect(scopeWords('activity')).toEqual(['mine', 'all']);
    expect(scopeWords('sandboxes')).toEqual(['accessible', 'all']);
    expect(personalScope('activity')).toBe('mine');
    expect(personalScope('sandboxes')).toBe('accessible');
  });

  it('drops the filters only a global caller may carry', () => {
    const narrowed = clearCrossActorFilters({
      ...DEFAULT_ACTIVITY_FILTERS,
      actorId: 9,
      actorLogin: 'someone',
      sessionId: 'sess-1',
    });
    expect(narrowed.actorId).toBeNull();
    expect(narrowed.actorLogin).toBeNull();
    // Everything a personal caller MAY state survives.
    expect(narrowed.sessionId).toBe('sess-1');
  });

  it('returns the same object when there is nothing to clear', () => {
    expect(clearCrossActorFilters(DEFAULT_ACTIVITY_FILTERS)).toBe(DEFAULT_ACTIVITY_FILTERS);
  });
});
