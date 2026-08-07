import { useCallback, useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { cn } from '@/lib/utils';
import { getObserve, ObserveError } from '@/lib/api/observe';
import { getSessionHealth, HealthError } from '@/lib/api/health';
import { canQueueSessionWork, decodeSessionStatus, sessionWorkLabels } from '@/lib/api/derive';
import type { SessionDetail } from '@/lib/api/types';
import { CreateWorkItemModal } from '@/components/modals/create-work-item-modal';
import { Chip } from '@/components/ui/chip';
import { ScrollArea } from '@/components/ui/scroll-area';
import { CopyButton } from '@/components/ui/copy-button';
import { FadeSwap } from '@/components/ui/motion';
import { PHASE_TONE } from './tones';
import type { ObserveState } from './observe-state';
import { TabStatus } from './tab-status';
import { TabPackages } from './tab-packages';
import { TabLogs } from './tab-logs';
import { TabOutcomes } from './tab-outcomes';
import { TabHealth, type HealthState } from './tab-health';
import { TabEngine } from './tab-engine';
import { SessionWorkflows } from '@/components/workflows/session-workflows';
import { healthChip } from './health-state';

type TabKey = 'status' | 'packages' | 'logs' | 'health' | 'workflows' | 'engine' | 'outcomes';

/** The reusable inner detail surface: a sticky header with the decoded status
 *  pill and a seven-tab body (status / packages / logs / health / workflows /
 *  engine / outcomes). It renders identically inside the overlay drawer
 *  (SessionDetailDrawer) and an inline workspace scroll area — the only
 *  difference is the header Close button, which is emitted only when an
 *  `onClose` handler is supplied. The observe fetch STATE is lifted here so the
 *  Engine and Packages tabs share one slow pod-exec call; only Engine triggers
 *  it.
 *
 *  Tabs follow the WAI-ARIA tabs pattern: each `role="tab"` owns the single
 *  stable `role="tabpanel"` (`aria-controls`), the panel is labelled back by the
 *  active tab (`aria-labelledby`), and the tablist implements a roving tabindex
 *  with ArrowLeft/ArrowRight/Home/End moving both focus and selection.
 *
 *  `titleId` lets a host (the drawer) supply the id its dialog labels itself
 *  by, so `aria-labelledby` on the surrounding dialog points at this header's
 *  heading exactly. When omitted (inline use) the heading self-generates a
 *  stable id, keeping it a valid label target on its own. */
export function SessionDetailView({
  owner,
  name,
  session,
  onChanged,
  readOnly = false,
  onClose,
  titleId: titleIdProp,
}: {
  owner: string;
  name: string;
  session: SessionDetail;
  /** Refresh the repository projection after a successful work-item mutation.
   *  When omitted, this host is inspection-only. */
  onChanged?: () => void;
  /** Suppress mutations for App-wide cross-account inspection. */
  readOnly?: boolean;
  onClose?: () => void;
  titleId?: string;
}) {
  const c = useContent().dashboard;
  const t = c.detail;
  const { apiFetch } = useAuth();
  const fallbackTitleId = useId();
  const titleId = titleIdProp ?? fallbackTitleId;
  // One base id yields stable, unique ids for every tab + the shared panel so
  // the aria-controls / aria-labelledby linkage survives re-renders.
  const baseId = useId();
  const tabId = (key: TabKey) => `${baseId}-tab-${key}`;
  const panelId = `${baseId}-panel`;

  const [tab, setTab] = useState<TabKey>('status');
  const [observe, setObserve] = useState<ObserveState>({ status: 'idle' });
  // The health listing is LIFTED here, not deferred to the tab, because the header
  // chip is the "at a glance" half of the feature and cannot wait for the reader to
  // open the tab. One fetch serves both surfaces, so this is strictly fewer requests
  // than fetching per tab activation; the backend serves it from a TTL-cached index.
  const [health, setHealth] = useState<HealthState>({ status: 'idle' });
  const [showWorkItem, setShowWorkItem] = useState(false);

  // Live refs to each tab button so the arrow-key handler can move focus onto
  // the newly-selected tab (roving tabindex requires focus follow selection).
  const tabRefs = useRef<Partial<Record<TabKey, HTMLButtonElement | null>>>({});

  const status = decodeSessionStatus(session);
  const workLabels = sessionWorkLabels(session);
  const canQueue = onChanged != null && !readOnly && canQueueSessionWork(session);

  const loadObserve = useCallback(() => {
    const sessionId = session.session_id;
    if (!sessionId) {
      setObserve({ status: 'error' });
      return;
    }
    setObserve({ status: 'loading' });
    getObserve(apiFetch, sessionId)
      .then((snapshot) => setObserve({ status: 'loaded', snapshot }))
      // Thread the HTTP status through so the Status tab can distinguish 409 (no
      // durable store to observe) from a transient failure; a non-ObserveError
      // throw (e.g. network) carries no status.
      .catch((err) =>
        setObserve({
          status: 'error',
          httpStatus: err instanceof ObserveError ? err.status : undefined,
        })
      );
  }, [apiFetch, session.session_id]);

  const loadHealth = useCallback(() => {
    const sessionId = session.session_id;
    if (!sessionId) {
      setHealth({ status: 'error' });
      return;
    }
    setHealth({ status: 'loading' });
    getSessionHealth(apiFetch, sessionId)
      .then((loaded) => setHealth({ status: 'loaded', health: loaded }))
      .catch((err) =>
        setHealth({
          status: 'error',
          httpStatus: err instanceof HealthError ? err.status : undefined,
        })
      );
  }, [apiFetch, session.session_id]);

  useEffect(() => {
    loadHealth();
  }, [loadHealth]);

  const chip = healthChip(health.status === 'loaded' ? health.health : null);

  const tabs: Array<{ key: TabKey; label: string }> = [
    { key: 'status', label: t.tabStatus },
    { key: 'packages', label: t.tabPackages },
    { key: 'logs', label: t.tabLogs },
    { key: 'health', label: t.tabHealth },
    // Workflows and Engine both sit between Health and Outcomes deliberately:
    // inserting there keeps ArrowRight from Status on Packages and {End} on
    // Outcomes, so the drawer's existing keyboard contract is unchanged by
    // adding a tab. Any further tab belongs in this same interior window.
    { key: 'workflows', label: t.tabWorkflows },
    { key: 'engine', label: t.tabEngine },
    { key: 'outcomes', label: t.tabOutcomes },
  ];

  // Roving-focus keyboard nav across the tablist. Selection follows focus
  // (automatic activation), which is the recommended pattern when panel content
  // is cheap to reveal — here each panel is already in the tree.
  const onTablistKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const idx = tabs.findIndex((x) => x.key === tab);
    let next = idx;
    if (e.key === 'ArrowRight') next = (idx + 1) % tabs.length;
    else if (e.key === 'ArrowLeft') next = (idx - 1 + tabs.length) % tabs.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = tabs.length - 1;
    else return;
    e.preventDefault();
    const nextKey = tabs[next]!.key;
    setTab(nextKey);
    tabRefs.current[nextKey]?.focus();
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* Frosted header: a translucent bg-glass strip with backdrop-blur
          keeps it legible while body content scrolls faintly beneath it, and a
          layered highlight/hairline seats it above the panel. */}
      <div className="flex-none bg-glass backdrop-blur-glass border-b border-line px-5 py-4 flex flex-col gap-3 shadow-[var(--shadow-1),var(--highlight-top)]">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0 flex flex-col gap-1.5">
            {/* Bright fg→dim display sweep on the session name for a premium
                heading; truncation and font are preserved. */}
            <h2
              id={titleId}
              className="font-display font-semibold text-[17px] grad-text grad-text-fg truncate"
            >
              {session.name ?? c.invalidTrigger}
            </h2>
            <div className="flex items-center gap-1.5 flex-wrap">
              {/* Header chips pop in on mount (anim-chip-in collapses to the
                  final state under prefers-reduced-motion). Chip renders a bare
                  span with no className slot, so the animation rides a wrapper. */}
              <span className="anim-chip-in inline-flex">
                <Chip tone={PHASE_TONE[status.phase]}>{t.phase[status.phase]}</Chip>
              </span>
              {status.liveness && (
                <span className="anim-chip-in inline-flex">
                  <Chip tone={status.liveness === 'live' ? 'green' : 'neutral'}>
                    {status.liveness}
                  </Chip>
                </span>
              )}
              {/* Business-aware health. A STALE heartbeat overrides the reported
                  status (a 35-minute-old "working" verdict is not evidence of
                  work); `not_running` renders neutral because a reaped pod is
                  normal; `never_reported` renders nothing at all. */}
              {chip && (
                <span className="anim-chip-in inline-flex">
                  <Chip tone={chip.tone}>
                    {chip.kind === 'stale' ? t.healthStaleChip : t.healthStatus[chip.status]}
                  </Chip>
                </span>
              )}
            </div>
            {session.session_id && (
              // Full session id (not truncated) + copy. break-all keeps a long
              // id from forcing the drawer wider than its panel.
              <div className="anim-chip-in flex items-center gap-1.5 min-w-0">
                <span className="font-mono text-[10.5px] text-ghost break-all min-w-0">
                  {session.session_id}
                </span>
                <CopyButton value={session.session_id} label={t.sessionIdCopy} />
              </div>
            )}
          </div>
          <div className="flex items-center gap-2 flex-none">
            {canQueue && (
              <button
                type="button"
                onClick={() => setShowWorkItem(true)}
                data-tour="new-work-item"
                className="anim-sheen font-ui font-semibold text-[12px] bg-grad-accent text-amber-ink rounded-control px-3 py-1.5 shadow-[var(--shadow-1),var(--glow-amber)] transition-[filter,box-shadow] hover:brightness-110 cursor-pointer"
              >
                {c.canvas.addWorkItem}
              </button>
            )}
            {/* Close button is drawer-only: inline/workspace hosts omit onClose. */}
            {onClose && (
              <button
                type="button"
                onClick={onClose}
                aria-label={t.closeAria}
                className="font-ui font-semibold text-[12px] border border-line rounded-control px-3 py-1.5 text-dim transition-[color,border-color,box-shadow] duration-150 hover:text-fg hover:border-line-2 hover:shadow-glow-amber cursor-pointer"
              >
                {t.close}
              </button>
            )}
          </div>
        </div>

        <dl className="flex items-start gap-x-5 gap-y-1.5 flex-wrap font-mono text-[10.5px]">
          <div className="flex items-baseline gap-1.5 min-w-0">
            <dt className="text-ghost">{t.creatorLabel}</dt>
            <dd className="text-dim break-all">
              {session.creator ? `@${session.creator}` : t.configUnset}
            </dd>
          </div>
          <div className="flex items-baseline gap-1.5 min-w-0">
            <dt className="text-ghost">{t.sourceBranchLabel}</dt>
            <dd className="text-dim break-all">{session.source_branch ?? t.repoDefault}</dd>
          </div>
          <div className="flex items-baseline gap-1.5 min-w-0">
            <dt className="text-ghost">{t.targetBranchLabel}</dt>
            <dd className="text-dim break-all">{session.target_branch}</dd>
          </div>
        </dl>

        <div
          role="tablist"
          aria-label={t.tabsAria}
          onKeyDown={onTablistKeyDown}
          // Roving tabindex lives on the tabs; -1 here just makes the container
          // a valid focus target for the delegated arrow-key handler.
          tabIndex={-1}
          // Glass segmented strip: a translucent frosted rail with a hairline
          // seats the tabs as one control off the header.
          className="flex items-center gap-1 flex-wrap glass border border-line rounded-control p-1"
        >
          {tabs.map(({ key, label }) => (
            <button
              key={key}
              ref={(el) => {
                tabRefs.current[key] = el;
              }}
              type="button"
              role="tab"
              id={tabId(key)}
              aria-selected={tab === key}
              aria-controls={panelId}
              // Roving tabindex: only the active tab is Tab-reachable; arrows
              // move between the rest.
              tabIndex={tab === key ? 0 : -1}
              onClick={() => setTab(key)}
              // `hover-underline` grows an amber gradient underline under any tab
              // on hover/focus (underline-grow); the active tab is a raised
              // frosted pill with amber text + a soft amber bloom.
              className={cn(
                'hover-underline font-ui font-semibold text-[12.5px] rounded-control px-3 py-1.5 transition-[color,background-color,box-shadow] duration-150 cursor-pointer',
                tab === key
                  ? 'bg-glass-2 text-amber shadow-[var(--shadow-1),var(--glow-amber)]'
                  : 'text-dim hover:text-fg'
              )}
            >
              {label}
            </button>
          ))}
        </div>
      </div>

      {/* Single stable tabpanel: all tabs point their aria-controls here, and it
          is labelled back by whichever tab is active. `relative` contains the
          FadeSwap's outgoing (absolutely-positioned) body during the crossfade. */}
      <div
        role="tabpanel"
        id={panelId}
        aria-labelledby={tabId(tab)}
        tabIndex={0}
        className="relative flex-1 min-h-0 flex flex-col outline-none"
      >
        {/* The PANEL is the fixed box; each tab owns its own scrollbar inside it.
            Scrolling here instead would drag the header and tablist away with the
            body, and — for a master/detail tab like Logs or Health — slide the
            navigation rail out of view while reading an entry. Every tab scrolls
            through the same themed ScrollArea so the scrollbar looks identical
            across tabs. */}
        <FadeSwap k={tab} className="flex-1 min-h-0 flex flex-col">
          {tab === 'status' && (
            <ScrollArea className="px-5 py-4">
              <TabStatus session={session} />
            </ScrollArea>
          )}
          {tab === 'packages' && (
            <ScrollArea className="px-5 py-4">
              <TabPackages session={session} observe={observe} />
            </ScrollArea>
          )}
          {/* Logs and Health are master/detail tabs: each manages TWO scroll
              regions of its own (rail + detail), so they get the fixed box
              rather than a scroller — nesting one inside another would give
              them two competing scrollbars, and the outer one would scroll the
              navigation rail out of view while reading. */}
          {tab === 'logs' && (
            <div className="flex-1 min-h-0 px-5 py-4">
              <TabLogs session={session} />
            </div>
          )}
          {tab === 'health' && (
            <div className="flex-1 min-h-0 px-5 py-4">
              <TabHealth sessionId={session.session_id ?? ''} state={health} onRetry={loadHealth} />
            </div>
          )}
          {/* Workflows is a third master/detail tab (schedule rail + detail), so
              it gets the fixed box for the same reason Logs and Health do. */}
          {tab === 'workflows' && (
            <div className="flex-1 min-h-0 flex flex-col px-5 py-4">
              <SessionWorkflows owner={owner} name={name} creator={session.creator} />
            </div>
          )}
          {/* The observe fetch is triggered HERE, by opening this tab — never by
              Status, which must cost no request. The STATE stays lifted above so
              Packages can surface the same snapshot's per-queue activity without
              a second pod exec. */}
          {tab === 'engine' && (
            <ScrollArea className="px-5 py-4">
              <TabEngine session={session} observe={observe} onLoadObserve={loadObserve} />
            </ScrollArea>
          )}
          {tab === 'outcomes' && (
            <ScrollArea className="px-5 py-4">
              <TabOutcomes owner={owner} name={name} issue={session.trigger.number} />
            </ScrollArea>
          )}
        </FadeSwap>
      </div>

      {showWorkItem && (
        <CreateWorkItemModal
          owner={owner}
          name={name}
          triggerIssue={session.trigger.number}
          creator={session.creator}
          workLabels={workLabels}
          onClose={() => setShowWorkItem(false)}
          onCreated={() => {
            setShowWorkItem(false);
            onChanged?.();
          }}
        />
      )}
    </div>
  );
}
