import { useContent } from '@/i18n';
import { Chip } from '@/components/ui/chip';
import type { IssueDetail, SessionDetail } from '@/lib/api/types';
import { decodeSessionStatus, decodeWorkItemStatus, type SessionPhase } from '@/lib/api/derive';
import { Note, SectionLabel, Spinner } from './parts';
import { ObserveView } from './observe-view';
import { PHASE_TONE, WORK_TONE } from './tones';
import type { ObserveState } from './observe-state';

/** The happy-path lifecycle stages, in order. Off-path phases (degraded /
 *  invalid / idle / picked-up) still surface as the prominent pill above. */
const STAGES: SessionPhase[] = ['registered', 'active', 'retired'];

function stageReached(stage: SessionPhase, phase: SessionPhase, liveness: string | null): boolean {
  const advanced = phase === 'active' || phase === 'picked-up' || phase === 'degraded' || phase === 'retired' || liveness != null;
  if (stage === 'registered') return true;
  if (stage === 'active') return advanced;
  return phase === 'retired';
}

function WorkItemRow({ issue }: { issue: IssueDetail }) {
  const t = useContent().dashboard.detail;
  const decoded = decodeWorkItemStatus(issue);
  return (
    <div className="flex items-center gap-2 py-1.5 text-[12.5px] min-w-0">
      <a
        href={issue.html_url}
        target="_blank"
        rel="noreferrer"
        className="font-mono text-[11px] text-ghost hover:text-amber transition-colors flex-none"
      >
        #{issue.number}
      </a>
      <span className="text-fg truncate min-w-0 flex-1">{issue.title}</span>
      <Chip tone={WORK_TONE[decoded.tone]}>{t.work[decoded.state]}</Chip>
    </div>
  );
}

/** Status tab: decoded lifecycle (pill + stage strip), per-work-item states,
 *  and an on-demand "Live engine details" fetch (slow pod exec — spinner +
 *  "may take a minute" note). */
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
  const status = decodeSessionStatus(session);

  return (
    <div className="flex flex-col gap-5">
      <section className="flex flex-col gap-2">
        <SectionLabel>{t.lifecycle}</SectionLabel>
        <div className="flex items-center gap-1.5 flex-wrap">
          <Chip tone={PHASE_TONE[status.phase]}>{t.phase[status.phase]}</Chip>
          <span className="font-mono text-[10.5px] text-ghost">
            {t.healthLabel}:{' '}
            <span className={status.health === 'degraded' ? 'text-red' : 'text-dim'}>
              {t.health[status.health]}
            </span>
          </span>
          {status.liveness && (
            <Chip tone={status.liveness === 'live' ? 'green' : 'neutral'}>{status.liveness}</Chip>
          )}
        </div>
        {/* Lifecycle stage strip. */}
        <ol className="flex items-center gap-2 mt-1">
          {STAGES.map((stage) => {
            const reached = stageReached(stage, status.phase, status.liveness);
            const current = stage === status.phase;
            return (
              <li key={stage} className="flex items-center gap-1.5">
                <span
                  aria-hidden="true"
                  className={
                    'w-1.5 h-1.5 rounded-full flex-none ' +
                    (current ? 'bg-amber' : reached ? 'bg-green' : 'bg-ghost')
                  }
                />
                <span
                  className={
                    'font-mono text-[10.5px] ' + (reached || current ? 'text-dim' : 'text-ghost')
                  }
                >
                  {t.phase[stage]}
                </span>
              </li>
            );
          })}
        </ol>
      </section>

      <section className="flex flex-col gap-1.5">
        <SectionLabel>
          {t.workItems}
          {session.work_issues.length > 0 && (
            <span className="ml-2 lowercase">· {session.work_issues.length}</span>
          )}
        </SectionLabel>
        {session.work_issues.length === 0 ? (
          <Note>{t.noWorkItems}</Note>
        ) : (
          <div className="divide-y divide-[color-mix(in_oklab,var(--line)_55%,transparent)]">
            {session.work_issues.map((issue) => (
              <WorkItemRow key={issue.number} issue={issue} />
            ))}
          </div>
        )}
      </section>

      <section className="flex flex-col gap-2">
        <SectionLabel>{t.liveEngine}</SectionLabel>
        {observe.status === 'idle' && (
          <button
            type="button"
            onClick={onLoadObserve}
            className="self-start font-ui font-semibold text-[12px] border border-line rounded-control px-3 py-1.5 text-dim hover:text-fg transition-colors cursor-pointer"
          >
            {t.liveEngine}
          </button>
        )}
        {observe.status === 'loading' && (
          <div className="flex flex-col gap-1.5">
            <span className="inline-flex items-center gap-2 font-mono text-[11.5px] text-dim">
              <Spinner />
              {t.liveEngineLoading}
            </span>
            <Note>{t.liveEngineSlow}</Note>
          </div>
        )}
        {observe.status === 'error' && (
          <div className="flex flex-col items-start gap-2">
            <p className="text-[12.5px] text-red">{t.liveEngineError}</p>
            <button
              type="button"
              onClick={onLoadObserve}
              className="font-ui font-semibold text-[12px] border border-line rounded-control px-3 py-1.5 text-dim hover:text-fg transition-colors cursor-pointer"
            >
              {t.logsRefresh}
            </button>
          </div>
        )}
        {observe.status === 'loaded' && <ObserveView snapshot={observe.snapshot} />}
      </section>
    </div>
  );
}
