import { useState } from 'react';
import { useContent } from '@/i18n';
import type { RepoSessionsResponse, SessionDetail } from '@/lib/api/types';
import { ConfirmDialog } from '@/components/modals/confirm-dialog';
import { CreateTriggerModal } from '@/components/modals/create-trigger-modal';
import { StatusLegend, ViewDescription } from './legend';
import { SessionCard } from './session-card';

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
  const [showCreate, setShowCreate] = useState(false);
  const [stopTarget, setStopTarget] = useState<SessionDetail | null>(null);

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

      <p className="font-mono text-[10.5px] text-ghost">{cc.pollNote}</p>

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
        <div className="border border-line border-l-2 border-l-red rounded-card bg-[color-mix(in_oklab,var(--raise)_55%,transparent)] px-4 py-3 text-[13px] text-dim">
          {cc.sessionsLoadFailed}
        </div>
      ) : data != null && data.sessions.length === 0 ? (
        <p className="font-mono text-[12px] text-ghost italic">{c.noSessions}</p>
      ) : data != null ? (
        <div className="flex flex-col gap-3">
          {data.sessions.map((session, i) => (
            <SessionCard
              key={session.session_id ?? `trigger-${session.trigger.number}-${i}`}
              session={session}
              onStop={setStopTarget}
            />
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
          path={`/api/v1/repos/${encodeURIComponent(owner)}/${encodeURIComponent(name)}/sessions/${stopTarget.trigger.number}`}
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
