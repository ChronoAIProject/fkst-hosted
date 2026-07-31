import { useCallback, useEffect, useRef, useState } from 'react';
import type { ApiFetch } from '@/lib/api/canvas';
import { getActivity, OperationsError } from '@/lib/api/operations';
import type { ActivityPage, ActivityRow, ActivityScope } from '@/lib/api/operations';
import type { ActivityFilters } from '@/lib/operations/state';
import { hasUsableWindow, needsSessionId, resolveWindow } from '@/lib/operations/state';
import { useScopedPoll } from './use-scoped-poll';

/** Activity refreshes every 15 seconds while its view is visible (epic
 *  `UI-02`). Historical records do not move fast enough to justify more. */
export const ACTIVITY_POLL_MS = 15_000;

export interface ActivityFeed {
  /** The first (newest) page, exactly as the server assembled it. */
  page: ActivityPage | null;
  /** The first page's rows plus every explicitly loaded older page's, in order. */
  rows: ActivityRow[];
  error: unknown;
  loading: boolean;
  refreshing: boolean;
  updatedAt: number | null;
  /** True when another page may exist. */
  hasMore: boolean;
  /** True while an older page is being fetched. */
  loadingOlder: boolean;
  /** The failure of the last "load older", kept separate so it cannot be
   *  mistaken for the first page having failed. */
  olderError: unknown;
  /** True while live refresh is suspended because the user opened older pages.
   *  Polling page 1 underneath an open investigation would either discard those
   *  pages or splice two inconsistent windows together; suspending is the only
   *  honest option, and the UI says so. */
  pollSuspended: boolean;
  /** Fetch the next page using the server's cursor. */
  loadOlder: () => void;
  /** Drop every older page and re-read page 1. */
  refresh: () => void;
}

export interface ActivityFeedOptions {
  apiFetch: ApiFetch;
  /** The result set identity; changing it clears rows and cursors. */
  cacheKey: string;
  /** `null` means "let the server pick", used only before the first response. */
  scope: ActivityScope | null;
  filters: ActivityFilters;
  /** False while another view is showing, or while the page cannot query. */
  enabled: boolean;
}

/**
 * The activity feed: one polled first page plus explicitly loaded older pages.
 *
 * Pagination is keyset-only. There are no page numbers, and a cursor is never
 * reused across a viewer, scope, session, or filter change — the cache key
 * covers all four, and every one of them clears `older` in the same render that
 * the poll engine drops its data.
 */
export function useOperationsActivity({
  apiFetch,
  cacheKey,
  scope,
  filters,
  enabled,
}: ActivityFeedOptions): ActivityFeed {
  const [older, setOlder] = useState<{ key: string; pages: ActivityPage[] }>({
    key: cacheKey,
    pages: [],
  });
  const [olderState, setOlderState] = useState<{ loading: boolean; error: unknown }>({
    loading: false,
    error: null,
  });
  const olderRequestRef = useRef(0);

  // A regular caller asking for lifecycle rows without an exact session id would
  // be asking for a global scan; the request is withheld rather than issued and
  // refused (see `needsSessionId`).
  const canQuery = enabled && !needsSessionId(filters, scope) && hasUsableWindow(filters);

  const fetchPage = useCallback(
    (cursor: string | undefined, signal: AbortSignal) => {
      // The window is resolved per request so a preset slides with the wall
      // clock. A CURSOR page deliberately states no window: the server keeps the
      // one its cursor was issued for, and restating a drifted `now` would make
      // the two disagree and get the cursor refused.
      const resolved = cursor === undefined ? resolveWindow(filters, Date.now()) : null;
      return getActivity(
        apiFetch,
        {
          scope,
          recordKind: filters.recordKind,
          from: resolved ? new Date(resolved.from).toISOString() : undefined,
          to: resolved ? new Date(resolved.to).toISOString() : undefined,
          actorId: filters.actorId ?? undefined,
          actorLogin: filters.actorLogin ?? undefined,
          operationId: filters.operationId ?? undefined,
          method: filters.method ?? undefined,
          statusCode: filters.statusCode ?? undefined,
          statusClass: filters.statusClass ?? undefined,
          outcome: filters.outcome ?? undefined,
          sessionId: filters.sessionId ?? undefined,
          repoFullName: filters.repoFullName ?? undefined,
          triggerIssue: filters.triggerIssue ?? undefined,
          requestId: filters.requestId ?? undefined,
          cursor,
        },
        signal
      );
    },
    [apiFetch, scope, filters]
  );

  const pages = older.key === cacheKey ? older.pages : [];

  const poll = useScopedPoll<ActivityPage>({
    key: cacheKey,
    intervalMs: ACTIVITY_POLL_MS,
    enabled: canQuery,
    // Live refresh stops while older pages are open; the first page is KEPT,
    // because discarding it is exactly what the pause exists to prevent.
    pollEnabled: pages.length === 0,
    fetcher: (signal) => fetchPage(undefined, signal),
  });

  // Any key change invalidates every cursor that was issued under the old one.
  useEffect(() => {
    setOlder((prev) => (prev.key === cacheKey ? prev : { key: cacheKey, pages: [] }));
    setOlderState({ loading: false, error: null });
    olderRequestRef.current += 1;
  }, [cacheKey]);
  const lastPage = pages.length > 0 ? pages[pages.length - 1] : poll.data;
  const nextCursor = lastPage?.next_cursor ?? null;

  const loadOlder = useCallback(() => {
    const cursor = nextCursor;
    if (cursor == null || olderState.loading) return;
    const requestId = ++olderRequestRef.current;
    const keyAtStart = cacheKey;
    setOlderState({ loading: true, error: null });
    const abort = new AbortController();
    fetchPage(cursor, abort.signal)
      .then((page) => {
        if (olderRequestRef.current !== requestId) return;
        setOlder((prev) =>
          prev.key === keyAtStart
            ? { key: keyAtStart, pages: [...prev.pages, page] }
            : prev
        );
        setOlderState({ loading: false, error: null });
      })
      .catch((error: unknown) => {
        if (olderRequestRef.current !== requestId) return;
        // A cursor the server refuses was issued for a query that no longer
        // exists — for a different viewer, scope, or session. Every already
        // loaded older page came from that same query, so they all go before a
        // retry can run.
        if (error instanceof OperationsError && rejectsCursor(error)) {
          setOlder({ key: keyAtStart, pages: [] });
        }
        setOlderState({ loading: false, error });
      });
  }, [cacheKey, fetchPage, nextCursor, olderState.loading]);

  const pollRefresh = poll.refresh;
  const refresh = useCallback(() => {
    olderRequestRef.current += 1;
    setOlder({ key: cacheKey, pages: [] });
    setOlderState({ loading: false, error: null });
    pollRefresh();
  }, [cacheKey, pollRefresh]);

  const rows = poll.data ? [...poll.data.items, ...pages.flatMap((page) => page.items)] : [];

  return {
    page: poll.data,
    rows,
    error: poll.error,
    loading: poll.loading && canQuery,
    refreshing: poll.refreshing,
    updatedAt: poll.updatedAt,
    hasMore: nextCursor != null,
    loadingOlder: olderState.loading,
    olderError: olderState.error,
    pollSuspended: pages.length > 0,
    loadOlder,
    refresh,
  };
}

/** Failures that mean the cursor itself is no longer valid for this caller. */
function rejectsCursor(error: OperationsError): boolean {
  return (
    error.code === 'invalid_activity_cursor' ||
    error.code === 'operations_scope_forbidden' ||
    error.code === 'scope_mismatch' ||
    error.status === 401
  );
}
