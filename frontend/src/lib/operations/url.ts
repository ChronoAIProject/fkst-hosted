// The `/operations` URL codec: search parameters ⇄ validated state.
//
// Two rules govern everything here.
//
// **Nothing invalid survives decoding.** Every parameter is validated against
// the same grammar the backend uses; a value that fails is dropped, recorded in
// `ignored`, and replaced by its default. The page surfaces `ignored` so a bad
// link degrades VISIBLY rather than silently becoming a different query — and,
// crucially, a malformed request is never issued.
//
// **The URL never asserts authority.** `scope=all` in a crafted link is decoded
// as "this caller asked for the global scope", never as "this caller has it".
// The request is still made, the server still decides, and the stable
// `403 operations_scope_forbidden` is what rewrites the URL back to the allowed
// scope. Because the row cache is keyed by scope, there is no cached global page
// for such a URL to flash.
//
// Only the ACTIVE tab's filters are encoded. `session_id`, `repo_full_name`, and
// `trigger_issue` are meaningful to both views, so one shared key per name is the
// only encoding under which the sandbox → activity cross-link produces a
// readable, hand-editable URL.

import {
  ACTIVITY_SCOPES,
  ATTRIBUTION_SOURCES,
  METHODS,
  OPERATION_IDS,
  OUTCOMES,
  RECORD_KINDS,
  SANDBOX_BACKENDS,
  SANDBOX_SCOPES,
  SANDBOX_STATUSES,
  STATUS_CLASSES,
} from '@/lib/api/operations';
import type { ActivityScope, SandboxScope } from '@/lib/api/operations';
import {
  DEFAULT_ACTIVITY_FILTERS,
  DEFAULT_SANDBOX_FILTERS,
  OPERATIONS_TABS,
  TIME_PRESETS,
  parseInstant,
  parseLogin,
  parsePositiveInt,
  parseRepoFullName,
  parseRequestId,
  parseSessionId,
  parseStatusCode,
} from './state';
import type {
  ActivityFilters,
  OperationsState,
  OperationsTab,
  SandboxFilters,
} from './state';

/** A decode result: the state to render, plus the parameter names that were
 *  present but unusable. */
export interface DecodedState {
  state: OperationsState;
  ignored: string[];
}

/** Pick `value` when it is a member of `vocabulary`, else `null`. */
function member<T extends string>(vocabulary: readonly T[], value: string | null): T | null {
  return value !== null && (vocabulary as readonly string[]).includes(value) ? (value as T) : null;
}

/** The scope vocabulary each tab speaks. */
export function scopeWords(tab: OperationsTab): readonly string[] {
  return tab === 'activity' ? ACTIVITY_SCOPES : SANDBOX_SCOPES;
}

/** The personal scope word for a tab — what a denied global request falls back
 *  to, and what a regular caller always runs in. */
export function personalScope(tab: OperationsTab): ActivityScope | SandboxScope {
  return tab === 'activity' ? 'mine' : 'accessible';
}

