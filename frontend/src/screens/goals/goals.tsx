import { useState } from 'react';
import { cn } from '@/lib/utils';
import { WindowControl } from '@/components/layout/window-control';
import { CiGlyph } from '@/components/status/ci-glyph';
import { StateBadge, StateBadgeState } from '@/components/status/state-badge';
import { useAuthSession, authRequired } from '@/lib/auth';
import { useGitHubAccounts } from '@/lib/hooks/useGitHubAccounts';
import { AccountView } from '@/lib/api/types';
import { GoalView } from '@/lib/api/goals';
import { goalStatusPresentation } from '@/lib/api/goal-status';
import { Link } from 'react-router-dom';

function formatAge(dateStr: string) {
  const date = new Date(dateStr);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  if (isNaN(diffMs)) return '—';
  const diffMins = Math.floor(diffMs / 60000);
  if (diffMins < 1) return 'just now';
  if (diffMins < 60) return `${diffMins}m`;
  const diffHours = Math.floor(diffMins / 60);
  if (diffHours < 24) return `${diffHours}h`;
  const diffDays = Math.floor(diffHours / 24);
  return `${diffDays}d`;
}

export interface GoalsGoal {
  id: string;
  title: string;
  stage: 'Design' | 'Build' | 'Review' | 'Ship' | 'Blocked' | 'Merged';
  state: StateBadgeState;
  age: string;
  repo: string;
  pr: string;
  ci: 'passing' | 'failing' | 'unknown';
  gated?: boolean;
}

export interface GoalsRun {
  id: string;
  goalId: string;
  goalTitle: string;
  action: string;
  attempt: string;
  duration: string;
  exitCode: number | null;
  when: string;
  lease: string;
  status: 'running' | 'ok' | 'fail' | 'retried';
}

export interface GoalsProps {
  view?: 'issues' | 'activity';
  goals?: (GoalsGoal | GoalView)[];
  runs?: GoalsRun[];
  vitals?: {
    runsDispatched?: string | 'unknown';
    successRate?: string | 'unknown';
    medianDuration?: string | 'unknown';
    inDlq?: string | 'unknown';
  };
  onNewGoal?: () => void;
  onViewChange?: (view: 'issues' | 'activity') => void;
  authSessionOverride?: {
    isAuthenticated: boolean;
  };
  accountsOverride?: AccountView[];
  accountsErrorOverride?: unknown;
  accountsLoadingOverride?: boolean;
  isLoadingGoals?: boolean;
  isErrorGoals?: boolean;
  goalsError?: unknown;
}

