import type { ReactNode } from 'react';
import { useReducedMotion } from 'framer-motion';
import { Cell, Pie, PieChart, ResponsiveContainer, Tooltip } from 'recharts';
import { useContent } from '@/i18n';
import type { IssueDetail } from '@/lib/api/types';
import { decodeWorkItemStatus } from '@/lib/api/derive';
import { Note, SectionLabel } from './parts';

// At-a-glance Status-tab charts. The donut follows the dataviz method: a nominal
// breakdown of the session's work items into five semaphore-keyed groups, one
// hue per group (green done, amber in-progress, green-gold ready, red failed,
// ghost queued), with the count carried by an HTML legend + a centered total so
// the meaning never rides color alone. The progress meter is a plain gradient
// bar (done ÷ total) — a bar library would be overkill for a single ratio.
//
// Recharts cannot measure itself under jsdom, so both charts render their
// numbers/legend as real HTML overlaid on the SVG (not recharts <Label>s): the
// figure stays informative at any size and the values stay unit-testable.

// ---- Work-item counting (pure) ---------------------------------------------

/** The five at-a-glance groups a session's work items fold into. `inProgress`
 *  spans thinking + implementing + claimed; `queued` spans queued + other — no
 *  single decoded state names either group, so they are aggregated here. */
export interface WorkItemCounts {
  total: number;
  done: number;
  inProgress: number;
  ready: number;
  failed: number;
  queued: number;
}

/** Fold a work-issue list into the five overview groups. Pure over its input so
 *  the cards' numbers, the progress ratio, and the donut slices are all
 *  unit-testable without rendering recharts. */
export function countWorkItems(issues: IssueDetail[]): WorkItemCounts {
  const counts: WorkItemCounts = {
    total: 0,
    done: 0,
    inProgress: 0,
    ready: 0,
    failed: 0,
    queued: 0,
  };
  for (const issue of issues) {
    counts.total += 1;
    switch (decodeWorkItemStatus(issue).state) {
      case 'done':
        counts.done += 1;
        break;
      case 'ready':
        counts.ready += 1;
        break;
      case 'failed':
        counts.failed += 1;
        break;
      case 'thinking':
      case 'implementing':
      case 'claimed':
        counts.inProgress += 1;
        break;
      // 'queued' | 'other' — waiting / unrecognized both read as backlog.
      default:
        counts.queued += 1;
    }
  }
  return counts;
}

// ---- Shared card shell ------------------------------------------------------

/** A glass/grad-border overview card. Shared by the progress + donut charts and
 *  the lifecycle card so every tile in the overview grid frames identically. */
export function StatusCard({
  label,
  children,
  className,
  ...rest
}: {
  label: ReactNode;
  children: ReactNode;
  className?: string;
} & React.HTMLAttributes<HTMLElement>) {
  return (
    <section
      className={
        'grad-border rounded-card bg-glass backdrop-blur-glass shadow-[var(--shadow-2),var(--highlight-top)] p-3.5 flex flex-col gap-2.5 min-w-0' +
        (className ? ` ${className}` : '')
      }
      {...rest}
    >
      <SectionLabel>{label}</SectionLabel>
      {children}
    </section>
  );
}

// ---- Progress card ----------------------------------------------------------

const STAT_TONE = {
  amber: 'text-amber',
  green: 'text-green',
  red: 'text-red',
  ghost: 'text-ghost',
} as const;

/** A tinted count stat (label + value) used under the progress meter. The label
 *  stays muted; only the value takes the semaphore hue so a glance reads the
 *  numbers, not a wall of color. */
function Stat({ tone, label, value }: { tone: keyof typeof STAT_TONE; label: string; value: number }) {
  return (
    <span className="font-mono text-[10.5px] text-ghost">
      {label} <span className={`${STAT_TONE[tone]} font-semibold`}>{value}</span>
    </span>
  );
}

/** Big "{done} / {total}" headline + a gradient progress meter (done ÷ total) +
 *  the in-progress / ready / failed sub-counts. Zero work items reads as a
 *  0-length meter and the empty note (never a divide-by-zero). */
export function ProgressCard({ counts }: { counts: WorkItemCounts }) {
  const t = useContent().dashboard.detail;
  const pct = counts.total > 0 ? Math.round((counts.done / counts.total) * 100) : 0;

  return (
    <StatusCard label={t.overviewProgress} aria-label={t.overviewProgress}>
      <div className="flex items-baseline gap-1.5">
        <span className="font-display text-display-md grad-text-fg leading-none">{counts.done}</span>
        <span className="font-mono text-[13px] text-ghost">/ {counts.total}</span>
      </div>
      {/* Gradient meter: the accent fill tweens its width as counts advance
          (collapsed to an instant set under reduced motion via the global
          transition suppression). The track is a quiet raised hairline. */}
      <div
        className="h-2 rounded-full bg-raise-2 border border-line overflow-hidden"
        role="progressbar"
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div
          className="h-full rounded-full bg-grad-accent shadow-glow-amber transition-[width] duration-500"
          style={{ width: `${pct}%` }}
        />
      </div>
      {/* The "0 / 0" headline + empty meter already read as "nothing yet", so
          the zero case needs no note; only render the sub-counts once there is
          something to break down. */}
      {counts.total > 0 && (
        <div className="flex flex-wrap gap-x-3 gap-y-1">
          <Stat tone="amber" label={t.statInProgress} value={counts.inProgress} />
          <Stat tone="green" label={t.work.ready} value={counts.ready} />
          <Stat tone="red" label={t.work.failed} value={counts.failed} />
        </div>
      )}
    </StatusCard>
  );
}

