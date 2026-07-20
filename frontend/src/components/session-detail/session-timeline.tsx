import { useContent, useLang } from '@/i18n';
import { formatIsoSgt } from '@/lib/format';
import { decodeSessionStatus, type SessionPhase } from '@/lib/api/derive';
import type { SessionDetail } from '@/lib/api/types';
import { staggerStyle } from '@/components/ui/motion';
import { StatusCard } from './status-charts';

// Session timeline — a chronological rail built entirely from the trigger / work
// / PR lifecycle that SessionDetail exposes (all GitHub-derived):
//
//   Session started (trigger.created_at)
//     → work items queued / finished  (work_issues created_at + closed_at)
//     → pull requests opened / merged (prs)
//     → the current derived state     (running / idle-paused / retired / …).
//
// Timestamps render in SGT (Asia/Singapore) via `formatIsoSgt` — the product
// requirement — so every reader sees the same wall-clock regardless of locale.
//
// NOTE: this reflects only what GitHub exposes. Explicit pod pause↔restart run
// boundaries (when an idle session's pod was reaped and later revived) are NOT
// surfaced by the API yet — that is a backend addition tracked separately — so
// this timeline deliberately does NOT fabricate pause/restart events it has no
// data for. `PrDetail` also carries no timestamps on the wire, so PR nodes show
// their state WITHOUT an absolute time (an honest omission, not a missing value).

/** Visual tone of a node's dot — maps onto the design semaphore tokens. */
export type NodeTone = 'start' | 'good' | 'progress' | 'bad' | 'neutral' | 'live';

/** The discrete kinds of timeline node, so the label mapping stays i18n-driven
 *  in the component while the builder stays a pure, testable classifier. */
export type TimelineKind =
  | 'started'
  | 'work-queued'
  | 'work-finished'
  | 'pr-opened'
  | 'pr-merged'
  | 'pr-closed'
  | 'now';

export interface TimelineNode {
  /** Stable React key. */
  key: string;
  kind: TimelineKind;
  /** Issue / PR reference for a work or PR node (absent for started / now). */
  ref?: { number: number; title: string };
  /** Raw ISO timestamp for a timestamped node; null when the source carries
   *  none (PR nodes, the "now" node). */
  iso: string | null;
  /** Present only on the `now` node — the current derived phase. */
  phase?: SessionPhase;
  tone: NodeTone;
  /** Sort key: real epoch-ms for timestamped nodes, high sentinels for the
   *  untimed PR nodes (before "now") and the terminal "now" node (always last). */
  sortMs: number;
}

/** Untimed PR nodes sort after every real timestamp but before the "now" node. */
const PR_TIER = Number.MAX_SAFE_INTEGER - 1;
/** The terminal "current state" node always sorts last. */
const NOW_TIER = Number.MAX_SAFE_INTEGER;

/** Epoch-ms of an ISO string, or `fallback` when absent/unparseable — so a bad
 *  timestamp degrades its ordering gracefully instead of poisoning the sort. */
function msOf(iso: string | null, fallback: number): number {
  if (!iso) return fallback;
  const ms = Date.parse(iso);
  return Number.isFinite(ms) ? ms : fallback;
}

/** Build the ordered timeline nodes from a session's GitHub-derived lifecycle.
 *  Pure over its input (no i18n / formatting), so the ordering and node kinds
 *  are unit-testable without rendering. */
export function buildTimeline(session: SessionDetail): TimelineNode[] {
  const nodes: TimelineNode[] = [];

  // 1. Session started — the trigger issue's creation anchors the timeline.
  nodes.push({
    key: 'started',
    kind: 'started',
    iso: session.trigger.created_at,
    tone: 'start',
    sortMs: msOf(session.trigger.created_at, Number.NEGATIVE_INFINITY),
  });

  // 2. Each work item: a queued moment (creation) and, if closed, a finished
  //    moment (its close) — interleaved chronologically with everything else.
  for (const issue of session.work_issues) {
    const ref = { number: issue.number, title: issue.title };
    nodes.push({
      key: `work-${issue.number}-queued`,
      kind: 'work-queued',
      ref,
      iso: issue.created_at,
      tone: 'neutral',
      sortMs: msOf(issue.created_at, 0),
    });
    if (issue.state === 'closed' && issue.closed_at) {
      nodes.push({
        key: `work-${issue.number}-finished`,
        kind: 'work-finished',
        ref,
        iso: issue.closed_at,
        tone: 'good',
        sortMs: msOf(issue.closed_at, 0),
      });
    }
  }

  // 3. Pull requests — no wire timestamps, so they park just before "now",
  //    ordered by number, carrying their merged/open/closed state.
  session.prs.forEach((pr, i) => {
    const kind: TimelineKind = pr.merged ? 'pr-merged' : pr.state === 'open' ? 'pr-opened' : 'pr-closed';
    nodes.push({
      key: `pr-${pr.number}`,
      kind,
      ref: { number: pr.number, title: pr.title },
      iso: null,
      tone: pr.merged ? 'good' : pr.state === 'open' ? 'progress' : 'neutral',
      sortMs: PR_TIER + i,
    });
  });

  // 4. The current derived state — the terminal node, always last.
  const phase = decodeSessionStatus(session).phase;
  nodes.push({
    key: 'now',
    kind: 'now',
    iso: null,
    phase,
    tone: nowTone(phase),
    sortMs: NOW_TIER,
  });

  // Stable ascending sort keeps ties (and untimed nodes) in insertion order.
  return nodes.sort((a, b) => a.sortMs - b.sortMs);
}

