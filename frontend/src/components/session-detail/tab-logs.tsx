import { useCallback, useEffect, useRef, useState } from 'react';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { cn } from '@/lib/utils';
import { getLogFile, getLogManifest, DEFAULT_LOG_TAIL_BYTES } from '@/lib/api/logs';
import type { LogFileContent, LogManifest, SessionDetail } from '@/lib/api/types';
import { Chip } from '@/components/ui/chip';
import { FadeSwap, StaggerItem } from '@/components/ui/motion';
import { Note, SectionLabel, Spinner } from './parts';
import { LogViewer } from './log-viewer';

type LoadState = 'idle' | 'loading' | 'error' | 'loaded';

/** A single file fetch's options. `full` drops the tail window; `keepOnError`
 *  preserves the last-good content (and flags staleness) instead of wiping it —
 *  used for Refresh / Load-full where a blank screen would be a regression. */
type LoadOpts = { full?: boolean; keepOnError?: boolean };

/** Logs tab: fetches the bundle manifest, lists its files (with the classified
 *  label), and renders a selected file's tail in a searchable mono viewer with
 *  a Refresh, a load-full action, and a whole-bundle download link. */
export function TabLogs({ session }: { session: SessionDetail }) {
  const t = useContent().dashboard.detail;
  const { apiFetch } = useAuth();
  const sessionId = session.session_id;

  const [manifest, setManifest] = useState<LogManifest | null>(null);
  const [manifestState, setManifestState] = useState<LoadState>('loading');
  const [selected, setSelected] = useState<string | null>(null);
  const [file, setFile] = useState<LogFileContent | null>(null);
  const [fileState, setFileState] = useState<LoadState>('idle');
  const [loadingFull, setLoadingFull] = useState(false);
  const [stale, setStale] = useState(false);

  // Monotonic request id: every file fetch captures the value at call time and
  // an out-of-order response drops itself if a newer request has since started
  // (B1 — switching files while a request is in flight must not leave the wrong
  // file's bytes on screen). Also doubles as an unmount guard.
  const reqSeq = useRef(0);
  // Mirror the latest file so the fetch catch handler can read last-good content
  // without a stale closure (runLoad is memoized on apiFetch/sessionId only).
  const fileRef = useRef<LogFileContent | null>(null);
  useEffect(() => {
    fileRef.current = file;
  }, [file]);
  // Mounted flag + monotonic-id bump on unmount so no late response (manifest
  // or file) can setState on a dead component.
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const loadManifest = useCallback(() => {
    if (!sessionId) {
      setManifestState('idle');
      return;
    }
    setManifestState('loading');
    getLogManifest(apiFetch, sessionId)
      .then((body) => {
        if (!mounted.current) return;
        setManifest(body);
        setManifestState('loaded');
        // Auto-select the first file so the viewer is never blank on open.
        if (body.files.length > 0) setSelected((prev) => prev ?? body.files[0]!.path);
      })
      .catch(() => mounted.current && setManifestState('error'));
  }, [apiFetch, sessionId]);

  useEffect(() => {
    loadManifest();
  }, [loadManifest]);

  const runLoad = useCallback(
    (path: string, opts: LoadOpts) => {
      if (!sessionId) return;
      const reqId = ++reqSeq.current;
      if (opts.full) setLoadingFull(true);
      else setFileState('loading');
      const tail = opts.full ? undefined : DEFAULT_LOG_TAIL_BYTES;
      getLogFile(apiFetch, sessionId, path, tail)
        .then((body) => {
          // Drop stale responses: a newer selection/refresh has superseded us,
          // or the component unmounted while this request was in flight.
          if (!mounted.current || reqId !== reqSeq.current) return;
          setFile(body);
          setFileState('loaded');
          setStale(false);
          setLoadingFull(false);
        })
        .catch(() => {
          if (!mounted.current || reqId !== reqSeq.current) return;
          setLoadingFull(false);
          if (opts.keepOnError && fileRef.current) {
            // Refresh / load-full failed but we still have last-good bytes —
            // keep them on screen and flag the content as stale rather than
            // blanking a viewer the user was reading.
            setStale(true);
            setFileState('loaded');
          } else {
            setFileState('error');
          }
        });
    },
    [apiFetch, sessionId]
  );

  // A genuine selection change clears the viewer (loading state) and loads the
  // tail fresh; the cleared content also prevents an old file flashing behind
  // the crossfade.
  useEffect(() => {
    if (!selected) return;
    setFile(null);
    setStale(false);
    runLoad(selected, {});
  }, [selected, runLoad]);

  // Refresh / load-full keep the last-good content on error (see runLoad).
  const refresh = useCallback(() => {
    if (selected) runLoad(selected, { keepOnError: true });
  }, [selected, runLoad]);
  const loadFull = useCallback(() => {
    if (selected) runLoad(selected, { full: true, keepOnError: true });
  }, [selected, runLoad]);

  // Arrow-key roving over the file tablist (Left/Right + Home/End).
  const onTablistKey = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      if (!manifest || manifest.files.length === 0) return;
      const paths = manifest.files.map((f) => f.path);
      const cur = selected ? paths.indexOf(selected) : 0;
      let next = cur;
      if (e.key === 'ArrowRight') next = (cur + 1) % paths.length;
      else if (e.key === 'ArrowLeft') next = (cur - 1 + paths.length) % paths.length;
      else if (e.key === 'Home') next = 0;
      else if (e.key === 'End') next = paths.length - 1;
      else return;
      e.preventDefault();
      const path = paths[next]!;
      setSelected(path);
      // Move focus to the newly selected tab (roving tabindex convention).
      const el = e.currentTarget.querySelector<HTMLButtonElement>(`[data-path="${CSS.escape(path)}"]`);
      el?.focus();
    },
    [manifest, selected]
  );

  if (!sessionId) return <Note>{t.logsUnavailable}</Note>;
  if (manifestState === 'loading') {
    return (
      <span className="inline-flex items-center gap-2 font-mono text-[11.5px] text-dim">
        <Spinner />
        {t.logsLoading}
      </span>
    );
  }
  if (manifestState === 'error') {
    return (
      <div className="flex items-center gap-2 flex-wrap">
        <p className="text-[12.5px] text-red">{t.logsError}</p>
        <button
          type="button"
          onClick={loadManifest}
          className="inline-flex items-center gap-1.5 font-ui font-semibold text-[11.5px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg hover:border-line-2 hover:shadow-glow-amber transition-[color,border-color,box-shadow] duration-150 cursor-pointer"
        >
          {t.logsRetry}
        </button>
      </div>
    );
  }
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
            className="hover-underline font-ui font-semibold text-[11.5px] text-amber hover:brightness-[1.1] transition-[filter] cursor-pointer"
          >
            {t.logsDownloadBundle}
          </a>
        )}
      </div>

      <div
        className="flex flex-wrap gap-1.5"
        role="tablist"
        aria-label={t.logsFilesAria}
        onKeyDown={onTablistKey}
        // Roving tabindex lives on the file tabs; -1 here just makes the
        // container a valid focus target for the delegated arrow-key handler.
        tabIndex={-1}
      >
        {manifest.files.map((entry, i) => {
          const active = selected === entry.path;
          return (
            <StaggerItem key={entry.path} index={i} className="max-w-full">
              <button
                type="button"
                role="tab"
                data-path={entry.path}
                aria-selected={active}
                // Roving tabindex: only the active tab is in the tab order; the
                // rest are reached with the arrow keys handled above.
                tabIndex={active ? 0 : -1}
                onClick={() => setSelected(entry.path)}
                title={entry.path}
                className={cn(
                  'inline-flex items-center gap-1.5 font-mono text-[11px] border rounded-control px-2.5 py-1 transition-[color,border-color,background-color,box-shadow] duration-150 cursor-pointer max-w-full',
                  // Active file: amber-tinted glass surface + a soft amber bloom
                  // so the selected tab reads at a glance; inactive tabs stay quiet
                  // and warm their border + a subtle glow on hover.
                  active
                    ? 'border-[color-mix(in_oklab,var(--amber)_40%,var(--line))] text-fg bg-[color-mix(in_oklab,var(--amber)_12%,var(--raise-2))] shadow-glow-amber'
                    : 'border-line text-dim hover:text-fg hover:border-line-2 hover:shadow-glow-amber'
                )}
              >
                <span className="truncate">{entry.path.split('/').pop()}</span>
                <Chip tone="neutral">{entry.label}</Chip>
              </button>
            </StaggerItem>
          );
        })}
      </div>

      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={refresh}
          disabled={fileState === 'loading' || !selected}
          className="inline-flex items-center gap-1.5 font-ui font-semibold text-[11.5px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg hover:border-line-2 hover:shadow-glow-amber transition-[color,border-color,box-shadow] duration-150 cursor-pointer disabled:cursor-default disabled:hover:text-dim disabled:hover:border-line disabled:hover:shadow-none"
        >
          {fileState === 'loading' && <Spinner />}
          {t.logsRefresh}
        </button>
      </div>

      {fileState === 'idle' && !file && <Note>{t.logsSelectFile}</Note>}
      {fileState === 'loading' && !file && (
        <span className="inline-flex items-center gap-2 font-mono text-[11.5px] text-dim">
          <Spinner />
          {t.logsFileLoading}
        </span>
      )}
      {fileState === 'error' && !file && <p className="text-[12.5px] text-red">{t.logsFileError}</p>}
      {/* Crossfade the viewer when the shown file changes (keyed on path). A
          same-file refresh/load-full keeps the key, so content updates in place. */}
      {file && (
        <FadeSwap k={file.path}>
          <LogViewer
            file={file}
            stale={stale}
            loadingFull={loadingFull}
            onLoadFull={file.truncated ? loadFull : undefined}
          />
        </FadeSwap>
      )}
    </div>
  );
}
