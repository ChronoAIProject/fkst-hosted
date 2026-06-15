import React from 'react';
import { Link } from 'react-router-dom';
import { useHealth } from '@/lib/hooks/useHealth';
import { usePackagesList } from '@/lib/hooks/usePackages';
import { useSessionRegistry } from '@/lib/hooks/session-registry';
import { useSession, useStopSession } from '@/lib/hooks/useSessions';
import { ApiError } from '@/lib/api/client';
import { PostureChip } from '@/components/status/posture-chip';
import { Eyebrow } from '@/components/layout/eyebrow';
import { Dialog, DialogTrigger, DialogContent, DialogTitle, DialogDescription } from '@/components/primitives/dialog';
import { HairlineList } from '@/components/layout/hairline-list';
import { SessionStateBadge } from './session-state-badge';
import { cn } from '@/lib/utils';
import { useAuthSession, authRequired, OAuthUserInfo } from '@/lib/auth';

interface SessionDetailsProps {
  packageName: string;
  sessionId: string;
  onMarkStale: (pkg: string) => void;
}

export function SessionDetails({ packageName, sessionId, onMarkStale }: SessionDetailsProps) {
  const sessionQuery = useSession(sessionId);
  const { clearSession } = useSessionRegistry();
  const stopMutation = useStopSession();
  const [confirmOpen, setConfirmOpen] = React.useState(false);

  // Stale session check: if 404 is encountered, clear it from registry
  React.useEffect(() => {
    if (sessionQuery.isError) {
      const err = sessionQuery.error;
      if (err instanceof ApiError && err.status === 404) {
        clearSession(packageName);
        onMarkStale(packageName);
      }
    }
  }, [sessionQuery.isError, sessionQuery.error, packageName, clearSession, onMarkStale]);

  const session = sessionQuery.data;

  // Poll-based progress indicator: true when stopping is in progress, false once stopped/failed (terminal)
  const isStopping =
    (session?.status === 'stopping' || stopMutation.isPending || stopMutation.isSuccess) &&
    session?.status !== 'stopped' &&
    session?.status !== 'failed';

  const handleStop = () => {
    stopMutation.mutate(sessionId, {
      onSuccess: () => {
        setConfirmOpen(false);
      },
    });
  };

  return (
    <div className="border border-line rounded-card bg-raise p-4 flex flex-col gap-3">
      <div className="flex items-center gap-3 flex-wrap">
        <span className="font-mono text-[12.5px] text-fg font-medium">{packageName}</span>
        <span className="font-mono text-[11px] text-ghost">id: {sessionId}</span>
        {sessionQuery.isError && sessionQuery.error instanceof ApiError && sessionQuery.error.status === 404 ? (
          <span className="inline-block font-mono text-[10.5px] font-medium tracking-[0.02em] px-2 py-[3px] rounded-chip border lowercase whitespace-nowrap bg-raise-2 border-line-2 text-dim">
            stale
          </span>
        ) : (
          <SessionStateBadge status={session?.status} />
        )}
      </div>

      {sessionQuery.isError && sessionQuery.error instanceof ApiError && sessionQuery.error.status === 404 && (
        <div className="text-[12px] text-gold font-mono leading-relaxed">
          session no longer found — stale registry entry from this tab
        </div>
      )}

      {session && (
        <div className="font-mono text-[11px] text-ghost flex flex-col gap-1">
          <div>created_at: {session.created_at}</div>
          {session.started_at && <div>started_at: {session.started_at}</div>}
          {session.stopped_at && <div>stopped_at: {session.stopped_at}</div>}
        </div>
      )}

      {isStopping && (
        <div className="text-[12px] text-gold font-mono leading-relaxed">
          stop requested · waiting for stopped — 202 is an ack, truth is the poll
        </div>
      )}

      <Dialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        {!(sessionQuery.isError && sessionQuery.error instanceof ApiError && sessionQuery.error.status === 404) &&
          session?.status !== 'stopped' &&
          session?.status !== 'failed' && (
            <DialogTrigger asChild>
              <button
                data-testid={`stop-session-${packageName}`}
                disabled={isStopping}
                className={cn(
                  'self-start text-[11px] font-semibold border rounded-control px-3 py-1 transition-colors',
                  isStopping
                    ? 'border-line-2 text-faint opacity-[0.5] cursor-not-allowed'
                    : 'border-[color-mix(in_oklab,var(--red)_50%,var(--line))] text-red hover:bg-[color-mix(in_oklab,var(--red)_13%,transparent)] hover:border-red cursor-pointer'
                )}
              >
                Stop session
              </button>
            </DialogTrigger>
          )}
        <DialogContent>
          <DialogTitle>Confirm Stop Session</DialogTitle>
          <DialogDescription>
            Requests a stop (202 ack); the console polls until stopped/failed.
          </DialogDescription>

          {stopMutation.isError && (
            <div className="text-[12px] text-red font-mono mt-3 leading-relaxed">
              stop request failed: {stopMutation.error instanceof Error ? stopMutation.error.message : String(stopMutation.error)} — session unchanged
            </div>
          )}

          <div className="flex justify-end gap-3 mt-5">
            <button
              disabled={stopMutation.isPending}
              onClick={() => setConfirmOpen(false)}
              className="text-dim bg-raise border border-line-2 rounded-control px-[13px] py-1.5 text-[12.5px] font-medium hover:text-fg hover:border-faint cursor-pointer transition-colors disabled:opacity-[0.5] disabled:cursor-not-allowed"
            >
              Cancel
            </button>
            <button
              disabled={stopMutation.isPending}
              onClick={handleStop}
              className="font-semibold text-[12.5px] border border-red bg-[color-mix(in_oklab,var(--red)_14%,transparent)] text-red hover:bg-[color-mix(in_oklab,var(--red)_22%,transparent)] rounded-control px-[13px] py-1.5 cursor-pointer transition-colors disabled:opacity-[0.5] disabled:cursor-not-allowed"
            >
              {stopMutation.isPending ? 'Stopping...' : 'Confirm Stop'}
            </button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}

export function SettingsScreen() {
  const health = useHealth();
  const { data: packages, isLoading: isPackagesLoading, isError: isPackagesError } = usePackagesList();
  const { getSessionId } = useSessionRegistry();
  const [stalePackages, setStalePackages] = React.useState<Set<string>>(new Set());

  const isAuthRequired = authRequired();
  const { isAuthenticated, login, logout, getUserInfo } = useAuthSession();
  const [userInfo, setUserInfo] = React.useState<OAuthUserInfo | null>(null);

  React.useEffect(() => {
    if (isAuthRequired && isAuthenticated) {
      getUserInfo()
        .then((info) => {
          setUserInfo(info);
        })
        .catch((err) => {
          console.error('Failed to fetch user info in SettingsScreen:', err);
        });
    }
  }, [isAuthRequired, isAuthenticated, getUserInfo]);

  const handleMarkStale = React.useCallback((pkg: string) => {
    setStalePackages((prev) => {
      const next = new Set(prev);
      next.add(pkg);
      return next;
    });
  }, []);

  // Connection states
  const healthDotClass =
    health.healthStatus === 'ok'
      ? 'bg-green'
      : health.healthStatus === 'degraded'
      ? 'bg-gold'
      : 'bg-ghost';

  const healthText =
    health.healthStatus === 'ok'
      ? 'healthy'
      : health.healthStatus === 'degraded'
      ? 'degraded'
      : 'unknown';

  const degradedNote =
    health.healthStatus === 'degraded'
      ? health.mongo === 'down'
        ? 'database connection lost (mongo down)'
        : 'backend is reporting degraded status'
      : null;

  // Render session status block per package
  let renderedSessions = null;
  if (isPackagesLoading) {
    renderedSessions = <div className="text-[12px] text-ghost font-mono">Loading packages...</div>;
  } else if (isPackagesError) {
    renderedSessions = (
      <div className="text-[12.5px] text-faint font-ui leading-relaxed">
        Package list unavailable — session controls require a package name returned by /api/v1/packages.
      </div>
    );
  } else if (!packages || packages.length === 0) {
    renderedSessions = (
      <div className="text-[12.5px] text-faint font-ui leading-relaxed">
        No packages returned by the hosted backend.
      </div>
    );
  } else {
    renderedSessions = (
      <div className="flex flex-col gap-3">
        {packages.map((pkg) => {
          const sessionId = getSessionId(pkg);
          const isStale = stalePackages.has(pkg);

          if (sessionId) {
            return (
              <SessionDetails
                key={pkg}
                packageName={pkg}
                sessionId={sessionId}
                onMarkStale={handleMarkStale}
              />
            );
          } else if (isStale) {
            return (
              <div key={pkg} className="border border-line rounded-card bg-raise p-4 flex flex-col gap-2">
                <div className="font-mono text-[12.5px] text-dim">{pkg}</div>
                <div className="text-[12.5px] text-gold font-mono leading-relaxed">
                  session no longer found — stale registry entry from this tab
                </div>
                <button
                  disabled
                  className="self-start text-[11px] font-medium border border-line-2 bg-raise-2 text-faint rounded-control px-3 py-1 cursor-not-allowed opacity-[0.5]"
                >
                  Stop session
                </button>
              </div>
            );
          } else {
            return (
              <div key={pkg} className="border border-line rounded-card bg-raise p-4 flex flex-col gap-2">
                <div className="font-mono text-[12.5px] text-dim">{pkg}</div>
                <div className="text-[12.5px] text-faint leading-relaxed font-ui">
                  current session id not exposed by the v1 API — this console can only manage sessions it started this tab.
                </div>
                <button
                  disabled
                  className="self-start text-[11px] font-medium border border-line-2 bg-raise-2 text-faint rounded-control px-3 py-1 cursor-not-allowed opacity-[0.5]"
                >
                  Stop session
                </button>
              </div>
            );
          }
        })}
      </div>
    );
  }

  return (
    <div className="flex flex-col max-w-shell mx-auto">
      {/* 1. ACCOUNT */}
      <section aria-labelledby="heading-account" className="py-[30px] border-t border-line first:border-t-0">
        <h2 id="heading-account" className="flex items-baseline gap-3 mb-[18px]">
          <Eyebrow>Account</Eyebrow>
          <span className="text-[12.5px] text-faint font-normal">
            the signed-in user for this console — identity is brokered by <b className="text-dim font-medium">NyxID</b>.
          </span>
        </h2>

        <div className="flex items-center gap-[18px] border border-line rounded-panel bg-raise p-[18px_22px] flex-wrap">
          {isAuthRequired && isAuthenticated && userInfo?.picture ? (
            <img src={userInfo.picture} className="w-12 h-12 rounded-full flex-shrink-0 object-cover border border-line-2" alt="" />
          ) : (
            <div className="w-12 h-12 rounded-full flex-shrink-0 bg-raise-2 border border-line-2 text-dim font-semibold text-[16px] tracking-[0.02em] flex items-center justify-center">
              {isAuthRequired && isAuthenticated ? (
                (userInfo?.name?.charAt(0) || userInfo?.email?.charAt(0) || 'U').toUpperCase()
              ) : (
                '–'
              )}
            </div>
          )}
          <div className="min-w-0 flex flex-col gap-0.5">
            <span className="font-display font-semibold text-[16px] tracking-[-0.01em] text-fg">
              {isAuthRequired && isAuthenticated ? (
                userInfo?.name || 'Authenticated User'
              ) : isAuthRequired && !isAuthenticated ? (
                'Sign in required'
              ) : (
                'Sign-in pending (NyxID)'
              )}
            </span>
            <span className="font-mono text-[12px] text-dim">
              {isAuthRequired && isAuthenticated ? (
                userInfo?.email || userInfo?.sub || 'Active secure session'
              ) : isAuthRequired && !isAuthenticated ? (
                'Please sign in with your NyxID account'
              ) : (
                'NyxID integration pending · no active identity'
              )}
            </span>
            <span className="font-mono text-[11px] text-ghost">
              {isAuthRequired && isAuthenticated ? (
                'Active secure session — the SPA carries only a short-lived NyxID token.'
              ) : (
                'When NyxID lands, the SPA will carry only a short-lived token — never a raw GitHub token.'
              )}
            </span>
          </div>
          <div className="ml-auto max-[780px]:ml-0 max-[780px]:w-full">
            {isAuthRequired && isAuthenticated ? (
              <button
                onClick={logout}
                className="w-full text-dim bg-raise border border-line-2 hover:border-red hover:text-red rounded-control px-3.5 py-1.5 text-[12px] font-medium cursor-pointer transition-colors"
              >
                Sign out
              </button>
            ) : isAuthRequired && !isAuthenticated ? (
              <button
                onClick={() => login()}
                className="w-full text-amber-ink bg-amber hover:brightness-105 rounded-control px-3.5 py-1.5 text-[12px] font-semibold cursor-pointer transition-all"
              >
                Sign in
              </button>
            ) : (
              <>
                <button
                  disabled
                  aria-describedby="account-disabled-note"
                  className="w-full text-dim bg-raise border border-line-2 rounded-control px-3.5 py-1.5 text-[12px] font-medium cursor-not-allowed opacity-[0.5] transition-colors"
                >
                  Sign out
                </button>
                <span id="account-disabled-note" className="sr-only">NyxID integration pending</span>
              </>
            )}
          </div>
        </div>
      </section>

      {/* 2. CONNECTED REPOSITORIES */}
      <section aria-labelledby="heading-repos" className="py-[30px] border-t border-line">
        <h2 id="heading-repos" className="flex items-baseline gap-3 mb-[18px]">
          <Eyebrow>Connected repositories</Eyebrow>
          <span className="text-[12.5px] text-faint font-normal">
            each connected repo is one <b className="text-dim font-medium">deployment</b> — a single host repo (<span className="font-mono">FKST_GITHUB_REPO</span>) running one composed graph. Goals are that repo's issues labeled <b className="text-dim font-medium">fkst-dev:enabled</b>. This console is a fleet view across them.
          </span>
        </h2>

        <HairlineList>
          <div className="bg-raise py-3 px-5 flex items-center justify-between gap-4 flex-wrap">
            <div className="flex items-center gap-4">
              <span className="w-2 h-2 rounded-full flex-shrink-0 bg-ghost" />
              <div className="flex flex-col gap-0.5 min-w-0">
                <span className="font-mono text-[13px] text-ghost">
                  No repositories connected
                </span>
                <span className="font-mono text-[11px] text-ghost">
                  NyxID integration pending
                </span>
              </div>
            </div>
          </div>

          <div className="flex items-center gap-3.5 py-3.5 px-5 bg-[color-mix(in_oklab,var(--raise)_40%,transparent)] flex-wrap">
            <span className="font-mono text-[11px] text-ghost flex-1">
              authorize a repo via <b className="text-faint font-medium">GitHub → NyxID</b> to stand up a new deployment · only its <b className="text-faint font-medium">fkst-dev:enabled</b> issues become goals
            </span>
            <button
              disabled
              aria-describedby="repos-disabled-note"
              className="text-dim bg-raise border border-line-2 rounded-control px-[13px] py-1.5 text-[12px] font-medium cursor-not-allowed opacity-[0.5] transition-colors"
            >
              Connect a repository
            </button>
            <span id="repos-disabled-note" className="sr-only">NyxID integration pending</span>
          </div>
        </HairlineList>
      </section>

      {/* 3. POLL INTERVAL */}
      <section aria-labelledby="heading-poll" className="py-[30px] border-t border-line">
        <h2 id="heading-poll" className="flex items-baseline gap-3 mb-[18px]">
          <Eyebrow>Poll cadence</Eyebrow>
          <span className="text-[12.5px] text-faint font-normal">
            how often the engine re-derives state from GitHub. Data is <b className="text-dim font-medium">poll-derived, not live</b>.
          </span>
        </h2>

        <div className="rounded-panel border border-line bg-raise p-[18px_22px] flex items-center gap-[18px] flex-wrap">
          <div className="flex-1 min-w-0 flex flex-col gap-1">
            <div className="text-[13.5px] text-dim">
              Re-derives every <b className="text-fg font-medium font-mono">5m</b> <span className="text-ghost">— set by the package raisers (cron)</span>
            </div>
            <div className="font-mono text-[11px] text-ghost leading-relaxed">
              <span className="font-mono">interval = "5m"</span> declared statically in each raiser (intake · branch · observability · github poll) · the package source tree is read-only at runtime, so this is not a settings knob · between ticks, counts &amp; state are as-of the last poll
            </div>
          </div>
          <Link
            to="/packages"
            className="flex-shrink-0 inline-flex items-center gap-2 font-mono text-[12px] text-dim border border-line rounded-control px-[11px] py-1.5 bg-raise hover:border-line-2 transition-colors no-underline"
          >
            change on Packages <span className="text-ghost">→</span>
          </Link>
        </div>
      </section>

      {/* 4. WRITE POSTURE & SAFETY */}
      <section aria-labelledby="heading-posture" className="py-[30px] border-t border-line">
        <h2 id="heading-posture" className="flex items-baseline gap-3 mb-[18px]">
          <Eyebrow>Write posture &amp; safety</Eyebrow>
          <span className="text-[12.5px] text-faint font-normal">
            the deployment's global <span className="font-mono">FKST_GITHUB_WRITE</span> — never per-goal. Elevation is deliberate.
          </span>
        </h2>

        <div className="border border-line rounded-panel overflow-hidden bg-raise">
          {/* the verdict, big */}
          <div className="flex items-start gap-[22px] p-[22px_24px] flex-wrap">
            <div className="flex flex-col gap-[9px] min-w-0">
              <div className="flex items-center gap-[13px]">
                <span className="font-display font-semibold text-ghost text-[14px] uppercase tracking-[0.01em]">verdict:</span>
                <PostureChip />
              </div>
              <div className="text-[13px] text-dim max-w-[560px] leading-relaxed">
                DRY-RUN (the deploy default) means no external GitHub writes — the engine plans but stops at execute. The v1 API cannot confirm this deployment's posture.
              </div>
            </div>
            <div className="ml-auto flex flex-col gap-1.5 items-end font-mono text-[11px] text-ghost text-right max-[780px]:ml-0 max-[780px]:items-start max-[780px]:text-left">
              <span className="whitespace-nowrap"><b>FKST_GITHUB_WRITE</b> = unknown</span>
              <span className="whitespace-nowrap">repo <b>unknown</b></span>
              <span className="whitespace-nowrap text-gold">checks read unknown</span>
              <span className="whitespace-nowrap">posture as of <b>unknown</b></span>
            </div>
          </div>
          
          {/* elevation control */}
          <div className="border-t border-line p-[18px_24px] flex items-center gap-4 flex-wrap">
            <div className="min-w-0 flex-1">
              <div className="text-[13px] text-dim">
                Set deployment to <b className="text-fg font-semibold">WRITE: REAL</b>
              </div>
              <div className="font-mono text-[11px] text-ghost mt-[3px]">
                Disabled — global FKST_GITHUB_WRITE is deploy-time env; no API to read or change it in v1; applied via session restart
              </div>
            </div>
            <div className="inline-flex items-center gap-3 cursor-not-allowed select-none">
              <span className="relative w-[46px] h-[26px] rounded-[14px] bg-line-2 transition-colors opacity-[0.5]">
                <i className="absolute top-[3px] left-[3px] w-5 h-5 rounded-full bg-faint" />
              </span>
              <span className="font-semibold text-[12.5px] text-faint">Arm REAL</span>
            </div>
          </div>

          {/* prerequisites */}
          <div className="border-t border-line p-[18px_24px]">
            <div className="font-mono font-semibold text-[11px] tracking-[0.16em] uppercase text-ghost mb-1">
              Prerequisites for REAL
            </div>
            <div className="font-mono text-[11px] text-ghost mb-3">
              poll-derived (~5-min ticks) · re-verified each tick · checks read "unknown" when the hosted service is unreachable
            </div>
            
            <div className="grid grid-cols-2 gap-px border border-line bg-line rounded-card overflow-hidden max-[780px]:grid-cols-1">
              <div className="flex items-start gap-[11px] p-[13px_15px] bg-raise">
                <span className="w-[18px] h-[18px] rounded-full border border-line-2 text-ghost flex items-center justify-center font-bold text-[12px] flex-shrink-0 mt-0.5">
                  ?
                </span>
                <div className="min-w-0 flex flex-col">
                  <span className="text-[12.5px] text-ghost font-medium">CI required &amp; green on the integration branch</span>
                  <span className="font-mono text-[11px] text-ghost mt-[3px]">unknown (no posture/config endpoint in v1; NyxID pending)</span>
                </div>
              </div>
              <div className="flex items-start gap-[11px] p-[13px_15px] bg-raise">
                <span className="w-[18px] h-[18px] rounded-full border border-line-2 text-ghost flex items-center justify-center font-bold text-[12px] flex-shrink-0 mt-0.5">
                  ?
                </span>
                <div className="min-w-0 flex flex-col">
                  <span className="text-[12.5px] text-ghost font-medium">Integration branch set</span>
                  <span className="font-mono text-[11px] text-ghost mt-[3px]">unknown (no posture/config endpoint in v1; NyxID pending)</span>
                </div>
              </div>
              <div className="flex items-start gap-[11px] p-[13px_15px] bg-raise">
                <span className="w-[18px] h-[18px] rounded-full border border-line-2 text-ghost flex items-center justify-center font-bold text-[12px] flex-shrink-0 mt-0.5">
                  ?
                </span>
                <div className="min-w-0 flex flex-col">
                  <span className="text-[12.5px] text-ghost font-medium">Bot login configured</span>
                  <span className="font-mono text-[11px] text-ghost mt-[3px]">unknown (no posture/config endpoint in v1; NyxID pending)</span>
                </div>
              </div>
              <div className="flex items-start gap-[11px] p-[13px_15px] bg-raise">
                <span className="w-[18px] h-[18px] rounded-full border border-line-2 text-ghost flex items-center justify-center font-bold text-[12px] flex-shrink-0 mt-0.5">
                  ?
                </span>
                <div className="min-w-0 flex flex-col">
                  <span className="text-[12.5px] text-ghost font-medium">Branch protection enforced</span>
                  <span className="font-mono text-[11px] text-ghost mt-[3px]">unknown (no posture/config endpoint in v1; NyxID pending)</span>
                </div>
              </div>
            </div>
          </div>

          {/* confirm box, visible but completely disabled */}
          <div className="m-[0_24px_22px] border border-line rounded-modal bg-[color-mix(in_oklab,var(--red)_7%,var(--raise))] p-[18px_20px]">
            <div className="flex gap-[11px] items-start">
              <span className="w-[18px] h-[18px] rounded-full border border-line text-ghost flex items-center justify-center font-semibold text-[11px] flex-shrink-0 mt-0.5">
                !
              </span>
              <div className="text-[13px] text-dim leading-relaxed">
                This sets <span className="font-mono text-fg font-medium">FKST_GITHUB_WRITE = 1</span> on the hosted deployment — REAL enables autonomous merges into the integration branch. This control is currently disabled.
              </div>
            </div>
            <div className="flex gap-3 items-center mt-[15px] flex-wrap">
              <label htmlFor="repo-confirm-input" className="font-mono text-[12px] text-ghost">repo</label>
              <input
                id="repo-confirm-input"
                disabled
                type="text"
                placeholder="disabled"
                className="font-mono text-[13px] text-dim bg-bg border border-line-2 rounded-control p-[9px_12px] w-60 outline-none cursor-not-allowed"
              />
              <button
                disabled
                aria-describedby="posture-disabled-note"
                className="font-semibold text-[12.5px] border border-line-2 bg-raise-2 text-faint rounded-control p-[9px_16px] cursor-not-allowed opacity-[0.5]"
              >
                Enable REAL writes
              </button>
            </div>
            <div className="font-mono text-[11px] text-ghost mt-3 border-t border-line pt-[11px] leading-relaxed">
              Note: real merges are fast and autonomous, so a <b>last-second hold is fictional</b> — there is no "cancel mid-merge." Your real controls are the global posture (switch the deployment back to DRY-RUN) or a GitHub mutation that removes the work (close the PR / remove the enabling label). There is no per-goal pause.
            </div>
            <div id="posture-disabled-note" className="font-mono text-[11px] text-ghost mt-3 border-t border-line pt-[11px] leading-relaxed">
              <b>Grounding · hosted v1 gap:</b> global FKST_GITHUB_WRITE is deploy-time env; no API to read or change it in v1; applied via session restart.
            </div>
          </div>
        </div>
      </section>

      {/* 5. CONNECTIONS */}
      <section aria-labelledby="heading-connections" className="py-[30px] border-t border-line">
        <h2 id="heading-connections" className="flex items-baseline gap-3 mb-[18px]">
          <Eyebrow>Connections</Eyebrow>
          <span className="text-[12.5px] text-faint font-normal">
            how the console reaches GitHub and the hosted engine. The SPA never holds a raw token.
          </span>
        </h2>

        <div className="grid grid-cols-2 gap-px border border-line rounded-panel overflow-hidden bg-line max-[780px]:grid-cols-1">
          {/* NyxID */}
          <div className="bg-raise p-5 min-w-0 flex flex-col gap-3">
            <div className="flex items-center gap-2.5">
              <span className="w-2 h-2 rounded-full flex-shrink-0 bg-ghost" />
              <span className="font-display font-semibold text-[14.5px] text-fg">NyxID — credential broker</span>
              <span className="ml-auto font-mono text-[10.5px] tracking-[0.06em] uppercase text-ghost">pending</span>
            </div>
            <div className="text-[12.5px] text-dim leading-relaxed">
              NyxID brokers the GitHub credential server-side and proxies every call. The SPA carries only a <b className="text-fg font-medium">~15-min NyxID token</b> — never a raw GitHub token.
            </div>
            <div className="font-mono text-[11px] text-ghost leading-relaxed">
              <span className="text-faint">service</span> api-github · https://api.github.com<br />
              <span className="text-faint">token</span> NyxID access · pending integration<br />
              <span className="text-faint">proxy</span> /api/v1/proxy/s/api-github/…
            </div>
            <div className="mt-auto pt-4 flex gap-2.5 items-center flex-wrap">
              <button
                disabled
                aria-describedby="nyxid-disabled-note"
                className="text-dim bg-raise border border-line-2 rounded-control px-3.5 py-1.5 text-[12px] font-medium cursor-not-allowed opacity-[0.5] transition-colors"
              >
                Reconnect
              </button>
              <span id="nyxid-disabled-note" className="inline-flex items-center gap-1.5 font-mono text-[10px] tracking-[0.1em] uppercase text-ghost">
                broker-held · revocable
              </span>
            </div>
          </div>

          {/* Hosted Engine */}
          <div className="bg-raise p-5 min-w-0 flex flex-col gap-3">
            <div className="flex items-center gap-2.5">
              <span className={cn('w-2 h-2 rounded-full flex-shrink-0', healthDotClass)} />
              <span className="font-display font-semibold text-[14.5px] text-fg">Hosted engine — ChronoAI cloud</span>
              <span className="ml-auto font-mono text-[10.5px] tracking-[0.06em] uppercase text-dim">{healthText}</span>
            </div>
            
            <div className="text-[12.5px] text-dim leading-relaxed">
              The Rust backend running <b className="text-fg font-medium">substrate + packages</b>, hosted on ChronoAI. The FE polls GitHub and reads the hosted service's reported status — it does <b className="text-fg font-medium">not</b> command the engine directly.
            </div>

            <div className="font-mono text-[11px] text-ghost leading-relaxed">
              <span className="text-faint">deployment</span> unknown · host-side deployment metadata is not exposed by the v1 API<br />
              <span className="text-faint">api v1</span> <span className="font-mono">GET /health · GET/POST /api/v1/packages · POST/GET/stop /api/v1/sessions</span><br />
              <span className="text-faint">backend build</span> <span data-testid="engine-version">{health.version || 'unknown'}</span><br />
              <span className="text-faint">MongoDB</span> <span className={cn('inline-block w-1.5 h-1.5 rounded-full mr-1', health.mongo === 'up' ? 'bg-green' : health.mongo === 'down' ? 'bg-red' : 'bg-ghost')} />
              {health.mongo || 'unknown'}
            </div>

            <span className="text-gold font-medium mt-1 font-mono text-[11px] block">
              when the hosted service is unreachable, status reads "unknown" — never 0
            </span>

            {degradedNote && (
              <div className="text-[12px] text-gold font-mono leading-relaxed mt-1">
                degraded: {degradedNote}
              </div>
            )}

            <div className="mt-4 border-t border-line pt-4 flex flex-col gap-2">
              <div className="font-mono font-semibold text-[10px] tracking-[0.1em] uppercase text-faint mb-1">
                Session Control
              </div>
              {renderedSessions}
            </div>

            <div className="mt-auto pt-4 flex gap-2.5 items-center flex-wrap">
              <span className="inline-flex items-center gap-1.5 font-mono text-[10px] tracking-[0.1em] uppercase text-ghost">
                engine state read-only · session lifecycle managed via tab registry
              </span>
            </div>
          </div>
        </div>
      </section>

      {/* 6. DEPLOYMENT KNOBS */}
      <section aria-labelledby="heading-knobs" className="py-[30px] border-t border-line">
        <h2 id="heading-knobs" className="flex items-baseline gap-3 mb-[18px]">
          <Eyebrow>Deployment knobs</Eyebrow>
          <span className="text-[12.5px] text-faint font-normal">
            resolved process env → <span className="font-mono">host_root/fkst.env</span> → default. The substrate <span className="font-mono">config</span> command is <b className="text-dim font-medium">read-only</b> (no set API) — changes are deploy-time env on the hosted service + a session restart.
          </span>
        </h2>

        <div className="border border-line rounded-panel overflow-hidden bg-raise">
          {/* Shared note at the top of the pane */}
          <div className="bg-[color-mix(in_oklab,var(--gold)_10%,transparent)] p-[12px_20px] border-b border-line text-[12.5px] text-gold font-mono">
            host-side config, not exposed by the v1 API · no posture/config endpoint in v1; NyxID pending
          </div>

          <HairlineList>
            {[
              { label: 'Host repo (this deployment)', env: 'FKST_GITHUB_REPO' },
              { label: 'Integration branch', env: 'FKST_DEVLOOP_INTEGRATION_BRANCH' },
              { label: 'Upstream branch', env: 'FKST_DEVLOOP_UPSTREAM_BRANCH' },
              { label: 'Rollup merge', env: 'FKST_DEVLOOP_ROLLUP_MERGE' },
              { label: 'Bot login (marker trust)', env: 'FKST_GITHUB_BOT_LOGIN' },
              { label: 'Durable root', env: 'FKST_DURABLE_ROOT' },
              { label: 'Package roots', env: 'FKST_PACKAGE_ROOTS' },
              { label: 'Codex permit slots', env: 'FKST_CODEX_PERMIT_SLOTS' },
              { label: 'Retry / DLQ max attempts', env: 'FKST_RETRY_DEFAULT_MAX_ATTEMPTS' },
            ].map((knob) => (
              <div
                key={knob.env}
                className="grid grid-cols-[minmax(0,300px)_minmax(0,1fr)_auto] items-center gap-4 p-[14px_20px] hover:bg-[color-mix(in_oklab,var(--raise)_28%,transparent)] transition-colors max-[780px]:grid-cols-1 max-[780px]:gap-2"
              >
                <div className="min-w-0">
                  <div className="text-[13px] text-dim font-medium">{knob.label}</div>
                  <div className="font-mono text-[10.5px] text-ghost mt-0.5 tracking-[0.01em]">{knob.env}</div>
                </div>
                <div className="font-mono text-[12.5px] text-ghost italic">
                  unknown
                </div>
                <div className="justify-self-end max-[780px]:justify-self-start">
                  <span className="inline-flex items-center gap-1.5 font-mono text-[10px] tracking-[0.1em] uppercase text-ghost">
                    read-only
                  </span>
                </div>
              </div>
            ))}
          </HairlineList>
        </div>
      </section>

      {/* 7. DANGER ZONE */}
      <section aria-labelledby="heading-danger" className="py-[30px] border-t border-line">
        <h2 id="heading-danger" className="flex items-baseline gap-3 mb-[18px]">
          <Eyebrow>Danger zone</Eyebrow>
          <span className="text-[12.5px] text-faint font-normal">
            hosted-service lifecycle &amp; account removal — irreversible actions live here.
          </span>
        </h2>

        <div className="border border-[color-mix(in_oklab,var(--red)_32%,var(--line))] rounded-panel overflow-hidden bg-raise">
          <div className="flex items-center gap-[18px] p-5 flex-wrap">
            <div className="flex-1 min-w-0">
              <div className="text-[13.5px] text-fg font-medium">Delete account</div>
              <div className="font-mono text-[11px] text-ghost mt-1 leading-relaxed">
                Removes this account and the <b>NyxID-brokered credentials</b> it holds (GitHub access, hosted backend). Connected repositories are released and the console can no longer reach GitHub or the engine. <span className="text-red">Irreversible.</span>
              </div>
            </div>
            <div className="max-[780px]:w-full">
              <button
                disabled
                aria-describedby="danger-disabled-note"
                className="w-full font-medium text-[12.5px] border border-[color-mix(in_oklab,var(--red)_50%,var(--line))] text-red hover:bg-[color-mix(in_oklab,var(--red)_13%,transparent)] rounded-control px-3.5 py-1.5 cursor-not-allowed opacity-[0.5] transition-colors"
              >
                Delete account
              </button>
            </div>
          </div>
          <div id="danger-disabled-note" className="bg-[color-mix(in_oklab,var(--red)_5%,var(--raise))] p-[12px_20px] border-t border-line text-[11px] text-ghost font-mono">
            NyxID integration pending
          </div>
        </div>
      </section>

      {/* Footer honesty strip */}
      <div className="mt-8 pt-4 border-t border-line flex gap-6 font-mono text-[11px] text-ghost flex-wrap">
        <span>posture &amp; prerequisites as of <b>unknown</b> · poll-derived (~5-min ticks)</span>
        <span>merges → <b>integration branch</b> · a rollup PR carries integration → dev · never main</span>
        <span className="text-gold font-medium">hosted engine status reads unknown when unreachable, not 0</span>
        <span>SPA holds a <b>~15-min NyxID token</b>, never a raw GitHub token</span>
      </div>
    </div>
  );
}

export default SettingsScreen;
