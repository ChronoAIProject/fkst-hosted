import { useState } from 'react';
import { cn } from '@/lib/utils';
import type { MutationResult } from '@/lib/api/canvas';
import { ModalShell } from './modal-shell';
import { ErrorNote } from '@/components/ui/error-note';

/** Danger confirmation dialog: runs the caller's mutation on confirm. Taking
 *  the API-layer function (not a raw path) keeps every URL construction in
 *  one tested place; on failure the envelope's `message` is shown inside the
 *  dialog, falling back to the caller's generic string. */
export function ConfirmDialog({
  title,
  body,
  confirmLabel,
  pendingLabel,
  cancelLabel,
  action,
  fallbackError,
  onClose,
  onDone,
}: {
  title: string;
  body: string;
  confirmLabel: string;
  pendingLabel: string;
  cancelLabel: string;
  /** The mutation to run, e.g. `() => stopTrigger(apiFetch, owner, name, n)`. */
  action: () => Promise<MutationResult<unknown>>;
  fallbackError: string;
  onClose: () => void;
  onDone: () => void;
}) {
  const [pending, setPending] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);

  const onConfirm = async () => {
    if (pending) return;
    setPending(true);
    setServerError(null);
    try {
      const result = await action();
      if (result.ok) {
        onDone();
        return;
      }
      setServerError(result.message ?? fallbackError);
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
