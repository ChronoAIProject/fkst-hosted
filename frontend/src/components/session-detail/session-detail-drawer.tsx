import { useCallback, useId, useState } from 'react';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { cn } from '@/lib/utils';
import { getObserve } from '@/lib/api/observe';
import { decodeSessionStatus } from '@/lib/api/derive';
import type { SessionDetail } from '@/lib/api/types';
import { Chip } from '@/components/ui/chip';
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
 *  lifted here so the Status and Packages tabs share one slow pod-exec call. */
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

  const [tab, setTab] = useState<TabKey>('status');
  const [observe, setObserve] = useState<ObserveState>({ status: 'idle' });

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

  return (
    <DrawerShell titleId={titleId} onClose={onClose}>
      <div className="sticky top-0 z-10 bg-raise border-b border-line px-5 py-4 flex flex-col gap-3">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0 flex flex-col gap-1.5">
            <h2 id={titleId} className="font-display font-semibold text-[17px] text-fg truncate">
              {session.name ?? c.invalidTrigger}
            </h2>
            <div className="flex items-center gap-1.5 flex-wrap">
              <Chip tone={PHASE_TONE[status.phase]}>{t.phase[status.phase]}</Chip>
              {status.liveness && (
                <Chip tone={status.liveness === 'live' ? 'green' : 'neutral'}>
                  {status.liveness}
                </Chip>
              )}
              {session.session_id && (
                <span className="font-mono text-[10.5px] text-ghost break-all">
                  {session.session_id.slice(0, 8)}
                </span>
              )}
            </div>
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

        <div role="tablist" aria-label={t.tabsAria} className="flex items-center gap-1 flex-wrap">
          {tabs.map(({ key, label }) => (
            <button
              key={key}
              type="button"
              role="tab"
              aria-selected={tab === key}
              onClick={() => setTab(key)}
              className={cn(
                'font-ui font-semibold text-[12.5px] rounded-control px-3 py-1.5 transition-colors cursor-pointer',
                tab === key ? 'bg-raise-2 text-fg border border-line-2' : 'text-dim hover:text-fg border border-transparent'
              )}
            >
              {label}
            </button>
          ))}
        </div>
      </div>

      <div className="px-5 py-4">
        {tab === 'status' && (
          <TabStatus session={session} observe={observe} onLoadObserve={loadObserve} />
        )}
        {tab === 'packages' && <TabPackages session={session} observe={observe} />}
        {tab === 'logs' && <TabLogs session={session} />}
        {tab === 'outcomes' && (
          <TabOutcomes owner={owner} name={name} issue={session.trigger.number} />
        )}
      </div>
    </DrawerShell>
  );
}
