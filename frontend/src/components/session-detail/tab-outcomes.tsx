import { useEffect, useState } from 'react';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { cn } from '@/lib/utils';
import { getSessionOutcomes } from '@/lib/api/outcomes';
import type { OutcomeFile, PrOutcome, SessionOutcomes } from '@/lib/api/types';
import { Chip } from '@/components/ui/chip';
import { Note, SectionLabel, Spinner } from './parts';
import { OutcomeFilePreview } from './outcome-file-preview';

type LoadState = 'loading' | 'error' | 'loaded';

const STATUS_TONE: Record<string, 'neutral' | 'amber' | 'green' | 'red'> = {
  added: 'green',
  modified: 'amber',
  removed: 'red',
  renamed: 'neutral',
};

function FileRow({
  owner,
  name,
  file,
  githubHref,
  expanded,
  onToggle,
}: {
  owner: string;
  name: string;
  file: OutcomeFile;
  githubHref: string;
  expanded: boolean;
  onToggle: () => void;
}) {
  const t = useContent().dashboard.detail;
  const known = file.status in t.fileStatus ? (file.status as keyof typeof t.fileStatus) : null;
  const statusLabel = known ? t.fileStatus[known] : file.status;

  return (
    <div className="flex flex-col">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={expanded}
        className="flex items-center gap-2 py-1.5 text-[12.5px] min-w-0 text-left cursor-pointer group"
      >
        <span
          aria-hidden="true"
          className={cn('font-mono text-[10px] text-ghost flex-none transition-transform', expanded && 'rotate-90')}
        >
          ▸
        </span>
        <span className="font-mono text-[11.5px] text-fg truncate min-w-0 flex-1 group-hover:text-amber transition-colors">
          {file.filename}
        </span>
        {file.additions > 0 && (
          <span
            className="font-mono text-[10.5px] text-green flex-none"
            aria-label={t.additionsAria.replace('{n}', String(file.additions))}
          >
            +{file.additions}
          </span>
        )}
        {file.deletions > 0 && (
          <span
            className="font-mono text-[10.5px] text-red flex-none"
            aria-label={t.deletionsAria.replace('{n}', String(file.deletions))}
          >
            -{file.deletions}
          </span>
        )}
        <Chip tone={STATUS_TONE[file.status] ?? 'neutral'}>{statusLabel}</Chip>
      </button>
      {file.previous_filename && (
        <span className="font-mono text-[10px] text-ghost pl-5 -mt-1 pb-1 truncate">
          {t.renamedFrom.replace('{from}', file.previous_filename)}
        </span>
      )}
      {expanded && (
        <OutcomeFilePreview owner={owner} name={name} file={file} githubHref={githubHref} />
      )}
    </div>
  );
}

function PrBlock({
  owner,
  name,
  pr,
  expandedKey,
  onToggle,
}: {
  owner: string;
  name: string;
  pr: PrOutcome;
  expandedKey: string | null;
  onToggle: (key: string) => void;
}) {
  const d = useContent().dashboard;
  const t = d.detail;
  const cc = d.canvas;
  const filesHref = `${pr.html_url}/files`;

  return (
    <div className="border border-line rounded-card bg-bg p-3 flex flex-col gap-2 min-w-0">
      <div className="flex items-center gap-2 min-w-0">
        <a
          href={pr.html_url}
          target="_blank"
          rel="noreferrer"
          className="font-mono text-[11px] text-ghost hover:text-amber transition-colors flex-none"
        >
          #{pr.number}
        </a>
        <span className="text-fg text-[12.5px] truncate min-w-0 flex-1">{pr.title}</span>
        {pr.work_issue != null && (
          <span className="font-mono text-[10.5px] text-ghost flex-none">
            {cc.prForIssue.replace('{n}', String(pr.work_issue))}
          </span>
        )}
        <Chip tone={pr.merged ? 'green' : 'neutral'}>
          {pr.merged ? cc.prMerged : pr.state === 'open' ? d.open : d.closed}
        </Chip>
      </div>

      {pr.files_error ? (
        <p className="text-[12px] text-red">{t.outcomesFilesError}</p>
      ) : pr.files.length === 0 ? (
        <Note>{t.outcomesNoFiles}</Note>
      ) : (
        <div className="divide-y divide-[color-mix(in_oklab,var(--line)_55%,transparent)]">
          {pr.files.map((file) => {
            const key = `${pr.number}:${file.filename}`;
            return (
              <FileRow
                key={key}
                owner={owner}
                name={name}
                file={file}
                githubHref={filesHref}
                expanded={expandedKey === key}
                onToggle={() => onToggle(key)}
              />
            );
          })}
        </div>
      )}
    </div>
  );
}

/** Outcomes tab: the session's devloop PRs, each with its committed files.
 *  Clicking a file expands an inline preview (single expansion at a time to
 *  avoid many concurrent media fetches). */
export function TabOutcomes({
  owner,
  name,
  issue,
}: {
  owner: string;
  name: string;
  /** The trigger issue number that identifies the session in-repo. */
  issue: number;
}) {
  const d = useContent().dashboard;
  const t = d.detail;
  const { apiFetch } = useAuth();

  const [outcomes, setOutcomes] = useState<SessionOutcomes | null>(null);
  const [state, setState] = useState<LoadState>('loading');
  const [expandedKey, setExpandedKey] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    setState('loading');
    getSessionOutcomes(apiFetch, owner, name, issue)
      .then((body) => {
        if (!active) return;
        setOutcomes(body);
        setState('loaded');
      })
      .catch(() => active && setState('error'));
    return () => {
      active = false;
    };
  }, [apiFetch, owner, name, issue]);

  if (state === 'loading') {
    return (
      <span className="inline-flex items-center gap-2 font-mono text-[11.5px] text-dim">
        <Spinner />
        {t.outcomesLoading}
      </span>
    );
  }
  if (state === 'error') return <p className="text-[12.5px] text-red">{t.outcomesError}</p>;
  if (!outcomes || outcomes.prs.length === 0) return <Note>{t.outcomesEmpty}</Note>;

  return (
    <div className="flex flex-col gap-3">
      <SectionLabel>
        {d.canvas.prsTitle}
        <span className="ml-2 lowercase">· {outcomes.prs.length}</span>
      </SectionLabel>
      {outcomes.prs.map((pr) => (
        <PrBlock
          key={pr.number}
          owner={owner}
          name={name}
          pr={pr}
          expandedKey={expandedKey}
          onToggle={(key) => setExpandedKey((prev) => (prev === key ? null : key))}
        />
      ))}
    </div>
  );
}
