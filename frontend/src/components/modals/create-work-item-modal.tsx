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
import { MarkdownPreview } from '@/components/ui/markdown-preview';
import { useToast } from '@/components/ui/toast';

/** The `<form>` element id, referenced by the sticky-footer submit button so it
 *  can live outside the form subtree yet still submit it. */
const FORM_ID = 'create-work-item-form';

/** Build the POST body from the form state — a blank body is omitted entirely,
 *  mirroring how the backend treats an absent `body`. Exported for unit tests. */
export function buildWorkItemRequest(form: {
  title: string;
  body: string;
  workLabel: string;
}): CreateWorkItemRequest {
  const request: CreateWorkItemRequest = {
    title: form.title.trim(),
    label: form.workLabel,
  };
  // Preserve populated Markdown verbatim: leading indentation and trailing
  // newlines can be meaningful. Whitespace-only content is still omitted.
  if (form.body.trim()) request.body = form.body;
  return request;
}

/** Queue-work-item dialog: a title + optional body that opens a new issue
 *  pre-stamped with the session's work label. Server-side validation failures
 *  (400/422) are surfaced verbatim inside the dialog. */
export function CreateWorkItemModal({
  owner,
  name,
  triggerIssue,
  creator,
  workLabels,
  onClose,
  onCreated,
}: {
  owner: string;
  name: string;
  triggerIssue: number;
  /** Effective session creator; the backend assigns the new issue to this login. */
  creator: string;
  /** Every label that can wake this session. */
  workLabels: readonly string[];
  onClose: () => void;
  onCreated: (created: CreateWorkItemResponse) => void;
}) {
  const c = useContent().dashboard;
  const cc = c.canvas;
  const { apiFetch } = useAuth();
  const toast = useToast();
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [bodyMode, setBodyMode] = useState<'write' | 'preview'>('write');
  const [workLabel, setWorkLabel] = useState(workLabels[0] ?? '');
  const [pending, setPending] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);

  const valid = title.trim() !== '' && workLabel !== '';

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
        buildWorkItemRequest({ title, body, workLabel })
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
      {/* Primary CTA: the brand accent-gradient fill, seated on a soft accent
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
          {workLabels.length > 1 ? (
            <>
              <label htmlFor="work-item-label" className={FIELD_LABEL}>
                {cc.workItemLabelLabel}
              </label>
              <select
                id="work-item-label"
                value={workLabel}
                onChange={(e) => setWorkLabel(e.target.value)}
                className={cn(FIELD_INPUT, 'font-mono cursor-pointer')}
              >
                {workLabels.map((label) => (
                  <option key={label} value={label}>
                    {label}
                  </option>
                ))}
              </select>
            </>
          ) : (
            <>
              <span className={FIELD_LABEL}>{cc.workItemLabelLabel}</span>
              <p className="font-mono text-[12px] text-fg break-all">{workLabel}</p>
            </>
          )}
          <p className="font-mono text-[11px] text-ghost">
            {cc.workItemLabelNote
              .replace('{label}', workLabel)
              .replace('{creator}', creator)}
          </p>
        </div>

        <div className="flex flex-col gap-1.5">
          <div className="flex items-center justify-between gap-3">
            <span id="work-item-body-label" className={FIELD_LABEL}>
              {cc.workItemBodyLabel}
            </span>
            <div
              role="group"
              aria-label={cc.workItemBodyModeAria}
              className="glass inline-flex flex-none items-center gap-1 rounded-control border border-line p-1"
            >
              {(['write', 'preview'] as const).map((mode) => {
                const active = bodyMode === mode;
                return (
                  <button
                    key={mode}
                    type="button"
                    aria-pressed={active}
                    onClick={() => setBodyMode(mode)}
                    className={cn(
                      'min-w-16 rounded-control px-2.5 py-1 font-ui text-[11px] font-semibold transition-[color,background-color,box-shadow] cursor-pointer',
                      active
                        ? 'bg-glass-2 text-amber shadow-[var(--shadow-1)]'
                        : 'text-ghost hover:text-fg'
                    )}
                  >
                    {mode === 'write' ? cc.workItemWrite : cc.workItemPreview}
                  </button>
                );
              })}
            </div>
          </div>
          {bodyMode === 'write' ? (
            <textarea
              id="work-item-body"
              aria-labelledby="work-item-body-label"
              value={body}
              onChange={(e) => setBody(e.target.value)}
              spellCheck={false}
              rows={5}
              className={cn(FIELD_INPUT, 'font-mono resize-y')}
            />
          ) : body.trim() ? (
            <MarkdownPreview markdown={body} ariaLabel={cc.workItemPreviewAria} />
          ) : (
            <div
              role="region"
              aria-label={cc.workItemPreviewAria}
              className="min-h-[132px] rounded-control border border-line bg-glass px-3 py-2.5 font-mono text-[12px] text-ghost"
            >
              {cc.workItemPreviewEmpty}
            </div>
          )}
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
