import type { CSSProperties, ReactNode } from 'react';
import { useContent } from '@/i18n';
import { Chip } from '@/components/ui/chip';
import { FadeSwap } from '@/components/ui/motion';
import type { IssueDetail, SessionDetail } from '@/lib/api/types';
import {
  decodeSessionStatus,
  decodeWorkItemStatus,
  type SessionPhase,
  type WorkItemTone,
} from '@/lib/api/derive';
import { Note, SectionLabel, Spinner } from './parts';
import { ObserveView } from './observe-view';
import { SessionTimeline } from './session-timeline';
import { PHASE_TONE, WORK_TONE } from './tones';
import type { ObserveState } from './observe-state';
import { ProgressCard, StatusCard, WorkDonut, countWorkItems } from './status-charts';

/** The happy-path lifecycle stages, in order. Off-path phases (degraded /
 *  invalid / picked-up) still surface as the prominent pill above; an idle
 *  (paused) session rests at the 'active' stage (see the paused rendering). */
const STAGES: SessionPhase[] = ['registered', 'active', 'retired'];

/** CSS-var accent color for a work-item tone. Container-agnostic so a row's left
 *  rule reads the same whether the row sits in a 1- or 2-column grid. */
const ACCENT: Record<WorkItemTone, string> = {
  good: 'var(--green)',
  progress: 'var(--amber)',
  bad: 'var(--red)',
  neutral: 'var(--ghost)',
};

function stageReached(stage: SessionPhase, phase: SessionPhase, liveness: string | null): boolean {
  // `idle` counts as advanced: a paused session ran at least once, so it has
  // moved past 'registered' and rests between 'active' and 'retired'.
  const advanced =
    phase === 'active' ||
    phase === 'picked-up' ||
    phase === 'degraded' ||
    phase === 'retired' ||
    phase === 'idle' ||
    liveness != null;
  if (stage === 'registered') return true;
  if (stage === 'active') return advanced;
  return phase === 'retired';
}

/** One promoted work-item card: a status-colored left accent + the #number link,
 *  the (truncated) title, and the decoded state chip. Laid out to sit in a
 *  responsive 1-/2-column grid. */
