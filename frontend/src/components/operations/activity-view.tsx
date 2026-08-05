import { useState } from 'react';
import { ArrowDownToLine } from 'lucide-react';
import { useContent } from '@/i18n';
import { describeError } from '@/lib/api/operations';
import type { ActivityFeed } from '@/lib/hooks/use-operations-activity';
import type { ActivityFilters, WindowProblem } from '@/lib/operations/state';
import { ActivityDetails } from './activity-details';
import { ActivityFiltersBar } from './activity-filters';
import { ActivityTable } from './activity-table';
import { LoadingState } from '@/components/ui/loading';
import { EmptyState, ErrorState, Notice } from './parts';
import { ActivityStatusLine } from './status-line';

/**
 * The Activity panel: toolbar, freshness line, one scrolling table region, and
 * an optional details panel.
 *
 * It renders exactly one body state, chosen in this order:
 *
 * 1. **withheld** — no request was issued at all, because the query names no
 *    session or names an unqueryable window. The panel states the one missing
 *    piece. This must NOT fall through to the empty state: "nothing matched"
 *    would report a result for a query that never ran.
 * 2. **error with no data** — the failure, its localized code copy, and a retry.
 * 3. **loading with no data** — the FIRST request for this query, with nothing
 *    yet to show. A third distinct shape: "we are still asking" must never be
 *    dressed as "nothing matched". Gated on `feed.loading` (first request only),
 *    never `feed.refreshing`, which is true on every poll and would blank the
 *    table under the reader every 15 seconds.
 * 4. **incomplete and empty** — zero rows on a page a source could not fill.
 *    Distinct copy and a distinct test id, because it is not a result.
 * 5. **complete and empty** — a plain sentence, no spinner. This IS a result.
 * 6. **rows** — with a partial banner above them when a source could not answer,
 *    so an incomplete page is never mistaken for a complete one.
 *
 * A failure that arrives while rows are on screen keeps them and adds a banner:
 * the rows are still true, they are just no longer fresh, and blanking them
 * would destroy the investigation in progress. The exception is a failure that
 * invalidates them outright (`clearsLastGood`), which drops the rows in the poll
 * engine before this component ever sees them.
 */
