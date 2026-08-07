import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { LoadingState } from '@/components/ui/loading';
import { ScrollArea } from '@/components/ui/scroll-area';
import { SplitPanes } from '@/components/session-detail/parts';
import {
  getSchedule,
  getScheduleRun,
  listRepoSchedules,
  pauseSchedule,
  resumeSchedule,
  runScheduleNow,
} from '@/lib/api/schedules';
import type {
  RepoSchedulesResponse,
  ScheduleDetail as ScheduleDetailData,
  ScheduleRunDetail,
  ScheduleSummary,
} from '@/lib/api/schedules';
import { ScheduleRail } from './schedule-rail';
import { ScheduleDetail } from './schedule-detail';

/** How often the in-flight run's age is re-rendered. A schedule's minimum
 *  cadence is fifteen minutes, so nothing else on this surface benefits from a
 *  timer — but a run that has been going for four minutes and still says "1m"
 *  reads as a stuck UI, so the one live number gets a coarse tick. Torn down the
 *  moment nothing is in flight. */
const LIVE_TICK_MS = 15_000;

/** Every non-split state of this tab owns a scroll region.
 *
 *  The tab panel hands a master/detail tab a fixed BOX rather than a scroller,
 *  because the loaded state manages two scrollers of its own. So each of the
 *  short states has to supply one, or a long error or empty-state explanation
 *  would be clipped with no way to reach it. Logs solves this the same way. */
function Short({ children }: { children: ReactNode }) {
  return (
    <div data-testid="session-workflows" className="flex flex-1 min-h-0 flex-col">
      <ScrollArea className="pr-1">
        <div className="flex flex-col items-start gap-2">{children}</div>
      </ScrollArea>
    </div>
  );
}

/**
 * The scheduled workflows one SESSION owns.
 *
 * A schedule is assigned to a session creator, its run issue is routed to that
 * creator by sole assignee, and the run executes inside that session's pod. A
 * repository may host several creators' sessions, so a repository-level list
 * mixed schedules that different sessions own and could never run for each
 * other — which is why this surface is scoped to the session rather than to the
 * repository it lives in.
 *
 * The endpoint is still repository-scoped (a schedule IS a repository issue), so
 * the filtering happens here, on `creator`. GitHub logins are case-insensitive,
 * so the comparison is too.
 *
 * Two rules survive from the repository-level view unchanged. **No cadence
 * arithmetic lives here** — `nextDue` and `upcoming` arrive from the API, which
 * computes them with the same code the control plane's clock uses, and a second
 * implementation in TypeScript would eventually show a firing the schedule does
 * not honour. And **a mutation re-reads rather than guessing**: pause, resume,
 * and run-now change durable GitHub state the reconciler also writes, so an
 * optimistic local patch would be a second, quieter source of truth.
 */