/** Dot tone for the terminal "now" node, from the derived phase. */
function nowTone(phase: SessionPhase): NodeTone {
  switch (phase) {
    case 'active':
      return 'live';
    case 'degraded':
    case 'invalid':
      return 'bad';
    case 'retired':
    case 'idle':
      return 'neutral';
    default:
      return 'progress';
  }
}

/** Dot color + decorative glow per tone (color reinforces, never carries, the
 *  meaning — the label text always states it). */
const TONE_STYLE: Record<NodeTone, { color: string; cls: string }> = {
  start: { color: 'var(--amber)', cls: 'shadow-glow-amber' },
  good: { color: 'var(--green)', cls: 'shadow-glow-green' },
  progress: { color: 'var(--amber)', cls: 'shadow-glow-amber' },
  bad: { color: 'var(--red)', cls: 'shadow-glow-red' },
  neutral: { color: 'var(--ghost)', cls: '' },
  // The live "now" dot blinks (opacity only — color-agnostic, reduced-motion
  // safe: index.css disables .anim-dot-blink under prefers-reduced-motion).
  live: { color: 'var(--green)', cls: 'shadow-glow-green anim-dot-blink' },
};

/** Session timeline card: a vertical status-colored rail with a dot per
 *  lifecycle moment, entrance-staggered (reduced-motion-safe via `.anim-row-in`).
 *  Framed as one full-width tile matching the overview grid's cards. */
export function SessionTimeline({ session }: { session: SessionDetail }) {
  const t = useContent().dashboard.detail;
  const { lang } = useLang();
  const nodes = buildTimeline(session);

  const label = (node: TimelineNode): string => {
    switch (node.kind) {
      case 'started':
        return t.timelineStarted;
      case 'work-queued':
        return t.timelineWorkQueued;
      case 'work-finished':
        return t.timelineWorkFinished;
      case 'pr-opened':
        return t.timelinePrOpened;
      case 'pr-merged':
        return t.timelinePrMerged;
      case 'pr-closed':
        return t.timelinePrClosed;
      case 'now':
        // "Now — <phase>": the terminal state, coherent with the header pill.
        return `${t.timelineNow} — ${t.phase[node.phase ?? 'registered']}`;
    }
  };

  return (
    <StatusCard label={t.timeline}>
      <ol className="flex flex-col">
        {nodes.map((node, i) => {
          const tone = TONE_STYLE[node.tone];
          const last = i === nodes.length - 1;
          const time = formatIsoSgt(node.iso, lang);
          return (
            <li
              key={node.key}
              className="anim-row-in relative flex gap-3 min-w-0"
              style={staggerStyle(i)}
            >
              {/* Rail column: the status dot + the connector down to the next
                  node (omitted on the last node). */}
              <div className="flex flex-col items-center flex-none">
                <span
                  aria-hidden="true"
                  className={`w-2.5 h-2.5 rounded-full flex-none mt-1 ${tone.cls}`}
                  style={{ background: tone.color }}
                />
                {!last && (
                  <span aria-hidden="true" className="w-px flex-1 min-h-4 bg-line mt-1" />
                )}
              </div>
              {/* Content: kind label, the #N — title reference, and the SGT time. */}
              <div className="flex flex-col gap-0.5 min-w-0 pb-3.5">
                <span className="text-fg text-[12.5px] font-medium leading-tight">{label(node)}</span>
                {node.ref && (
                  <span className="text-dim text-[11.5px] truncate min-w-0">
                    <span className="font-mono text-ghost">#{node.ref.number}</span> {node.ref.title}
                  </span>
                )}
                {time && <span className="font-mono text-[10.5px] text-ghost">{time}</span>}
              </div>
            </li>
          );
        })}
      </ol>
    </StatusCard>
  );
}