function WorkItemRow({ issue }: { issue: IssueDetail }) {
  const t = useContent().dashboard.detail;
  const decoded = decodeWorkItemStatus(issue);
  return (
    <div className="relative flex items-center gap-2 rounded-chip bg-glass-2 border border-line pl-3.5 pr-2.5 py-2 min-w-0 overflow-hidden shadow-1">
      {/* Status-matched left rule — decorative reinforcement of the chip. */}
      <span
        aria-hidden="true"
        className="absolute left-0 top-0 bottom-0 w-1"
        style={{ background: ACCENT[decoded.tone] }}
      />
      <a
        href={issue.html_url}
        target="_blank"
        rel="noreferrer"
        className="hover-underline font-mono text-[11px] text-ghost hover:text-amber transition-colors flex-none"
      >
        #{issue.number}
      </a>
      <span className="text-fg text-[12.5px] truncate min-w-0 flex-1">{issue.title}</span>
      <Chip tone={WORK_TONE[decoded.tone]}>{t.work[decoded.state]}</Chip>
    </div>
  );
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
          <span className={status.health === 'degraded' ? 'text-red' : 'text-dim'}>
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

/** Status tab: an at-a-glance overview grid (progress meter, work-item
 *  distribution donut, lifecycle), a chronological session timeline, the
 *  promoted per-work-item list, and an on-demand "Live engine details" fetch —
 *  the fetch is only offered while the pod is LIVE (`liveness === 'live'`); when
 *  the session is paused/idle it shows a calm note instead. Fills the wide detail
 *  panel: the overview grid is CSS auto-fit so it lays 2–3 tiles wide and stacks
 *  when narrow; the work items flow into two columns on wider viewports. */
export function TabStatus({
  session,
  observe,
  onLoadObserve,
}: {
  session: SessionDetail;
  observe: ObserveState;
  onLoadObserve: () => void;
}) {
  const t = useContent().dashboard.detail;
  const counts = countWorkItems(session.work_issues);
  // The live-engine observe fetch pod-execs INTO the running pod, so it is only
  // meaningful — and only permitted — while the pod is live. A latched active
  // label whose pod has been reaped is NOT live (see decodeSessionStatus).
  const isLive = session.liveness === 'live';

  // Hard gate: never let the observe fetch fire unless the pod is live, even if
  // a stray caller reaches the handler.
  const handleLoadObserve = () => {
    if (isLive) onLoadObserve();
  };

  // Inline auto-fit template: the tiles size themselves to the panel width
  // (container-driven), unlike Tailwind's viewport breakpoints — so the grid
  // reflows correctly inside the fluid detail panel, not just at page widths.
  const overviewGrid: CSSProperties = {
    gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))',
  };

  return (
    <div className="flex flex-col gap-5">
      <section className="grid gap-3" style={overviewGrid}>
        <ProgressCard counts={counts} />
        <WorkDonut counts={counts} />
        <LifecycleCard session={session} />
      </section>

      <SessionTimeline session={session} />

      <section className="flex flex-col gap-2">
        <SectionLabel>
          {t.workItems}
          {session.work_issues.length > 0 && (
            <span className="ml-2 lowercase">· {session.work_issues.length}</span>
          )}
        </SectionLabel>
        {session.work_issues.length === 0 ? (
          <Note>{t.noWorkItems}</Note>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-2 gap-2">
            {session.work_issues.map((issue) => (
              <WorkItemRow key={issue.number} issue={issue} />
            ))}
          </div>
        )}
      </section>

      <section className="flex flex-col gap-2">
        <SectionLabel>{t.liveEngine}</SectionLabel>
        {isLive ? (
          // Crossfade the observe states keyed on `status`: the fetched engine
          // snapshot slides in under the label as loading resolves to loaded,
          // rather than popping the panel in. Instant under reduced motion.
          <FadeSwap k={observe.status}>{renderObserve()}</FadeSwap>
        ) : (
          // Paused/idle: the pod is gone, so there is nothing to observe. Explain
          // it calmly instead of offering a fetch that would only error.
          <Note>{t.liveEnginePaused}</Note>
        )}
      </section>
    </div>
  );

  function renderObserve(): ReactNode {
    switch (observe.status) {
      case 'idle':
        return (
          <button
            type="button"
            onClick={handleLoadObserve}
            className="self-start font-ui font-semibold text-[12px] border border-line rounded-control px-3 py-1.5 text-dim transition-[color,border-color,box-shadow] duration-150 hover:text-fg hover:border-line-2 hover:shadow-glow-amber cursor-pointer"
          >
            {t.liveEngine}
          </button>
        );
      case 'loading':
        return (
          <div className="flex flex-col gap-1.5">
            <span className="inline-flex items-center gap-2 font-mono text-[11.5px] text-dim">
              <Spinner />
              {t.liveEngineLoading}
            </span>
            <Note>{t.liveEngineSlow}</Note>
          </div>
        );
      case 'error': {
        // Explain the failure: 409 == no durable delivery store to observe;
        // anything else is a transient/defensive fallback (the section is
        // already gated on live, so this is rarely reached). A 409 will not
        // recover on retry, so only offer the retry for the transient case.
        const noStore = observe.httpStatus === 409;
        return (
          <div className="flex flex-col items-start gap-2">
            <p className="text-[12.5px] text-red">
              {noStore ? t.liveEngineErrorNoStore : t.liveEngineNotLive}
            </p>
            {!noStore && (
              <button
                type="button"
                onClick={handleLoadObserve}
                className="font-ui font-semibold text-[12px] border border-line rounded-control px-3 py-1.5 text-dim transition-[color,border-color,box-shadow] duration-150 hover:text-fg hover:border-line-2 hover:shadow-glow-amber cursor-pointer"
              >
                {t.logsRefresh}
              </button>
            )}
          </div>
        );
      }
      case 'loaded':
        return <ObserveView snapshot={observe.snapshot} />;
    }
  }
}
