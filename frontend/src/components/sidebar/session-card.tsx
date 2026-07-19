import { useState } from 'react';
import { cn } from '@/lib/utils';
import { useContent, useLang } from '@/i18n';
import { formatIsoSgt } from '@/lib/format';
import type { IssueDetail, SessionDetail } from '@/lib/api/types';
import { Chip } from '@/components/ui/chip';
import { SessionDetailDrawer } from '@/components/session-detail/session-detail-drawer';

function IssueLine({ issue }: { issue: IssueDetail }) {
  const d = useContent().dashboard;
  const closed = issue.state === 'closed';
  return (
    <div className="flex items-center gap-2 py-1.5 text-[12.5px] min-w-0">
      <span
        className={cn('w-1.5 h-1.5 rounded-full flex-none', closed ? 'bg-ghost' : 'bg-green')}
        aria-hidden="true"
      />
      <a
        href={issue.html_url}
        target="_blank"
        rel="noreferrer"
        className="font-mono text-[11px] text-ghost hover:text-amber transition-colors flex-none"
      >
        #{issue.number}
      </a>
      <span className="text-fg truncate min-w-0 flex-1">{issue.title}</span>
      <span className="font-mono text-[10.5px] text-ghost flex-none">
        {closed ? d.closed : d.open}
      </span>
    </div>
  );
}

function LivenessChip({ liveness }: { liveness: NonNullable<SessionDetail['liveness']> }) {
  const cc = useContent().dashboard.canvas;
  const label = {
    starting: cc.livenessStarting,
    live: cc.livenessLive,
    terminating: cc.livenessTerminating,
  }[liveness];
  return <Chip tone={liveness === 'live' ? 'green' : liveness === 'starting' ? 'amber' : 'neutral'}>{label}</Chip>;
}

/** One session (trigger issue) of the level-2 sidebar: config metadata,
 *  packages, liveness, log download, trigger + work issues, PR outcomes,
 *  and the stop affordance for open triggers. */
