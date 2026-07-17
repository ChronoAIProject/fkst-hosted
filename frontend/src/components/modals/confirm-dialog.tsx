import { useState } from 'react';
import { cn } from '@/lib/utils';
import { useAuth } from '@/lib/auth/github-auth';
import { readErrorMessage } from '@/lib/api/canvas';
import { ModalShell } from './modal-shell';
import { ErrorNote } from '@/components/ui/error-note';

/** Danger confirmation dialog: issues a DELETE to `path` on confirm; on a
 *  non-2xx answer the error envelope's `message` is shown inside the dialog. */
export function ConfirmDialog({
  title,
  body,
  confirmLabel,
  pendingLabel,
  cancelLabel,
  path,
  fallbackError,
  onClose,
  onDone,
}: {
  title: string;
  body: string;
  confirmLabel: string;
  pendingLabel: string;
  cancelLabel: string;
  /** DELETE target, e.g. `/api/v1/installations/{owner}`. */
  path: string;
  fallbackError: string;
  onClose: () => void;
  onDone: () => void;
}) {
  const { apiFetch } = useAuth();
  const [pending, setPending] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);

  const onConfirm = async () => {
    if (pending) return;
    setPending(true);
    setServerError(null);
    try {
      const res = await apiFetch(path, { method: 'DELETE' });
      if (res.ok) {
        onDone();
        return;
      }
      // Error envelope: {"error", "message"} — surface `message` verbatim.
      setServerError((await readErrorMessage(res)) ?? fallbackError);
    } catch {
      setServerError(fallbackError);
    } finally {
      setPending(false);
    }
  };

  return (
    <ModalShell titleId="confirm-dialog-title" title={title} onClose={onClose}>
      <p className="text-[13.5px] leading-relaxed text-dim">{body}</p>

      {serverError && <ErrorNote message={serverError} />}

      <div className="flex items-center justify-end gap-2 pt-1">
        <button
          type="button"
          onClick={onClose}
          className="font-ui font-semibold text-[12.5px] border border-line rounded-control px-4 py-2 text-dim hover:text-fg transition-colors cursor-pointer"
        >
          {cancelLabel}
        </button>
        <button
          type="button"
          onClick={onConfirm}
          disabled={pending}
          className={cn(
            'font-ui font-semibold text-[12.5px] rounded-control px-4 py-2 transition-colors',
            pending
              ? 'bg-red/50 text-white/60 cursor-not-allowed'
              : 'bg-red text-white hover:brightness-[1.06] cursor-pointer'
          )}
        >
          {pending ? pendingLabel : confirmLabel}
        </button>
      </div>
    </ModalShell>
  );
}
