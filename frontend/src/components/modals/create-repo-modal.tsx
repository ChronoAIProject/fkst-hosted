import { useState } from 'react';
import type React from 'react';
import { cn } from '@/lib/utils';
import { useAuth } from '@/lib/auth/github-auth';
import { readErrorMessage } from '@/lib/api/canvas';
import type { SiteContent } from '@/i18n';
import { ModalShell } from './modal-shell';
import { ErrorNote } from '@/components/ui/error-note';
import { FIELD_INPUT, FIELD_LABEL } from '@/components/ui/field';
import { useToast } from '@/components/ui/toast';

type ReposContent = SiteContent['dashboard']['repos'];

/** The `<form>` element id, referenced by the sticky-footer submit button so it
 *  can live outside the form subtree yet still submit it. */
const FORM_ID = 'create-repo-form';

/** The wire shape of a repo row from POST /api/v1/repos. */
export interface UserRepo {
  id: number;
  owner: string;
  name: string;
  private: boolean;
  org: boolean;
  admin: boolean;
  installed: boolean;
}

interface CreateRepoBody {
  owner: string | null;
  name: string;
  private: boolean;
  description?: string;
}

/** Client-side mirror of GitHub's allowed repository-name characters. */
const REPO_NAME_RE = /^[A-Za-z0-9._-]+$/;

export function CreateRepoModal({
  viewerLogin,
  orgs,
  rc,
  onClose,
  onCreated,
}: {
  viewerLogin: string;
  orgs: string[];
  rc: ReposContent;
  onClose: () => void;
  onCreated: (repo: UserRepo) => void;
}) {
  const { apiFetch } = useAuth();
  const toast = useToast();
  const [owner, setOwner] = useState(viewerLogin);
  const [name, setName] = useState('');
  const [priv, setPriv] = useState(true);
  const [description, setDescription] = useState('');
  const [creating, setCreating] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);

  const nameValid = REPO_NAME_RE.test(name);

  const onSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!nameValid || creating) return;
    setCreating(true);
    setServerError(null);
    try {
      const body: CreateRepoBody = {
        owner: owner === viewerLogin ? null : owner,
        name,
        private: priv,
      };
      const desc = description.trim();
      if (desc) body.description = desc;
      const res = await apiFetch('/api/v1/repos', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      if (res.ok) {
        const created = (await res.json()) as UserRepo;
        // Confirm the mutation before handing back — the parent closes the
        // dialog on `onCreated`, and previously the create landed silently,
        // leaving the user to infer success from the list poll.
        toast.show({ kind: 'success', message: rc.createdToast });
        onCreated(created);
        return;
      }
      // Error envelope: {"error", "message"} — surface `message` verbatim.
      setServerError((await readErrorMessage(res)) ?? rc.createFailed);
    } catch {
      setServerError(rc.createFailed);
    } finally {
      setCreating(false);
    }
  };

  const footer = (
    <div className="flex items-center justify-end gap-2">
      <button
        type="button"
        onClick={onClose}
        className="font-ui font-semibold text-[12.5px] bg-glass border border-line rounded-control px-4 py-2 text-dim transition-[color,border-color,box-shadow,background] hover:text-fg hover:border-line-2 hover:bg-glass-2 hover:shadow-glow-amber cursor-pointer"
      >
        {rc.cancel}
      </button>
      {/* Primary CTA: the brand amber→gold gradient fill, seated on a soft amber
          glow with a one-shot sheen sweep on mount; hover lifts brightness. */}
      <button
        type="submit"
        form={FORM_ID}
        disabled={!nameValid || creating}
        className={cn(
          'font-ui font-semibold text-[12.5px] rounded-control px-4 py-2 transition-[filter,box-shadow]',
          !nameValid || creating
            ? 'bg-amber/40 text-amber-ink/50 cursor-not-allowed'
            : 'bg-grad-accent text-amber-ink shadow-[var(--shadow-1),var(--glow-amber)] anim-sheen hover:brightness-110 cursor-pointer'
        )}
      >
        {creating ? rc.creating : rc.submit}
      </button>
    </div>
  );

  return (
    <ModalShell
      titleId="create-repo-title"
      title={rc.createTitle}
      onClose={onClose}
      footer={footer}
    >
      <form id={FORM_ID} onSubmit={onSubmit} className="flex flex-col gap-4">
        <div className="flex flex-col gap-1.5">
          <label htmlFor="create-repo-owner" className={FIELD_LABEL}>
            {rc.ownerLabel}
          </label>
          <select
            id="create-repo-owner"
            value={owner}
            onChange={(e) => setOwner(e.target.value)}
            className={cn(FIELD_INPUT, 'cursor-pointer')}
          >
            <option value={viewerLogin}>{rc.ownerPersonal.replace('{login}', viewerLogin)}</option>
            {orgs.map((o) => (
              <option key={o} value={o}>
                {o}
              </option>
            ))}
          </select>
        </div>

        <div className="flex flex-col gap-1.5">
          <label htmlFor="create-repo-name" className={FIELD_LABEL}>
            {rc.nameLabel}
          </label>
          <input
            id="create-repo-name"
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            spellCheck={false}
            autoComplete="off"
            className={cn(FIELD_INPUT, 'font-mono', name !== '' && !nameValid && 'border-red')}
          />
          <p
            className={cn(
              'font-mono text-[11px]',
              name !== '' && !nameValid ? 'text-red' : 'text-ghost'
            )}
          >
            {rc.nameHint}
          </p>
        </div>

        <label className="flex items-center gap-2 text-[13px] text-fg cursor-pointer select-none">
          <input
            type="checkbox"
            checked={priv}
            onChange={(e) => setPriv(e.target.checked)}
            className="w-3.5 h-3.5 accent-amber"
          />
          {rc.privateLabel}
        </label>

        <div className="flex flex-col gap-1.5">
          <label htmlFor="create-repo-description" className={FIELD_LABEL}>
            {rc.descriptionLabel}
          </label>
          <input
            id="create-repo-description"
            type="text"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            autoComplete="off"
            className={FIELD_INPUT}
          />
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
