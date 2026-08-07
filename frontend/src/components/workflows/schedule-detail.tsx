import { useContent } from '@/i18n';
import { ScrollArea } from '@/components/ui/scroll-area';
import type {
  ScheduleDetail as ScheduleDetailData,
  ScheduleRunDetail,
} from '@/lib/api/schedules';
import { Absent, LifecycleBadge, SuccessMeter, relativeTo } from './parts';
import { EarlierRuns, LatestRun } from './run-history';

/**
 * One scheduled workflow: what it is, when it fires next, what it is doing or
 * last did, and the three things an operator can do to it.
 *
 * There is deliberately **no inline cadence editor**. The schedule lives on its
 * GitHub issue and stays editable there; a second editing surface would have to
 * re-implement the grammar, and the two would drift. The header says so in one
 * line rather than leaving the user hunting for an edit button that is not
 * coming.
 *
 * There is also no back button. The rail beside this pane is the way back, and a
 * detail that could be dismissed into nothing would leave the tab blank while
 * the session still owns schedules.
 */
export function ScheduleDetail({
  owner,
  name,
  detail,
  run,
  liveElapsedS,
  now,
  busy,
  actionError,
  onSelectRun,
  onRunNow,
  onPause,
  onResume,
}: {
  /** The definition's repository, so a run issue's number can become a link. */
  owner: string;
  name: string;
  detail: ScheduleDetailData;
  /** An EARLIER run's per-step outcomes, when one is expanded. The most recent
   *  run needs none of this — it rides on `detail.latestRun`. */
  run: ScheduleRunDetail | null;
  /** The in-flight run's live age in seconds; null when nothing is in flight. */
  liveElapsedS: number | null;
  now: number;
  busy: boolean;
  actionError: string | null;
  onSelectRun: (slot: string) => void;
  onRunNow: () => void;
  onPause: () => void;
  onResume: () => void;
}) {
  const t = useContent().workflows;
  const { summary } = detail;
  const paused = summary.state === 'paused';
  // A run already in flight, or a definition the control plane refused, cannot
  // be dispatched — the server answers 409 either way, so the button says so
  // first instead of inviting a click that always fails.
  const canRunNow = summary.state !== 'running' && summary.state !== 'invalid';

  return (
    // The detail owns its own scroll region: the rail beside it must stay put
    // while a long run history is read.
    <ScrollArea className="pr-1">
      <div data-testid="schedule-detail" className="flex flex-col gap-5">
        <div className="flex flex-col gap-2">
          <div className="flex flex-wrap items-center gap-3">
            <h3 className="font-display font-semibold text-[16px] text-fg">
              {summary.workflowId || summary.title}
            </h3>
            <LifecycleBadge state={summary.state} label={t.lifecycle[summary.state]} />
            <a
              href={summary.htmlUrl}
              target="_blank"
              rel="noreferrer"
              className="font-ui text-[12px] text-amber hover:brightness-110"
            >
              {t.openOnGithub}
            </a>
          </div>
          <dl className="flex flex-wrap items-center gap-x-5 gap-y-1 font-mono text-[11px]">
            <div className="flex items-baseline gap-1.5 min-w-0">
              <dt className="text-ghost">{t.cadenceLabel}</dt>
              <dd className="text-dim" title={summary.runMode}>
                {summary.cadence || <Absent label={t.never} />}
              </dd>
            </div>
            <div className="flex items-baseline gap-1.5 min-w-0">
              <dt className="text-ghost">{t.successLabel}</dt>
              <dd className="text-dim">
                <SuccessMeter rate={summary.successRate30d} absent={t.never} />
              </dd>
            </div>
          </dl>
          {summary.invalidDetail && (
            <p className="font-ui text-[12.5px] leading-snug text-red">{summary.invalidDetail}</p>
          )}
          <p className="font-ui text-[11.5px] leading-snug text-ghost max-w-[70ch]">{t.editHint}</p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            disabled={busy || !canRunNow}
            onClick={onRunNow}
            data-testid="action-run-now"
            className="font-ui font-semibold text-[12.5px] bg-grad-accent text-amber-ink rounded-control px-3 py-1.5 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer"
          >
            {busy ? t.actionBusy : t.actionRunNow}
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={paused ? onResume : onPause}
            data-testid="action-pause-resume"
            className="font-ui text-[12.5px] text-fg border border-line rounded-control px-3 py-1.5 disabled:opacity-50 cursor-pointer"
          >
            {paused ? t.actionResume : t.actionPause}
          </button>
          {actionError && (
            // The server's own message, verbatim: a 409 explaining that a run is
            // already in flight is far more useful than a generic failure line.
            <span data-testid="action-error" className="font-ui text-[12px] text-red">
              {actionError}
            </span>
          )}
        </div>

        {/* Above the definition's own fields on purpose: what this schedule is
            doing right now outranks what it is configured to do. */}
        {detail.latestRun ? (
          <LatestRun
            owner={owner}
            name={name}
            run={detail.latestRun}
            liveElapsedS={liveElapsedS}
          />
        ) : (
          <section className="flex flex-col gap-1.5">
            <h3 className="font-mono text-[9.5px] uppercase tracking-[0.14em] text-ghost">
              {t.latestRunTitle}
            </h3>
            <p className="font-ui text-[12px] text-ghost">{t.noRuns}</p>
          </section>
        )}

        <section className="flex flex-col gap-1.5">
          <h3 className="font-mono text-[9.5px] uppercase tracking-[0.14em] text-ghost">
            {t.upcoming}
          </h3>
          {detail.upcoming.length === 0 ? (
            <Absent label={t.never} />
          ) : (
            <ul data-testid="upcoming" className="flex flex-wrap gap-2">
              {detail.upcoming.map((at) => (
                <li
                  key={at}
                  title={at}
                  className="rounded-chip border border-line px-2 py-[2px] font-mono text-[11px] text-dim"
                >
                  {relativeTo(at, now, t)}
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="flex flex-col gap-1.5">
          <h3 className="font-mono text-[9.5px] uppercase tracking-[0.14em] text-ghost">
            {t.argumentsTitle}
          </h3>
          {Object.keys(detail.arguments).length === 0 ? (
            <p className="font-ui text-[12px] text-ghost">{t.noArguments}</p>
          ) : (
            <dl data-testid="arguments" className="flex flex-col gap-0.5">
              {Object.entries(detail.arguments).map(([key, value]) => (
                <div key={key} className="flex gap-2 font-mono text-[11.5px]">
                  <dt className="text-ghost">{key}</dt>
                  <dd className="text-dim break-all">{value}</dd>
                </div>
              ))}
            </dl>
          )}
        </section>

        <EarlierRuns
          runs={detail.runs}
          latestSlot={detail.latestRun?.run.slot ?? null}
          selected={run}
          onSelectRun={onSelectRun}
        />
      </div>
    </ScrollArea>
  );
}
