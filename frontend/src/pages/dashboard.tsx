import { useCallback, useEffect, useRef, useState } from 'react';
import { FadeSwap } from '@/components/ui/motion';
import { Spinner } from '@/components/ui/loading';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { useBroaderOAuth } from '@/lib/auth/broader-oauth';
import { getOverview } from '@/lib/api/canvas';
import type { OverviewResponse } from '@/lib/api/types';
import { filterAccounts, filterRepos } from '@/lib/api/derive';
import { CanvasBreadcrumb } from '@/components/canvas/breadcrumb';
import { BroaderVisibilityBanner } from '@/components/canvas/broader-visibility';
import { CanvasFlow } from '@/components/canvas/flow';
import { parentLevel } from '@/components/canvas/level';
import type { CanvasLevel } from '@/components/canvas/level';
import { CanvasSkeleton, SidebarSkeleton } from '@/components/canvas/skeletons';
import type { UserRepo } from '@/components/modals/create-repo-modal';
import { Level0Sidebar } from '@/components/sidebar/level0';
import { Level1Sidebar } from '@/components/sidebar/level1';
import { RepoWorkspace } from '@/components/repo-workspace/repo-workspace';
import { SidebarPanel } from '@/components/sidebar/panel';
import { useTour } from '@/components/tour/tour-context';
import { useLevelParams } from '@/lib/hooks/use-level-params';
import { useRepoSessions } from '@/lib/hooks/use-repo-sessions';
import { DashboardGate, DashboardHeader, DashboardUnconfigured } from './dashboard-gate';

// Re-exported from its new home so existing imports (tests included) hold.
export { formatSgt } from '@/lib/format';

/**
 * The dashboard page is a thin orchestrator: it owns the fetches (overview +
 * per-repo sessions with polling), the canvas level, and the name filters —
 * everything else lives in components/canvas/* and components/sidebar/*.
 */
