import type { CSSProperties } from 'react';
import { useContent } from '@/i18n';
import { Chip } from '@/components/ui/chip';
import type {
  SessionDetail,
  SessionRecoveryProjection,
  SessionRecoveryState,
} from '@/lib/api/types';
import { decodeSessionStatus, type SessionPhase } from '@/lib/api/derive';
import { SplitPanes } from './parts';
import { fallbackRecovery } from './recovery-state';
import { SessionTimeline } from './session-timeline';
import { WorkItemsPane } from './work-items';
import { PHASE_TONE } from './tones';
import { ProgressCard, StatusCard, WorkDonut, countWorkItems } from './status-charts';

/** The happy-path lifecycle stages, in order. Off-path phases (degraded /
 *  invalid / picked-up) still surface as the prominent pill above; an idle
 *  (paused) session rests at the 'active' stage (see the paused rendering). */
const STAGES: SessionPhase[] = ['registered', 'active', 'retired'];

function stageReached(stage: SessionPhase, phase: SessionPhase, liveness: string | null): boolean {
  // `idle` counts as advanced: a paused session ran at least once, so it has
  // moved past 'registered' and rests between 'active' and 'retired'.
  const advanced =
    phase === 'active' ||
    phase === 'picked-up' ||
    phase === 'recovering' ||
    phase === 'degraded' ||
    phase === 'retired' ||
    phase === 'idle' ||
    liveness != null;
  if (stage === 'registered') return true;
  if (stage === 'active') return advanced;
  return phase === 'retired';
}

/** Lifecycle card: the decoded phase pill + health + the Registered→Active→
 *  Retired stage strip (unchanged stage-dot styling/animation), framed as one
 *  tile in the overview grid. */
function LifecycleCard({ session }: { session: SessionDetail }) {
  const t = useContent().dashboard.detail;
  const status = decodeSessionStatus(session);
  return (
    <StatusCard label={t.lifecycle}>
      <div className="flex items-center gap-1.5 flex-wrap">
        {/* Chips pop in on mount (anim-chip-in): the phase pill always, and the
            liveness chip whenever it (re)appears on a fresh snapshot. Chip has
            no className slot, so the entrance rides a wrapper span. */}
        <span className="anim-chip-in inline-flex">
          <Chip tone={PHASE_TONE[status.phase]}>{t.phase[status.phase]}</Chip>
        </span>
        <span className="font-mono text-[10.5px] text-ghost">
          {t.healthLabel}:{' '}
          <span
            className={
              status.health === 'degraded'
                ? 'text-red'
                : status.health === 'recovering'
                  ? 'text-amber'
                  : 'text-dim'
            }
          >
            {t.health[status.health]}
          </span>
        </span>
        {status.liveness && (
          <span className="anim-chip-in inline-flex">
            <Chip tone={status.liveness === 'live' ? 'green' : 'neutral'}>{status.liveness}</Chip>
          </span>
        )}
      </div>
      {/* Lifecycle stage strip. */}
      <ol className="flex items-center gap-2 mt-0.5">
        {STAGES.map((stage) => {
          const reached = stageReached(stage, status.phase, status.liveness);
          const current = stage === status.phase;
          // An idle session rests at the 'active' stage: it ran but its pod was
          // paused (reaped for lack of work). Render it as a distinct "paused"
          // node — not completed (green) nor currently-live (pulsing amber).
          const paused = status.phase === 'idle' && stage === 'active';
          return (
            <li key={stage} className="flex items-center gap-1.5">
              <span
                aria-hidden="true"
                className={
                  // transition-colors tweens the fill as a stage advances, so the
                  // strip animates a stage lighting up rather than snapping. The
                  // current stage breathes an amber glow (anim-glow-pulse);
                  // reached stages carry a static green bloom; a paused stage is a
                  // quiet ghost dot inside an amber ring. All collapse to their
                  // resting fill under prefers-reduced-motion via the global
                  // suppression.
                  'w-2 h-2 rounded-full flex-none transition-colors ' +
                  (current
                    ? 'bg-amber anim-glow-pulse'
                    : paused
                      ? 'bg-ghost ring-2 ring-[color-mix(in_oklab,var(--amber)_45%,transparent)]'
                      : reached
                        ? 'bg-green shadow-glow-green'
                        : 'bg-ghost')
                }
              />
              <span
                className={
                  'font-mono text-[10.5px] ' +
                  (reached || current || paused ? 'text-dim' : 'text-ghost')
                }
              >
                {paused ? t.stagePaused : t.phase[stage]}
              </span>
            </li>
          );
        })}
      </ol>
    </StatusCard>
  );
}

