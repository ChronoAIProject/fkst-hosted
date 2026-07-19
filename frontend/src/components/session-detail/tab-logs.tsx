import { useCallback, useEffect, useMemo, useState } from 'react';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { cn } from '@/lib/utils';
import { getLogFile, getLogManifest, DEFAULT_LOG_TAIL_BYTES } from '@/lib/api/logs';
import type { LogFileContent, LogManifest, SessionDetail } from '@/lib/api/types';
import { Chip } from '@/components/ui/chip';
import { Note, NoticeLine, SectionLabel, Spinner } from './parts';

type LoadState = 'idle' | 'loading' | 'error' | 'loaded';

/** Format a byte count as a rounded-KB label for the tail notice. */
function kb(bytes: number): string {
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

/** Split one line around case-insensitive matches of `query`, wrapping each hit
 *  in a <mark>. Only called for lines that actually contain the query, so the
 *  React node count stays bounded to matching lines. */
function highlight(line: string, query: string): React.ReactNode {
  const lower = line.toLowerCase();
  const q = query.toLowerCase();
  const out: React.ReactNode[] = [];
  let from = 0;
  let at = lower.indexOf(q);
  let key = 0;
  while (at !== -1) {
    if (at > from) out.push(line.slice(from, at));
    out.push(
      <mark key={key++} className="bg-[color-mix(in_oklab,var(--amber)_35%,transparent)] text-fg">
        {line.slice(at, at + query.length)}
      </mark>
    );
    from = at + query.length;
    at = lower.indexOf(q, from);
  }
  if (from < line.length) out.push(line.slice(from));
  return out;
}

function LogViewer({ file }: { file: LogFileContent }) {
  const t = useContent().dashboard.detail;
  const [search, setSearch] = useState('');
  const query = search.trim();

  const { lines, matches } = useMemo(() => {
    const split = file.content.split('\n');
    const count = query
      ? file.content.toLowerCase().split(query.toLowerCase()).length - 1
      : 0;
    return { lines: split, matches: count };
  }, [file.content, query]);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2 flex-wrap">
        <input
          type="search"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={t.logsSearchPlaceholder}
          aria-label={t.logsSearchPlaceholder}
          className="flex-1 min-w-[160px] font-mono text-[12px] bg-bg border border-line rounded-control px-2.5 py-1.5 text-fg placeholder:text-ghost focus:outline-none focus:border-line-2"
        />
        {query && (
          <span className="font-mono text-[10.5px] text-ghost flex-none">
            {t.logsSearchCount.replace('{n}', String(matches))}
          </span>
        )}
      </div>
      {file.truncated && (
        <NoticeLine>
          {t.logsTruncated
            .replace('{shown}', kb(file.returned_bytes))
            .replace('{total}', kb(file.total_bytes))}
        </NoticeLine>
      )}
      <pre className="max-h-[46vh] overflow-auto border border-line rounded-card bg-bg p-3 font-mono text-[11.5px] leading-relaxed text-dim whitespace-pre-wrap break-words">
        {lines.map((line, i) => (
          <div key={i}>
            {query && line.toLowerCase().includes(query.toLowerCase())
              ? highlight(line, query)
              : line || ' '}
          </div>
        ))}
      </pre>
    </div>
  );
}

/** Logs tab: fetches the bundle manifest, lists its files (with the classified
 *  label), and renders a selected file's tail in a searchable mono viewer with
 *  a Refresh and a whole-bundle download link. */
export function TabLogs({ session }: { session: SessionDetail }) {
  const t = useContent().dashboard.detail;
  const { apiFetch } = useAuth();
  const sessionId = session.session_id;

  const [manifest, setManifest] = useState<LogManifest | null>(null);
  const [manifestState, setManifestState] = useState<LoadState>('loading');
  const [selected, setSelected] = useState<string | null>(null);
  const [file, setFile] = useState<LogFileContent | null>(null);
  const [fileState, setFileState] = useState<LoadState>('idle');

  useEffect(() => {
    if (!sessionId) {
      setManifestState('idle');
      return;
    }
    let active = true;
    setManifestState('loading');
    getLogManifest(apiFetch, sessionId)
      .then((body) => {
        if (!active) return;
        setManifest(body);
        setManifestState('loaded');
        // Auto-select the first file so the viewer is never blank on open.
        if (body.files.length > 0) setSelected((prev) => prev ?? body.files[0]!.path);
      })
      .catch(() => active && setManifestState('error'));
    return () => {
      active = false;
    };
  }, [apiFetch, sessionId]);

  const loadFile = useCallback(
    (path: string) => {
      if (!sessionId) return;
      setFileState('loading');
      getLogFile(apiFetch, sessionId, path, DEFAULT_LOG_TAIL_BYTES)
        .then((body) => {
          setFile(body);
          setFileState('loaded');
        })
        .catch(() => setFileState('error'));
    },
    [apiFetch, sessionId]
  );

  // Load whenever the selection changes.
  useEffect(() => {
    if (selected) loadFile(selected);
  }, [selected, loadFile]);

  if (!sessionId) return <Note>{t.logsUnavailable}</Note>;
  if (manifestState === 'loading') {
    return (
      <span className="inline-flex items-center gap-2 font-mono text-[11.5px] text-dim">
        <Spinner />
        {t.logsLoading}
      </span>
    );
  }
  if (manifestState === 'error') return <p className="text-[12.5px] text-red">{t.logsError}</p>;
  if (!manifest || manifest.files.length === 0) return <Note>{t.logsEmpty}</Note>;

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-2 flex-wrap">
        <SectionLabel>{t.logsFilesAria}</SectionLabel>
        {session.log_url && (
          <a
            href={session.log_url}
            target="_blank"
            rel="noreferrer"
            className="font-ui font-semibold text-[11.5px] text-amber hover:brightness-[1.1] transition-colors"
          >
            {t.logsDownloadBundle}
          </a>
        )}
      </div>

      <div className="flex flex-wrap gap-1.5" role="tablist" aria-label={t.logsFilesAria}>
        {manifest.files.map((entry) => (
          <button
            key={entry.path}
            type="button"
            role="tab"
            aria-selected={selected === entry.path}
            onClick={() => setSelected(entry.path)}
            title={entry.path}
            className={cn(
              'inline-flex items-center gap-1.5 font-mono text-[11px] border rounded-control px-2.5 py-1 transition-colors cursor-pointer max-w-full',
              selected === entry.path
                ? 'border-line-2 text-fg bg-raise-2'
                : 'border-line text-dim hover:text-fg'
            )}
          >
            <span className="truncate">{entry.path.split('/').pop()}</span>
            <Chip tone="neutral">{entry.label}</Chip>
          </button>
        ))}
      </div>

      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={() => selected && loadFile(selected)}
          disabled={fileState === 'loading' || !selected}
          className="inline-flex items-center gap-1.5 font-ui font-semibold text-[11.5px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg transition-colors cursor-pointer disabled:cursor-default disabled:hover:text-dim"
        >
          {fileState === 'loading' && <Spinner />}
          {t.logsRefresh}
        </button>
      </div>

      {fileState === 'idle' && <Note>{t.logsSelectFile}</Note>}
      {fileState === 'loading' && !file && (
        <span className="inline-flex items-center gap-2 font-mono text-[11.5px] text-dim">
          <Spinner />
          {t.logsFileLoading}
        </span>
      )}
      {fileState === 'error' && <p className="text-[12.5px] text-red">{t.logsFileError}</p>}
      {file && fileState !== 'error' && <LogViewer key={file.path} file={file} />}
    </div>
  );
}
