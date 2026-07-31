// The validated UI state of `/operations`, and the value grammars it accepts.
//
// The types here are the ONLY shapes the request builders read. Anything that
// reaches them has already passed the validators below, so a component can never
// hand a raw URL fragment to the API client. The grammars mirror the backend's
// own validators (`backend/src/operations/filters.rs`,
// `backend/src/audit/arguments/bounds.rs`) — deliberately, so a value the UI
// accepts is a value the backend accepts, and a bad URL is refused here rather
// than becoming a `400` the user has to interpret.

import type {
  ActivityScope,
  RecordKindFilter,
  SandboxScope,
} from '@/lib/api/operations';

export const OPERATIONS_TABS = ['activity', 'sandboxes'] as const;
export type OperationsTab = (typeof OPERATIONS_TABS)[number];

/** Bounded time presets, plus the explicit UTC window. */
export const TIME_PRESETS = ['1h', '24h', '7d', '30d', 'custom'] as const;
export type TimePreset = (typeof TIME_PRESETS)[number];

/** Preset → window length in milliseconds. `custom` has none; the state carries
 *  explicit bounds instead. */
export const PRESET_MS: Record<Exclude<TimePreset, 'custom'>, number> = {
  '1h': 3_600_000,
  '24h': 86_400_000,
  '7d': 604_800_000,
  '30d': 2_592_000_000,
};

/** The deployment's own hard ceiling on one window (`FKST_POSTHOG_ACTIVITY_
 *  MAX_RANGE_DAYS`, default 30 days). Enforced client-side too so a custom range
 *  the server will refuse is refused before the request. */
export const MAX_RANGE_MS = 30 * 86_400_000;

export interface ActivityFilters {
  preset: TimePreset;
  /** Epoch-ms bounds, present only for `custom`. */
  from: number | null;
  to: number | null;
  recordKind: RecordKindFilter;
  /** Global scope only — the server injects a regular caller's own identity. */
  actorId: number | null;
  actorLogin: string | null;
  operationId: string | null;
  method: string | null;
  statusClass: string | null;
  statusCode: number | null;
  outcome: string | null;
  repoFullName: string | null;
  triggerIssue: number | null;
  sessionId: string | null;
  requestId: string | null;
}

export interface SandboxFilters {
  status: string | null;
  backend: string | null;
  creatorId: number | null;
  creatorLogin: string | null;
  repoFullName: string | null;
  sessionId: string | null;
  triggerIssue: number | null;
  attributionSource: string | null;
}

/** The whole addressable state of the page. */
export interface OperationsState {
  tab: OperationsTab;
  /** The scope word for the ACTIVE tab, or `null` to let the server choose its
   *  natural default (only ever true before the first response). */
  scope: ActivityScope | SandboxScope | null;
  activity: ActivityFilters;
  sandbox: SandboxFilters;
}

export const DEFAULT_ACTIVITY_FILTERS: ActivityFilters = {
  preset: '24h',
  from: null,
  to: null,
  // The cheapest, least-privileged shape: a personal API-request timeline needs
  // no session authorization at all.
  recordKind: 'api_request',
  actorId: null,
  actorLogin: null,
  operationId: null,
  method: null,
  statusClass: null,
  statusCode: null,
  outcome: null,
  repoFullName: null,
  triggerIssue: null,
  sessionId: null,
  requestId: null,
};

export const DEFAULT_SANDBOX_FILTERS: SandboxFilters = {
  status: null,
  backend: null,
  creatorId: null,
  creatorLogin: null,
  repoFullName: null,
  sessionId: null,
  triggerIssue: null,
  attributionSource: null,
};

export const DEFAULT_STATE: OperationsState = {
  tab: 'activity',
  scope: null,
  activity: DEFAULT_ACTIVITY_FILTERS,
  sandbox: DEFAULT_SANDBOX_FILTERS,
};