export function SessionCard({
  owner,
  name,
  session,
  onStop,
}: {
  /** Repo coordinates from the level-2 context — the detail drawer's fetches
   *  are repo-scoped (outcomes, blobs). */
  owner: string;
  name: string;
  session: SessionDetail;
  /** Open the stop-confirm flow; absent for closed triggers. */
  onStop: (session: SessionDetail) => void;
}) {
  const c = useContent().dashboard;
  const cc = c.canvas;
  const { lang } = useLang();
  const [showDetail, setShowDetail] = useState(false);
  const invalid = !!session.invalid_reason;
  const created = formatIsoSgt(session.trigger.created_at, lang);
  const updated = formatIsoSgt(session.trigger.updated_at, lang);
  const closedAt = formatIsoSgt(session.trigger.closed_at, lang);

  return (
    <div className="border border-line rounded-card bg-bg p-4 flex flex-col gap-3 min-w-0">
      <div className="flex items-start justify-between gap-3 flex-wrap">
        <div className="min-w-0">
          <span className="font-display font-semibold text-[15px] text-fg">
            {invalid ? c.invalidTrigger : (session.name ?? '—')}
          </span>
          {session.session_id && (
            <span className="font-mono text-[10.5px] text-ghost ml-2 break-all">
              {session.session_id.slice(0, 8)}
            </span>
          )}
        </div>
        <div className="flex items-center gap-1.5 flex-wrap">
          {session.liveness && <LivenessChip liveness={session.liveness} />}
          {session.auto_merge && <Chip tone="green">{c.autoMerge}</Chip>}
          {session.status_labels.map((label) => (
            <Chip key={label} tone="amber">
              {label}
            </Chip>
          ))}
          <button
            type="button"
            onClick={() => setShowDetail(true)}
            aria-label={c.detail.openAria.replace('{name}', session.name ?? `#${session.trigger.number}`)}
            className="font-ui font-semibold text-[11px] border border-line rounded-control px-2.5 py-1 text-dim transition-colors hover:text-fg cursor-pointer"
          >
            {c.detail.open}
          </button>
          {session.trigger.state === 'open' && (
            <button
              type="button"
              onClick={() => onStop(session)}
              aria-label={cc.stopAria.replace('{name}', session.name ?? `#${session.trigger.number}`)}
              className="font-ui font-semibold text-[11px] border border-line rounded-control px-2.5 py-1 text-red transition-colors hover:border-[color-mix(in_oklab,var(--red)_45%,var(--line))] cursor-pointer"
            >
              {cc.stop}
            </button>
          )}
        </div>
      </div>

      {invalid ? (
        <p className="text-[12.5px] text-red leading-relaxed">{session.invalid_reason}</p>
      ) : (
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[12px] text-dim">
          {session.work_label && (
            <span>
              {c.workLabel}: <code className="font-mono text-fg">{session.work_label}</code>
            </span>
          )}
          {session.environment && (
            <span>
              {c.environment}: <code className="font-mono text-fg">{session.environment}</code>
            </span>
          )}
          {session.log_url && (
            <a
              href={session.log_url}
              target="_blank"
              rel="noreferrer"
              className="font-ui font-semibold text-[11.5px] text-amber hover:brightness-[1.1] transition-colors"
            >
              {cc.logDownload} ↓
            </a>
          )}
        </div>
      )}

      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-[10.5px] text-ghost">
        {created && (
          <span>
            {cc.createdWord} {created}
          </span>
        )}
        {updated && (
          <span>
            {cc.updatedWord} {updated}
          </span>
        )}
        {closedAt && (
          <span>
            {cc.closedWord} {closedAt}
          </span>
        )}
      </div>

      {session.packages.length > 0 && (
        <div className="flex flex-col gap-1">
          <span className="font-mono text-eyebrow text-ghost uppercase">{c.packages}</span>
          <div className="flex flex-col gap-0.5">
            {session.packages.map((p) => (
              <code key={p} className="font-mono text-[11.5px] text-dim break-all">
                {p}
              </code>
            ))}
          </div>
        </div>
      )}

      <div className="border-t border-line pt-2 flex flex-col">
        <span className="font-mono text-eyebrow text-ghost uppercase mb-1">{c.trigger}</span>
        <IssueLine issue={session.trigger} />

        {session.work_issues.length > 0 && (
          <>
            <span className="font-mono text-eyebrow text-ghost uppercase mt-2 mb-1">
              {c.workIssues} · {session.work_issues.length}
            </span>
            <div className="flex flex-col divide-y divide-[color-mix(in_oklab,var(--line)_55%,transparent)]">
              {session.work_issues.map((issue) => (
                <IssueLine key={issue.number} issue={issue} />
              ))}
            </div>
          </>
        )}

        {session.prs.length > 0 && (
          <>
            <span className="font-mono text-eyebrow text-ghost uppercase mt-2 mb-1">
              {cc.prsTitle} · {session.prs.length}
            </span>
            <div className="flex flex-col divide-y divide-[color-mix(in_oklab,var(--line)_55%,transparent)]">
              {session.prs.map((pr) => (
                <div key={pr.number} className="flex items-center gap-2 py-1.5 text-[12.5px] min-w-0">
                  <a
                    href={pr.html_url}
                    target="_blank"
                    rel="noreferrer"
                    className="font-mono text-[11px] text-ghost hover:text-amber transition-colors flex-none"
                  >
                    #{pr.number}
                  </a>
                  <span className="text-fg truncate min-w-0 flex-1">{pr.title}</span>
                  {pr.work_issue != null && (
                    <span className="font-mono text-[10.5px] text-ghost flex-none">
                      {cc.prForIssue.replace('{n}', String(pr.work_issue))}
                    </span>
                  )}
                  <Chip tone={pr.merged ? 'green' : 'neutral'}>
                    {pr.merged ? cc.prMerged : pr.state === 'open' ? c.open : c.closed}
                  </Chip>
                </div>
              ))}
            </div>
          </>
        )}
      </div>

      {showDetail && (
        <SessionDetailDrawer
          owner={owner}
          name={name}
          session={session}
          onClose={() => setShowDetail(false)}
        />
      )}
    </div>
  );
}
