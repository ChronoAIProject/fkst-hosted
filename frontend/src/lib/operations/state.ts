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

export const DAY_MS = 86_400_000;

/** The ceiling assumed until a page states this deployment's own.
 *
 *  The real bound is `FKST_POSTHOG_ACTIVITY_MAX_RANGE_DAYS`, which every
 *  successful page reports as `max_range_days`; this is only the documented
 *  DEFAULT, used before the first response has arrived. Hard-coding it as if it
 *  were the bound is what makes a UI refuse windows a widened deployment would
 *  have answered — and issue windows a narrowed one is guaranteed to refuse. */
export const DEFAULT_MAX_RANGE_DAYS = 30;
const DEFAULT_MAX_RANGE_MS = DEFAULT_MAX_RANGE_DAYS * DAY_MS;

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

/**
 * Why a filter set's window cannot be queried, or `null` when it can.
 *
 * Each member mirrors one refusal in `backend/src/operations/filters.rs`
 * (`check_range`), so a window this returns `null` for is a window that
 * deployment's validator accepts:
 *
 * - `incomplete` — a `custom` preset with a bound still missing. The UI's own
 *   state, not a server rule: there is nothing to send yet.
 * - `unordered` — `from >= to` (`from must be strictly before to`).
 * - `too_wide` — wider than `max_range_days`.
 * - `future` — `from` after now. A window that has not happened can only ever be
 *   empty, and answering it with a confident empty page would be a lie.
 */
export type WindowProblem = 'incomplete' | 'unordered' | 'too_wide' | 'future';

/** The problem with the window a filter set names at instant `now`, if any.
 *  A preset can only ever be `too_wide` (against a deployment that configured a
 *  ceiling narrower than the preset); every other problem needs explicit bounds. */
export function windowProblem(
  filters: ActivityFilters,
  maxRangeMs: number = DEFAULT_MAX_RANGE_MS,
  now: number = Date.now()
): WindowProblem | null {
  if (filters.preset !== 'custom') {
    return PRESET_MS[filters.preset] > maxRangeMs ? 'too_wide' : null;
  }
  const { from, to } = filters;
  if (from === null || to === null) return 'incomplete';
  if (to <= from) return 'unordered';
  if (to - from > maxRangeMs) return 'too_wide';
  if (from > now) return 'future';
  return null;
}

/** Whether a filter set names a window that can actually be queried. */
export function hasUsableWindow(
  filters: ActivityFilters,
  maxRangeMs?: number,
  now?: number
): boolean {
  return windowProblem(filters, maxRangeMs, now) === null;
}

/** The concrete `[from, to)` window a filter set means at instant `now`.
 *  `null` whenever the window is unusable — the caller must not issue a request
 *  in that state. */
export function resolveWindow(
  filters: ActivityFilters,
  now: number,
  maxRangeMs?: number
): { from: number; to: number } | null {
  if (windowProblem(filters, maxRangeMs, now) !== null) return null;
  if (filters.preset === 'custom') {
    return { from: filters.from as number, to: filters.to as number };
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
