// Display projections for operations rows.
//
// Every function here is pure and total, and every one of them has one job:
// render a server FACT without inventing one. The edge cases are the point.
//
// - A `null` maximum lifetime means the deployment configured NO ceiling. It
//   renders as "Unlimited" and never as `0s remaining`, because those are
//   opposite facts.
// - A `null` restart count means the backend has no restart concept at all. It
//   renders as "Not reported" and never as `0`, because `0` claims a
//   measurement that was never taken.
// - A missing creator is never guessed from a repository owner, a trigger
//   author, or anything else — the caller renders the attribution-state label.
// - `backend_location` is a namespace or a bounded server label. Anything that
//   still looks like a URL is reduced to its authority, because the epic forbids
//   rendering a raw URL anywhere in this surface.
//
// Localized words are supplied BY the caller (from the i18n catalog); this
// module returns numbers, symbols, and already-safe strings only.

/** The dash rendered where a value legitimately does not exist. */
export const EMPTY_VALUE = '—';

/** Seconds as a compact `1d 2h`, `3h 04m`, `5m 06s`, `42s` duration. Negative and
 *  non-finite inputs collapse to `0s`: a clock-skewed snapshot must not render a
 *  negative age. */
export function formatDurationSeconds(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return '0s';
  const total = Math.floor(seconds);
  const d = Math.floor(total / 86400);
  const h = Math.floor((total % 86400) / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${String(m).padStart(2, '0')}m`;
  if (m > 0) return `${m}m ${String(s).padStart(2, '0')}s`;
  return `${s}s`;
}

/** A request duration. Sub-second values keep millisecond precision, because a
 *  12ms call and a 900ms call are different stories and `0s` tells neither. */
export function formatDurationMs(ms: number | null | undefined): string | null {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return null;
  if (ms < 1000) return `${Math.round(ms)}ms`;
  return formatDurationSeconds(ms / 1000);
}

/** Seconds elapsed between `from` (RFC3339) and `now` (epoch-ms), or `null` when
 *  the timestamp is absent or unparseable. */
export function elapsedSeconds(from: string | null | undefined, now: number): number | null {
  if (!from) return null;
  const ms = Date.parse(from);
  if (!Number.isFinite(ms)) return null;
  return Math.max(0, (now - ms) / 1000);
}

/** Seconds remaining until `until` (RFC3339) at `now`, clamped at zero. */
export function remainingSeconds(until: string | null | undefined, now: number): number | null {
  if (!until) return null;
  const ms = Date.parse(until);
  if (!Number.isFinite(ms)) return null;
  return Math.max(0, (ms - now) / 1000);
}

/** How a runtime's configured lifetime should read.
 *
 *  `unlimited` is a first-class outcome, distinct from `0` remaining: the former
 *  means no ceiling was configured, the latter means the ceiling has arrived. */
export type LifetimeDisplay =
  | { kind: 'unlimited' }
  | { kind: 'bounded'; maxSeconds: number; remaining: number | null; expiresAt: string | null };

export function lifetimeDisplay(
  maxLifetimeSeconds: number | null,
  expiresAt: string | null,
  fallbackRemaining: number | null,
  now: number
): LifetimeDisplay {
  if (maxLifetimeSeconds === null) return { kind: 'unlimited' };
  // The live clock is preferred over the snapshot's own figure so a countdown
  // keeps moving between polls; the snapshot's value is the fallback for a
  // runtime whose expiry instant the backend could not compute.
  const live = remainingSeconds(expiresAt, now);
  return {
    kind: 'bounded',
    maxSeconds: maxLifetimeSeconds,
    remaining: live ?? fallbackRemaining,
    expiresAt,
  };
}

/** Reduce a backend location to something that can never read as a URL: an
 *  authority (`host:port`) or a bare namespace. Query strings, paths, and
 *  userinfo are already stripped server-side; this is the client-side belt. */
export function displayLocation(value: string | null | undefined): string | null {
  if (!value) return null;
  const withoutScheme = value.replace(/^[A-Za-z][A-Za-z0-9+.-]*:\/\//, '');
  const authority = withoutScheme.split('/')[0] ?? '';
  const withoutUserinfo = authority.includes('@')
    ? authority.slice(authority.lastIndexOf('@') + 1)
    : authority;
  return withoutUserinfo || null;
}

/** The longest safe-argument summary a table cell will hold. */
const SUMMARY_MAX = 140;
/** The longest single value inside that summary. */
const SUMMARY_VALUE_MAX = 32;

/** One deterministic, bounded `key=value` summary of an operation's already
 *  allowlisted safe arguments.
 *
 *  Deterministic because the keys are sorted: the same record renders the same
 *  cell on every poll, so a column can never resize under a refresh. Bounded
 *  because a table cell is not a document — the full structured values live in
 *  the row details, where they are rendered as discrete fields rather than
 *  concatenated text. */
export function summarizeArguments(args: Record<string, unknown>): string {
  const parts: string[] = [];
  for (const key of Object.keys(args).sort()) {
    parts.push(`${key}=${summarizeValue(args[key])}`);
  }
  const joined = parts.join(' ');
  return joined.length > SUMMARY_MAX ? `${joined.slice(0, SUMMARY_MAX - 1)}…` : joined;
}

/** One argument value, reduced to a bounded scalar rendering. Containers report
 *  their SIZE rather than their contents — the details view shows those. */
export function summarizeValue(value: unknown): string {
  if (value === null || value === undefined) return 'null';
  if (Array.isArray(value)) return `[${value.length}]`;
  if (typeof value === 'object') return `{${Object.keys(value as object).length}}`;
  const text = String(value);
  return text.length > SUMMARY_VALUE_MAX ? `${text.slice(0, SUMMARY_VALUE_MAX - 1)}…` : text;
}

/** Flatten safe arguments into ordered `label`/`value` pairs for the details
 *  panel. One level deep: nested containers are rendered as compact JSON, which
 *  is safe because every value reaching here already passed the server's
 *  allowlist. */
export function argumentEntries(
  args: Record<string, unknown>
): Array<{ key: string; value: string }> {
  return Object.keys(args)
    .sort()
    .map((key) => {
      const raw = args[key];
      const value =
        raw === null || raw === undefined
          ? 'null'
          : typeof raw === 'object'
            ? JSON.stringify(raw)
            : String(raw);
      return { key, value };
    });
}

/** The tone a normalized sandbox status carries. Colour is reinforcement only —
 *  every status also renders its localized text label. */
export function sandboxTone(status: string): 'neutral' | 'amber' | 'green' | 'red' {
  switch (status) {
    case 'running':
    case 'succeeded':
      return 'green';
    case 'failed':
      return 'red';
    case 'pending':
    case 'transitioning':
    case 'terminating':
    case 'paused':
      return 'amber';
    default:
      return 'neutral';
  }
}

/** The tone an outcome carries, on the same colour-plus-text rule. */
export function outcomeTone(outcome: string): 'neutral' | 'amber' | 'green' | 'red' {
  switch (outcome) {
    case 'success':
      return 'green';
    case 'server_error':
    case 'timeout':
      return 'red';
    case 'client_error':
    case 'rejected':
    case 'incomplete':
      return 'amber';
    default:
      return 'neutral';
  }
}

/** The tone a delivery state carries. `verified_in_posthog` is the only state
 *  that means "durably query-visible"; everything else is in flight or lost. */
export function deliveryTone(state: string): 'neutral' | 'amber' | 'green' | 'red' {
  switch (state) {
    case 'verified_in_posthog':
      return 'green';
    case 'dead_letter':
      return 'red';
    case 'queued':
    case 'incomplete':
    case 'accepted_pending_verification':
      return 'amber';
    default:
      return 'neutral';
  }
}
