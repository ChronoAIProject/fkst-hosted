import { useContent } from '@/i18n';
import type { RunSummary, ScheduleRunDetail } from '@/lib/api/schedules';
import { RunStatusPill, StepStatusMark, formatDuration } from './parts';

/** The GitHub issue a run was worked as. The API returns the NUMBER only; the
 *  repository is unambiguous because a run issue is always opened on the
 *  definition's own repository. */
function runIssueUrl(owner: string, name: string, issue: number): string {
  return `https://github.com/${owner}/${name}/issues/${issue}`;
}

/**
 * The most recent run, always expanded.
 *
 * This is the block that makes a run's steps reachable without hunting for them:
 * the newest run's outcome rides on the schedule detail already, so showing it
 * costs nothing and asking for a click cost the reader the one thing they opened
 * the schedule to see.
 *
 * It is also the only surface that can say anything about a run STILL GOING. The
 * runner posts a single record when it finishes, so mid-run there are no step
 * outcomes to show and inventing a "current step" would be a guess. What the
 * dispatch record does carry is when the run started and which issue it is
 * running as, which is what this renders instead of an empty stepper.
 */
export function LatestRun({
  owner,
  name,
  run,
  /** The in-flight run's age in seconds, kept current by the host's tick. Null
   *  whenever the run is not in flight — a finished run reports its duration. */
  liveElapsedS,
}: {
  owner: string;
  name: string;
  run: ScheduleRunDetail;
  liveElapsedS: number | null;
}) {
  const t = useContent().workflows;
  const inFlight = run.run.status === 'dispatched';
  return (
    <section data-testid="latest-run" className="flex flex-col gap-2">
      <h3 className="font-mono text-[9.5px] uppercase tracking-[0.14em] text-ghost">
        {t.latestRunTitle}
      </h3>
      <div className="flex flex-wrap items-center gap-3">
        <span className="font-mono text-[11.5px] text-dim">{run.run.slot}</span>
        <RunStatusPill status={run.run.status} label={t.runStatus[run.run.status]} />
        <span data-testid="latest-run-timing" className="font-mono text-[11px] text-ghost">
          {inFlight
            ? t.runningFor.replace('{d}', formatDuration(liveElapsedS, t.never))
            : formatDuration(run.run.durationS, t.never)}
        </span>
        {run.run.manual && (
          <span className="rounded-chip border border-line px-1.5 font-ui text-[10.5px] text-ghost">
            {t.manual}
          </span>
        )}
        {run.runIssue !== null && (
          <a
            href={runIssueUrl(owner, name, run.runIssue)}
            target="_blank"
            rel="noreferrer"
            data-testid="run-issue-link"
            className="font-ui text-[12px] text-amber hover:brightness-110"
          >
            {t.openRunIssue} #{run.runIssue}
          </a>
        )}
      </div>
      {run.run.detail && (
        <p className="font-ui text-[11.5px] leading-snug text-dim">{run.run.detail}</p>
      )}
      <RunStepper run={run} inFlight={inFlight} />
    </section>
  );
}

/** One run's steps, in declared order.
 *
 *  Three distinct empty cases, because they mean different things: a run still
 *  going has not reported yet, a finished run with no steps genuinely recorded
 *  none, and those must never share a message. */
export function RunStepper({ run, inFlight }: { run: ScheduleRunDetail; inFlight: boolean }) {
  const t = useContent().workflows;
  if (run.steps.length === 0) {
    return (
      <p data-testid="run-stepper" className="font-ui text-[12px] text-ghost">
        {inFlight ? t.awaitingSteps : t.noSteps}
      </p>
    );
  }
  return (
    <ol data-testid="run-stepper" aria-label={t.stepperAria} className="flex flex-col gap-1">
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
  );
}

/**
 * The runs BEFORE the most recent one, each expandable into its own stepper.
 *
 * The newest run is excluded rather than repeated: it is already open above, and
 * a list whose first row silently duplicated the block over it would read as a
 * rendering bug. Older runs still cost a fetch each, which is why they stay
 * behind a click.
 */
export function EarlierRuns({
  runs,
  latestSlot,
  openSlot,
  selected,
  onSelectRun,
}: {
  /** Every run newest-first, INCLUDING the latest — the exclusion happens here so
   *  the caller never has to keep two lists in step. */
  runs: RunSummary[];
  /** The slot {@link LatestRun} is already showing, or null when nothing has run. */
  latestSlot: string | null;
  /** The slot the reader ASKED for. Drives the disclosure state, so a row reports
   *  itself expanded the moment it is clicked rather than once its fetch lands —
   *  deriving that from the payload leaves `aria-expanded="false"` on a row the
   *  user has already opened, and a spinner-free row with no acknowledgement. */
  openSlot: string | null;
  /** The expanded older run's detail, once it has loaded. */
  selected: ScheduleRunDetail | null;
  onSelectRun: (slot: string) => void;
}) {
  const t = useContent().workflows;
  const earlier = runs.filter((entry) => entry.slot !== latestSlot);
  if (earlier.length === 0) return null;
  return (
    <section className="flex flex-col gap-1.5 min-h-0">
      <h3 className="font-mono text-[9.5px] uppercase tracking-[0.14em] text-ghost">
        {t.earlierRunsTitle}
      </h3>
      <ul aria-label={t.runsAria} data-testid="run-history" className="flex flex-col">
        {earlier.map((entry) => {
          const open = openSlot === entry.slot;
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
              {open && (
                <div className="px-1 pb-3">
                  {/* `selected` trails `openSlot` by one fetch, and can name the
                      PREVIOUSLY opened run while this one loads — so it is shown
                      only once it is this row's. An older run is terminal by
                      definition: its slot was superseded by a newer one, so it
                      cannot still be waiting on a first step record. */}
                  {selected?.run.slot === entry.slot ? (
                    <RunStepper run={selected} inFlight={false} />
                  ) : (
                    <p className="font-ui text-[12px] text-ghost">{t.loading}</p>
                  )}
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
