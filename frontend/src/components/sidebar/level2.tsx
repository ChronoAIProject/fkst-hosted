import { useCallback, useEffect, useRef, useState } from 'react';
import { useContent, useLang } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { stopTrigger } from '@/lib/api/canvas';
import { formatRelative } from '@/lib/format';
import type { RepoSessionsResponse, SessionDetail } from '@/lib/api/types';
import { ConfirmDialog } from '@/components/modals/confirm-dialog';
import { CreateTriggerModal } from '@/components/modals/create-trigger-modal';
import { Spinner } from '@/components/session-detail/parts';
import { StaggerItem } from '@/components/ui/motion';
import { StatusLegend, ViewDescription } from './legend';
import { SessionCard } from './session-card';

/** How often the freshness line re-renders so its relative "N min ago" text
 *  advances between the parent's silent polls. Minute-grained buckets don't
 *  need a tighter cadence. */
const FRESHNESS_TICK_MS = 30_000;

/** Safety net so the manual-refresh spinner can never hang. A repeated failure
 *  with no prior data leaves both props unchanged (data stays null, loadFailed
 *  stays true), so the resolve-detection effect below sees nothing to react to;
 *  this timeout force-clears the spinner in that one case. */
const REFRESH_SPINNER_MAX_MS = 20_000;

/** Level-2 sidebar: the sessions of one repository — trigger list with
 *  config metadata and outcomes, session creation, and session stop. */
