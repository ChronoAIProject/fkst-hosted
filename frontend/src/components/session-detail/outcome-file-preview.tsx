import { useCallback, useEffect, useRef, useState } from 'react';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { fetchBlob, saveBlob } from '@/lib/api/outcomes';
import type { OutcomeFile } from '@/lib/api/types';
import { Note, Spinner } from './parts';

// The preview never streams a file's bytes on mount. A committed file can be
// large media; auto-fetching on expand pulls the whole thing into memory blind.
// Instead we start in `idle` (an explicit "Load preview" affordance) and only
// fetch when the user asks, so expanding a row is always cheap.
type PreviewState = 'idle' | 'loading' | 'error' | 'tooLarge' | 'ready';

const basename = (path: string): string => path.split('/').pop() ?? path;

const PREVIEWABLE = new Set(['text', 'image', 'video', 'audio']);

/** Inline preview + download for one committed file. Bytes are fetched through
 *  `apiFetch` (single auth path — no token in a media URL) only after an
 *  explicit "Load preview" click: text is read as a string, media becomes an
 *  object URL. The object URL is revoked whenever it is replaced and on unmount
 *  (which the parent's Reveal triggers on collapse), so no blob handle leaks.
 *  A 413 degrades to an "open on GitHub" affordance; a transport/HTTP error
 *  offers Retry. The Download button re-fetches with `download=1`. */
export function OutcomeFilePreview({
  owner,
  name,
  file,
  githubHref,
}: {
  owner: string;
  name: string;
  file: OutcomeFile;
  /** Where "open on GitHub" points when the file is too large to preview. */
  githubHref: string;
}) {
  const t = useContent().dashboard.detail;
  const { apiFetch } = useAuth();
  const previewable = PREVIEWABLE.has(file.kind);

  // Binary (and any unknown non-previewable kind) has nothing to fetch — land
  // directly on `ready`, which renders the "download to view" note.
  const [state, setState] = useState<PreviewState>(previewable ? 'idle' : 'ready');
  const [text, setText] = useState<string | null>(null);
  const [objectUrl, setObjectUrl] = useState<string | null>(null);
  const [downloading, setDownloading] = useState(false);

  // Track the live object URL and mount status outside React state so the async
  // load handler can revoke a stale/orphaned URL even after the component has
  // unmounted mid-fetch (the parent can collapse the row while bytes are still
  // in flight).
  const objectUrlRef = useRef<string | null>(null);
  const mountedRef = useRef(true);

  const setObjectUrlSafe = useCallback((url: string | null) => {
    // Replacing the current URL: revoke the previous handle first.
    if (objectUrlRef.current && objectUrlRef.current !== url) {
      URL.revokeObjectURL(objectUrlRef.current);
    }
    objectUrlRef.current = url;
    setObjectUrl(url);
  }, []);

  const load = useCallback(() => {
    if (!previewable) return;
    setState('loading');
    fetchBlob(apiFetch, owner, name, file.sha, file.filename, false)
      .then(async (res) => {
        if (!res.ok) {
          if (mountedRef.current) setState(res.tooLarge ? 'tooLarge' : 'error');
          return;
        }
        if (file.kind === 'text') {
          const body = await res.blob.text();
          if (mountedRef.current) {
            setText(body);
            setState('ready');
          }
          return;
        }
        // Media: mint the object URL. If the component unmounted while the blob
        // was being read, revoke immediately rather than leaking the handle.
        const url = URL.createObjectURL(res.blob);
        if (!mountedRef.current) {
          URL.revokeObjectURL(url);
          return;
        }
        setObjectUrlSafe(url);
        setState('ready');
      })
      .catch(() => mountedRef.current && setState('error'));
  }, [apiFetch, owner, name, file.sha, file.filename, file.kind, previewable, setObjectUrlSafe]);

  // Revoke any live object URL when the row unmounts (collapse or drawer close).
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      if (objectUrlRef.current) {
        URL.revokeObjectURL(objectUrlRef.current);
        objectUrlRef.current = null;
      }
    };
  }, []);

  const onDownload = async () => {
    if (downloading) return;
    setDownloading(true);
    try {
      const res = await fetchBlob(apiFetch, owner, name, file.sha, file.filename, true);
      if (res.ok) saveBlob(res.blob, basename(file.filename));
      else if (res.tooLarge && mountedRef.current) setState('tooLarge');
    } finally {
      if (mountedRef.current) setDownloading(false);
    }
  };

  const retryClasses =
    'self-start inline-flex items-center gap-1.5 font-ui font-semibold text-[11.5px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg transition-colors cursor-pointer';

  return (
    <div className="flex flex-col gap-2 border-t border-line pt-2 mt-1">
      {state === 'idle' && (
        <button type="button" onClick={load} className={retryClasses}>
          {t.previewLoad}
        </button>
      )}
      {state === 'loading' && (
        <span className="inline-flex items-center gap-2 font-mono text-[11.5px] text-dim">
          <Spinner />
          {t.previewLoading}
        </span>
      )}
      {state === 'error' && (
        <div className="flex items-center gap-2 flex-wrap">
          <p className="text-[12.5px] text-red">{t.previewError}</p>
          <button type="button" onClick={load} className={retryClasses}>
            {t.logsRetry}
          </button>
        </div>
      )}
      {state === 'tooLarge' && (
        <div className="flex flex-col items-start gap-1.5">
          <Note>{t.previewTooLarge}</Note>
          <a
            href={githubHref}
            target="_blank"
            rel="noreferrer"
            className="font-ui font-semibold text-[11.5px] text-amber hover:brightness-[1.1] transition-colors"
          >
            {t.openOnGithub}
          </a>
        </div>
      )}

      {state === 'ready' && file.kind === 'text' && text != null && (
        <pre className="max-h-[40vh] overflow-auto border border-line rounded-card bg-bg p-3 font-mono text-[11.5px] leading-relaxed text-dim whitespace-pre-wrap break-words">
          {text}
        </pre>
      )}
      {state === 'ready' && file.kind === 'image' && objectUrl && (
        <img
          src={objectUrl}
          alt={basename(file.filename)}
          className="max-w-full max-h-[40vh] rounded-card border border-line object-contain"
        />
      )}
      {state === 'ready' && file.kind === 'video' && objectUrl && (
        // eslint-disable-next-line jsx-a11y/media-has-caption -- user-committed media has no caption track
        <video src={objectUrl} controls className="max-w-full max-h-[40vh] rounded-card border border-line" />
      )}
      {state === 'ready' && file.kind === 'audio' && objectUrl && (
        // eslint-disable-next-line jsx-a11y/media-has-caption -- user-committed audio has no caption track
        <audio src={objectUrl} controls className="w-full" />
      )}
      {state === 'ready' && file.kind === 'binary' && <Note>{t.previewBinary}</Note>}

      {/* Download stays available regardless of preview state (it is already a
          deliberate, user-initiated fetch) — only the 413 degrade hides it in
          favor of the open-on-GitHub path. */}
      {state !== 'tooLarge' && (
        <button
          type="button"
          onClick={onDownload}
          disabled={downloading}
          aria-label={t.downloadAria.replace('{name}', basename(file.filename))}
          className="self-start inline-flex items-center gap-1.5 font-ui font-semibold text-[11.5px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg transition-colors cursor-pointer disabled:cursor-default disabled:hover:text-dim"
        >
          {downloading && <Spinner />}
          {t.download}
        </button>
      )}
    </div>
  );
}
