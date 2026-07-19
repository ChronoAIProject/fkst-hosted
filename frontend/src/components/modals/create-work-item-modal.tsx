import { useState } from 'react';
import type React from 'react';
import { cn } from '@/lib/utils';
import { useAuth } from '@/lib/auth/github-auth';
import { useContent } from '@/i18n';
import { createWorkItem } from '@/lib/api/canvas';
import type { CreateWorkItemRequest, CreateWorkItemResponse } from '@/lib/api/canvas';
import { ModalShell } from './modal-shell';
import { ErrorNote } from '@/components/ui/error-note';
import { FIELD_INPUT, FIELD_LABEL } from '@/components/ui/field';
import { useToast } from '@/components/ui/toast';

/** The `<form>` element id, referenced by the sticky-footer submit button so it
 *  can live outside the form subtree yet still submit it. */
const FORM_ID = 'create-work-item-form';

/** Build the POST body from the form state — a blank body is omitted entirely,
 *  mirroring how the backend treats an absent `body`. Exported for unit tests. */
export function buildWorkItemRequest(form: { title: string; body: string }): CreateWorkItemRequest {
  const request: CreateWorkItemRequest = { title: form.title.trim() };
  const body = form.body.trim();
  if (body) request.body = body;
  return request;
}

/** Queue-work-item dialog: a title + optional body that opens a new issue
 *  pre-stamped with the session's work label. Server-side validation failures
 *  (400/422) are surfaced verbatim inside the dialog. */
export function CreateWorkItemModal({
  owner,
  name,
  triggerIssue,
  workLabel,
  onClose,
  onCreated,
}: {
  owner: string;
  name: string;
  triggerIssue: number;
  /** The session's work label — shown in the note so the user knows which queue
   *  the item joins. */
  workLabel: string;
  onClose: () => void;
  onCreated: (created: CreateWorkItemResponse) => void;
}) {
  const c = useContent().dashboard;
  const cc = c.canvas;
  const { apiFetch } = useAuth();
  const toast = useToast();
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [pending, setPending] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);

  const valid = title.trim() !== '';

  const onSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!valid || pending) return;
    setPending(true);
    setServerError(null);
    try {
      const result = await createWorkItem(
        apiFetch,
        owner,
        name,
        triggerIssue,
        buildWorkItemRequest({ title, body })
      );
      if (result.ok) {
        toast.show({ kind: 'success', message: cc.workItemCreatedToast });
        onCreated(result.data);
        return;
      }
      setServerError(result.message ?? cc.workItemFailed);
    } catch {
      setServerError(cc.workItemFailed);
    } finally {
      setPending(false);
    }
  };

  const footer = (
    <div className="flex items-center justify-end gap-2">
      <button
        type="button"
        onClick={onClose}
        className="font-ui font-semibold text-[12.5px] bg-glass border border-line rounded-control px-4 py-2 text-dim transition-[color,border-color,box-shadow,background] hover:text-fg hover:border-line-2 hover:bg-glass-2 hover:shadow-glow-amber cursor-pointer"
      >
        {c.repos.cancel}
      </button>
      {/* Primary CTA: the brand amber→gold gradient fill, seated on a soft amber
          glow with a one-shot sheen sweep on mount; hover lifts brightness. */}
      <button
        type="submit"
        form={FORM_ID}
        disabled={!valid || pending}
        className={cn(
          'font-ui font-semibold text-[12.5px] rounded-control px-4 py-2 transition-[filter,box-shadow]',
          !valid || pending
            ? 'bg-amber/40 text-amber-ink/50 cursor-not-allowed'
            : 'bg-grad-accent text-amber-ink shadow-[var(--shadow-1),var(--glow-amber)] anim-sheen hover:brightness-110 cursor-pointer'
        )}
      >
        {pending ? cc.workItemPending : cc.workItemSubmit}
      </button>
    </div>
  );

  return (
    <ModalShell
      titleId="create-work-item-title"
      title={cc.workItemTitle}
      onClose={onClose}
      footer={footer}
    >
      <form id={FORM_ID} onSubmit={onSubmit} className="flex flex-col gap-4">
        <p className="font-mono text-[11px] text-ghost">
          {cc.workItemLabelNote.replace('{label}', workLabel)}
        </p>

        <div className="flex flex-col gap-1.5">
          <label htmlFor="work-item-title" className={FIELD_LABEL}>
            {cc.workItemTitleLabel}
          </label>
          <input
            id="work-item-title"
            type="text"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            spellCheck={false}
            autoComplete="off"
            className={cn(FIELD_INPUT, 'font-mono')}
          />
          <p className="font-mono text-[11px] text-ghost">{cc.workItemTitleHint}</p>
        </div>

        <div className="flex flex-col gap-1.5">
          <label htmlFor="work-item-body" className={FIELD_LABEL}>
            {cc.workItemBodyLabel}
          </label>
          <textarea
            id="work-item-body"
            value={body}
            onChange={(e) => setBody(e.target.value)}
            spellCheck={false}
            rows={5}
            className={cn(FIELD_INPUT, 'font-mono resize-y')}
          />
          <p className="font-mono text-[11px] text-ghost">{cc.workItemBodyHint}</p>
        </div>

        {/* Keyed on the message so a fresh error re-triggers the entrance: the
            note rises + fades in (`.anim-notice-in`, disabled under reduced
            motion) rather than popping. */}
        {serverError && (
          <div key={serverError} className="anim-notice-in">
            <ErrorNote message={serverError} />
          </div>
        )}
      </form>
    </ModalShell>
  );
}