export function Dashboard() {
  const c = useContent();
  const d = c.dashboard;
  const cc = d.canvas;
  const { configured, isAuthenticated, error, sessionExpired, signIn, apiFetch } = useAuth();
  // The optional broader-visibility credential: its token is threaded into the
  // overview fetch so non-installed repos/orgs are enumerated, and its state
  // drives the connect affordance below.
  const {
    connected: broaderConnected,
    token: broaderToken,
    connectBroader,
    disconnectBroader,
  } = useBroaderOAuth();
  const { startIfUnseen } = useTour();

  const [overview, setOverview] = useState<OverviewResponse | null>(null);
  const [overviewFailed, setOverviewFailed] = useState(false);
  // True while an overview (re-)fetch is in flight — drives the Refresh
  // button's spinner so every fetch has a visible loading state.
  const [overviewRefreshing, setOverviewRefreshing] = useState(false);
  const [tick, setTick] = useState(0);

  // The dashboard's location is URL-addressable, so a refresh restores the exact
  // view and a link (from chat, a notification, a colleague) opens it directly.
  const { initial, navigateLevel, clearParams, isUnknownLocation } = useLevelParams();
  const [level, setLevel] = useState<CanvasLevel>(initial.level);
  // The session the URL asked for. Handed to RepoWorkspace as its initial
  // selection, then kept in step with the user's own choices.
  const [selectedSessionKey, setSelectedSessionKey] = useState<string | null>(
    initial.sessionKey ?? null
  );
  const [accountQuery, setAccountQuery] = useState('');
  const [repoQuery, setRepoQuery] = useState('');
  // `owner/name` of a repo created via the modal — highlighted at level 1.
  const [createdKey, setCreatedKey] = useState<string | null>(null);

  // The level the Escape handler walks up from, so its listener binds once.
  const levelForEscapeRef = useRef(level);
  levelForEscapeRef.current = level;

  // The repo-level session projection with its poll and race guards.
  const { sessions, sessionsFailed, refreshSessions } = useRepoSessions(level, apiFetch);

  useEffect(() => {
    document.title = d.metaTitle;
  }, [d.metaTitle]);

  // Auto-prompt the guided tour once, on the first authenticated visit for this
  // login on this browser. The per-user key comes from the overview's viewer,
  // so we wait for it to load. A ref guards against the effect firing twice
  // (overview polls bump its identity); startIfUnseen itself is also idempotent
  // per login, so this is belt-and-braces.
  const tourPromptedRef = useRef(false);
  useEffect(() => {
    if (tourPromptedRef.current) return;
    if (!isAuthenticated) return;
    const login = overview?.viewer.login;
    if (!login) return;
    tourPromptedRef.current = true;
    startIfUnseen(login);
  }, [isAuthenticated, overview, startIfUnseen]);

  // Load (and on `tick` bumps, re-load) the overview. Existing data is kept
  // during a refetch, so refreshes never blank the canvas. `broaderToken` is a
  // dep so capturing (or clearing) the broader credential re-fetches WITH (or
  // WITHOUT) the header, making non-installed repos appear (or disappear).
  useEffect(() => {
    if (!isAuthenticated || !configured) return;
    let active = true;
    setOverviewFailed(false);
    setOverviewRefreshing(true);
    getOverview(apiFetch, broaderToken)
      .then((body) => {
        if (active) setOverview(body);
      })
      .catch(() => {
        if (active) setOverviewFailed(true);
      })
      .finally(() => {
        if (active) setOverviewRefreshing(false);
      });
    return () => {
      active = false;
    };
  }, [isAuthenticated, configured, apiFetch, tick, broaderToken]);


  // Escape mirrors the Back button — unless a dialog is open (dialogs own
  // Escape; ModalShell also stops propagation as the first line of defense)
  // or the key was pressed inside an editable field (WebKit/Blink natively
  // clear a search input on Escape; that must not also change the level).

  // The ONE place a level change happens: it moves the view and writes the URL
  // together, so no call site can update one without the other.
  const goToLevel = useCallback(
    (target: CanvasLevel, selectedKey?: string | null) => {
      if (target.kind === 'account') setRepoQuery('');
      setLevel(target);
      setSelectedSessionKey(selectedKey ?? null);
      navigateLevel(target, selectedKey);
    },
    [navigateLevel]
  );

  const openAccount = useCallback(
    (login: string) => goToLevel({ kind: 'account', login }),
    [goToLevel]
  );

  const openRepo = useCallback(
    (owner: string, name: string) => goToLevel({ kind: 'repo', owner, name }),
    [goToLevel]
  );

  // A session was selected inside the repo workspace: reflect it in the URL so the
  // exact pane is linkable. Keyed off the CURRENT level, which is always the repo
  // the workspace belongs to.
  const onSelectedSessionChange = useCallback(
    (key: string) => {
      setSelectedSessionKey(key);
      if (level.kind === 'repo') navigateLevel(level, key);
    },
    [level, navigateLevel]
  );

  // Escape mirrors the Back button — unless a dialog is open (dialogs own
  // Escape; ModalShell also stops propagation as the first line of defense)
  // or the key was pressed inside an editable field (WebKit/Blink natively
  // clear a search input on Escape; that must not also change the level).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      if (
        e.target instanceof Element &&
        e.target.closest('input, textarea, select, [contenteditable]')
      ) {
        return;
      }
      if (document.querySelector('[role="dialog"]')) return;
      // Walking up rewrites the URL too, so Escape and a breadcrumb click leave
      // the same addressable state.
      const parent = parentLevel(levelForEscapeRef.current);
      if (parent != null) goToLevel(parent);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [goToLevel]);

  const refetchOverview = useCallback(() => setTick((t) => t + 1), []);

  // The header Refresh must feel complete: at level 2 the visible session list
  // is what the user is looking at, so refresh THAT too, not just the overview
  // counts behind it. (refetchOverview keeps the last-good data on screen.)
  const onRefreshClick = useCallback(() => {
    refetchOverview();
    if (level.kind === 'repo') refreshSessions(true);
  }, [refetchOverview, level.kind, refreshSessions]);

  // A repo was created: clear filters so it is visible, re-fetch, and zoom
  // into its owner account where the new row is highlighted with the
  // install-next callout.
  const onRepoCreated = useCallback((repo: UserRepo) => {
    setCreatedKey(`${repo.owner}/${repo.name}`);
    setAccountQuery('');
    setRepoQuery('');
    setTick((t) => t + 1);
    goToLevel({ kind: 'account', login: repo.owner });
  }, [goToLevel]);

  // A trigger was created/stopped: refresh the session list now and the
  // overview counts quietly behind it.
  const onSessionsChanged = useCallback(() => {
    refreshSessions(true);
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
      setSelectedSessionKey(null);
      // Without this a stale `?owner` survives the fallback and re-opens the
      // vanished account on the next refresh.
      clearParams();
    }
  }, [overview, selectedLogin, selectedAccount, clearParams]);

  // A URL naming an owner/repo this viewer cannot see (a typo, a stale link, a
  // repo they lost access to) falls back to the root cleanly — never a crash,
  // never a half-rendered level. Checked once, after the overview lands, so a
  // poll cannot fight the user's navigation.
  useEffect(() => {
    if (isUnknownLocation(overview, level)) {
      setLevel({ kind: 'root' });
      setSelectedSessionKey(null);
      clearParams();
    }
  }, [overview, level, isUnknownLocation, clearParams]);

  // Entering a DIFFERENT repository must not carry the previous repo's session
  // selection into the URL.
  const repoIdentity = level.kind === 'repo' ? `${level.owner}/${level.name}` : null;
  const previousRepoIdentityRef = useRef(repoIdentity);
  useEffect(() => {
    if (previousRepoIdentityRef.current !== repoIdentity) {
      previousRepoIdentityRef.current = repoIdentity;
      setSelectedSessionKey(null);
    }
  }, [repoIdentity]);

  // The cold sign-in card is shown ONLY for a never-signed-in visitor. An
  // involuntary expiry (sessionExpired) keeps the dashboard body mounted with a
  // context-preserving re-auth prompt instead (see `expiredBanner`), so the
  // user's level/selection survives.
  const showColdGate = !isAuthenticated && !sessionExpired;
  const view: 'gate' | 'unconfigured' | 'app' = showColdGate
    ? 'gate'
    : !configured
      ? 'unconfigured'
      : 'app';

  const filteredAccounts = overview != null ? filterAccounts(overview.accounts, accountQuery) : [];
  const filteredRepos =
    selectedAccount != null ? filterRepos(selectedAccount.repos, repoQuery) : [];
  const selectedRepo =
    level.kind === 'repo'
      ? (selectedAccount?.repos.find(
          (repo) => repo.name.toLowerCase() === level.name.toLowerCase()
        ) ?? null)
      : null;
  const repoInstalled =
    sessions?.installed ?? (level.kind === 'repo' ? (selectedRepo?.installed ?? false) : false);
  // A global administrator can inspect every App installation. Mutations are
  // restored only when this repo also came from the signed-in user's normal
  // GitHub visibility; GitHub remains authoritative for actual write access.
  const repoReadOnly = overview?.global_admin === true && selectedRepo?.viewer_visible !== true;

  // The initial overview load failed with nothing to show → an in-panel error
  // (with Retry) replaces the whole canvas/sidebar row, rather than a blank
  // canvas next to a sidebar skeleton that would spin forever.
  const overviewLoadError = overviewFailed && overview == null;

  // What the sidebar currently renders: real content vs a skeleton. Folded into
  // SidebarPanel's crossfade key so its skeleton→content swap animates.
  const sidebarLoaded =
    overview != null &&
    !(level.kind === 'account' && selectedAccount == null) &&
    !(level.kind === 'repo' && sessions == null && !sessionsFailed);

  // Canvas body state, keyed for the skeleton↔empty↔content crossfade.
  const canvasView: 'loading' | 'empty' | 'ready' =
    overview == null ? 'loading' : overview.accounts.length === 0 ? 'empty' : 'ready';

  const canvasBody =
    canvasView === 'loading' ? (
      <CanvasSkeleton />
    ) : canvasView === 'empty' ? (
      <div className="w-full h-full flex items-center justify-center p-8">
        {/* Gradient-hairline glass card lifts the empty message off the canvas. */}
        <div className="anim-row-in grad-border rounded-card px-6 py-5 shadow-2 shadow-highlight-top">
          <p className="text-[13.5px] text-dim text-center max-w-[36ch]">{cc.noAccounts}</p>
        </div>
      </div>
    ) : (
      <CanvasFlow
        level={level}
        accounts={filteredAccounts}
        repos={filteredRepos}
        repoSessions={sessions}
        repoInstalled={repoInstalled}
        onOpenAccount={openAccount}
        onOpenRepo={openRepo}
      />
    );

  const sidebarBody =
    overview == null ? (
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
    ) : level.kind === 'account' && selectedAccount != null ? (
      <Level1Sidebar
        account={selectedAccount}
        appSlug={overview.app_slug}
        query={repoQuery}
        onQueryChange={setRepoQuery}
        createdKey={createdKey}
        onOpenRepo={openRepo}
      />
    ) : (
      // The repo level no longer uses the sidebar — it renders the full-width
      // RepoWorkspace in the main region instead (see appBody's row below).
      <SidebarSkeleton />
    );

  const appBody = (
    // Root fills the routed region so the fixed header/breadcrumb pin and only
    // the canvas/sidebar row consumes the remaining height (min-h-0 lets the
    // flex child actually shrink so the row's internal scroll — not the page —
    // absorbs overflow).
    <div className="h-full flex flex-col min-h-0 gap-6">
      <DashboardHeader globalAdmin={overview?.global_admin === true} />

      {/* Involuntary expiry: prompt to re-authenticate WITHOUT tearing down the
          body, so the last-good canvas + the user's level/selection persist. */}
      {sessionExpired && (
        // Frosted re-auth prompt: glass fill, amber left accent + a soft amber
        // bloom, and a gradient CTA — without tearing down the last-good body.
        <div className="anim-row-in flex-none border border-line border-l-2 border-l-amber rounded-card bg-glass backdrop-blur-glass shadow-[var(--shadow-1),var(--glow-amber)] px-4 py-3 flex items-center gap-4 flex-wrap">
          <div className="min-w-0">
            <p className="font-ui font-semibold text-[13.5px] text-fg">{d.sessionExpiredTitle}</p>
            <p className="text-[12.5px] text-dim mt-0.5 max-w-[64ch]">{d.sessionExpiredBody}</p>
          </div>
          <button
            type="button"
            onClick={signIn}
            className="anim-sheen relative overflow-hidden ml-auto flex-none font-ui font-semibold text-[12.5px] bg-grad-accent text-amber-ink rounded-control px-4 py-2 transition-[filter] hover:brightness-110 cursor-pointer shadow-[var(--shadow-1),var(--glow-amber)]"
          >
            {d.sessionExpiredAction}
          </button>
        </div>
      )}

      <div
        data-tour="breadcrumb"
        className="flex-none flex items-center gap-3 flex-wrap pb-3 border-b border-line"
      >
        <CanvasBreadcrumb level={level} onNavigate={goToLevel} />
        <span className="flex-1" aria-hidden="true" />
        <button
          type="button"
          onClick={onRefreshClick}
          disabled={overviewRefreshing}
          aria-busy={overviewRefreshing}
          data-tour="refresh"
          className="font-ui font-semibold text-[12px] border border-line rounded-control px-3 py-1.5 text-dim hover:text-fg hover:shadow-glow-amber transition-[color,box-shadow] cursor-pointer disabled:cursor-default disabled:hover:text-dim disabled:hover:shadow-none inline-flex items-center gap-1.5"
        >
          {overviewRefreshing && <Spinner />}
          {overviewRefreshing ? d.repos.refreshing : d.repos.refresh}
        </button>
      </div>

      {/* Broader-visibility connect affordance — offered only when the backend
          advertises the feature (overview.broader_oauth_available); nothing
          renders otherwise. Connecting shows repos/orgs where the App is not
          installed. */}
      <BroaderVisibilityBanner
        available={overview?.broader_oauth_available ?? false}
        connected={broaderConnected}
        onConnect={connectBroader}
        onDisconnect={disconnectBroader}
      />

      {/* A refresh that fails with data on screen must not blank it — flag the
          staleness without blocking the (still valid) last-good view. */}
      {overviewFailed && overview != null && (
        <p className="anim-row-in flex-none border border-line border-l-2 border-l-amber rounded-card bg-glass backdrop-blur-glass shadow-[var(--shadow-1),var(--glow-amber)] px-3 py-2 font-mono text-[11.5px] text-dim">
          {d.repos.refreshFailedStale}
        </p>
      )}

      {overviewLoadError ? (
        <div className="flex-1 min-h-0 flex items-center justify-center border border-line rounded-panel bg-bg p-8">
          {/* Gradient-hairline glass card centers the failure + a glowing retry. */}
          <div className="anim-row-in grad-border rounded-card px-8 py-7 shadow-2 shadow-highlight-top flex flex-col items-center gap-4 text-center max-w-[42ch]">
            <p className="text-[14px] text-dim">{d.repos.loadFailed}</p>
            <button
              type="button"
              onClick={refetchOverview}
              disabled={overviewRefreshing}
              className="font-ui font-semibold text-[12.5px] border border-line rounded-control px-4 py-2 text-fg hover:shadow-glow-amber hover:brightness-110 transition-[filter,box-shadow] cursor-pointer disabled:cursor-default disabled:hover:shadow-none"
            >
              {d.retry}
            </button>
          </div>
        </div>
      ) : (
        <div className="flex-1 min-h-0 flex gap-5 items-stretch max-[1100px]:flex-col">
          {level.kind === 'repo' ? (
            // Repo details: the full-width workspace (session rail + inline
            // detail) replaces the graph + sidebar — the detail now lives in
            // the canvas region rather than the cramped sidebar.
            <section
              aria-label={cc.repoWorkspaceAria}
              data-tour="canvas"
              className="flex-1 min-w-0 border border-line rounded-panel bg-bg overflow-hidden h-full"
            >
              <FadeSwap
                k={sessions == null && !sessionsFailed ? 'loading' : 'ready'}
                className="w-full h-full"
              >
                {sessions == null && !sessionsFailed ? (
                  <CanvasSkeleton />
                ) : (
                  <RepoWorkspace
                    owner={level.owner}
                    name={level.name}
                    data={sessions}
                    loadFailed={sessionsFailed}
                    onChanged={onSessionsChanged}
                    viewerLogin={overview?.viewer.login ?? ''}
                    readOnly={repoReadOnly}
                    initialSelectedKey={selectedSessionKey}
                    onSelectedKeyChange={onSelectedSessionChange}
                  />
                )}
              </FadeSwap>
            </section>
          ) : (
            <>
              <section
                aria-label={cc.canvasAria}
                data-tour="canvas"
                className="flex-1 min-w-0 border border-line rounded-panel bg-bg overflow-hidden h-full"
              >
                {/* Skeleton↔empty↔canvas crossfade (instant under reduced motion). */}
                <FadeSwap k={canvasView} className="w-full h-full">
                  {canvasBody}
                </FadeSwap>
              </section>

              <SidebarPanel level={level} loaded={sidebarLoaded}>
                {sidebarBody}
              </SidebarPanel>
            </>
          )}
        </div>
      )}
    </div>
  );

  // Crossfade the sign-in gate ↔ the authenticated body on the auth state
  // (instant under reduced motion via FadeSwap).
  return (
    <FadeSwap k={view} className="h-full">
      {view === 'gate' ? (
        <DashboardGate error={error} configured={configured} onSignIn={signIn} />
      ) : view === 'unconfigured' ? (
        <DashboardUnconfigured />
      ) : (
        appBody
      )}
    </FadeSwap>
  );
}
