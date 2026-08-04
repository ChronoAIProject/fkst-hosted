import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { isScopeDenied, isUnauthenticated } from '@/lib/api/operations';
import type { ActivityScope, SandboxScope } from '@/lib/api/operations';
import { activityCacheKey, sandboxCacheKey } from '@/lib/operations/keys';
import {
  DAY_MS,
  DEFAULT_ACTIVITY_FILTERS,
  DEFAULT_MAX_RANGE_DAYS,
  DEFAULT_SANDBOX_FILTERS,
  needsSessionId,
  windowProblem,
} from '@/lib/operations/state';
import type { ActivityFilters, OperationsState, SandboxFilters } from '@/lib/operations/state';
import { clearCrossActorFilters, decodeState, encodeState, personalScope } from '@/lib/operations/url';
import { useOperationsActivity } from '@/lib/hooks/use-operations-activity';
import { useOperationsSandboxes } from '@/lib/hooks/use-operations-sandboxes';
import { ActivityView } from '@/components/operations/activity-view';
import { SandboxView } from '@/components/operations/sandbox-view';
import { Tabs } from '@/components/operations/tabs';
import { Notice } from '@/components/operations/parts';
import { OperationsGate, OperationsUnconfigured } from './operations-gate';

/**
 * `/operations` — the authenticated operational workspace.
 *
 * The page is a thin orchestrator over three things: the URL (which IS the
 * state), two independent feeds, and one server-stated capability. The
 * authorization rules it must not break, and how each is honoured here:
 *
 * - **`effective_scope` and `can_view_all` are the only truth.** `canViewAll`
 *   below is derived exclusively from a successful response for the CURRENT
 *   identity generation, and it fails closed: unknown means "regular user". No
 *   dashboard overview, token claim, localStorage value or URL parameter can
 *   raise it. A `403 operations_scope_forbidden` lowers it immediately.
 * - **A crafted `?scope=all` cannot flash hidden rows.** The scope is part of
 *   every cache key, so there is no cached global page to show; the request is
 *   made, the denial is handled, and the URL is rewritten to the allowed scope.
 * - **Identity changes clear everything synchronously.** The identity generation
 *   is the first component of both cache keys, so a sign-out or account switch
 *   drops rows, cursors, and the capability in the same render.
 * - **The two feeds are independent.** Each owns its own error state, so an
 *   analytics outage cannot remove the live sandbox table and a runtime outage
 *   cannot falsify activity.
 *
 * One deliberate cost is documented rather than hidden: the page writes an
 * explicit personal scope into the URL on mount and upgrades a global
 * administrator to the global scope once their capability is known. That costs
 * an administrator one extra request on their first load, and buys an exact
 * scope in the URL from the first render — which is what lets the personal
 * lifecycle guard below be correct instead of guessing.
 */