const RECOVERY_TONE: Record<SessionRecoveryState, 'neutral' | 'amber' | 'green' | 'red'> = {
  normal: 'green',
  idle: 'neutral',
  recovering: 'amber',
  degraded: 'red',
  unknown: 'neutral',
  retired: 'neutral',
  invalid: 'red',
};

/** Bounded operator read model. It deliberately renders enum-backed labels only:
 * provider errors and private issue content never enter this surface. */
function RecoveryCard({ recovery }: { recovery: SessionRecoveryProjection }) {
  const t = useContent().dashboard.detail;
  return (
    <StatusCard label={t.recoveryDiagnostics} aria-label={t.recoveryDiagnostics}>
      <div className="flex items-center gap-2 min-w-0">
        <span className="anim-chip-in inline-flex flex-none">
          <Chip tone={RECOVERY_TONE[recovery.state]}>{t.recoveryState[recovery.state]}</Chip>
        </span>
        <p className="text-[11.5px] text-dim leading-relaxed min-w-0">
          {t.recoveryReason[recovery.reason]}
        </p>
      </div>
      <dl className="grid grid-cols-2 gap-x-3 gap-y-1 font-mono text-[10.5px]">
        <div className="min-w-0">
          <dt className="text-ghost">{t.recoveryOpenWork}</dt>
          <dd className="text-fg tabular-nums">{recovery.open_work_items}</dd>
        </div>
        <div className="min-w-0">
          <dt className="text-ghost">{t.recoveryRuntime}</dt>
          <dd className="text-fg truncate">{t.runtimeState[recovery.runtime]}</dd>
        </div>
      </dl>
    </StatusCard>
  );
}

/** Status tab: where the session is in its LIFECYCLE — an at-a-glance overview
 *  grid (progress meter, work-item distribution donut, lifecycle, recovery
 *  diagnostics), a chronological session timeline, and the promoted per-work-item
 *  list. All of it derives from data already in hand, so opening this tab costs
 *  no request; live runtime observation is the Engine tab's job (#5841). Fills
 *  the wide detail panel: the overview grid is CSS auto-fit so it lays 2–3 tiles
 *  wide and stacks when narrow; the work items flow into two columns on wider
 *  viewports. */
export function TabStatus({ session }: { session: SessionDetail }) {
  const counts = countWorkItems(session.work_issues);
  const recovery = session.recovery ?? fallbackRecovery(session);
  // Inline auto-fit template: the tiles size themselves to the panel width
  // (container-driven), unlike Tailwind's viewport breakpoints — so the grid
  // reflows correctly inside the fluid detail panel, not just at page widths.
  const overviewGrid: CSSProperties = {
    gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))',
  };

  return (
    <div className="flex flex-col gap-5 md:h-full md:min-h-0">
      {/* The overview band is session-level and sized by its own content; it must
          not be squeezed by the split below it. */}
      <section className="grid gap-3 flex-none" style={overviewGrid}>
        <ProgressCard counts={counts} />
        <WorkDonut counts={counts} />
        <LifecycleCard session={session} />
        <RecoveryCard recovery={recovery} />
      </section>

      {/* Timeline ‖ work items. The timeline narrates what happened to the very
          items listed beside it, so reading them together is the point — stacked,
          the list starts below the fold of the thing that refers to it.

          Peer panes, so the first track is `minmax(0,1fr)` rather than a fixed
          rail; the work items take the wider share because their rows carry a
          number, a title and a chip. The height comes from the panel (md:h-full
          on the root → md:flex-1 here), so each pane scrolls its OWN content and
          neither can scroll the other away.

          `md:min-h-[16rem]` is the graceful-degradation floor, not decoration: if
          the overview band leaves less than that, the root overflows and the
          tab's own scroller takes over, instead of flex-1 collapsing both panes
          to nothing. */}
      <SplitPanes
        className="md:min-h-[16rem]"
        startTrack="minmax(0,1fr)"
        start={<SessionTimeline session={session} className="min-h-0" />}
        end={<WorkItemsPane issues={session.work_issues} className="min-h-0" />}
      />
    </div>
  );
}