/** Decode the whole page state. Never throws: an unusable value is dropped. */
export function decodeState(params: URLSearchParams): DecodedState {
  const ignored: string[] = [];
  /** Read one parameter through a validator, recording an unusable value. */
  const read = <T>(key: string, parse: (raw: string) => T | null): T | null => {
    const raw = params.get(key);
    if (raw === null) return null;
    const parsed = parse(raw);
    if (parsed === null && raw.trim() !== '') ignored.push(key);
    return parsed;
  };

  const tab = member(OPERATIONS_TABS, params.get('tab')) ?? 'activity';
  if (params.has('tab') && member(OPERATIONS_TABS, params.get('tab')) === null) ignored.push('tab');

  const rawScope = params.get('scope');
  const scope = rawScope === null ? null : (member(scopeWords(tab), rawScope) as
    | ActivityScope
    | SandboxScope
    | null);
  if (rawScope !== null && scope === null) ignored.push('scope');

  const preset = read('range', (raw) => member(TIME_PRESETS, raw)) ?? DEFAULT_ACTIVITY_FILTERS.preset;
  const from = read('from', parseInstant);
  const to = read('to', parseInstant);

  const activity: ActivityFilters = {
    preset,
    // An explicit window is only meaningful under the `custom` preset; dropping
    // it otherwise is what stops a stale `from=` from silently narrowing a
    // preset window into something the user never selected.
    from: preset === 'custom' ? from : null,
    to: preset === 'custom' ? to : null,
    recordKind: read('record_kind', (raw) => member(RECORD_KINDS, raw)) ?? 'api_request',
    actorId: read('actor_id', parsePositiveInt),
    actorLogin: read('actor_login', parseLogin),
    operationId: read('operation_id', (raw) => member(OPERATION_IDS, raw.trim())),
    method: read('method', (raw) => member(METHODS, raw.trim().toUpperCase())),
    statusClass: read('status_class', (raw) => member(STATUS_CLASSES, raw.trim())),
    statusCode: read('status_code', parseStatusCode),
    outcome: read('outcome', (raw) => member(OUTCOMES, raw.trim())),
    repoFullName: read('repo_full_name', parseRepoFullName),
    triggerIssue: read('trigger_issue', parsePositiveInt),
    sessionId: read('session_id', parseSessionId),
    requestId: read('request_id', parseRequestId),
  };

  const sandbox: SandboxFilters = {
    status: read('status', (raw) => member(SANDBOX_STATUSES, raw.trim())),
    backend: read('backend', (raw) => member(SANDBOX_BACKENDS, raw.trim())),
    creatorId: read('creator_id', parsePositiveInt),
    creatorLogin: read('creator_login', parseLogin),
    repoFullName: read('repo_full_name', parseRepoFullName),
    sessionId: read('session_id', parseSessionId),
    triggerIssue: read('trigger_issue', parsePositiveInt),
    attributionSource: read('attribution_source', (raw) => member(ATTRIBUTION_SOURCES, raw.trim())),
  };

  return {
    state: {
      tab,
      scope,
      activity: tab === 'activity' ? activity : DEFAULT_ACTIVITY_FILTERS,
      sandbox: tab === 'sandboxes' ? sandbox : DEFAULT_SANDBOX_FILTERS,
    },
    // A shared key is reported once even when both tabs' validators saw it.
    ignored: Array.from(new Set(ignored)),
  };
}

/** Encode the state back. Only non-default values of the ACTIVE tab are written,
 *  so a default view keeps a short, shareable URL. */
export function encodeState(state: OperationsState): URLSearchParams {
  const params = new URLSearchParams();
  params.set('tab', state.tab);
  if (state.scope !== null) params.set('scope', state.scope);

  const put = (key: string, value: string | number | null) => {
    if (value === null || value === '') return;
    params.set(key, String(value));
  };

  if (state.tab === 'activity') {
    const f = state.activity;
    if (f.preset !== DEFAULT_ACTIVITY_FILTERS.preset) params.set('range', f.preset);
    if (f.preset === 'custom') {
      if (f.from !== null) params.set('from', new Date(f.from).toISOString());
      if (f.to !== null) params.set('to', new Date(f.to).toISOString());
    }
    if (f.recordKind !== 'api_request') params.set('record_kind', f.recordKind);
    put('actor_id', f.actorId);
    put('actor_login', f.actorLogin);
    put('operation_id', f.operationId);
    put('method', f.method);
    put('status_class', f.statusClass);
    put('status_code', f.statusCode);
    put('outcome', f.outcome);
    put('repo_full_name', f.repoFullName);
    put('trigger_issue', f.triggerIssue);
    put('session_id', f.sessionId);
    put('request_id', f.requestId);
  } else {
    const f = state.sandbox;
    put('status', f.status);
    put('backend', f.backend);
    put('creator_id', f.creatorId);
    put('creator_login', f.creatorLogin);
    put('repo_full_name', f.repoFullName);
    put('trigger_issue', f.triggerIssue);
    put('session_id', f.sessionId);
    put('attribution_source', f.attributionSource);
  }
  return params;
}

/** Strip every filter a personal scope may not carry.
 *
 *  Applied whenever the effective scope narrows — a deliberate switch to `Mine`,
 *  or a server-side downgrade. Leaving an `actor_id` in place would keep asking
 *  a question the server answers with `403`, and leaving it in the URL would
 *  make the denial look like the user's own doing. */
export function clearCrossActorFilters(filters: ActivityFilters): ActivityFilters {
  if (filters.actorId === null && filters.actorLogin === null) return filters;
  return { ...filters, actorId: null, actorLogin: null };
}
