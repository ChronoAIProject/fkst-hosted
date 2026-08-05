import { useContent } from '@/i18n';
import type { ScheduleSummary } from '@/lib/api/schedules';
import { Absent, LifecycleBadge, RunStatusPill, SuccessMeter, relativeTo } from './parts';

/**
 * One repository's scheduled workflows.
 *
 * The single design decision here is that an INVALID schedule renders its reason
 * inline rather than only on the detail page. A schedule that has silently
 * stopped firing is exactly the failure this surface exists to catch, and it
 * would be invisible if the operator had to open each row to find it.
 */
export function ScheduleList({
  schedules,
  now,
  onOpen,
}: {
  schedules: ScheduleSummary[];
  /** Injected so the relative "next run" is deterministic under test. */
  now: number;
  onOpen: (scheduleIssue: number) => void;
}) {
  const t = useContent().workflows;
  return (
    <table
      aria-label={t.schedulesAria}
      data-testid="schedule-list"
      className="w-full table-fixed border-collapse"
    >
      <thead>
        <tr className="border-b border-line">
          {[t.colWorkflow, t.colCadence, t.colNextRun, t.colState, t.colLastRun, t.colSuccess].map(
            (heading) => (
              <th
                key={heading}
                scope="col"
                className="px-2 py-1.5 text-left font-mono text-[9.5px] uppercase tracking-[0.14em] text-ghost"
              >
                {heading}
              </th>
            )
          )}
        </tr>
      </thead>
      <tbody>
        {schedules.map((schedule) => (
          <tr
            key={schedule.scheduleIssue}
            data-testid={`schedule-row-${schedule.scheduleIssue}`}
            className="border-b border-line align-top"
          >
            <td className="px-2 py-2 overflow-hidden">
              <button
                type="button"
                onClick={() => onOpen(schedule.scheduleIssue)}
                className="block max-w-full truncate text-left font-ui text-[13px] text-fg hover:text-amber cursor-pointer"
              >
                {schedule.workflowId || schedule.title}
              </button>
              <span className="block font-mono text-[10px] text-ghost">
                #{schedule.scheduleIssue}
              </span>
              {schedule.invalidDetail && (
                // Inline, not behind a click: an operator scanning the list must
                // see WHY a schedule stopped without opening anything.
                <p
                  data-testid={`invalid-detail-${schedule.scheduleIssue}`}
                  className="mt-1 font-ui text-[11.5px] leading-snug text-red"
                >
                  {schedule.invalidDetail}
                </p>
              )}
            </td>
            <td className="px-2 py-2 font-ui text-[12px] text-dim overflow-hidden">
              <span className="block truncate" title={schedule.runMode}>
                {schedule.cadence || <Absent label={t.never} />}
              </span>
            </td>
            <td className="px-2 py-2 font-ui text-[12px] text-dim">
              <span title={schedule.nextDue ?? undefined}>
                {relativeTo(schedule.nextDue, now, t)}
              </span>
            </td>
            <td className="px-2 py-2">
              <LifecycleBadge state={schedule.state} label={t.lifecycle[schedule.state]} />
            </td>
            <td className="px-2 py-2">
              {schedule.lastRun ? (
                <RunStatusPill
                  status={schedule.lastRun.status}
                  label={t.runStatus[schedule.lastRun.status]}
                />
              ) : (
                <Absent label={t.never} />
              )}
            </td>
            <td className="px-2 py-2">
              <SuccessMeter rate={schedule.successRate30d} absent={t.never} />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
