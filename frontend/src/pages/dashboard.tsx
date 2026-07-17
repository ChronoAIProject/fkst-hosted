import { useCallback, useEffect, useRef, useState } from 'react';
import { motion } from 'framer-motion';
import { Eyebrow } from '@/components/layout/eyebrow';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { getOverview, getRepoSessions } from '@/lib/api/canvas';
import type { OverviewResponse, RepoSessionsResponse } from '@/lib/api/types';
import { filterAccounts, filterRepos } from '@/lib/api/derive';
import { CanvasBreadcrumb } from '@/components/canvas/breadcrumb';
import { CanvasFlow } from '@/components/canvas/flow';
import { levelKey, parentLevel } from '@/components/canvas/level';
import type { CanvasLevel } from '@/components/canvas/level';
import { CanvasSkeleton, SidebarSkeleton } from '@/components/canvas/skeletons';
import type { UserRepo } from '@/components/modals/create-repo-modal';
import { Level0Sidebar } from '@/components/sidebar/level0';
import { Level1Sidebar } from '@/components/sidebar/level1';
import { Level2Sidebar } from '@/components/sidebar/level2';
import { SidebarPanel } from '@/components/sidebar/panel';
import { useVisibilityPoll } from '@/lib/hooks/use-visibility-poll';

// Re-exported from its new home so existing imports (tests included) hold.
export { formatSgt } from '@/lib/format';

/** How often the level-2 session view refreshes while mounted and visible. */
const SESSIONS_POLL_MS = 15_000;

/**
 * The dashboard page is a thin orchestrator: it owns the fetches (overview +
 * per-repo sessions with polling), the canvas level, and the name filters —
 * everything else lives in components/canvas/* and components/sidebar/*.
 */
