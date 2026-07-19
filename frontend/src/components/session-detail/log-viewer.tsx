import { useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { useContent } from '@/i18n';
import type { LogFileContent } from '@/lib/api/types';
import { CopyButton } from '@/components/ui/copy-button';
import { NoticeLine, Spinner } from './parts';

/** Format a byte count as a rounded-KB label for the tail notice. */
function kb(bytes: number): string {
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

/** Split one line around case-insensitive matches of `query`, wrapping each hit
 *  in a <mark>. Only called for lines that actually contain the query, so the
 *  React node count stays bounded to matching lines. */
function highlight(line: string, query: string): ReactNode {
  const lower = line.toLowerCase();
  const q = query.toLowerCase();
  const out: ReactNode[] = [];
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

/** Debounce keeps large-file highlighting off the keystroke path: the split /
 *  match / re-render only runs after typing settles. */
const SEARCH_DEBOUNCE_MS = 180;

export interface LogViewerProps {
  file: LogFileContent;
  /** Fetch the whole file (drops the tail window). Present only when there is a
   *  fuller file to load — i.e. the current content is a truncated tail. */
  onLoadFull?: () => void;
  /** A full-file fetch is in flight (drives the load-full affordance's spinner). */
  loadingFull?: boolean;
  /** The shown content is the last-good copy after a failed refresh. */
  stale?: boolean;
}

/**
 * Renders one log file's text in a searchable mono viewer. Extracted from
 * `TabLogs` so the tab owns only fetch/selection orchestration and this owns
 * the <pre> rendering + in-file find. Search is debounced and, because it only
 * ever covers the fetched tail, its count copy is truncation-aware: on a tail
 * it says so and points at the load-full action rather than implying the count
 * is over the whole file.
 */
export function LogViewer({ file, onLoadFull, loadingFull, stale }: LogViewerProps) {
  const t = useContent().dashboard.detail;
  const [search, setSearch] = useState('');
  const [debounced, setDebounced] = useState('');

  // Reset the query when the shown file changes so a stale term never carries
  // its highlights onto a different file's bytes.
  useEffect(() => {
    setSearch('');
    setDebounced('');
  }, [file.path]);

  useEffect(() => {
    const trimmed = search.trim();
    if (trimmed === debounced) return;
    const id = window.setTimeout(() => setDebounced(trimmed), SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(id);
  }, [search, debounced]);

  const query = debounced;
  const { lines, matches } = useMemo(() => {
    const split = file.content.split('\n');
    const count = query
      ? file.content.toLowerCase().split(query.toLowerCase()).length - 1
      : 0;
    return { lines: split, matches: count };
  }, [file.content, query]);

  // Truncated tails caveat the count and point at load-full; a whole file does
  // not, so the reader can trust the number covers everything shown.
  const countLabel = file.truncated
    ? t.logsSearchCountTail.replace('{n}', String(matches))
    : t.logsSearchCount.replace('{n}', String(matches));

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-2 flex-wrap">
        <code className="font-mono text-[11px] text-dim truncate min-w-0">{file.path}</code>
        <CopyButton value={file.path} label={t.logsFilenameCopy} />
      </div>

      {stale && <NoticeLine>{t.logsStale}</NoticeLine>}

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
          <span className="font-mono text-[10.5px] text-ghost flex-none">{countLabel}</span>
        )}
      </div>

      {file.truncated && (
        <NoticeLine>
          <span className="flex items-center gap-2 flex-wrap">
            <span>
              {t.logsTruncated
                .replace('{shown}', kb(file.returned_bytes))
                .replace('{total}', kb(file.total_bytes))}
            </span>
            {onLoadFull && (
              <button
                type="button"
                onClick={onLoadFull}
                disabled={loadingFull}
                className="inline-flex items-center gap-1.5 font-ui font-semibold text-[11px] text-amber hover:brightness-[1.1] transition-colors cursor-pointer disabled:cursor-default disabled:opacity-70"
              >
                {loadingFull && <Spinner />}
                {loadingFull ? t.logsLoadingFull : t.logsLoadFull}
              </button>
            )}
          </span>
        </NoticeLine>
      )}

      <pre className="max-h-[46vh] overflow-auto border border-line rounded-card bg-bg p-3 font-mono text-[11.5px] leading-relaxed text-dim whitespace-pre-wrap break-words">
        {lines.map((line, i) => (
          <div key={i}>
            {query && line.toLowerCase().includes(query.toLowerCase())
              ? highlight(line, query)
              : line || ' '}
          </div>
        ))}
      </pre>
    </div>
  );
}