export function Operations() {
  const t = useContent().operations;
  const { configured, isAuthenticated, identityGeneration, error, sessionExpired, signIn, apiFetch } =
    useAuth();
  const [searchParams, setSearchParams] = useSearchParams();
  const baseId = useId();
  const panelId = `${baseId}-panel`;

  const search = searchParams.toString();
  const decoded = useMemo(() => decodeState(new URLSearchParams(search)), [search]);
  const state = decoded.state;

  // Server-stated capability, valid only for the identity that earned it.
  const [capability, setCapability] = useState<{ generation: number; canViewAll: boolean } | null>(
    null
  );
  const canViewAll = capability?.generation === identityGeneration && capability.canViewAll;
  // Which identity generation has already had its default scope resolved, so an
  // administrator is upgraded to the global scope exactly once and a deliberate
  // switch back to `Mine` is never undone.
  const scopeResolvedRef = useRef<number | null>(null);
  const [scopeReset, setScopeReset] = useState(false);
  // This deployment's own window ceiling, as the last page stated it. It is
  // deployment policy rather than a client constant: guessing it would either
  // refuse windows this deployment answers, or send windows it always refuses.
  const [maxRangeDays, setMaxRangeDays] = useState(DEFAULT_MAX_RANGE_DAYS);

  useEffect(() => {
    document.title = t.metaTitle;
  }, [t.metaTitle]);

  const writeState = useCallback(
    (next: OperationsState) => {
      setSearchParams(encodeState(next), { replace: true });
    },
    [setSearchParams]
  );

  // A new identity has no capability and no resolved scope of its own.
  useEffect(() => {
    setCapability(null);
    setScopeReset(false);
    scopeResolvedRef.current = null;
  }, [identityGeneration]);

  // Fail closed on mount: an absent scope becomes the PERSONAL one before any
  // request is issued, so the lifecycle guard below is exact from the first
  // render rather than guessing at a capability nobody has stated yet.
  useEffect(() => {
    if (state.scope !== null) return;
    writeState({ ...state, scope: personalScope(state.tab) });
  }, [state, writeState]);

  const isGlobal = state.scope === 'all';
  const activityScope: ActivityScope | null =
    state.tab === 'activity' ? (isGlobal ? 'all' : 'mine') : null;
  const sandboxScope: SandboxScope | null =
    state.tab === 'sandboxes' ? (isGlobal ? 'all' : 'accessible') : null;

  // The two reasons the UI deliberately withholds a request. Neither is a
  // failure and neither is an empty result: no query ran, so the panel states
  // which one piece is missing instead of claiming that nothing matched. Both
  // are re-derived by the feed hook, which is what actually withholds.
  const sessionRequired = needsSessionId(state.activity, activityScope);
  const windowIssue = windowProblem(state.activity, maxRangeDays * DAY_MS);
  const authed = isAuthenticated && configured;

  const activityFeed = useOperationsActivity({
    apiFetch,
    cacheKey: activityCacheKey(identityGeneration, activityScope, state.activity),
    scope: activityScope,
    filters: state.activity,
    enabled: authed && state.tab === 'activity' && state.scope !== null,
    maxRangeDays,
  });

  const sandboxFeed = useOperationsSandboxes({
    apiFetch,
    cacheKey: sandboxCacheKey(identityGeneration, sandboxScope, state.sandbox),
    scope: sandboxScope,
    filters: state.sandbox,
    enabled: authed && state.tab === 'sandboxes' && state.scope !== null,
  });

  // Adopt the bound the deployment states, so the controls refuse exactly what
  // its validator would.
  //
  // Adjusted DURING render rather than in an effect. An effect commits one
  // render later, which leaves a window where the rows of the first page are on
  // screen while the ceiling is still the client's optimistic default — and a
  // range change landing in that window is authorized against the wrong bound
  // and issues a request the deployment is guaranteed to refuse. React re-runs
  // this component with the corrected value before any effect fires, so the feed
  // hook below never sees the stale one. `setState` during render is the
  // documented way to derive state from a changed input; it is guarded on
  // inequality, so it cannot loop.
  const answeredMaxRange = activityFeed.page?.max_range_days;
  if (answeredMaxRange !== undefined && answeredMaxRange !== maxRangeDays) {
    setMaxRangeDays(answeredMaxRange);
  }

  // Adopt the capability every successful response states, and upgrade a global
  // administrator to their documented default scope exactly once.
  const answeredCanViewAll = activityFeed.page?.can_view_all ?? sandboxFeed.inventory?.can_view_all;
  useEffect(() => {
    if (answeredCanViewAll === undefined) return;
    setCapability({ generation: identityGeneration, canViewAll: answeredCanViewAll });
    if (scopeResolvedRef.current === identityGeneration) return;
    scopeResolvedRef.current = identityGeneration;
    if (answeredCanViewAll && state.scope !== 'all') {
      writeState({ ...state, scope: 'all' });
    }
  }, [answeredCanViewAll, identityGeneration, state, writeState]);

  // A denied scope is a server statement about this caller: lower the
  // capability, drop the filters only the global scope may carry, and rewrite
  // the URL so the view and the address bar agree.
  const denied = isScopeDenied(activityFeed.error) || isScopeDenied(sandboxFeed.error);
  useEffect(() => {
    if (!denied) return;
    setCapability({ generation: identityGeneration, canViewAll: false });
    scopeResolvedRef.current = identityGeneration;
    setScopeReset(true);
    writeState({
      ...state,
      scope: personalScope(state.tab),
      activity: clearCrossActorFilters(state.activity),
    });
  }, [denied, identityGeneration, state, writeState]);

  const onTabChange = useCallback(
    (tab: OperationsState['tab']) => {
      setScopeReset(false);
      writeState({
        ...state,
        tab,
        // `all` is the one scope word both views share; every other value must
        // be re-expressed in the destination view's own vocabulary.
        scope: state.scope === 'all' ? 'all' : personalScope(tab),
      });
    },
    [state, writeState]
  );

  const onScopeChange = useCallback(
    (scope: 'all' | 'personal') => {
      setScopeReset(false);
      // A deliberate choice ends the one-time automatic upgrade.
      scopeResolvedRef.current = identityGeneration;
      const next = scope === 'all' ? 'all' : personalScope(state.tab);
      writeState({
        ...state,
        scope: next,
        activity: next === 'all' ? state.activity : clearCrossActorFilters(state.activity),
      });
    },
    [identityGeneration, state, writeState]
  );

  const onActivityFilters = useCallback(
    (activity: ActivityFilters) => writeState({ ...state, activity }),
    [state, writeState]
  );
  const onSandboxFilters = useCallback(
    (sandbox: SandboxFilters) => writeState({ ...state, sandbox }),
    [state, writeState]
  );

  /** The sandbox → activity cross-link. It carries the session id and asks for
   *  the whole record kind; WHAT that yields is the server's decision — a
   *  regular caller gets their own calls plus this session's lifecycle rows and
   *  no other human's calls, because the API applies that predicate. */
  const onViewActivity = useCallback(
    (sessionId: string) => {
      setScopeReset(false);
      writeState({
        tab: 'activity',
        scope: state.scope === 'all' ? 'all' : 'mine',
        activity: { ...DEFAULT_ACTIVITY_FILTERS, recordKind: 'all', sessionId },
        sandbox: DEFAULT_SANDBOX_FILTERS,
      });
    },
    [state.scope, writeState]
  );

  // An involuntary 401 that survived a refresh means the session is gone; the
  // workspace stays mounted with a re-authenticate prompt so the user's filters
  // and tab survive the round trip.
  const expired =
    sessionExpired ||
    isUnauthenticated(activityFeed.error) ||
    isUnauthenticated(sandboxFeed.error);

  if (!isAuthenticated && !sessionExpired) {
    return <OperationsGate error={error} configured={configured} onSignIn={signIn} />;
  }
  if (!configured) {
    return <OperationsUnconfigured />;
  }

  const heading =
    state.tab === 'activity'
      ? isGlobal
        ? t.headingActivityAll
        : t.headingActivityMine
      : isGlobal
        ? t.headingSandboxAll
        : t.headingSandboxAccessible;

  return (
    <div className="h-full min-h-0 flex flex-col gap-3">
      <div className="flex-none flex items-center gap-3 flex-wrap">
        <h1 className="font-display font-semibold text-[19px] text-fg flex-none">{t.title}</h1>
        <span className="font-mono text-eyebrow text-ghost uppercase">{t.effectiveScope}</span>
        <span data-testid="operations-scope" className="font-mono text-[11.5px] text-amber">
          {heading}
        </span>
        <span className="flex-1" aria-hidden="true" />
        <Tabs
          tabs={[
            { key: 'activity' as const, label: t.tabActivity },
            { key: 'sandboxes' as const, label: t.tabSandboxes },
          ]}
          value={state.tab}
          onChange={onTabChange}
          ariaLabel={t.tabsAria}
          idBase={baseId}
          panelId={panelId}
          testId="operations-tabs"
        />
        {/* The scope control exists only while the SERVER says this caller may
            select the global scope. Hiding it is convenience; the API is the
            boundary either way. */}
        {canViewAll && (
          <Tabs
            tabs={[
              { key: 'all' as const, label: t.scopeAll },
              {
                key: 'personal' as const,
                label: state.tab === 'activity' ? t.scopeMine : t.scopeAccessible,
              },
            ]}
            value={isGlobal ? 'all' : 'personal'}
            onChange={onScopeChange}
            ariaLabel={t.scopeAria}
            idBase={`${baseId}-scope`}
            panelId={panelId}
            size="sm"
            testId="operations-scope-control"
          />
        )}
      </div>

      {expired && (
        <div className="anim-row-in flex-none border border-line border-l-2 border-l-amber rounded-card bg-glass backdrop-blur-glass px-3 py-2 flex items-center gap-3 flex-wrap">
          <div className="min-w-0">
            <p className="font-ui font-semibold text-[12.5px] text-fg">{t.expiredTitle}</p>
            <p className="font-mono text-[11px] text-dim">{t.expiredBody}</p>
          </div>
          <button
            type="button"
            onClick={signIn}
            className="ml-auto flex-none font-ui font-semibold text-[12px] bg-grad-accent text-amber-ink rounded-control px-3 py-1.5 cursor-pointer"
          >
            {t.expiredAction}
          </button>
        </div>
      )}
      {decoded.ignored.length > 0 && (
        <Notice testId="operations-ignored-params">
          {t.ignoredParams.replace('{names}', decoded.ignored.join(', '))}
        </Notice>
      )}
      {scopeReset && <Notice testId="operations-scope-reset">{t.scopeReset}</Notice>}

      <div
        role="tabpanel"
        id={panelId}
        aria-labelledby={`${baseId}-tab-${state.tab}`}
        tabIndex={0}
        className="flex-1 min-h-0 flex flex-col outline-none"
      >
        {state.tab === 'activity' ? (
          <ActivityView
            feed={activityFeed}
            filters={state.activity}
            showActorFilters={isGlobal}
            sessionRequired={sessionRequired}
            windowIssue={windowIssue}
            maxRangeDays={maxRangeDays}
            onFiltersChange={onActivityFilters}
            onReset={() => writeState({ ...state, activity: DEFAULT_ACTIVITY_FILTERS })}
          />
        ) : (
          <SandboxView
            feed={sandboxFeed}
            filters={state.sandbox}
            onFiltersChange={onSandboxFilters}
            onReset={() => writeState({ ...state, sandbox: DEFAULT_SANDBOX_FILTERS })}
            onViewActivity={onViewActivity}
          />
        )}
      </div>
    </div>
  );
}
