import { cn } from '@/lib/utils';
import type { WorkflowsContent } from '@/i18n/workflows-types';
import type { RunStatus, ScheduleLifecycle, StepStatus } from '@/lib/api/schedules';

/**
 * The small repeated pieces of the workflows workspace, plus the two pure
 * formatters the list and the detail both need.
 *
 * The formatters are exported and unit-tested rather than inlined because they
 * are the only place this surface does arithmetic on a time, and the rule is
 * narrow: they may only render a distance the API already committed to. They
 * never COMPUTE a firing — that stays server-side so the dashboard and the clock
 * cannot disagree.
 */

/** A value that legitimately does not exist, rendered as a fact rather than as
 *  an empty cell (which reads as a rendering bug). */
export function Absent({ label }: { label: string }) {
  return (
    <span aria-hidden="true" className="text-ghost">
      {label}
    </span>
  );
}

/** How far away `iso` is, as a coarse human distance.
 *
 * Coarse on purpose: a live-ticking seconds counter would re-render the whole
 * list every second to tell an operator something they cannot act on. Minutes
 * are the finest useful resolution for a schedule whose minimum cadence is
 * fifteen of them.
 */
export function relativeTo(
  iso: string | null,
  now: number,
  t: Pick<WorkflowsContent, 'inDays' | 'inHours' | 'inMinutes' | 'imminent' | 'overdue' | 'never'>
): string {
  if (!iso) return t.never;
  const at = Date.parse(iso);
  if (Number.isNaN(at)) return t.never;
  const seconds = Math.round((at - now) / 1000);
  // A slot that has passed without a run is not "in -3 minutes": the clock is
  // late, and saying so is more honest than rendering a negative distance.
  if (seconds < -60) return t.overdue;
  if (seconds < 60) return t.imminent;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return t.inMinutes.replace('{m}', String(minutes));
  const hours = Math.round(minutes / 60);
  if (hours < 48) return t.inHours.replace('{h}', String(hours));
  return t.inDays.replace('{d}', String(Math.round(hours / 24)));
}

/** A duration in seconds, rendered compactly. */
export function formatDuration(seconds: number | null, absent: string): string {
  if (seconds === null || seconds < 0) return absent;
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/** Tone per lifecycle. `invalid` is the only one that must be impossible to
 *  overlook — a schedule that has silently stopped is the failure this surface
 *  exists to prevent. */
const LIFECYCLE_TONE: Record<ScheduleLifecycle, string> = {
  idle: 'text-dim border-line',
  running: 'text-green border-green',
  paused: 'text-warn border-warn',
  invalid: 'text-red border-red',
};

const RUN_TONE: Record<RunStatus, string> = {
  dispatched: 'text-dim',
  ok: 'text-green',
  failed: 'text-red',
  timeout: 'text-red',
  'skipped-overlap': 'text-ghost',
};

const STEP_TONE: Record<StepStatus, string> = {
  ok: 'text-green',
  failed: 'text-red',
  skipped: 'text-ghost',
};

export function LifecycleBadge({
  state,
  label,
}: {
  state: ScheduleLifecycle;
  label: string;
}) {
  return (
    <span
      data-testid={`lifecycle-${state}`}
      className={cn(
        'inline-flex items-center rounded-chip border px-2 py-[2px] font-ui text-[11.5px]',
        LIFECYCLE_TONE[state]
      )}
    >
      {label}
    </span>
  );
}

export function RunStatusPill({ status, label }: { status: RunStatus; label: string }) {
  return (
    <span
      data-testid={`run-status-${status}`}
      className={cn('font-ui text-[12px]', RUN_TONE[status])}
    >
      {label}
    </span>
  );
}

export function StepStatusMark({ status, label }: { status: StepStatus; label: string }) {
  return (
    <span data-testid={`step-status-${status}`} className={cn('font-ui text-[12px]', STEP_TONE[status])}>
      {label}
    </span>
  );
}

/** The 30-day success meter. `null` renders as absent rather than as 0%: "no
 *  attempts yet" and "every attempt failed" are opposite facts. */
export function SuccessMeter({ rate, absent }: { rate: number | null; absent: string }) {
  if (rate === null) return <Absent label={absent} />;
  const percent = Math.round(rate * 100);
  return (
    <span className="inline-flex items-center gap-2" data-testid="success-meter">
      <span aria-hidden="true" className="h-[6px] w-[52px] rounded-chip bg-line">
        <span
          className={cn('block h-[6px] rounded-chip', percent >= 80 ? 'bg-green' : 'bg-warn')}
          style={{ width: `${percent}%` }}
        />
      </span>
      <span className="font-mono text-[11.5px] text-dim">{percent}%</span>
    </span>
  );
}
