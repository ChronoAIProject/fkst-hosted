import { useCallback, useId, useRef, useState, type KeyboardEvent } from 'react';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { cn } from '@/lib/utils';
import { getObserve } from '@/lib/api/observe';
import { decodeSessionStatus } from '@/lib/api/derive';
import type { SessionDetail } from '@/lib/api/types';
import { Chip } from '@/components/ui/chip';
import { CopyButton } from '@/components/ui/copy-button';
import { FadeSwap } from '@/components/ui/motion';
import { DrawerShell } from './drawer-shell';
import { PHASE_TONE } from './tones';
import type { ObserveState } from './observe-state';
import { TabStatus } from './tab-status';
import { TabPackages } from './tab-packages';
import { TabLogs } from './tab-logs';
import { TabOutcomes } from './tab-outcomes';

type TabKey = 'status' | 'packages' | 'logs' | 'outcomes';

/** The per-session detail drawer: a header with the decoded status pill and a
 *  four-tab body (status / packages / logs / outcomes). The observe fetch is
 *  lifted here so the Status and Packages tabs share one slow pod-exec call.
 *
 *  Tabs follow the WAI-ARIA tabs pattern: each `role="tab"` owns the single
 *  stable `role="tabpanel"` (`aria-controls`), the panel is labelled back by the
 *  active tab (`aria-labelledby`), and the tablist implements a roving tabindex
 *  with ArrowLeft/ArrowRight/Home/End moving both focus and selection. */
export function SessionDetailDrawer({
  owner,
  name,
  session,
  onClose,
}: {
  owner: string;
  name: string;
  session: SessionDetail;
  onClose: () => void;
}) {
  const c = useContent().dashboard;
  const t = c.detail;
  const { apiFetch } = useAuth();
  const titleId = useId();
  // One base id yields stable, unique ids for every tab + the shared panel so
  // the aria-controls / aria-labelledby linkage survives re-renders.
  const baseId = useId();
  const tabId = (key: TabKey) => `${baseId}-tab-${key}`;
  const panelId = `${baseId}-panel`;

  const [tab, setTab] = useState<TabKey>('status');
  const [observe, setObserve] = useState<ObserveState>({ status: 'idle' });

  // Live refs to each tab button so the arrow-key handler can move focus onto
  // the newly-selected tab (roving tabindex requires focus follow selection).
  const tabRefs = useRef<Partial<Record<TabKey, HTMLButtonElement | null>>>({});

  const status = decodeSessionStatus(session);

  const loadObserve = useCallback(() => {
    const sessionId = session.session_id;
    if (!sessionId) {
      setObserve({ status: 'error' });
      return;
    }
    setObserve({ status: 'loading' });
    getObserve(apiFetch, sessionId)
      .then((snapshot) => setObserve({ status: 'loaded', snapshot }))
      .catch(() => setObserve({ status: 'error' }));
  }, [apiFetch, session.session_id]);

  const tabs: Array<{ key: TabKey; label: string }> = [
    { key: 'status', label: t.tabStatus },
    { key: 'packages', label: t.tabPackages },
    { key: 'logs', label: t.tabLogs },
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
    <DrawerShell titleId={titleId} onClose={onClose}>
      <div className="sticky top-0 z-10 bg-raise border-b border-line px-5 py-4 flex flex-col gap-3">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0 flex flex-col gap-1.5">
            <h2 id={titleId} className="font-display font-semibold text-[17px] text-fg truncate">
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
          <button
            type="button"
            onClick={onClose}
            aria-label={t.closeAria}
            className="font-ui font-semibold text-[12px] border border-line rounded-control px-3 py-1.5 text-dim hover:text-fg transition-colors cursor-pointer flex-none"
          >
            {t.close}
          </button>
        </div>

        <div
          role="tablist"
          aria-label={t.tabsAria}
          onKeyDown={onTablistKeyDown}
          // Roving tabindex lives on the tabs; -1 here just makes the container
          // a valid focus target for the delegated arrow-key handler.
          tabIndex={-1}
          className="flex items-center gap-1 flex-wrap"
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
              className={cn(
                'font-ui font-semibold text-[12.5px] rounded-control px-3 py-1.5 transition-colors cursor-pointer',
                tab === key
                  ? 'bg-raise-2 text-fg border border-line-2'
                  : 'text-dim hover:text-fg border border-transparent'
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
        className="relative px-5 py-4 outline-none"
      >
        <FadeSwap k={tab}>
          {tab === 'status' && (
            <TabStatus session={session} observe={observe} onLoadObserve={loadObserve} />
          )}
          {tab === 'packages' && <TabPackages session={session} observe={observe} />}
          {tab === 'logs' && <TabLogs session={session} />}
          {tab === 'outcomes' && (
            <TabOutcomes owner={owner} name={name} issue={session.trigger.number} />
          )}
        </FadeSwap>
      </div>
    </DrawerShell>
  );
}
