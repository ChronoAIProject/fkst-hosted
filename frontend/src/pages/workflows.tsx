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
import { WorkflowsGate, WorkflowsUnconfigured } from './workflows-gate';

/**
 * `/workflows` — a repository's scheduled workflows.
 *
 * Three rules shape this page.
 *
 * **The URL is the state.** `?repo=owner/name`, `?schedule=<issue>`, and
 * `?run=<slot>` are the whole navigational surface, so every view is a link an
 * operator can send to a colleague, and the browser's back button works without
 * this component keeping a history of its own.
 *
 * **No cadence arithmetic lives here.** `nextDue` and `upcoming` arrive from the
 * API, which computes them with the same code the control plane's clock uses. A
 * second implementation in TypeScript would eventually drift, and the symptom
 * would be a dashboard confidently showing a firing time the schedule does not
 * honour. The only time arithmetic on this page renders a distance the API has
 * already committed to.
 *
 * **A mutation re-reads rather than guessing.** Pause, resume, and run-now all
 * change durable GitHub state that the reconciler also writes, so the page
 * refetches instead of optimistically patching a local copy — an optimistic
 * update here would be a second, quieter source of truth.
 */
export function Workflows() {
  const t = useContent().workflows;
  const { configured, isAuthenticated, identityGeneration, error, signIn, apiFetch } = useAuth();
  const [searchParams, setSearchParams] = useSearchParams();

  const repo = searchParams.get('repo') ?? '';
  const scheduleParam = searchParams.get('schedule');
  const runParam = searchParams.get('run');
  const scheduleIssue = scheduleParam ? Number(scheduleParam) : null;

  const [repoInput, setRepoInput] = useState(repo);
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

  // `noUncheckedIndexedAccess` is on, so the split yields `string | undefined`.
  // Coercing to '' rather than asserting keeps the "valid" guard the single
  // place that decides whether a request may be made at all.
  const [ownerPart, namePart] = repo.split('/');
  const owner = ownerPart ?? '';
  const name = namePart ?? '';
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
  }, [apiFetch, isAuthenticated, identityGeneration, owner, name, valid, scheduleIssue, runParam, reload]);

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

  if (!configured) return <WorkflowsUnconfigured />;
  if (!isAuthenticated) {
    return <WorkflowsGate error={error} configured={configured} onSignIn={signIn} />;
  }

  return (
    <div className="h-full flex flex-col gap-4 min-h-0">
      <header className="flex flex-wrap items-end gap-3 flex-none">
        <h1 className="font-display font-semibold text-[20px] text-fg">{t.title}</h1>
        <label className="flex flex-col gap-1">
          <span className="font-mono text-[9.5px] uppercase tracking-[0.14em] text-ghost">
            {t.repoLabel}
          </span>
          <input
            value={repoInput}
            onChange={(event) => setRepoInput(event.target.value)}
            onBlur={() => setParam({ repo: repoInput || null, schedule: null, run: null })}
            onKeyDown={(event) => {
              if (event.key === 'Enter') {
                setParam({ repo: repoInput || null, schedule: null, run: null });
              }
            }}
            placeholder={t.repoPlaceholder}
            aria-label={t.repoLabel}
            className="w-[240px] rounded-control border border-line bg-raise px-2 py-1 font-mono text-[12px] text-fg"
          />
        </label>
        <span className="font-ui text-[11.5px] text-ghost">{t.repoHint}</span>
      </header>

      <div className="flex-1 min-h-0 overflow-auto">
        {!valid ? null : loadError ? (
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
    </div>
  );
}