export function ActivityView({
  feed,
  filters,
  showActorFilters,
  sessionRequired,
  windowIssue,
  maxRangeDays,
  onFiltersChange,
  onReset,
}: {
  feed: ActivityFeed;
  filters: ActivityFilters;
  showActorFilters: boolean;
  /** True when a personal lifecycle query has named no session, so no request
   *  was issued. */
  sessionRequired: boolean;
  /** Why the named window cannot be queried, when it cannot. Also means no
   *  request was issued. */
  windowIssue: WindowProblem | null;
  /** This deployment's stated `to - from` ceiling, in days. */
  maxRangeDays: number;
  onFiltersChange: (next: ActivityFilters) => void;
  onReset: () => void;
}) {
  const t = useContent().operations;
  const c = useContent();
  // Only the row's ID is state. The row itself is looked up in the CURRENT
  // result set every render, so a filter, scope, or identity change that drops
  // the row closes the panel synchronously — while a routine 15-second poll that
  // still contains it leaves the open investigation exactly where it was.
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const selected = feed.rows.find((row) => row.event_id === selectedId) ?? null;

  const failure = feed.error ? describeError(feed.error) : null;
  const olderFailure = feed.olderError ? describeError(feed.olderError) : null;

  return (
    <div className="flex-1 min-h-0 flex flex-col gap-3">
      <ActivityFiltersBar
        filters={filters}
        showActorFilters={showActorFilters}
        refreshing={feed.refreshing}
        windowIssue={windowIssue}
        maxRangeDays={maxRangeDays}
        onChange={onFiltersChange}
        onReset={onReset}
        onRefresh={feed.refresh}
      />

      {feed.page && (
        <ActivityStatusLine status={feed.page.source_status} queriedAt={feed.page.queried_at} />
      )}
      {failure && feed.rows.length > 0 && (
        <Notice testId="activity-stale">{t.errorMessage[failure.code]}</Notice>
      )}
      {feed.pollSuspended && <Notice testId="activity-poll-paused">{t.pollPaused}</Notice>}

      <div className="flex-1 min-h-0 flex gap-3 max-[1100px]:flex-col">
        <div className="flex-1 min-w-0 min-h-0 flex flex-col border border-line rounded-panel bg-bg overflow-hidden">
          {sessionRequired ? (
            <Withheld
              testId="activity-session-required"
              title={t.sessionRequiredTitle}
              body={t.sessionRequiredBody}
            />
          ) : windowIssue !== null ? (
            <Withheld
              testId="activity-window-required"
              title={t.windowRequiredTitle}
              body={t.rangeProblem[windowIssue].replace('{days}', String(maxRangeDays))}
            />
          ) : failure && feed.rows.length === 0 ? (
            <ErrorState
              title={t.errorTitle}
              message={t.errorMessage[failure.code]}
              requestId={failure.requestId}
              requestIdLabel={t.errorRequestId}
              retryLabel={t.retry}
              onRetry={feed.refresh}
            />
          ) : feed.loading && feed.rows.length === 0 ? (
            // Still asking. Before this, the first load fell through to the rows
            // branch and rendered an EMPTY TABLE plus a pager — indistinguishable
            // from a query that genuinely matched nothing.
            <LoadingState
              variant="block"
              testId="operations-loading-activity"
              label={t.loadingActivity}
              detail={c.loading.service}
            />
          ) : feed.rows.length === 0 && !feed.loading ? (
            // A page a source could not fill is NOT an empty result, and must
            // never borrow the copy that says nothing matched.
            feed.page?.source_status.partial ? (
              <EmptyState testId="operations-incomplete" message={t.emptyActivityIncomplete} />
            ) : (
              <EmptyState message={t.emptyActivity} />
            )
          ) : (
            <>
              {/* The ONE scroll region: the table scrolls inside it in both axes
                  and the page body never does. */}
              <div className="flex-1 min-h-0 overflow-auto">
                <ActivityTable
                  rows={feed.rows}
                  selectedId={selected?.event_id ?? null}
                  onSelect={(row) => setSelectedId(row.event_id)}
                />
              </div>
              <div className="flex-none border-t border-line px-3 py-2 flex items-center gap-3">
                {feed.hasMore ? (
                  <button
                    type="button"
                    onClick={feed.loadOlder}
                    disabled={feed.loadingOlder}
                    aria-busy={feed.loadingOlder}
                    className="font-ui font-semibold text-[12px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg hover:shadow-glow-amber transition-[color,box-shadow] cursor-pointer disabled:opacity-50 disabled:cursor-default inline-flex items-center gap-1.5"
                  >
                    <ArrowDownToLine aria-hidden="true" className="w-3 h-3" />
                    {feed.loadingOlder ? t.loadingOlder : t.loadOlder}
                  </button>
                ) : (
                  <span className="font-mono text-[10.5px] text-ghost">{t.noMore}</span>
                )}
                {olderFailure && (
                  <span className="font-mono text-[10.5px] text-warn">
                    {t.errorMessage[olderFailure.code]}
                  </span>
                )}
              </div>
            </>
          )}
        </div>

        {selected && <ActivityDetails row={selected} onClose={() => setSelectedId(null)} />}
      </div>
    </div>
  );
}

/** The panel shown when the UI deliberately issued no request. It states the one
 *  missing piece — never a result, never a spinner. */
function Withheld({ testId, title, body }: { testId: string; title: string; body: string }) {
  return (
    <div
      data-testid={testId}
      className="flex-1 flex flex-col items-center justify-center gap-2 px-6 py-10 text-center"
    >
      <p className="font-ui font-semibold text-[13px] text-fg">{title}</p>
      <p className="font-mono text-[11.5px] text-dim max-w-[52ch]">{body}</p>
    </div>
  );
}
