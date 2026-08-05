import { useContent } from '@/i18n';
import type {
  RunSummary,
  ScheduleDetail as ScheduleDetailData,
  ScheduleRunDetail,
} from '@/lib/api/schedules';
import { Absent, LifecycleBadge, RunStatusPill, StepStatusMark, formatDuration, relativeTo } from './parts';

/**
 * One scheduled workflow: what it is, when it fires next, what it has done, and
 * the three things an operator can do to it.
 *
 * There is deliberately **no inline cadence editor**. The schedule lives on its
 * GitHub issue and stays editable there; a second editing surface would have to
 * re-implement the grammar, and the two would drift. The header says so in one
 * line rather than leaving the user hunting for an edit button that is not
 * coming.
 */
export function ScheduleDetail({
  detail,
  run,
  now,
  busy,
  actionError,
  onBack,
  onSelectRun,
  onRunNow,
  onPause,
  onResume,
}: {
  detail: ScheduleDetailData;
  /** The selected run's per-step outcomes, when one is open. */
  run: ScheduleRunDetail | null;
  now: number;
  busy: boolean;
  actionError: string | null;
  onBack: () => void;
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
    <div data-testid="schedule-detail" className="flex flex-col gap-5 min-h-0">
      <div className="flex flex-col gap-2">
        <button
          type="button"
          onClick={onBack}
          className="self-start font-ui text-[12px] text-dim hover:text-fg cursor-pointer"
        >
          ← {t.detailBack}
        </button>
        <div className="flex flex-wrap items-center gap-3">
          <h2 className="font-display font-semibold text-[18px] text-fg">
            {summary.workflowId || summary.title}
          </h2>
          <LifecycleBadge state={summary.state} label={t.lifecycle[summary.state]} />
          <span className="font-ui text-[12px] text-dim">{summary.cadence}</span>
          <a
            href={summary.htmlUrl}
            target="_blank"
            rel="noreferrer"
            className="font-ui text-[12px] text-amber hover:brightness-110"
          >
            {t.openOnGithub}
          </a>
        </div>
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

      <RunHistory
        runs={detail.runs}
        selected={run}
        onSelectRun={onSelectRun}
      />
    </div>
  );
}

/** The run list, with the selected run's stepper inlined under its row. */
function RunHistory({
  runs,
  selected,
  onSelectRun,
}: {
  runs: RunSummary[];
  selected: ScheduleRunDetail | null;
  onSelectRun: (slot: string) => void;
}) {
  const t = useContent().workflows;
  if (runs.length === 0) {
    return (
      <section className="flex flex-col gap-1.5">
        <h3 className="font-mono text-[9.5px] uppercase tracking-[0.14em] text-ghost">
          {t.runsTitle}
        </h3>
        <p className="font-ui text-[12px] text-ghost">{t.noRuns}</p>
      </section>
    );
  }
  return (
    <section className="flex flex-col gap-1.5 min-h-0">
      <h3 className="font-mono text-[9.5px] uppercase tracking-[0.14em] text-ghost">
        {t.runsTitle}
      </h3>
      <ul aria-label={t.runsAria} data-testid="run-history" className="flex flex-col">
        {runs.map((entry) => {
          const open = selected?.run.slot === entry.slot;
          return (
            <li key={entry.slot} className="border-b border-line">
              <button
                type="button"
                onClick={() => onSelectRun(entry.slot)}
                data-testid={`run-row-${entry.slot}`}
                aria-expanded={open}
                className="flex w-full flex-wrap items-center gap-3 px-1 py-2 text-left hover:bg-raise cursor-pointer"
              >
                <span className="font-mono text-[11.5px] text-dim">{entry.slot}</span>
                <RunStatusPill status={entry.status} label={t.runStatus[entry.status]} />
                <span className="font-mono text-[11px] text-ghost">
                  {formatDuration(entry.durationS, t.never)}
                </span>
                {entry.manual && (
                  <span className="rounded-chip border border-line px-1.5 font-ui text-[10.5px] text-ghost">
                    {t.manual}
                  </span>
                )}
                {entry.detail && (
                  <span className="min-w-0 flex-1 truncate font-ui text-[11.5px] text-ghost">
                    {entry.detail}
                  </span>
                )}
              </button>
              {open && <RunStepper run={selected} />}
            </li>
          );
        })}
      </ul>
    </section>
  );
}

/** One run's steps, in declared order. */
export function RunStepper({ run }: { run: ScheduleRunDetail }) {
  const t = useContent().workflows;
  return (
    <div data-testid="run-stepper" className="px-1 pb-3">
      {run.runIssue !== null && (
        <p className="pb-1 font-mono text-[11px] text-ghost">
          {t.runIssue} #{run.runIssue}
        </p>
      )}
      {run.steps.length === 0 ? (
        <p className="font-ui text-[12px] text-ghost">{t.noSteps}</p>
      ) : (
        <ol aria-label={t.stepperAria} className="flex flex-col gap-1">
          {run.steps.map((step) => (
            <li
              key={`${step.index}-${step.id}`}
              data-testid={`step-${step.index}`}
              className="flex items-center gap-3 border-l-2 border-line pl-3"
            >
              <span className="font-mono text-[11px] text-ghost">{step.index}</span>
              <span className="font-ui text-[12.5px] text-fg">{step.id}</span>
              <StepStatusMark status={step.status} label={t.stepStatus[step.status]} />
              <span className="font-mono text-[11px] text-ghost">
                {formatDuration(step.durationS, t.never)}
              </span>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
