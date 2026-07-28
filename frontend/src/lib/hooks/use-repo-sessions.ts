import { useCallback, useEffect, useRef, useState } from 'react';
import { levelKey } from '@/components/canvas/level';
import type { CanvasLevel } from '@/components/canvas/level';
import { getRepoSessions } from '@/lib/api/canvas';
import type { ApiFetch } from '@/lib/api/canvas';
import type { RepoSessionsResponse } from '@/lib/api/types';
import { useVisibilityPoll } from '@/lib/hooks/use-visibility-poll';

/** How often the level-2 session view refreshes while mounted and visible. */
const SESSIONS_POLL_MS = 15_000;

/**
 * The repo-level session projection: fetch, poll, and the race-guards that keep it
 * honest. Extracted from the dashboard page so that page stays the thin
 * orchestrator its doc comment claims to be.
 *
 * Four guards, each earned by a real failure mode:
 *
 * - **Level-key guard** — a response only lands when it is still for the level the
 *   user is looking at, so leaving a repo mid-flight cannot paint its data over the
 *   next one.
 * - **Request-id guard** — for the SAME level, only the latest request may land. A
 *   slow poll racing a post-mutation refetch could otherwise resurrect a
 *   just-stopped session.
 * - **Single-flight** — a repo projection can legitimately take longer than the
 *   poll interval (many historical triggers needing package resolution), so
 *   background polls never stack; otherwise they would continually supersede the
 *   one response that could clear the loading skeleton.
 * - **Coalesced refresh** — a user or post-mutation refresh arriving during an
 *   in-flight request is stronger than a poll, so it is remembered and issued once
 *   the current request settles.
 */
export function useRepoSessions(level: CanvasLevel, apiFetch: ApiFetch) {
  const [sessions, setSessions] = useState<RepoSessionsResponse | null>(null);
  const [sessionsFailed, setSessionsFailed] = useState(false);

  // Guards stale async responses after the level moved on.
  const levelRef = useRef(levelKey(level));
  levelRef.current = levelKey(level);
  const requestIdRef = useRef(0);
  const inFlightRef = useRef<{ key: string; requestId: number } | null>(null);
  const refreshPendingRef = useRef(false);

  /** Re-fetch keeping the current frame on screen (used by the poll and after
   *  mutations). `queueIfBusy` marks a refresh to run once an in-flight request
   *  settles, instead of dropping it. */
  const refreshSessions = useCallback(
    (queueIfBusy = false) => {
      if (level.kind !== 'repo') return;
      const requestedFor = levelKey(level);
      if (inFlightRef.current?.key === requestedFor) {
        if (queueIfBusy) refreshPendingRef.current = true;
        return;
      }

      const startRequest = () => {
        const requestId = ++requestIdRef.current;
        inFlightRef.current = { key: requestedFor, requestId };
        const isCurrent = () =>
          levelRef.current === requestedFor && requestIdRef.current === requestId;
        getRepoSessions(apiFetch, level.owner, level.name)
          .then((body) => {
            if (!isCurrent()) return;
            setSessions(body);
            setSessionsFailed(false);
          })
          .catch(() => {
            if (!isCurrent()) return;
            setSessionsFailed(true);
          })
          .finally(() => {
            if (inFlightRef.current?.requestId !== requestId) return;
            inFlightRef.current = null;
            if (refreshPendingRef.current && levelRef.current === requestedFor) {
              refreshPendingRef.current = false;
              startRequest();
            }
          });
      };

      startRequest();
    },
    [level, apiFetch]
  );

  // Entering (or switching) a repo clears the old repo's data → skeleton, then
  // fetches. Leaving the repo level just drops the data.
  const currentLevelKey = levelKey(level);
  useEffect(() => {
    // Invalidate the single-flight slot even when a user leaves and re-enters the
    // same repository before its old request finishes. The request id still
    // prevents that abandoned response from landing over the new selection.
    inFlightRef.current = null;
    refreshPendingRef.current = false;
    setSessions(null);
    setSessionsFailed(false);
    if (level.kind === 'repo') refreshSessions();
    // Reacting to the level identity only: refreshSessions is re-created with
    // `level`, so listing it here would double-fire every fetch.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentLevelKey]);

  useVisibilityPoll(refreshSessions, SESSIONS_POLL_MS, level.kind === 'repo');

  return { sessions, sessionsFailed, refreshSessions };
}
