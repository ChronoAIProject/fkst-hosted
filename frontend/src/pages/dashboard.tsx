import { useCallback, useEffect, useRef, useState } from 'react';
import { Eyebrow } from '@/components/layout/eyebrow';
import { FadeSwap } from '@/components/ui/motion';
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
import { useTour } from '@/components/tour/tour-context';
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
  const { configured, isAuthenticated, error, sessionExpired, signIn, apiFetch } = useAuth();
  const { startIfUnseen } = useTour();

  const [overview, setOverview] = useState<OverviewResponse | null>(null);
  const [overviewFailed, setOverviewFailed] = useState(false);
  // True while an overview (re-)fetch is in flight — drives the Refresh
  // button's spinner so every fetch has a visible loading state.
  const [overviewRefreshing, setOverviewRefreshing] = useState(false);
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
  // Monotonic id per sessions request: an out-of-order response for the SAME
  // level key (slow poll racing a post-mutation refetch) must not win either.
  const sessionsRequestRef = useRef(0);

  // The OAuth error banner is locally dismissable. The context has no clearError
  // (a fresh sign-in clears it), so a per-slug local flag hides the banner until
  // a NEW error arrives — the reset effect below keys the flag to `error`.
  const [errorDismissed, setErrorDismissed] = useState(false);
  useEffect(() => {
    setErrorDismissed(false);
  }, [error]);

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
  // during a refetch, so refreshes never blank the canvas.
  useEffect(() => {
    if (!isAuthenticated || !configured) return;
    let active = true;
    setOverviewFailed(false);
    setOverviewRefreshing(true);
    getOverview(apiFetch)
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
  }, [isAuthenticated, configured, apiFetch, tick]);

  // Level-2 sessions: re-fetch keeping the current frame (used by the poll
  // and after mutations); the level-change effect below handles the reset.
  const refreshSessions = useCallback(() => {
    if (level.kind !== 'repo') return;
    const requestedFor = levelKey(level);
    const requestId = ++sessionsRequestRef.current;
    // A response only lands when it is BOTH for the current level key and the
    // latest request issued — otherwise a stale slow response could overwrite
    // fresher data (e.g. resurrect a just-stopped session).
    const isCurrent = () =>
      levelRef.current === requestedFor && sessionsRequestRef.current === requestId;
    getRepoSessions(apiFetch, level.owner, level.name)
      .then((body) => {
        if (!isCurrent()) return;
        setSessions(body);
        setSessionsFailed(false);
      })
      .catch(() => {
        if (!isCurrent()) return;
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

  // The header Refresh must feel complete: at level 2 the visible session list
  // is what the user is looking at, so refresh THAT too, not just the overview
  // counts behind it. (refetchOverview keeps the last-good data on screen.)
  const onRefreshClick = useCallback(() => {
    refetchOverview();
    if (level.kind === 'repo') refreshSessions();
  }, [refetchOverview, level.kind, refreshSessions]);

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
    <header className="flex-none">
      <Eyebrow>{d.eyebrow}</Eyebrow>
      {/* Page headline as a bright fg->dim gradient sweep (legible low end). */}
      <h1 className="grad-text grad-text-fg mt-5 font-display font-bold text-[clamp(28px,4vw,40px)] leading-[1.1] tracking-[-0.02em]">
        {d.title}
      </h1>
      <p className="mt-5 text-[15px] leading-relaxed text-dim max-w-[68ch]">{d.lede}</p>
    </header>
  );

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

  const gateBody = (
    <div className="flex flex-col gap-8 max-w-[960px]">
      {header}
      {error && !errorDismissed && (
        // Frosted danger notice: glass fill, red left accent + a soft red bloom.
        <div className="anim-row-in border border-line border-l-2 border-l-red rounded-card bg-glass backdrop-blur-glass shadow-[var(--shadow-1),var(--glow-red)] px-4 py-3 flex items-start gap-3">
          <div className="min-w-0 flex-1">
            {/* Map the callback's real OAuth slug to specific copy; the raw slug
                stays visible (mono) so an unrecognized one is still diagnosable. */}
            <p className="text-[13px] text-dim">{d.authErrorBySlug[error] ?? d.authError}</p>
            <p className="font-mono text-[11px] text-ghost mt-1">{error}</p>
          </div>
          <button
            type="button"
            onClick={() => setErrorDismissed(true)}
            className="flex-none font-ui font-semibold text-[12px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg transition-colors cursor-pointer"
          >
            {c.shell.toastDismiss}
          </button>
        </div>
      )}
      {/* Hero-accent sign-in card: amber->gold hairline + card depth & amber bloom. */}
      <section className="anim-row-in grad-border grad-border-accent rounded-panel p-8 max-[600px]:p-5 flex flex-col items-start gap-4 shadow-glow shadow-highlight-top">
        <h2 className="grad-text grad-text-fg font-display font-semibold text-[20px]">{d.signInTitle}</h2>
        <p className="text-[14px] leading-relaxed text-dim max-w-[56ch]">{d.signInBody}</p>
        {configured ? (
          <button
            type="button"
            onClick={signIn}
            className="anim-sheen relative overflow-hidden font-ui font-semibold text-[13.5px] bg-grad-accent text-amber-ink rounded-control px-5 py-2.5 transition-[filter] hover:brightness-110 cursor-pointer shadow-[var(--shadow-2),var(--glow-amber)]"
          >
            {c.auth.signIn}
          </button>
        ) : (
          <p className="font-mono text-[12px] text-ghost">{d.notConfigured}</p>
        )}
      </section>
    </div>
  );

  const unconfiguredBody = (
    <div className="flex flex-col gap-8 max-w-[960px]">
      {header}
      {/* Gradient-hairline glass card frames the not-configured notice. */}
      <section className="anim-row-in grad-border rounded-panel p-8 max-[600px]:p-5 shadow-2 shadow-highlight-top">
        <p className="font-mono text-[12px] text-ghost">{d.notConfigured}</p>
      </section>
    </div>
  );

  const filteredAccounts = overview != null ? filterAccounts(overview.accounts, accountQuery) : [];
  const filteredRepos = selectedAccount != null ? filterRepos(selectedAccount.repos, repoQuery) : [];
  const repoInstalled =
    sessions?.installed ??
    (level.kind === 'repo'
      ? (selectedAccount?.repos.find((r) => r.name === level.name)?.installed ?? false)
      : false);

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
    );

  const appBody = (
    // Root fills the routed region so the fixed header/breadcrumb pin and only
    // the canvas/sidebar row consumes the remaining height (min-h-0 lets the
    // flex child actually shrink so the row's internal scroll — not the page —
    // absorbs overflow).
    <div className="h-full flex flex-col min-h-0 gap-6">
      {header}

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
        <CanvasBreadcrumb level={level} onNavigate={navigate} />
        <span className="flex-1" aria-hidden="true" />
        <button
          type="button"
          onClick={onRefreshClick}
          disabled={overviewRefreshing}
          aria-busy={overviewRefreshing}
          data-tour="refresh"
          className="font-ui font-semibold text-[12px] border border-line rounded-control px-3 py-1.5 text-dim hover:text-fg hover:shadow-glow-amber transition-[color,box-shadow] cursor-pointer disabled:cursor-default disabled:hover:text-dim disabled:hover:shadow-none inline-flex items-center gap-1.5"
        >
          {overviewRefreshing && (
            <span
              aria-hidden="true"
              className="anim-spin inline-block w-3 h-3 border border-line-2 border-t-amber rounded-full flex-none"
            />
          )}
          {overviewRefreshing ? d.repos.refreshing : d.repos.refresh}
        </button>
      </div>

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
        </div>
      )}
    </div>
  );

  // Crossfade the sign-in gate ↔ the authenticated body on the auth state
  // (instant under reduced motion via FadeSwap).
  return (
    <FadeSwap k={view} className="h-full">
      {view === 'gate' ? gateBody : view === 'unconfigured' ? unconfiguredBody : appBody}
    </FadeSwap>
  );
}