export function Dashboard() {
  const c = useContent();
  const d = c.dashboard;
  const cc = d.canvas;
  const { configured, isAuthenticated, error, signIn, apiFetch } = useAuth();

  const [overview, setOverview] = useState<OverviewResponse | null>(null);
  const [overviewFailed, setOverviewFailed] = useState(false);
  const [tick, setTick] = useState(0);

  const [level, setLevel] = useState<CanvasLevel>({ kind: 'root' });
  const [accountQuery, setAccountQuery] = useState('');
  const [repoQuery, setRepoQuery] = useState('');
  // `owner/name` of a repo created via the modal — highlighted at level 1.
  const [createdKey, setCreatedKey] = useState<string | null>(null);

  const [sessions, setSessions] = useState<RepoSessionsResponse | null>(null);
  const [sessionsFailed, setSessionsFailed] = useState(false);
  // Guards stale async responses after the level moved on.
  const levelRef = useRef(levelKey(level));
  levelRef.current = levelKey(level);

  useEffect(() => {
    document.title = d.metaTitle;
  }, [d.metaTitle]);

  // Load (and on `tick` bumps, re-load) the overview. Existing data is kept
  // during a refetch, so refreshes never blank the canvas.
  useEffect(() => {
    if (!isAuthenticated || !configured) return;
    let active = true;
    setOverviewFailed(false);
    getOverview(apiFetch)
      .then((body) => {
        if (active) setOverview(body);
      })
      .catch(() => {
        if (active) setOverviewFailed(true);
      });
    return () => {
      active = false;
    };
  }, [isAuthenticated, configured, apiFetch, tick]);

  // Level-2 sessions: re-fetch keeping the current frame (used by the poll
  // and after mutations); the level-change effect below handles the reset.
  const refreshSessions = useCallback(() => {
    if (level.kind !== 'repo') return;
    const requestedFor = levelKey(level);
    getRepoSessions(apiFetch, level.owner, level.name)
      .then((body) => {
        if (levelRef.current !== requestedFor) return; // stale — level moved on
        setSessions(body);
        setSessionsFailed(false);
      })
      .catch(() => {
        if (levelRef.current !== requestedFor) return;
        setSessionsFailed(true);
      });
  }, [level, apiFetch]);

  // Entering (or switching) a repo clears the old repo's data → skeleton,
  // then fetches. Leaving level 2 just drops the data.
  const currentLevelKey = levelKey(level);
  useEffect(() => {
    setSessions(null);
    setSessionsFailed(false);
    if (level.kind === 'repo') refreshSessions();
    // Reacting to the level identity only: refreshSessions is re-created with
    // `level`, so listing it here would double-fire every fetch.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentLevelKey]);

  useVisibilityPoll(refreshSessions, SESSIONS_POLL_MS, level.kind === 'repo');

  // Escape mirrors the Back button — unless a dialog is open (dialogs own
  // Escape; ModalShell also stops propagation as the first line of defense)
  // or the key was pressed inside an editable field (WebKit/Blink natively
  // clear a search input on Escape; that must not also change the level).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      if (e.target instanceof Element && e.target.closest('input, textarea, select, [contenteditable]')) {
        return;
      }
      if (document.querySelector('[role="dialog"]')) return;
      setLevel((current) => parentLevel(current) ?? current);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const openAccount = useCallback((login: string) => {
    setRepoQuery('');
    setLevel({ kind: 'account', login });
  }, []);

  const openRepo = useCallback((owner: string, name: string) => {
    setLevel({ kind: 'repo', owner, name });
  }, []);

  const navigate = useCallback((target: CanvasLevel) => {
    if (target.kind === 'account') setRepoQuery('');
    setLevel(target);
  }, []);

  const refetchOverview = useCallback(() => setTick((t) => t + 1), []);

  // A repo was created: clear filters so it is visible, re-fetch, and zoom
  // into its owner account where the new row is highlighted with the
  // install-next callout.
  const onRepoCreated = useCallback((repo: UserRepo) => {
    setCreatedKey(`${repo.owner}/${repo.name}`);
    setAccountQuery('');
    setRepoQuery('');
    setTick((t) => t + 1);
    setLevel({ kind: 'account', login: repo.owner });
  }, []);

  // A trigger was created/stopped: refresh the session list now and the
  // overview counts quietly behind it.
  const onSessionsChanged = useCallback(() => {
    refreshSessions();
    setTick((t) => t + 1);
  }, [refreshSessions]);

  // The account a non-root level points at (owner login at level 2).
  const selectedLogin =
    level.kind === 'account' ? level.login : level.kind === 'repo' ? level.owner : null;
  const selectedAccount =
    selectedLogin != null
      ? (overview?.accounts.find((a) => a.login === selectedLogin) ?? null)
      : null;

  // If a refetch dropped the selected account (uninstall, org left), the
  // level points at nothing — fall back to the root view.
  useEffect(() => {
    if (overview != null && selectedLogin != null && selectedAccount == null) {
      setLevel({ kind: 'root' });
    }
  }, [overview, selectedLogin, selectedAccount]);

  const header = (
    <header>
      <Eyebrow>{d.eyebrow}</Eyebrow>
      <h1 className="mt-5 font-display font-bold text-[clamp(28px,4vw,40px)] leading-[1.1] tracking-[-0.02em] text-fg">
        {d.title}
      </h1>
      <p className="mt-5 text-[15px] leading-relaxed text-dim max-w-[68ch]">{d.lede}</p>
    </header>
  );

  if (!isAuthenticated) {
    return (
      <div className="flex flex-col gap-8 max-w-[960px]">
        {header}
        {error && (
          <div className="border border-line border-l-2 border-l-red rounded-card bg-[color-mix(in_oklab,var(--raise)_55%,transparent)] px-4 py-3 text-[13px] text-dim">
            {d.authError}
          </div>
        )}
        <section className="border border-line rounded-panel bg-raise p-8 max-[600px]:p-5 flex flex-col items-start gap-4">
          <h2 className="font-display font-semibold text-[20px] text-fg">{d.signInTitle}</h2>
          <p className="text-[14px] leading-relaxed text-dim max-w-[56ch]">{d.signInBody}</p>
          {configured ? (
            <button
              type="button"
              onClick={signIn}
              className="font-ui font-semibold text-[13.5px] bg-amber text-amber-ink rounded-control px-5 py-2.5 transition-colors hover:brightness-[1.06] cursor-pointer"
            >
              {c.auth.signIn}
            </button>
          ) : (
            <p className="font-mono text-[12px] text-ghost">{d.notConfigured}</p>
          )}
        </section>
      </div>
    );
  }

  if (!configured) {
    return (
      <div className="flex flex-col gap-8 max-w-[960px]">
        {header}
        <p className="font-mono text-[12px] text-ghost">{d.notConfigured}</p>
      </div>
    );
  }

  const filteredAccounts = overview != null ? filterAccounts(overview.accounts, accountQuery) : [];
  const filteredRepos = selectedAccount != null ? filterRepos(selectedAccount.repos, repoQuery) : [];
  const repoInstalled =
    sessions?.installed ??
    (level.kind === 'repo'
      ? (selectedAccount?.repos.find((r) => r.name === level.name)?.installed ?? false)
      : false);

  return (
    <div className="flex flex-col gap-6">
      {header}

      <div className="flex items-center gap-3 flex-wrap pb-3 border-b border-line">
        <CanvasBreadcrumb level={level} onNavigate={navigate} />
        <span className="flex-1" aria-hidden="true" />
        <button
          type="button"
          onClick={refetchOverview}
          className="font-ui font-semibold text-[12px] border border-line rounded-control px-3 py-1.5 text-dim hover:text-fg transition-colors cursor-pointer"
        >
          {d.repos.refresh}
        </button>
      </div>

      {overviewFailed && overview == null && (
        <div className="border border-line border-l-2 border-l-red rounded-card bg-[color-mix(in_oklab,var(--raise)_55%,transparent)] px-4 py-3 text-[13px] text-dim">
          {d.repos.loadFailed}
        </div>
      )}

      <div className="flex gap-5 items-stretch max-[1100px]:flex-col">
        <section
          aria-label={cc.canvasAria}
          className="flex-1 min-w-0 border border-line rounded-panel bg-bg overflow-hidden h-[640px] max-[1100px]:h-[440px]"
        >
          {overview == null ? (
            !overviewFailed && <CanvasSkeleton />
          ) : (
            <motion.div
              className="w-full h-full"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              transition={{ duration: 0.25 }}
            >
              <CanvasFlow
                level={level}
                accounts={filteredAccounts}
                repos={filteredRepos}
                repoSessions={sessions}
                repoInstalled={repoInstalled}
                onOpenAccount={openAccount}
                onOpenRepo={openRepo}
              />
            </motion.div>
          )}
        </section>

        <SidebarPanel level={level}>
          {overview == null ? (
            <SidebarSkeleton />
          ) : level.kind === 'root' ? (
            <Level0Sidebar
              overview={overview}
              query={accountQuery}
              onQueryChange={setAccountQuery}
              onOpenAccount={openAccount}
              onRepoCreated={onRepoCreated}
              onChanged={refetchOverview}
            />
          ) : level.kind === 'account' ? (
            selectedAccount != null ? (
              <Level1Sidebar
                account={selectedAccount}
                appSlug={overview.app_slug}
                query={repoQuery}
                onQueryChange={setRepoQuery}
                createdKey={createdKey}
                onOpenRepo={openRepo}
              />
            ) : (
              <SidebarSkeleton />
            )
          ) : sessions == null && !sessionsFailed ? (
            <SidebarSkeleton />
          ) : (
            <Level2Sidebar
              owner={level.owner}
              name={level.name}
              data={sessions}
              loadFailed={sessionsFailed}
              onChanged={onSessionsChanged}
            />
          )}
        </SidebarPanel>
      </div>
    </div>
  );
}