// ---- Work-item distribution donut ------------------------------------------

/** One donut slice: a semaphore-keyed group with its localized label + count. */
interface DonutSlice {
  key: string;
  label: string;
  value: number;
  color: string;
}

/** Slice descriptors in a stable order. Done + ready both read green (per the
 *  design tokens) but ready is nudged toward gold so two adjacent green arcs
 *  stay distinguishable; the legend label carries the meaning regardless. */
function donutSlices(counts: WorkItemCounts, t: ReturnType<typeof useContent>['dashboard']['detail']): DonutSlice[] {
  return [
    { key: 'done', label: t.work.done, value: counts.done, color: 'var(--green)' },
    { key: 'inProgress', label: t.statInProgress, value: counts.inProgress, color: 'var(--amber)' },
    {
      key: 'ready',
      label: t.work.ready,
      value: counts.ready,
      color: 'color-mix(in oklab, var(--green) 60%, var(--gold))',
    },
    { key: 'failed', label: t.work.failed, value: counts.failed, color: 'var(--red)' },
    { key: 'queued', label: t.work.queued, value: counts.queued, color: 'var(--ghost)' },
  ];
}

/** Glass tooltip mirroring the sidebar charts: the hovered group's count + name
 *  on a frosted, softly-glowing surface. */
function DonutTooltip({
  active,
  payload,
}: {
  active?: boolean;
  payload?: { payload: DonutSlice }[];
}) {
  if (!active || !payload?.length) return null;
  const slice = payload[0]!.payload;
  return (
    <div className="grad-border rounded-card bg-glass backdrop-blur-glass px-2.5 py-1.5 flex items-baseline gap-2 shadow-[var(--shadow-2),var(--glow-amber)]">
      <span className="font-mono text-[12px] font-semibold grad-text-fg">{slice.value}</span>
      <span className="font-mono text-[10.5px] text-dim">{slice.label}</span>
    </div>
  );
}

/** A ring chart of the work-item distribution with a centered total + a compact
 *  legend (only non-empty groups appear). No work items shows a friendly note
 *  rather than a hollow ring. */
export function WorkDonut({ counts }: { counts: WorkItemCounts }) {
  const t = useContent().dashboard.detail;
  // The ring grow/sweep is decorative; suppressed entirely under reduced motion
  // so the arcs render at their final angle instantly.
  const reduce = useReducedMotion();
  const shown = donutSlices(counts, t).filter((s) => s.value > 0);

  return (
    <StatusCard label={t.overviewDistribution} aria-label={t.overviewDistribution}>
      {counts.total === 0 ? (
        <Note>{t.donutEmpty}</Note>
      ) : (
        <div className="flex items-center gap-3.5 min-w-0">
          {/* Fixed-size ring so the centered total anchors cleanly; the legend
              takes the remaining fluid width. */}
          <div className="relative flex-none" style={{ width: 104, height: 104 }}>
            <ResponsiveContainer width="100%" height="100%">
              <PieChart>
                <Pie
                  data={shown}
                  dataKey="value"
                  nameKey="label"
                  cx="50%"
                  cy="50%"
                  innerRadius="64%"
                  outerRadius="100%"
                  paddingAngle={shown.length > 1 ? 2 : 0}
                  stroke="none"
                  isAnimationActive={!reduce}
                  animationDuration={reduce ? 0 : 480}
                >
                  {shown.map((slice) => (
                    <Cell key={slice.key} fill={slice.color} />
                  ))}
                </Pie>
                <Tooltip content={<DonutTooltip />} />
              </PieChart>
            </ResponsiveContainer>
            {/* HTML overlay — renders regardless of the SVG's measured size, so
                the total is always legible and testable. */}
            <div className="absolute inset-0 flex flex-col items-center justify-center pointer-events-none">
              <span className="font-display text-[22px] leading-none grad-text-fg">{counts.total}</span>
              <span className="font-mono text-[9px] text-ghost uppercase tracking-[0.14em]">
                {t.donutTotalLabel}
              </span>
            </div>
          </div>
          <ul className="flex flex-col gap-1 min-w-0 flex-1">
            {shown.map((slice) => (
              <li
                key={slice.key}
                className="flex items-center gap-1.5 font-mono text-[10.5px] text-dim min-w-0"
              >
                <span
                  aria-hidden="true"
                  className="w-2 h-2 rounded-full flex-none"
                  style={{ background: slice.color }}
                />
                <span className="truncate min-w-0 flex-1">{slice.label}</span>
                <span className="text-fg font-semibold flex-none">{slice.value}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </StatusCard>
  );
}
