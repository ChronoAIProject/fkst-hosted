import { useCallback } from 'react';
import type { ApiFetch } from '@/lib/api/canvas';
import { clearsLastGood, getSandboxes } from '@/lib/api/operations';
import type { SandboxInventory, SandboxScope } from '@/lib/api/operations';
import type { SandboxFilters } from '@/lib/operations/state';
import { useScopedPoll } from './use-scoped-poll';

/** Live runtimes refresh every 5 seconds while their view is visible (epic
 *  `UI-02`) — the cadence a starting, failing, or expiring Pod needs. */
export const SANDBOX_POLL_MS = 5_000;

/** How old a snapshot may be before it is labelled stale (epic `UI-02`). One
 *  missed 5s poll is normal jitter; three is a stopped feed. */
export const SANDBOX_STALE_MS = 15_000;

export interface SandboxFeed {
  inventory: SandboxInventory | null;
  error: unknown;
  loading: boolean;
  refreshing: boolean;
  updatedAt: number | null;
  refresh: () => void;
}

export interface SandboxFeedOptions {
  apiFetch: ApiFetch;
  cacheKey: string;
  /** `null` means "let the server pick", used only before the first response. */
  scope: SandboxScope | null;
  filters: SandboxFilters;
  enabled: boolean;
}

/**
 * The live sandbox feed.
 *
 * There is no pagination and no cursor: the response is a COMPLETE authorized
 * snapshot of one instant, and stitching two snapshots together would produce a
 * fleet that never existed. A failure therefore keeps the previous snapshot on
 * screen — but only while the cache key (identity, scope, filters) is unchanged,
 * which the polling engine enforces by construction, and only while the failure
 * is a freshness problem rather than an authorization or validation one (see
 * `clearsLastGood`).
 */
export function useOperationsSandboxes({
  apiFetch,
  cacheKey,
  scope,
  filters,
  enabled,
}: SandboxFeedOptions): SandboxFeed {
  const fetcher = useCallback(
    (signal: AbortSignal) =>
      getSandboxes(
        apiFetch,
        {
          scope,
          status: filters.status ?? undefined,
          backend: filters.backend ?? undefined,
          creatorId: filters.creatorId ?? undefined,
          creatorLogin: filters.creatorLogin ?? undefined,
          repoFullName: filters.repoFullName ?? undefined,
          sessionId: filters.sessionId ?? undefined,
          triggerIssue: filters.triggerIssue ?? undefined,
          attributionSource: filters.attributionSource ?? undefined,
        },
        signal
      ),
    [apiFetch, scope, filters]
  );

  const poll = useScopedPoll<SandboxInventory>({
    key: cacheKey,
    intervalMs: SANDBOX_POLL_MS,
    enabled,
    fetcher,
    clearsData: clearsLastGood,
  });

  return {
    inventory: poll.data,
    error: poll.error,
    loading: poll.loading,
    refreshing: poll.refreshing,
    updatedAt: poll.updatedAt,
    refresh: poll.refresh,
  };
}

/** Whether a snapshot observed at `observedAt` is stale at `now`. The instant
 *  compared is the BACKEND's own `observed_at`, never the moment the browser
 *  received it: a response that spent ten seconds in flight describes a fleet
 *  that is ten seconds old, and saying otherwise would be a lie. */
export function isSnapshotStale(observedAt: string | null | undefined, now: number): boolean {
  if (!observedAt) return false;
  const ms = Date.parse(observedAt);
  if (!Number.isFinite(ms)) return false;
  return now - ms > SANDBOX_STALE_MS;
}