export function Goals({
  view = 'issues',
  goals = [],
  runs = [],
  vitals,
  onNewGoal,
  onViewChange,
  authSessionOverride,
  accountsOverride,
  accountsErrorOverride,
  accountsLoadingOverride,
  isLoadingGoals,
  isErrorGoals,
  goalsError,
}: GoalsProps) {
  const [prevView, setPrevView] = useState(view);
  const [currentView, setCurrentView] = useState<'issues' | 'activity'>(view);
  const [timeWindow, setTimeWindow] = useState<string>('24h');

  // Resolve auth session state
  const realAuth = useAuthSession();
  const auth = authSessionOverride !== undefined ? authSessionOverride : realAuth;
  const isAuthRequired = authRequired();
  const isUserAuthenticated = auth ? auth.isAuthenticated : false;
  const isSigned = !isAuthRequired || isUserAuthenticated;

  // Resolve GitHub accounts state
  const { data: realAccounts, isLoading: realAccountsLoading, isError: realAccountsError } = useGitHubAccounts({
    enabled: isSigned && accountsOverride === undefined,
  });

  const accounts = accountsOverride !== undefined ? accountsOverride : realAccounts;
  const isAccountsLoading = accountsLoadingOverride !== undefined ? accountsLoadingOverride : realAccountsLoading;
  const isAccountsError = accountsErrorOverride !== undefined ? accountsErrorOverride : realAccountsError;

  const isAuthPending = isAuthRequired && !isUserAuthenticated;

  const handleViewChange = (v: 'issues' | 'activity') => {
    setCurrentView(v);
    onViewChange?.(v);
  };

  if (view !== prevView) {
    setCurrentView(view);
    setPrevView(view);
  }

  const isHostedMode = goals && goals.length > 0 && goals[0] && 'status' in goals[0];
  const isRealHostedMode = isLoadingGoals !== undefined || isHostedMode;
  const isGatePassed = !isAuthPending && !isAccountsLoading && !isAccountsError && accounts !== undefined && accounts.length > 0;
  const showData = (isGatePassed || isRealHostedMode) && goals.length > 0;
  const showRuns = runs.length > 0;

  return (
    <div className="flex flex-col gap-6">
      {/* Toolbar */}
      <div className="flex items-center gap-4 flex-wrap pb-3.5 border-b border-line">
        <button
          onClick={onNewGoal}
          disabled={!onNewGoal}
          className={cn(
            "font-ui font-semibold text-[12.5px] rounded-control px-3.5 py-[7px] flex-shrink-0 transition-colors",
            onNewGoal
              ? "bg-amber text-amber-ink hover:brightness-[1.06] cursor-pointer"
              : "bg-amber/50 text-amber-ink/50 cursor-not-allowed opacity-50 select-none"
          )}
        >
          + New goal
        </button>

        {/* View Segment Switcher */}
        <div className="flex items-center gap-2">
          <span className="font-mono text-eyebrow text-ghost uppercase">View</span>
          <div className="bg-raise border border-line rounded-control p-[2px] inline-flex items-center select-none">
            <button
              type="button"
              onClick={() => handleViewChange('issues')}
              className={cn(
                'py-[5px] px-[13px] text-[12.5px] font-medium rounded-chip transition-colors cursor-pointer outline-none',
                currentView === 'issues'
                  ? 'bg-amber text-amber-ink font-semibold'
                  : 'text-faint hover:text-dim hover:bg-raise-2'
              )}
            >
              Issues
            </button>
            <button
              type="button"
              onClick={() => handleViewChange('activity')}
              className={cn(
                'py-[5px] px-[13px] text-[12.5px] font-medium rounded-chip transition-colors cursor-pointer outline-none',
                currentView === 'activity'
                  ? 'bg-amber text-amber-ink font-semibold'
                  : 'text-faint hover:text-dim hover:bg-raise-2'
              )}
            >
              Activity
            </button>
          </div>
        </div>

        {/* View-specific filters */}
        {currentView === 'issues' ? (
          <div className="flex items-center gap-3 flex-wrap max-[780px]:w-full">
            <div className="search flex items-center gap-[9px] border border-line rounded-control bg-raise px-[11px] py-1.5 opacity-50 cursor-not-allowed max-[780px]:w-full">
              <svg className="w-3.5 h-3.5 text-ghost flex-shrink-0" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4">
                <circle cx="7" cy="7" r="4.5" />
                <path d="M10.5 10.5L14 14" />
              </svg>
              <input
                disabled
                type="text"
                placeholder="Search goals (requires GitHub plane)..."
                className="bg-transparent border-0 outline-none text-fg text-[13px] placeholder:text-ghost w-[180px] max-[780px]:w-full"
                aria-label="Search goals"
              />
            </div>

            <button
              disabled
              type="button"
              aria-label="Stage filter: all"
              className="flex items-center gap-2 text-[13px] text-dim border border-line rounded-control px-[11px] py-1.5 bg-raise cursor-not-allowed select-none opacity-50 outline-none"
            >
              Stage <span className="mono text-fg">all</span> <span className="text-ghost text-[10px]">▾</span>
            </button>
            <button
              disabled
              type="button"
              aria-label="Repository filter: all"
              className="flex items-center gap-2 text-[13px] text-dim border border-line rounded-control px-[11px] py-1.5 bg-raise cursor-not-allowed select-none opacity-50 outline-none"
            >
              Repo <span className="mono text-fg">all</span> <span className="text-ghost text-[10px]">▾</span>
            </button>
            <button
              disabled
              type="button"
              aria-label="State filter: all"
              className="flex items-center gap-2 text-[13px] text-dim border border-line rounded-control px-[11px] py-1.5 bg-raise cursor-not-allowed select-none opacity-50 outline-none"
            >
              State <span className="mono text-fg">all</span> <span className="text-ghost text-[10px]">▾</span>
            </button>
            <button disabled className="text-[12px] font-medium text-faint cursor-not-allowed opacity-50 underline underline-offset-[3px] decoration-line-2">
              Clear
            </button>
          </div>
        ) : (
          <div className="flex items-center gap-2">
            <span className="font-mono text-eyebrow text-ghost uppercase">Status</span>
            <div className="bg-raise border border-line rounded-control p-[2px] inline-flex items-center select-none opacity-50 cursor-not-allowed">
              {['All', 'Running', 'OK', 'Retried', 'Timed-out', 'Dead'].map((f, idx) => (
                <button
                  disabled
                  key={f}
                  type="button"
                  className={cn(
                    "py-[5px] px-[13px] text-[12.5px] rounded-chip select-none font-medium text-faint",
                    idx === 0 && "font-semibold bg-raise-2"
                  )}
                >
                  {f}
                </button>
              ))}
            </div>
          </div>
        )}

        {/* Shared Window Control */}
        <span className="font-mono text-eyebrow text-ghost uppercase ml-auto max-[780px]:ml-0">Window</span>
        <WindowControl value={timeWindow} onChange={setTimeWindow} />
      </div>

      {/* Canvas view rendering */}
      {currentView === 'issues' ? (
        /* ISSUES VIEW */
        <div className="flex flex-col gap-4">
          <p className="text-[12px] leading-relaxed text-faint">
            {isHostedMode ? (
              <span>State and status are managed by the hosted service. Stage pipeline mapping is not applicable to hosted goals.</span>
            ) : (
              <span>State is derived from trusted-bot <span className="font-mono text-[11.5px] text-dim">state:v1</span> markers; <span className="font-mono text-[11.5px] text-dim">fkst-dev:*</span> labels are self-heal hints only. Stage groups the design / build / review / ship flow; <b>ready · gated</b> is consensus output held at the dependency gate (<span className="font-mono text-[11.5px] text-dim">fkst-dev:n</span> · <span className="font-mono text-[11.5px] text-dim">n:v1</span> marker).</span>
            )}
          </p>

          <div className="w-full overflow-x-auto max-[980px]:scrollbar-thin">
            <div className="min-w-[880px] flex flex-col">
              {/* Header row */}
              <div className="grid grid-cols-[14px_52px_minmax(0,1fr)_104px_128px_56px_40px_64px_116px] gap-4 px-1.5 h-[34px] border-b border-line-2 items-center">
                <span />
                <span className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost">#</span>
                <span className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost">Goal</span>
                <span className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost">Stage</span>
                <span className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost">Repo</span>
                <span className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost">PR</span>
                <span className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost">CI</span>
                <span className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost text-right">Age</span>
                <span className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost text-right">State</span>
              </div>

              {/* Rows / Empty State */}
              {isRealHostedMode ? (
                isLoadingGoals ? (
                  <div className="flex items-center justify-center py-16 text-ghost font-mono text-[12px]">
                    loading goals...
                  </div>
                ) : isErrorGoals ? (
                  <div className="flex items-center justify-center py-16 text-red font-mono text-[12px]">
                    failed to load goals: {String(goalsError)}
                  </div>
                ) : goals.length > 0 ? (
                  <div className="flex flex-col">
                    {goals.map((g) => {
                      if ('status' in g) {
                        const repoStr = g.repo ? `${g.repo.owner}/${g.repo.name}` : '—';
                        const pres = goalStatusPresentation(g.status);
                        const ageStr = formatAge(g.created_at);
                        
                        let dotTone: 'green' | 'red' | 'gold' | 'neutral' = 'neutral';
                        if (g.status === 'running') dotTone = 'green';
                        else if (g.status === 'failed') dotTone = 'red';
                        else if (g.status === 'triggered' || g.status === 'stopped') dotTone = 'gold';
                        
                        return (
                          <Link
                            to={`/goals/${g.id}`}
                            key={g.id}
                            className="grid grid-cols-[14px_52px_minmax(0,1fr)_104px_128px_56px_40px_64px_116px] gap-4 px-1.5 min-h-[52px] py-2 border-b border-line items-center text-dim text-[13.5px] hover:bg-[color-mix(in_oklab,var(--raise)_30%,transparent)] transition-colors no-underline"
                          >
                            <span className={cn(
                              "w-2.5 h-2.5 rounded-full justify-self-center",
                              dotTone === 'green' && "bg-green",
                              dotTone === 'red' && "bg-red",
                              dotTone === 'gold' && "bg-gold",
                              dotTone === 'neutral' && "bg-faint"
                            )} />
                            <span className="font-mono text-faint text-[12px]">{g.id}</span>
                            <span className="text-fg font-medium line-clamp-2 leading-[1.32] min-w-0 pr-2">{g.title}</span>
                            <span className="truncate min-w-0 text-[12.5px] text-ghost">—</span>
                            <span className="font-mono text-faint text-[11.5px] truncate min-w-0">{repoStr}</span>
                            <span className="font-mono text-[11.5px] text-ghost">—</span>
                            <span className="text-ghost text-[11.5px]">—</span>
                            <span className="font-mono text-ghost text-[11.5px] text-right">{ageStr}</span>
                            <div className="justify-self-end">
                              <div className={cn(
                                "inline-flex items-center gap-[7px] font-ui font-semibold text-[11px] tracking-[0.02em] uppercase px-2 py-[4px] rounded-[6px] border select-none",
                                pres.tone === 'neutral' && "border-line-2 text-ghost bg-raise",
                                pres.tone === 'green' && "border-[color-mix(in_oklab,var(--green)_40%,var(--line))] text-green bg-raise-2",
                                pres.tone === 'red' && "border-[color-mix(in_oklab,var(--red)_45%,var(--line))] text-red bg-raise-2",
                                pres.tone === 'gold' && "border-[color-mix(in_oklab,var(--gold)_40%,var(--line))] text-gold bg-raise-2",
                                pres.tone === 'amber' && "border-[color-mix(in_oklab,var(--amber)_45%,var(--line))] text-amber bg-raise-2"
                              )}>
                                <span className={cn(
                                  "w-1.5 h-1.5 rounded-full",
                                  pres.tone === 'neutral' && "bg-ghost",
                                  pres.tone === 'green' && "bg-green",
                                  pres.tone === 'red' && "bg-red",
                                  pres.tone === 'gold' && "bg-gold",
                                  pres.tone === 'amber' && "bg-amber"
                                )} />
                                <span>{pres.label}</span>
                              </div>
                            </div>
                          </Link>
                        );
                      }
                      return null;
                    })}
                  </div>
                ) : (
                  <div className="flex items-center justify-center py-16 text-ghost font-mono text-[12px]">
                    no goals found
                  </div>
                )
              ) : isAuthPending ? (
                <div className="flex items-center justify-center py-16 text-ghost font-mono text-[12px]">
                  no GitHub plane connected — sign-in pending
                </div>
              ) : isAccountsLoading ? (
                <div className="flex items-center justify-center py-16 text-ghost font-mono text-[12px]">
                  loading GitHub accounts...
                </div>
              ) : isAccountsError || accounts === undefined ? (
                <div className="flex items-center justify-center py-16 text-ghost font-mono text-[12px]">
                  GitHub status unknown — couldn't reach the connection service
                </div>
              ) : accounts.length === 0 ? (
                <div className="flex flex-col items-center justify-center py-16 gap-3">
                  <span className="text-ghost font-mono text-[12px]">
                    no GitHub accounts connected
                  </span>
                  {import.meta.env.VITE_NYXID_CONNECT_GITHUB_URL ? (
                    <a
                      href={import.meta.env.VITE_NYXID_CONNECT_GITHUB_URL}
                      className="font-ui font-semibold text-[12.5px] rounded-control px-3.5 py-[7px] bg-amber text-amber-ink hover:brightness-[1.06] cursor-pointer transition-colors no-underline inline-block"
                    >
                      Connect GitHub
                    </a>
                  ) : (
                    <div className="flex flex-col items-center gap-1.5">
                      <button
                        disabled
                        className="font-ui font-semibold text-[12.5px] rounded-control px-3.5 py-[7px] bg-amber/50 text-amber-ink/50 cursor-not-allowed opacity-50 select-none"
                      >
                        Connect GitHub
                      </button>
                      <span className="text-[11px] text-ghost font-mono">
                        GitHub connection URL is not configured
                      </span>
                    </div>
                  )}
                </div>
              ) : goals.length > 0 ? (
                <div className="flex flex-col">
                  {goals.map((g) => {
                    const isReview = 'stage' in g && g.stage === 'Review';
                    const isShip = 'stage' in g && g.stage === 'Ship';
                    const isMerged = 'stage' in g && g.stage === 'Merged';
                    const isBlocked = 'stage' in g && g.stage === 'Blocked';

                    let dotTone: 'green' | 'red' | 'gold' | 'neutral' = 'neutral';
                    if (isMerged) dotTone = 'green';
                    else if (isBlocked || ('state' in g && g.state === 'impl-failed')) dotTone = 'red';
                    else if (isReview && ('state' in g && g.state === 'reviewing')) dotTone = 'gold';

                    const stageVal = 'stage' in g ? g.stage : '—';
                    const stateVal = 'state' in g ? g.state : 'unknown';
                    const repoVal = 'repo' in g && typeof g.repo === 'string' ? g.repo : '—';
                    const prVal = 'pr' in g && typeof g.pr === 'string' ? g.pr : '—';
                    const ageVal = 'age' in g ? g.age : '—';
                    const ciVal = 'ci' in g ? g.ci : 'unknown';
                    const gatedVal = 'gated' in g ? g.gated : false;

                    return (
                      <Link
                        to={`/goals/${g.id}`}
                        key={g.id}
                        className="grid grid-cols-[14px_52px_minmax(0,1fr)_104px_128px_56px_40px_64px_116px] gap-4 px-1.5 min-h-[52px] py-2 border-b border-line items-center text-dim text-[13.5px] hover:bg-[color-mix(in_oklab,var(--raise)_30%,transparent)] transition-colors no-underline"
                      >
                        <span className={cn(
                          "w-2 h-2 rounded-full justify-self-center",
                          dotTone === 'green' && "bg-green",
                          dotTone === 'red' && "bg-red",
                          dotTone === 'gold' && "bg-gold",
                          dotTone === 'neutral' && "bg-faint"
                        )} />
                        <span className="font-mono text-faint text-[12px]">{g.id}</span>
                        <span className="text-fg font-medium line-clamp-2 leading-[1.32] min-w-0 pr-2">{g.title}</span>
                        <span className={cn(
                          "truncate min-w-0 text-[12.5px]",
                          isReview && "text-gold",
                          isShip && "text-red",
                          isMerged && "text-green",
                          isBlocked && "text-red"
                        )}>
                          {stageVal}
                        </span>
                        <span className="font-mono text-faint text-[11.5px] truncate min-w-0">{repoVal}</span>
                        <span className={cn("font-mono text-[11.5px]", prVal && prVal !== '—' ? "text-faint" : "text-ghost")}>
                          {prVal || '—'}
                        </span>
                        <CiGlyph status={ciVal} />
                        <span className="font-mono text-ghost text-[11.5px] text-right">{ageVal}</span>
                        <div className="justify-self-end">
                          {stateVal === 'ready' ? (
                            <StateBadge state="ready" gated={gatedVal} />
                          ) : stateVal === 'reviewing' || stateVal === 'review-meta' ? (
                            <StateBadge state={stateVal} />
                          ) : (
                            <StateBadge state={stateVal as Exclude<StateBadgeState, 'ready' | 'reviewing' | 'review-meta'>} />
                          )}
                        </div>
                      </Link>
                    );
                  })}
                </div>
              ) : (
                <div className="flex items-center justify-center py-16 text-ghost font-mono text-[12px]">
                  no goals found
                </div>
              )}
            </div>
          </div>

          {/* Counts band */}
          <div className="flex items-baseline gap-3 pt-3.5 border-t border-line flex-wrap">
            <span className={cn(
              "font-display font-semibold text-[18px] tracking-[-0.01em]",
              showData ? "text-fg" : "text-ghost"
            )}>
              {showData ? goals.length : '—'}
            </span>
            <span className="text-[12.5px] text-dim">goals · <span className="font-mono text-faint">state: all</span></span>
            <button disabled className="text-[12.5px] font-medium text-faint cursor-not-allowed opacity-50 underline underline-offset-[3px] decoration-line-2">
              Load all →
            </button>
            <span className="font-mono text-[11px] text-ghost ml-auto max-[600px]:ml-0 max-[600px]:w-full">
              showing {showData ? goals.length : '—'} · sorted newest · {isHostedMode ? 'status from hosted database' : 'state from trusted GitHub markers (labels are hints)'} · poll-derived
            </span>
          </div>

          {/* Footer */}
          <div className="foot flex gap-6 font-mono text-[11px] text-ghost flex-wrap mt-6 pt-[14px] border-t border-line">
            {isHostedMode ? (
              <span>a goal is a record in the hosted database</span>
            ) : (
              <span>a goal is a <b>GitHub issue</b> labeled <b>fkst-dev:enabled</b></span>
            )}
            <span>rows scoped to the <b>{timeWindow}</b> window · open rows drill into the goal page</span>
            <span className="text-gold">fkst-packages CI unknown shown as — , never a pass</span>
            <span>state as of <b>unknown</b> · poll-derived (~5-min ticks), not live</span>
          </div>
        </div>
      ) : (
        /* ACTIVITY VIEW */
        <div className="flex flex-col gap-4">
          {/* Vitals Strip */}
          <div className="grid grid-cols-4 max-[780px]:grid-cols-2 gap-px bg-line border border-line rounded-panel overflow-hidden mt-1">
            <div className="rv bg-raise p-[14px_20px] min-w-0">
              <div className={cn(
                "v font-display font-semibold text-[24px] tracking-[-0.02em] leading-none",
                vitals?.runsDispatched && vitals.runsDispatched !== 'unknown' ? "text-fg" : "text-ghost"
              )}>
                {vitals?.runsDispatched ?? '—'}
              </div>
              <div className="k text-[12px] text-faint mt-1.5 font-ui">
                runs dispatched <span className="sub font-mono text-[10.5px] text-ghost">· est. from logs</span>
              </div>
            </div>
            <div className="rv bg-raise p-[14px_20px] min-w-0">
              <div className={cn(
                "v font-display font-semibold text-[24px] tracking-[-0.02em] leading-none",
                vitals?.successRate && vitals.successRate !== 'unknown' ? "text-green" : "text-ghost"
              )}>
                {vitals?.successRate ?? '—'}
              </div>
              <div className="k text-[12px] text-faint mt-1.5 font-ui">
                success rate <span className="sub font-mono text-[10.5px] text-ghost">· diagnostic est.</span>
              </div>
            </div>
            <div className="rv bg-raise p-[14px_20px] min-w-0">
              <div className={cn(
                "v font-display font-semibold text-[24px] tracking-[-0.02em] leading-none",
                vitals?.medianDuration && vitals.medianDuration !== 'unknown' ? "text-fg" : "text-ghost"
              )}>
                {vitals?.medianDuration ?? '—'}
              </div>
              <div className="k text-[12px] text-faint mt-1.5 font-ui">
                median duration <span className="sub font-mono text-[10.5px] text-ghost">· est. · codex exec</span>
              </div>
            </div>
            <div className="rv bg-raise p-[14px_20px] min-w-0">
              <div className={cn(
                "v font-display font-semibold text-[24px] tracking-[-0.02em] leading-none",
                vitals?.inDlq && vitals.inDlq !== 'unknown' ? "text-faint" : "text-ghost"
              )}>
                {vitals?.inDlq ?? '—'}
              </div>
              <div className="k text-[12px] text-faint mt-1.5 font-ui">
                in-DLQ <span className="sub font-mono text-[10.5px] text-ghost">· transport not visible</span>
              </div>
            </div>
          </div>

          {/* Derivation honesty */}
          <p className="border border-line border-l-2 border-l-gold rounded-[9px] p-[11px_14px] bg-[color-mix(in_oklab,var(--raise)_55%,transparent)] text-[12.5px] leading-relaxed text-dim mt-1">
            <b>Reconstructed from logs/transport, not a durable ledger.</b> Rows are <b>derived from engine logs &amp; transport</b> as the hosted service reports them — diagnostic and poll-derived (~5m). redb is a delivery queue, not a run archive: <b>ack deletes</b> the delivery record, so there is <b>no durable run-history store</b> here. The vitals above are <b>diagnostic estimates</b>; when the hosted service can't see transport they read <span className="font-mono text-[11px] text-ghost">unknown</span>, never 0. The FE only reads — it cannot mutate any run.
          </p>

          {/* Runs table header */}
          <div className="flex items-baseline gap-3 mt-4 flex-wrap max-[780px]:gap-1">
            <span className="font-mono text-eyebrow text-ghost uppercase">Activity</span>
            <span className="text-[12.5px] text-ghost">
              what the engine is doing on your goals · dispatches seen in logs/transport · best-effort · newest first
            </span>
            <span className="font-mono text-[11px] text-ghost ml-auto max-[780px]:ml-0 max-[780px]:w-full">
              {showRuns ? `${runs.length} runs` : '— runs'} · {timeWindow}
            </span>
          </div>

          {/* Table / Empty State */}
          <div className="w-full overflow-x-auto max-[980px]:scrollbar-thin mt-1.5">
            {showRuns ? (
              <table className="w-full border-collapse text-[13px] min-w-[780px]">
                <thead>
                  <tr className="border-b border-line-2">
                    <th className="w-[34px] pr-0 py-3 text-left" />
                    <th className="font-mono font-semibold text-[10.5px] tracking-[0.13em] uppercase text-ghost text-left pb-2.5 px-3.5">Goal</th>
                    <th className="font-mono font-semibold text-[10.5px] tracking-[0.13em] uppercase text-ghost text-left pb-2.5 px-3.5">Action</th>
                    <th className="font-mono font-semibold text-[10.5px] tracking-[0.13em] uppercase text-ghost text-left pb-2.5 px-3.5 max-[780px]:hidden">Attempt</th>
                    <th className="font-mono font-semibold text-[10.5px] tracking-[0.13em] uppercase text-ghost text-right pb-2.5 px-3.5">Duration</th>
                    <th className="font-mono font-semibold text-[10.5px] tracking-[0.13em] uppercase text-ghost text-right pb-2.5 px-3.5 max-[600px]:hidden">Exit</th>
                    <th className="font-mono font-semibold text-[10.5px] tracking-[0.13em] uppercase text-ghost text-right pb-2.5 px-3.5">When</th>
                    <th className="font-mono font-semibold text-[10.5px] tracking-[0.13em] uppercase text-ghost text-right pb-2.5 px-3.5 max-[600px]:hidden">Run · lease</th>
                  </tr>
                </thead>
                <tbody>
                  {runs.map((r) => (
                    <tr key={r.id} className="border-b border-line hover:bg-[color-mix(in_oklab,var(--raise)_55%,transparent)] transition-colors cursor-not-allowed">
                      <td className="w-[34px] pr-0 py-3 text-left pl-1">
                        <span className={cn(
                          "w-2.5 h-2.5 rounded-full block mt-1",
                          r.status === 'running' && "bg-transparent border-2 border-faint",
                          r.status === 'ok' && "bg-green",
                          r.status === 'fail' && "bg-red",
                          r.status === 'retried' && "bg-gold"
                        )} />
                      </td>
                      <td className="py-3 px-3.5 min-w-0 max-w-[360px] max-[780px]:max-w-[220px]">
                        <span className="font-mono text-[11.5px] text-faint">#{r.goalId}</span>
                        <div className="text-fg text-[13px] font-medium leading-[1.34] mt-0.5 line-clamp-2">{r.goalTitle}</div>
                      </td>
                      <td className="py-3 px-3.5">
                        <span className={cn(
                          "font-mono text-[11px] border border-line-2 rounded-[6px] px-2 py-0.5 bg-[color-mix(in_oklab,var(--raise)_60%,transparent)] text-dim",
                          r.action.startsWith('merge') && "text-red border-[color-mix(in_oklab,var(--red)_45%,var(--line))] font-medium"
                        )}>
                          {r.action}
                        </span>
                      </td>
                      <td className="py-3 px-3.5 font-mono text-[12px] text-faint max-[780px]:hidden">{r.attempt}</td>
                      <td className="py-3 px-3.5 text-right font-mono text-[12px] text-dim">
                        {r.status === 'running' ? <span className="text-faint">{r.duration}</span> : r.duration}
                      </td>
                      <td className="py-3 px-3.5 text-right max-[600px]:hidden">
                        <span className="font-mono text-[12px]">
                          {r.exitCode === null ? (
                            <span className="text-ghost">—</span>
                          ) : (
                            <span className={r.exitCode === 0 ? "text-green" : "text-red"}>{r.exitCode}</span>
                          )}
                        </span>
                      </td>
                      <td className="py-3 px-3.5 text-right font-mono text-[11.5px] text-ghost">{r.when}</td>
                      <td className="py-3 px-3.5 text-right max-[600px]:hidden font-mono text-[11px] text-ghost">
                        <span className="text-faint">{r.id}</span> · lease <b>{r.lease}</b>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <div className="flex items-center justify-center py-16 text-ghost font-mono text-[12px] border-t border-line">
                host telemetry not connected
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