const OWNER = /^[A-Za-z0-9._-]{1,64}$/;
const REPO = /^[A-Za-z0-9._-]{1,100}$/;
const LOGIN = /^[A-Za-z0-9_-]{1,39}$/;
const SESSION_ID = /^[A-Za-z0-9._-]{1,128}$/;
const REQUEST_ID = /^[A-Za-z0-9._:-]{1,128}$/;

/** A GitHub login snapshot, with the optional leading `@` stripped. */
export function parseLogin(value: string): string | null {
  const trimmed = value.trim().replace(/^@/, '');
  return LOGIN.test(trimmed) ? trimmed : null;
}

/** An exact `owner/name` pair; both halves must validate. */
export function parseRepoFullName(value: string): string | null {
  const trimmed = value.trim();
  const slash = trimmed.indexOf('/');
  if (slash <= 0) return null;
  const owner = trimmed.slice(0, slash);
  const name = trimmed.slice(slash + 1);
  return OWNER.test(owner) && REPO.test(name) ? `${owner}/${name}` : null;
}

export function parseSessionId(value: string): string | null {
  const trimmed = value.trim();
  return SESSION_ID.test(trimmed) ? trimmed : null;
}

export function parseRequestId(value: string): string | null {
  const trimmed = value.trim();
  return REQUEST_ID.test(trimmed) ? trimmed : null;
}

/** A positive integer, rejecting `1.5`, `1e3`, and every other numeric spelling
 *  that would round-trip differently through the query string. */
export function parsePositiveInt(value: string): number | null {
  const trimmed = value.trim();
  if (!/^[0-9]{1,15}$/.test(trimmed)) return null;
  const parsed = Number(trimmed);
  return parsed > 0 ? parsed : null;
}

/** An exact HTTP status, 100..=599. */
export function parseStatusCode(value: string): number | null {
  const parsed = parsePositiveInt(value);
  return parsed !== null && parsed >= 100 && parsed <= 599 ? parsed : null;
}

/** An RFC3339/ISO instant, as epoch-ms. */
export function parseInstant(value: string): number | null {
  const ms = Date.parse(value.trim());
  return Number.isFinite(ms) ? ms : null;
}

/** Whether a custom window is usable: ordered, non-empty, and within the
 *  deployment's ceiling. A window that fails this is never sent. */
export function isUsableRange(from: number | null, to: number | null): boolean {
  if (from === null || to === null) return false;
  return to > from && to - from <= MAX_RANGE_MS;
}

/** Whether a filter set names a window that can actually be queried. Only a
 *  `custom` preset can fail this — the presets are always well-formed. Checked
 *  without a clock so it can gate a render without making it time-dependent. */
export function hasUsableWindow(filters: ActivityFilters): boolean {
  return filters.preset !== 'custom' || isUsableRange(filters.from, filters.to);
}

/** The concrete `[from, to)` window a filter set means at instant `now`.
 *  `null` for a `custom` preset whose bounds are unusable — the caller must not
 *  issue a request in that state. */
export function resolveWindow(
  filters: ActivityFilters,
  now: number
): { from: number; to: number } | null {
  if (filters.preset === 'custom') {
    return isUsableRange(filters.from, filters.to)
      ? { from: filters.from as number, to: filters.to as number }
      : null;
  }
  return { from: now - PRESET_MS[filters.preset], to: now };
}

/** Whether a personal-scope caller may issue this activity query at all.
 *
 *  A regular caller asking for lifecycle rows must name ONE exact session, which
 *  the server then authorizes. Issuing the request without it would be asking
 *  for a global lifecycle scan the backend is guaranteed to refuse, so the UI
 *  refuses first — and says why — instead of turning a predictable `404` into a
 *  mysterious empty table. */
export function needsSessionId(
  filters: ActivityFilters,
  scope: ActivityScope | null
): boolean {
  return scope === 'mine' && filters.recordKind !== 'api_request' && filters.sessionId === null;
}
