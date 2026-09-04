import { useContent } from '@/i18n';
import { Chip } from '@/components/ui/chip';
import { ScrollArea } from '@/components/ui/scroll-area';
import type { IssueDetail } from '@/lib/api/types';
import { decodeWorkItemStatus, isRetiredWorkItem, type WorkItemTone } from '@/lib/api/derive';
import { Note } from './parts';
import { StatusCard } from './status-charts';
import { WORK_TONE } from './tones';

/** CSS-var accent color for a work-item tone. Container-agnostic so a row's left
 *  rule reads the same whether the row sits in a 1- or 2-column grid. */
const ACCENT: Record<WorkItemTone, string> = {
  good: 'var(--green)',
  progress: 'var(--amber)',
  bad: 'var(--red)',
  neutral: 'var(--ghost)',
};

/** One promoted work-item card: a status-colored left accent + the #number link,
 *  the (truncated) title, and the decoded state chip. Laid out to sit in a
 *  responsive 1-/2-column grid. */
function WorkItemRow({ issue }: { issue: IssueDetail }) {
  const t = useContent().dashboard.detail;
  const decoded = decodeWorkItemStatus(issue);
  return (
    <div className="relative flex items-center gap-2 rounded-chip bg-glass-2 border border-line pl-3.5 pr-2.5 py-2 min-w-0 overflow-hidden shadow-1">
      {/* Status-matched left rule — decorative reinforcement of the chip. */}
      <span
        aria-hidden="true"
        className="absolute left-0 top-0 bottom-0 w-1"
        style={{ background: ACCENT[decoded.tone] }}
      />
      <a
        href={issue.html_url}
        target="_blank"
        rel="noreferrer"
        className="hover-underline font-mono text-[11px] text-ghost hover:text-amber transition-colors flex-none"
      >
        #{issue.number}
      </a>
      <a
        href={issue.html_url}
        target="_blank"
        rel="noreferrer"
        className="hover-underline text-fg text-[12.5px] truncate min-w-0 flex-1 hover:text-amber transition-colors"
      >
        {issue.title}
      </a>
      <Chip tone={WORK_TONE[decoded.tone]}>{t.work[decoded.state]}</Chip>
    </div>
  );
}

/** The session's promoted work items, framed as one pane of the Status tab's
 *  split. It owns its own scroller: laid beside the timeline, a long backlog
 *  must overflow INSIDE this pane rather than growing the row and pushing the
 *  timeline out of view. Rows flow into two columns only when the pane is wide
 *  enough to hold them, which is the stacked (below-`md`) case. */
export function WorkItemsPane({
  issues,
  className,
}: {
  issues: IssueDetail[];
  className?: string;
}) {
  const t = useContent().dashboard.detail;
  const actionableIssues = issues.filter((issue) => !isRetiredWorkItem(issue));
  return (
    <StatusCard
      aria-label={t.workItems}
      className={className}
      label={
        <>
          {t.workItems}
          {actionableIssues.length > 0 && (
            <span className="ml-2 lowercase">· {actionableIssues.length}</span>
          )}
        </>
      }
    >
      {actionableIssues.length === 0 ? (
        <Note>{t.noWorkItems}</Note>
      ) : (
        <ScrollArea className="pr-1 max-h-[18rem] md:max-h-none">
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-1 xl:grid-cols-2 gap-2">
            {actionableIssues.map((issue) => (
              <WorkItemRow key={issue.number} issue={issue} />
            ))}
          </div>
        </ScrollArea>
      )}
    </StatusCard>
  );
}
