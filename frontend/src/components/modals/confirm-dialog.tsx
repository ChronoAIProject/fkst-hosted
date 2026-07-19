import { useState } from 'react';
import { cn } from '@/lib/utils';
import type { MutationResult } from '@/lib/api/canvas';
import { ModalShell } from './modal-shell';
import { ErrorNote } from '@/components/ui/error-note';
import { useToast } from '@/components/ui/toast';

/** Danger confirmation dialog: runs the caller's mutation on confirm. Taking
 *  the API-layer function (not a raw path) keeps every URL construction in
 *  one tested place; on failure the envelope's `message` is shown inside the
 *  dialog, falling back to the caller's generic string.
 *
 *  On success, when the caller supplies `successMessage`, a success toast is
 *  raised before `onDone` — so a destructive action (stop a session, uninstall
 *  the App) confirms itself rather than silently closing and leaving the user
 *  to infer the outcome from a list refetch. Callers that already toast their
 *  own outcome in `onDone` simply omit the prop, so no double notice fires. */
export function ConfirmDialog({
  title,
  body,
  confirmLabel,
  pendingLabel,
  cancelLabel,
  action,
  fallbackError,
  successMessage,
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
  /** Optional already-localized success notice; raised as a toast on success.
   *  Omit it to keep the silent close (or to toast from `onDone` instead). */
  successMessage?: string;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const [pending, setPending] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);

  const onConfirm = async () => {
    if (pending) return;
    setPending(true);
    setServerError(null);
    try {
      const result = await action();
      if (result.ok) {
        // `show` rejects an empty message on its own, so an undefined/blank
        // successMessage simply raises nothing.
        if (successMessage) toast.show({ kind: 'success', message: successMessage });
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

      {/* Keyed on the message so a fresh error re-triggers the entrance: the
          note rises + fades in (`.anim-notice-in`) rather than popping. The
          class is disabled under prefers-reduced-motion, leaving it in place. */}
      {serverError && (
        <div key={serverError} className="anim-notice-in">
          <ErrorNote message={serverError} />
        </div>
      )}

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
