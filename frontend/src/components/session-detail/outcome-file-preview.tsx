import { useEffect, useState } from 'react';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { fetchBlob, saveBlob } from '@/lib/api/outcomes';
import type { OutcomeFile } from '@/lib/api/types';
import { Note, Spinner } from './parts';

type PreviewState = 'loading' | 'error' | 'tooLarge' | 'ready';

const basename = (path: string): string => path.split('/').pop() ?? path;

const PREVIEWABLE = new Set(['text', 'image', 'video', 'audio']);

/** Inline preview + download for one committed file. Bytes are fetched through
 *  `apiFetch` (single auth path — no token in a media URL): text is read as a
 *  string, media becomes an object URL (revoked on unmount). A 413 degrades to
 *  an "open on GitHub" affordance. The Download button re-fetches with
 *  `download=1` and saves the same bytes. */
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

  const [state, setState] = useState<PreviewState>(previewable ? 'loading' : 'ready');
  const [text, setText] = useState<string | null>(null);
  const [objectUrl, setObjectUrl] = useState<string | null>(null);
  const [downloading, setDownloading] = useState(false);

  useEffect(() => {
    if (!previewable) return;
    let active = true;
    let createdUrl: string | null = null;
    setState('loading');
    fetchBlob(apiFetch, owner, name, file.sha, file.filename, false)
      .then(async (res) => {
        if (!active) return;
        if (!res.ok) {
          setState(res.tooLarge ? 'tooLarge' : 'error');
          return;
        }
        if (file.kind === 'text') {
          setText(await res.blob.text());
        } else {
          createdUrl = URL.createObjectURL(res.blob);
          setObjectUrl(createdUrl);
        }
        if (active) setState('ready');
      })
      .catch(() => active && setState('error'));
    return () => {
      active = false;
      // Revoke the object URL this effect created so no handle leaks.
      if (createdUrl) URL.revokeObjectURL(createdUrl);
    };
  }, [apiFetch, owner, name, file.sha, file.filename, file.kind, previewable]);

  const onDownload = async () => {
    if (downloading) return;
    setDownloading(true);
    try {
      const res = await fetchBlob(apiFetch, owner, name, file.sha, file.filename, true);
      if (res.ok) saveBlob(res.blob, basename(file.filename));
      else if (res.tooLarge) setState('tooLarge');
    } finally {
      setDownloading(false);
    }
  };

  return (
    <div className="flex flex-col gap-2 border-t border-line pt-2 mt-1">
      {state === 'loading' && (
        <span className="inline-flex items-center gap-2 font-mono text-[11.5px] text-dim">
          <Spinner />
          {t.previewLoading}
        </span>
      )}
      {state === 'error' && <p className="text-[12.5px] text-red">{t.previewError}</p>}
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