export function SessionWorkflows({
  owner,
  name,
  creator,
}: {
  owner: string;
  name: string;
  /** The session's effective creator — the routing key a schedule belongs by. */
  creator: string;
}) {
  const c = useContent();
  const t = c.workflows;
  const { isAuthenticated, identityGeneration, apiFetch } = useAuth();

  const [list, setList] = useState<RepoSchedulesResponse | null>(null);
  const [detail, setDetail] = useState<ScheduleDetailData | null>(null);
  const [run, setRun] = useState<ScheduleRunDetail | null>(null);
  const [selectedIssue, setSelectedIssue] = useState<number | null>(null);
  const [openSlot, setOpenSlot] = useState<string | null>(null);
  const [loadError, setLoadError] = useState(false);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  // One clock read per load rather than a ticking timer: the finest useful
  // resolution for a schedule whose minimum cadence is fifteen minutes is a
  // minute, so a per-second re-render would buy nothing and cost every row.
  // `detailLoadedAt` anchors the live elapsed below to the response it extends.
  const [now, setNow] = useState(() => Date.now());
  const [detailLoadedAt, setDetailLoadedAt] = useState<number | null>(null);
  // Bumped by every mutation to force a refetch without duplicating the loader.
  const [reload, setReload] = useState(0);

  useEffect(() => {
    if (!isAuthenticated) {
      setList(null);
      return;
    }
    let cancelled = false;
    setLoadError(false);
    listRepoSchedules(apiFetch, owner, name)
      .then((response) => {
        if (cancelled) return;
        setList(response);
        setNow(Date.now());
      })
      .catch(() => {
        if (!cancelled) setLoadError(true);
      });
    return () => {
      cancelled = true;
    };
  }, [apiFetch, isAuthenticated, identityGeneration, owner, name, reload]);

  // Partition once per list: this session's schedules, and the ones that route
  // to no session at all (see ScheduleRail's UnroutedSection for why those are
  // still shown). Another creator's schedules are dropped — this session can
  // neither run nor operate them.
  const { mine, unrouted } = useMemo(() => {
    const schedules = list?.schedules ?? [];
    const wanted = creator.toLowerCase();
    return {
      mine: schedules.filter((s) => s.creator?.toLowerCase() === wanted),
      unrouted: schedules.filter((s) => s.creator === null),
    };
  }, [list, creator]);

  // Selection is stored as the schedule's ISSUE NUMBER, not an index or an
  // object, so it survives a reload that reorders or replaces the array. A miss
  // falls back to the first schedule, so the pane is never blank while the
  // session still owns one.
  const selected: ScheduleSummary | null =
    mine.find((s) => s.scheduleIssue === selectedIssue) ?? mine[0] ?? null;
  const selectedScheduleIssue = selected?.scheduleIssue ?? null;

  useEffect(() => {
    if (!isAuthenticated || selectedScheduleIssue === null) {
      setDetail(null);
      setDetailLoadedAt(null);
      return;
    }
    let cancelled = false;
    getSchedule(apiFetch, owner, name, selectedScheduleIssue)
      .then((response) => {
        if (cancelled) return;
        setDetail(response);
        setDetailLoadedAt(Date.now());
        setNow(Date.now());
      })
      .catch(() => {
        if (!cancelled) {
          setDetail(null);
          setDetailLoadedAt(null);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [apiFetch, isAuthenticated, identityGeneration, owner, name, selectedScheduleIssue, reload]);

  useEffect(() => {
    if (!isAuthenticated || selectedScheduleIssue === null || !openSlot) {
      setRun(null);
      return;
    }
    let cancelled = false;
    getScheduleRun(apiFetch, owner, name, selectedScheduleIssue, openSlot)
      .then((response) => {
        if (!cancelled) setRun(response);
      })
      .catch(() => {
        if (!cancelled) setRun(null);
      });
    return () => {
      cancelled = true;
    };
  }, [
    apiFetch,
    isAuthenticated,
    identityGeneration,
    owner,
    name,
    selectedScheduleIssue,
    openSlot,
    reload,
  ]);

  const inFlight = detail?.latestRun?.run.status === 'dispatched';

  // The one live number on this surface. Only while something is actually
  // running, so an idle schedule costs no timer at all.
  useEffect(() => {
    if (!inFlight) return;
    const timer = window.setInterval(() => setNow(Date.now()), LIVE_TICK_MS);
    return () => window.clearInterval(timer);
  }, [inFlight]);

  // Extend the SERVER's number rather than re-deriving an age from `startedAt`:
  // the API computed `elapsedS` against the clock the reconciler uses, and a
  // browser clock minutes off would otherwise render a run as having started in
  // the future.
  const liveElapsedS =
    inFlight && detail?.latestRun && detailLoadedAt !== null
      ? (detail.latestRun.run.elapsedS ?? 0) + Math.max(0, Math.round((now - detailLoadedAt) / 1000))
      : null;

  const selectRun = useCallback((slot: string) => {
    // Clicking the open run closes it — the row is a disclosure, not a radio.
    setOpenSlot((current) => (current === slot ? null : slot));
  }, []);

  const selectSchedule = useCallback((scheduleIssue: number) => {
    setSelectedIssue(scheduleIssue);
    // A different schedule's history is a different set of slots, so an open one
    // must not survive the switch and request a slot this schedule never had.
    setOpenSlot(null);
    setActionError(null);
  }, []);

  const act = useCallback(
    async (perform: () => Promise<{ ok: boolean; message?: string | null }>) => {
      setBusy(true);
      setActionError(null);
      const result = await perform();
      setBusy(false);
      if (!result.ok) {
        setActionError(result.message ?? t.actionFailed);
        return;
      }
      // Re-read rather than patch: these mutations change durable GitHub state
      // the reconciler also writes, so a local guess would be a second and
      // quieter source of truth.
      setReload((value) => value + 1);
    },
    [t.actionFailed]
  );

  if (loadError) {
    return (
      <Short>
        <p className="font-ui text-[13px] text-red">{t.loadFailed}</p>
        <button
          type="button"
          onClick={() => setReload((value) => value + 1)}
          className="self-start font-ui text-[12.5px] text-fg border border-line rounded-control px-3 py-1.5 cursor-pointer"
        >
          {t.retry}
        </button>
      </Short>
    );
  }
  if (!list) {
    return (
      <Short>
        <LoadingState label={t.loading} detail={c.loading.service} />
      </Short>
    );
  }
  if (!list.installed) {
    return (
      <Short>
        <p className="font-ui text-[13px] text-warn">{t.notInstalled}</p>
      </Short>
    );
  }
  if (mine.length === 0 && unrouted.length === 0) {
    return (
      <Short>
        <div className="flex flex-col items-start gap-2 max-w-[64ch]">
          <h3 className="font-display font-semibold text-[15px] text-fg">{t.emptyTitle}</h3>
          <p className="font-ui text-[13px] leading-relaxed text-dim">{t.emptyBody}</p>
          <a
            href={`https://github.com/${owner}/${name}/issues/new?template=fkst-scheduled-workflow.md`}
            target="_blank"
            rel="noreferrer"
            className="font-ui text-[12.5px] text-amber hover:brightness-110"
          >
            {t.emptyAction}
          </a>
        </div>
      </Short>
    );
  }

  return (
    <div data-testid="session-workflows" className="flex flex-1 min-h-0 flex-col">
      <SplitPanes
        // Wider than Health's rail: a row carries a workflow id plus a lifecycle
        // chip, and the workflow id is what a schedule is picked by.
        startTrack="14rem"
        start={
          <ScheduleRail
            schedules={mine}
            unrouted={unrouted}
            selectedIssue={selectedScheduleIssue}
            now={now}
            onSelect={selectSchedule}
          />
        }
        end={
          detail && selected ? (
            <ScheduleDetail
              owner={owner}
              name={name}
              detail={detail}
              run={run}
              liveElapsedS={liveElapsedS}
              now={now}
              busy={busy}
              actionError={actionError}
              onSelectRun={selectRun}
              onRunNow={() =>
                act(() => runScheduleNow(apiFetch, owner, name, selected.scheduleIssue))
              }
              onPause={() =>
                act(() => pauseSchedule(apiFetch, owner, name, selected.scheduleIssue))
              }
              onResume={() =>
                act(() => resumeSchedule(apiFetch, owner, name, selected.scheduleIssue))
              }
            />
          ) : (
            // Two shapes reach here: the first detail is still in flight, or the
            // session owns nothing but the rail's unrouted section — which is
            // never selectable, so there is nothing to show beside it.
            <ScrollArea className="pr-1">
              {selected ? (
                <LoadingState label={t.loading} detail={c.loading.service} />
              ) : (
                <p className="font-ui text-[12.5px] text-ghost italic">{t.unroutedOnly}</p>
              )}
            </ScrollArea>
          )
        }
      />
    </div>
  );
}
