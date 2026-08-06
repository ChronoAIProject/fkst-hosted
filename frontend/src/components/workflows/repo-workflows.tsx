import { useCallback, useEffect, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
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
} from '@/lib/api/schedules';
import { ScheduleList } from '@/components/workflows/schedule-list';
import { ScheduleDetail } from '@/components/workflows/schedule-detail';

/**
 * One repository's scheduled workflows, rendered inside its workspace.
 *
 * This used to be a top-level `/workflows` route whose first job was asking
 * which repository you meant. A schedule is a repository's issue — it names a
 * workflow file in that repo, it is worked by that repo's session, and its runs
 * are that repo's run issues — so the repository is not a parameter to choose,
 * it is the context you are already in. The owner and name arrive as props and
 * there is no picker.
 *
 * Three rules survive the move unchanged.
 *
 * **The URL still carries the selection.** `?schedule=<issue>` and `?run=<slot>`
 * remain query parameters rather than local state, so a particular run is still
 * a link an operator can send to a colleague and the back button still works
 * without this component keeping a history of its own.
 *
 * **No cadence arithmetic lives here.** `nextDue` and `upcoming` arrive from the
 * API, which computes them with the same code the control plane's clock uses. A
 * second implementation in TypeScript would eventually drift, and the symptom
 * would be a dashboard confidently showing a firing time the schedule does not
 * honour. The only time arithmetic here renders a distance the API already
 * committed to.
 *
 * **A mutation re-reads rather than guessing.** Pause, resume, and run-now all
 * change durable GitHub state the reconciler also writes, so this refetches
 * instead of optimistically patching a local copy — an optimistic update would
 * be a second, quieter source of truth.
 */
export function RepoWorkflows({ owner, name }: { owner: string; name: string }) {
  const t = useContent().workflows;
  const { isAuthenticated, identityGeneration, apiFetch } = useAuth();
  const [searchParams, setSearchParams] = useSearchParams();

  const scheduleParam = searchParams.get('schedule');
  const runParam = searchParams.get('run');
  const scheduleIssue = scheduleParam ? Number(scheduleParam) : null;

  const [list, setList] = useState<RepoSchedulesResponse | null>(null);
  const [detail, setDetail] = useState<ScheduleDetailData | null>(null);
  const [run, setRun] = useState<ScheduleRunDetail | null>(null);
  const [loadError, setLoadError] = useState(false);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  // One clock read per data load rather than a ticking timer: the finest useful
  // resolution for a schedule whose minimum cadence is fifteen minutes is a
  // minute, so a per-second re-render would buy nothing and cost every row.
  const [now, setNow] = useState(() => Date.now());
  // Bumped by every mutation to force a refetch without duplicating the loader.
  const [reload, setReload] = useState(0);

  const valid = Boolean(owner && name);

  useEffect(() => {
    if (!isAuthenticated || !valid) {
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
  }, [apiFetch, isAuthenticated, identityGeneration, owner, name, valid, reload]);

  useEffect(() => {
    if (!isAuthenticated || !valid || scheduleIssue === null) {
      setDetail(null);
      return;
    }
    let cancelled = false;
    getSchedule(apiFetch, owner, name, scheduleIssue)
      .then((response) => {
        if (!cancelled) setDetail(response);
      })
      .catch(() => {
        if (!cancelled) setDetail(null);
      });
    return () => {
      cancelled = true;
    };
  }, [apiFetch, isAuthenticated, identityGeneration, owner, name, valid, scheduleIssue, reload]);

  useEffect(() => {
    if (!isAuthenticated || !valid || scheduleIssue === null || !runParam) {
      setRun(null);
      return;
    }
    let cancelled = false;
    getScheduleRun(apiFetch, owner, name, scheduleIssue, runParam)
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
    valid,
    scheduleIssue,
    runParam,
    reload,
  ]);

  const setParam = useCallback(
    (patch: Record<string, string | null>) => {
      const next = new URLSearchParams(searchParams);
      for (const [key, value] of Object.entries(patch)) {
        if (value === null) next.delete(key);
        else next.set(key, value);
      }
      setSearchParams(next, { replace: false });
    },
    [searchParams, setSearchParams]
  );

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

  if (!valid) return null;

  return (
    <div data-testid="repo-workflows" className="h-full min-h-0 overflow-auto">
      {loadError ? (
        <div className="flex flex-col items-start gap-2">
          <p className="font-ui text-[13px] text-red">{t.loadFailed}</p>
          <button
            type="button"
            onClick={() => setReload((value) => value + 1)}
            className="font-ui text-[12.5px] text-fg border border-line rounded-control px-3 py-1.5 cursor-pointer"
          >
            {t.retry}
          </button>
        </div>
      ) : list && !list.installed ? (
        <p className="font-ui text-[13px] text-warn">{t.notInstalled}</p>
      ) : detail && scheduleIssue !== null ? (
        <ScheduleDetail
          detail={detail}
          run={run}
          now={now}
          busy={busy}
          actionError={actionError}
          onBack={() => setParam({ schedule: null, run: null })}
          onSelectRun={(slot) => setParam({ run: runParam === slot ? null : slot })}
          onRunNow={() => act(() => runScheduleNow(apiFetch, owner, name, scheduleIssue))}
          onPause={() => act(() => pauseSchedule(apiFetch, owner, name, scheduleIssue))}
          onResume={() => act(() => resumeSchedule(apiFetch, owner, name, scheduleIssue))}
        />
      ) : list && list.schedules.length === 0 ? (
        <div className="flex flex-col items-start gap-2 max-w-[64ch]">
          <h2 className="font-display font-semibold text-[16px] text-fg">{t.emptyTitle}</h2>
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
      ) : list ? (
        <ScheduleList
          schedules={list.schedules}
          now={now}
          onOpen={(issue) => setParam({ schedule: String(issue), run: null })}
        />
      ) : null}
    </div>
  );
}
