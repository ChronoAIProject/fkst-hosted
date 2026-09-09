import type { HealthStatus, SessionHealth } from '@/lib/api/health';
import type { ChipTone } from './tones';

// The two rendering rules that decide what a reader sees at a glance. Pure and
// unit-tested, because getting either one wrong is actively misleading rather
// than merely ugly.

/** Chip tone per report status. */
export const HEALTH_TONE: Record<HealthStatus, ChipTone> = {
  working: 'green',
  idle: 'neutral',
  blocked: 'amber',
  stalled: 'red',
  failing: 'red',
  unknown: 'neutral',
};

/** What the header chip should render, or `null` for no chip at all. */
export type HealthChip =
  | { kind: 'status'; status: HealthStatus; tone: ChipTone }
  | { kind: 'stale'; tone: ChipTone };

/**
 * Decide the header chip.
 *
 * Three rules, in this order — each exists because the naive rendering is worse
 * than showing nothing:
 *
 * 1. **`stale` overrides the status.** A 35-minute-old `working` verdict is not
 *    evidence that the session is working; rendering it green would actively
 *    mislead. The chip becomes amber and says so.
 * 2. **`not_running` must never look like a fault.** A reaped pod is the normal
 *    end of a session's work, so the last known status renders NEUTRAL — never
 *    amber, never red, never the stale string. Getting this wrong puts a false
 *    alarm on every idle session in the dashboard, which is the exact failure the
 *    whole design exists to avoid.
 * 3. **`never_reported` renders no chip at all**, rather than an "unknown" chip,
 *    so a session whose first report has not landed yet does not look broken.
 */
export function healthChip(health: SessionHealth | null): HealthChip | null {
  if (!health) return null;
  const state = health.staleness?.state;
  const latest = health.latest ?? health.reports[0] ?? null;

  if (state === 'stale') return { kind: 'stale', tone: 'amber' };
  if (state === 'never_reported' || !latest) return null;
  if (state === 'not_running') return { kind: 'status', status: latest.status, tone: 'neutral' };
  return { kind: 'status', status: latest.status, tone: HEALTH_TONE[latest.status] ?? 'neutral' };
}

/** Should the prominent heartbeat callout render? ONLY for `stale` — the case
 *  where the runtime is live and its own reporting has stopped. Never for
 *  `not_running`, which is normal. */
export function showsStaleNotice(health: SessionHealth | null): boolean {
  return health?.staleness?.state === 'stale';
}

/** Whole minutes, rounded down, for the "N minutes" copy. `null` when the
 *  backend reported no number (it fails open rather than guessing). */
export function minutes(seconds: number | null | undefined): number | null {
  if (seconds == null || !Number.isFinite(seconds) || seconds < 0) return null;
  return Math.floor(seconds / 60);
}
