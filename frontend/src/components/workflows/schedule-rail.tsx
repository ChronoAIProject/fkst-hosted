import { useContent } from '@/i18n';
import { cn } from '@/lib/utils';
import { ScrollArea } from '@/components/ui/scroll-area';
import { SectionLabel } from '@/components/session-detail/parts';
import type { ScheduleSummary } from '@/lib/api/schedules';
import { LifecycleBadge, relativeTo } from './parts';

/**
 * The navigation half of the workflows tab: one selectable row per schedule this
 * session owns, plus a trailing section for the ones that belong to no session
 * at all.
 *
 * A plain `<ul>` of buttons with `aria-current`, deliberately NOT a nested
 * `role="tablist"`. This rail lives inside the session detail's tabpanel, whose
 * tablist already owns an arrow-key contract; a second roving-focus tablist
 * nested under it would leave the reader with two different meanings for the
 * same key. The Health tab settled this the same way.
 *
 * Each row carries the schedule's lifecycle and its next firing because those
 * are the two facts an operator scans a list of schedules FOR — "is anything
 * broken, and what happens next". A row that showed only a name would make the
 * rail a table of contents rather than a status board.
 */
export function ScheduleRail({
  schedules,
  unrouted,
  selectedIssue,
  now,
  onSelect,
}: {
  /** The schedules this session owns, in API order. */
  schedules: ScheduleSummary[];
  /** Schedules whose definition names no single session creator, so they belong
   *  to no session and can never run. Listed but not selectable — see below. */
  unrouted: ScheduleSummary[];
  /** The EFFECTIVE selection (the default first row, not just an explicit
   *  click), so the matching row highlights before the user touches anything. */
  selectedIssue: number | null;
  now: number;
  onSelect: (scheduleIssue: number) => void;
}) {
  const t = useContent().workflows;
  return (
    <nav className="flex flex-col gap-1.5 min-w-0 min-h-0">
      <SectionLabel>{t.railTitle}</SectionLabel>
      {/* The rail scrolls INTERNALLY. Without its own region a long list would
          push the split past the tab panel and scroll the rail itself out of
          view — the very thing being navigated by. Capped on narrow screens,
          where the single column stacks it above the detail. */}
      <ScrollArea className="pr-1 max-h-[12rem] md:max-h-none">
        <ul aria-label={t.railAria} className="flex flex-col gap-1">
          {schedules.map((schedule) => {
            const active = schedule.scheduleIssue === selectedIssue;
            return (
              <li key={schedule.scheduleIssue}>
                <button
                  type="button"
                  aria-current={active}
                  data-testid={`schedule-row-${schedule.scheduleIssue}`}
                  onClick={() => onSelect(schedule.scheduleIssue)}
                  // The workflow id is what a schedule is picked by and it is the
                  // part that truncates, so the full string rides on `title`.
                  title={schedule.workflowId || schedule.title}
                  className={cn(
                    'w-full text-left rounded-control border px-2.5 py-1.5',
                    'flex flex-col items-start gap-1 min-w-0',
                    active ? 'border-line-2 bg-raise-2' : 'border-line hover:bg-raise-1'
                  )}
                >
                  <span className="w-full truncate font-ui text-[12.5px] text-fg">
                    {schedule.workflowId || schedule.title}
                  </span>
                  <span className="flex items-center gap-1.5 flex-wrap">
                    <LifecycleBadge
                      state={schedule.state}
                      label={t.lifecycle[schedule.state]}
                    />
                    {/* The absolute instant on `title`; the row shows the coarse
                        distance, which is what a cadence is read as. */}
                    <span
                      title={schedule.nextDue ?? undefined}
                      className="font-mono text-[10.5px] text-ghost"
                    >
                      {relativeTo(schedule.nextDue, now, t)}
                    </span>
                  </span>
                </button>
              </li>
            );
          })}
          {unrouted.length > 0 && <UnroutedSection schedules={unrouted} />}
        </ul>
      </ScrollArea>
    </nav>
  );
}

/**
 * Schedules that name no single session creator.
 *
 * They are shown here rather than nowhere. A schedule runs only when exactly one
 * of its assignees is a session creator; with zero or several there is no
 * session to route its run issue to, so no session tab can honestly claim it —
 * and once the repository-level list is gone, omitting it would delete it from
 * the product while it silently never fires. Listing it under every session in
 * the repository is the deliberate cost of never hiding a broken one.
 *
 * They are NOT selectable, and they show no lifecycle or next firing: the API
 * derives those from the definition body, which parses fine here, so a confident
 * "idle, next run in 3h" would be a firing this schedule will never honour. The
 * only true thing to say is that nothing will run it, and the fix — assigning
 * one session creator — is a GitHub action, so the row links out.
 */
function UnroutedSection({ schedules }: { schedules: ScheduleSummary[] }) {
  const t = useContent().workflows;
  return (
    <li data-testid="unrouted-schedules" className="mt-3 flex flex-col gap-1">
      <SectionLabel>{t.unroutedTitle}</SectionLabel>
      <p className="font-ui text-[11px] leading-snug text-ghost">{t.unroutedBody}</p>
      <ul className="flex flex-col gap-1">
        {schedules.map((schedule) => (
          <li key={schedule.scheduleIssue}>
            <a
              href={schedule.htmlUrl}
              target="_blank"
              rel="noreferrer"
              data-testid={`unrouted-row-${schedule.scheduleIssue}`}
              title={schedule.workflowId || schedule.title}
              className="block w-full truncate rounded-control border border-dashed border-line px-2.5 py-1.5 font-ui text-[12px] text-dim hover:text-fg hover:border-line-2"
            >
              {schedule.workflowId || schedule.title}
            </a>
          </li>
        ))}
      </ul>
    </li>
  );
}
