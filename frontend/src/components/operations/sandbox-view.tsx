import { useEffect, useState } from 'react';
import { useContent } from '@/i18n';
import { describeError } from '@/lib/api/operations';
import type { SandboxFeed } from '@/lib/hooks/use-operations-sandboxes';
import { isSnapshotStale } from '@/lib/hooks/use-operations-sandboxes';
import type { SandboxFilters } from '@/lib/operations/state';
import { LoadingState } from '@/components/ui/loading';
import { EmptyState, ErrorState, Notice } from './parts';
import { SandboxDetails } from './sandbox-details';
import { SandboxFiltersBar } from './sandbox-filters';
import { SandboxTable, rowKey } from './sandbox-table';
import { SandboxStatusLine } from './status-line';

/** How often the display clock advances so ages and countdowns keep moving
 *  between 5-second polls. It recomputes DERIVED values only — it never touches
 *  a server fact. */
const CLOCK_TICK_MS = 1_000;

/**
 * The Sandboxes panel.
 *
 * Its body states mirror the Activity panel's, with one addition that matters:
 * `session_visibility_unavailable` is a FAILURE, not an empty snapshot. A cold
 * session-visibility projection cannot tell "you have no sandboxes" from "I do
 * not yet know which are yours", and rendering it as an authorized empty list
 * during a restart is precisely the incident the backend's `503` exists to
 * prevent. The panel keeps that distinction by rendering every failure — this
 * one included — as an error with its own copy and a retry.
 *
 * A failed poll with a snapshot on screen keeps that snapshot and marks it
 * stale. It is still what was last observed; it is simply no longer now.
 */
export function SandboxView({
  feed,
  filters,
  onFiltersChange,
  onReset,
  onViewActivity,
}: {
  feed: SandboxFeed;
  filters: SandboxFilters;
  onFiltersChange: (next: SandboxFilters) => void;
  onReset: () => void;
  /** Switch to the Activity view for one session. */
  onViewActivity: (sessionId: string) => void;
}) {
  const t = useContent().operations;
  const c = useContent();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());

  // The display clock runs only while this view is mounted (the page unmounts it
  // with the tab) and only advances derived values.
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), CLOCK_TICK_MS);
    return () => window.clearInterval(timer);
  }, []);

  const rows = feed.inventory?.items ?? [];
  const selected = rows.find((row) => rowKey(row) === selectedId) ?? null;
  const failure = feed.error ? describeError(feed.error) : null;
  const stale = isSnapshotStale(feed.inventory?.observed_at, now);

  return (
    <div className="flex-1 min-h-0 flex flex-col gap-3">
      <SandboxFiltersBar
        filters={filters}
        refreshing={feed.refreshing}
        onChange={onFiltersChange}
        onReset={onReset}
        onRefresh={feed.refresh}
      />

      {feed.inventory && (
        <SandboxStatusLine
          observedAt={feed.inventory.observed_at}
          stale={stale}
          warningCodes={feed.inventory.warning_codes}
        />
      )}
      {failure && feed.inventory && (
        <Notice testId="sandbox-refresh-failed">{t.errorMessage[failure.code]}</Notice>
      )}

      <div className="flex-1 min-h-0 flex gap-3 max-[1100px]:flex-col">
        <div className="flex-1 min-w-0 min-h-0 flex flex-col border border-line rounded-panel bg-bg overflow-hidden">
          {failure && !feed.inventory ? (
            <ErrorState
              title={t.errorTitle}
              message={t.errorMessage[failure.code]}
              requestId={failure.requestId}
              requestIdLabel={t.errorRequestId}
              retryLabel={t.retry}
              onRetry={feed.refresh}
            />
          ) : feed.loading && rows.length === 0 ? (
            // Still asking: a distinct third shape from "empty" below. Before
            // this, the first load fell through and rendered an empty table, so
            // an in-flight fleet read looked exactly like an empty fleet.
            <LoadingState
              variant="block"
              testId="operations-loading-sandboxes"
              label={t.loadingSandboxes}
              detail={c.loading.service}
            />
          ) : rows.length === 0 && !feed.loading ? (
            // A COMPLETE authorized snapshot that happens to be empty. Nothing
            // here is a failure state, so nothing spins — which is why the
            // loading branch above has to be its own shape rather than this one.
            <EmptyState message={t.emptySandboxes} />
          ) : (
            <div className="flex-1 min-h-0 overflow-auto">
              <SandboxTable
                rows={rows}
                now={now}
                selectedId={selectedId}
                onSelect={(row) => setSelectedId(rowKey(row))}
              />
            </div>
          )}
        </div>

        {selected && (
          <SandboxDetails
            row={selected}
            onClose={() => setSelectedId(null)}
            onViewActivity={onViewActivity}
          />
        )}
      </div>
    </div>
  );
}