export function Level2Sidebar({
  owner,
  name,
  data,
  loadFailed,
  onChanged,
}: {
  owner: string;
  name: string;
  /** Poll payload; null while the first fetch is in flight. */
  data: RepoSessionsResponse | null;
  loadFailed: boolean;
  /** A trigger was created or stopped — the page re-fetches immediately. */
  onChanged: () => void;
}) {
  const c = useContent().dashboard;
  const cc = c.canvas;
  const { lang } = useLang();
  const { apiFetch } = useAuth();
  const [showCreate, setShowCreate] = useState(false);
  const [stopTarget, setStopTarget] = useState<SessionDetail | null>(null);

  // Freshness bookkeeping. `lastUpdated` is the wall-clock of the most recent
  // SUCCESSFUL poll (data present, no failure) — seeded on mount when the first
  // payload is already in hand so the header shows a time immediately rather
  // than a blank until the next poll. `tick` only exists to re-render the
  // relative label as time passes.
  const [lastUpdated, setLastUpdated] = useState<number | null>(() =>
    data != null && !loadFailed ? Date.now() : null
  );
  const [, setTick] = useState(0);
  // True while a caller-initiated refresh (Retry, or the header affordance) is
  // in flight — the parent owns the fetch, so completion is inferred from the
  // next prop change (see the resolve effect).
  const [refreshing, setRefreshing] = useState(false);
  const spinnerTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Previous prop identities, so the resolve effect can tell a real poll result
  // (new `data` object, or a flipped `loadFailed`) from an unrelated re-render.
  const prevData = useRef(data);
  const prevFailed = useRef(loadFailed);

  useEffect(() => {
    const dataChanged = data !== prevData.current;
    const failedChanged = loadFailed !== prevFailed.current;
    if (!dataChanged && !failedChanged) return;
    prevData.current = data;
    prevFailed.current = loadFailed;
    // A poll (silent or requested) just resolved: stamp freshness on success,
    // and end any in-flight spinner regardless of outcome.
    if (data != null && !loadFailed) setLastUpdated(Date.now());
    setRefreshing(false);
    if (spinnerTimer.current != null) {
      clearTimeout(spinnerTimer.current);
      spinnerTimer.current = null;
    }
  }, [data, loadFailed]);

  // Advance the relative-time label between polls. Only runs while a timestamp
  // exists — nothing to age otherwise.
  useEffect(() => {
    if (lastUpdated == null) return;
    const id = setInterval(() => setTick((n) => n + 1), FRESHNESS_TICK_MS);
    return () => clearInterval(id);
  }, [lastUpdated]);

  useEffect(
    () => () => {
      if (spinnerTimer.current != null) clearTimeout(spinnerTimer.current);
    },
    []
  );

  const requestRefresh = useCallback(() => {
    setRefreshing(true);
    if (spinnerTimer.current != null) clearTimeout(spinnerTimer.current);
    spinnerTimer.current = setTimeout(() => {
      setRefreshing(false);
      spinnerTimer.current = null;
    }, REFRESH_SPINNER_MAX_MS);
    onChanged();
  }, [onChanged]);

  const full = `${owner}/${name}`;

  return (
    <div className="flex flex-col gap-4">
      <ViewDescription text={cc.viewRepo.replace('{repo}', full)} />
      <StatusLegend />

      <div className="flex items-center justify-between gap-2 flex-wrap">
        <h3 className="font-display font-semibold text-[15px] text-fg">
          {cc.sessionsTitle}
          {data != null && (
            <span className="font-mono text-[11px] text-ghost ml-2">· {data.sessions.length}</span>
          )}
        </h3>
        <button
          type="button"
          onClick={() => setShowCreate(true)}
          className="font-ui font-semibold text-[12px] bg-amber text-amber-ink rounded-control px-3 py-1.5 transition-colors hover:brightness-[1.06] cursor-pointer"
        >
          {cc.newTrigger}
        </button>
      </div>

      {/* Live freshness in place of the old static poll note: "updated 2 min
          ago", advancing on every successful poll, with an inline spinner while
          a manual refresh is in flight. The poll cadence lives on in the
          tooltip. */}
      {data != null && lastUpdated != null && (
        <p
          title={cc.pollNote}
          className="flex items-center gap-1.5 font-mono text-[10.5px] text-ghost"
        >
          {cc.sessionsFreshness.replace('{time}', formatRelative(lastUpdated, lang))}
          {refreshing && <Spinner className="w-2.5 h-2.5" />}
        </p>
      )}

      {data != null && !data.installed && (
        <p className="font-mono text-[12px] text-ghost">{cc.notInstalledNote}</p>
      )}

      {/* A failed refresh with last-good data present keeps the list and
          only flags staleness; the blocking error is for no-data-at-all. */}
      {loadFailed && data != null && (
        <p className="border border-line border-l-2 border-l-amber rounded-card bg-[color-mix(in_oklab,var(--raise)_55%,transparent)] px-3 py-2 font-mono text-[11.5px] text-dim">
          {cc.sessionsStaleNotice}
        </p>
      )}

      {loadFailed && data == null ? (
        // The first fetch failed with nothing to show: recovery must not depend
        // on the silent 15 s poll, so offer an immediate Retry.
        <div className="border border-line border-l-2 border-l-red rounded-card bg-[color-mix(in_oklab,var(--raise)_55%,transparent)] px-4 py-3 flex flex-col gap-3 text-[13px] text-dim">
          <span>{cc.sessionsLoadFailed}</span>
          <button
            type="button"
            onClick={requestRefresh}
            disabled={refreshing}
            className="self-start inline-flex items-center gap-1.5 font-ui font-semibold text-[12px] border border-line rounded-control px-3 py-1.5 text-fg transition-colors hover:border-[color-mix(in_oklab,var(--amber)_45%,var(--line))] disabled:opacity-60 disabled:cursor-default cursor-pointer"
          >
            {refreshing && <Spinner className="w-2.5 h-2.5" />}
            {refreshing ? cc.sessionsRefreshing : cc.sessionsRetry}
          </button>
        </div>
      ) : data != null && data.sessions.length === 0 ? (
        <p className="font-mono text-[12px] text-ghost italic">{c.noSessions}</p>
      ) : data != null ? (
        <div className="flex flex-col gap-3">
          {data.sessions.map((session, i) => (
            // Stable per-repo key: session_id when present, else the trigger
            // number. Deliberately NOT the positional index — a poll that
            // reorders the list must not remount a card (which would slam an
            // open detail drawer shut).
            <StaggerItem
              key={session.session_id ?? `trigger-${session.trigger.number}`}
              index={i}
            >
              <SessionCard owner={owner} name={name} session={session} onStop={setStopTarget} />
            </StaggerItem>
          ))}
        </div>
      ) : null}

      {showCreate && (
        <CreateTriggerModal
          owner={owner}
          name={name}
          onClose={() => setShowCreate(false)}
          onCreated={() => {
            setShowCreate(false);
            onChanged();
          }}
        />
      )}

      {stopTarget != null && (
        <ConfirmDialog
          title={cc.stopConfirmTitle.replace(
            '{name}',
            stopTarget.name ?? `#${stopTarget.trigger.number}`
          )}
          body={cc.stopConfirmBody.replace('{number}', String(stopTarget.trigger.number))}
          confirmLabel={cc.stopConfirm}
          pendingLabel={cc.stopPending}
          cancelLabel={c.repos.cancel}
          action={() => stopTrigger(apiFetch, owner, name, stopTarget.trigger.number)}
          fallbackError={cc.stopFailed}
          onClose={() => setStopTarget(null)}
          onDone={() => {
            setStopTarget(null);
            onChanged();
          }}
        />
      )}
    </div>
  );
}
