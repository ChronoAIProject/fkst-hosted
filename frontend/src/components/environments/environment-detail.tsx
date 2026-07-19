import { useEffect, useState } from 'react';
import { useLang } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { useToast } from '@/components/ui/toast';
import { formatLocal } from '@/lib/format';
import { cn } from '@/lib/utils';
import { Chip } from '@/components/ui/chip';
import { staggerStyle } from '@/components/ui/motion';
import { ConfirmDialog } from '@/components/modals/confirm-dialog';
import {
  deleteEnvironmentProfile,
  getEnvironmentProfile,
} from '@/lib/api/environments';
import type { EnvironmentProfileView } from '@/lib/api/types';
import type { EnvManagerStrings } from '@/i18n/en/environments';
import { Note, SectionLabel, Spinner, fmt, statusTone } from './environments-drawer';

type DetailState =
  | { status: 'loading' }
  | { status: 'error' }
  | { status: 'loaded'; view: EnvironmentProfileView };

/** A titled block that lists strings, or an empty note when there are none.
 *  `index` drives a staggered entrance so the read-view sections reveal in
 *  sequence (collapses to the final state under reduced motion). */
function ListBlock({
  title,
  empty,
  items,
  index,
}: {
  title: string;
  empty: string;
  items: string[];
  index: number;
}) {
  return (
    <section className="anim-row-in flex flex-col gap-1.5" style={staggerStyle(index)}>
      <SectionLabel>{title}</SectionLabel>
      {items.length === 0 ? (
        <Note>{empty}</Note>
      ) : (
        <ul className="flex flex-col gap-1">
          {items.map((item, i) => (
            <li
              key={`${item}-${i}`}
              className="grad-border font-mono text-[11.5px] text-dim rounded-control px-2.5 py-1.5 break-all shadow-[var(--shadow-1),var(--highlight-top)]"
            >
              {item}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/**
 * Read-only view of one named environment: status, validated time, install
 * commands, non-secret variables, and secret KEY names only (values are never
 * returned by the API and never rendered here). Offers Edit (reopens the editor
 * pre-filled) and a Delete flow guarded by a confirm dialog.
 */
export function EnvironmentDetail({
  t,
  name,
  onEdit,
  onDeleted,
}: {
  t: EnvManagerStrings;
  name: string;
  onEdit: (initial: EnvironmentProfileView) => void;
  onDeleted: () => void;
}) {
  const { lang } = useLang();
  const { apiFetch } = useAuth();
  const toast = useToast();

  const [state, setState] = useState<DetailState>({ status: 'loading' });
  const [confirming, setConfirming] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setState({ status: 'loading' });
    getEnvironmentProfile(apiFetch, name)
      .then((view) => {
        if (!cancelled) setState({ status: 'loaded', view });
      })
      .catch(() => {
        if (!cancelled) setState({ status: 'error' });
      });
    // Guard against a resolved fetch writing state after the view changed.
    return () => {
      cancelled = true;
    };
  }, [apiFetch, name]);

  if (state.status === 'loading') {
    return (
      <div className="flex items-center gap-2">
        <Spinner />
        <Note>{t.detailLoading}</Note>
      </div>
    );
  }

  if (state.status === 'error') {
    return <Note>{t.detailLoadFailed}</Note>;
  }

  const { view } = state;
  const variableLines = Object.entries(view.variables).map(([k, v]) => `${k}=${v}`);
  const validated = view.validated_at
    ? formatLocal(view.validated_at, lang)
    : t.neverValidated;

  return (
    <div className="flex flex-col gap-5">
      <div className="anim-row-in flex items-start justify-between gap-3" style={staggerStyle(0)}>
        <div className="min-w-0 flex flex-col gap-1.5">
          <h3 className="font-mono text-[14px] text-fg break-all">{view.name}</h3>
          <div className="flex items-center gap-1.5 flex-wrap">
            <Chip tone={statusTone(view.status)}>{view.status}</Chip>
          </div>
        </div>
        <div className="flex items-center gap-2 flex-none">
          <button
            type="button"
            onClick={() => onEdit(view)}
            className="font-ui font-semibold text-[12px] border border-line rounded-control px-3 py-1.5 text-dim hover:text-fg hover:border-line-2 hover:shadow-glow-amber transition-[color,border-color,box-shadow] cursor-pointer"
          >
            {t.edit}
          </button>
          <button
            type="button"
            onClick={() => setConfirming(true)}
            className="font-ui font-semibold text-[12px] border border-[color-mix(in_oklab,var(--red)_40%,var(--line))] rounded-control px-3 py-1.5 text-red hover:brightness-[1.1] hover:shadow-glow-red transition-[filter,color,box-shadow] cursor-pointer"
          >
            {t.deleteButton}
          </button>
        </div>
      </div>

      <div
        className="anim-row-in grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 items-baseline"
        style={staggerStyle(1)}
      >
        <span className="font-mono text-eyebrow text-ghost uppercase">{t.validatedLabel}</span>
        <span className="font-mono text-[11.5px] text-dim">{validated}</span>
      </div>

      <ListBlock title={t.installTitle} empty={t.installEmpty} items={view.install} index={2} />
      <ListBlock title={t.variablesTitle} empty={t.variablesEmpty} items={variableLines} index={3} />

      <section className="anim-row-in flex flex-col gap-1.5" style={staggerStyle(4)}>
        <SectionLabel>{t.secretsTitle}</SectionLabel>
        {view.secret_keys.length === 0 ? (
          <Note>{t.secretsEmpty}</Note>
        ) : (
          <>
            <ul className="flex flex-col gap-1">
              {view.secret_keys.map((key) => (
                <li
                  key={key}
                  className={cn(
                    // Secret KEY chip: gradient hairline + a faint amber tint edge
                    // signals a credential; the value stays masked (write-only).
                    'grad-border font-mono text-[11.5px] text-dim rounded-control',
                    'shadow-[var(--shadow-1),var(--highlight-top)]',
                    'px-2.5 py-1.5 break-all flex items-center justify-between gap-2'
                  )}
                >
                  <span>{key}</span>
                  {/* Deliberately no value — secret values are write-only. */}
                  <span aria-hidden="true" className="text-amber/60 tracking-widest">
                    ••••
                  </span>
                </li>
              ))}
            </ul>
            <Note>{t.secretsValueNote}</Note>
          </>
        )}
      </section>

      {confirming && (
        <ConfirmDialog
          title={t.deleteConfirmTitle}
          body={fmt(t.deleteConfirmBody, { name: view.name })}
          confirmLabel={t.deleteConfirm}
          pendingLabel={t.deletePending}
          cancelLabel={t.deleteCancel}
          action={() => deleteEnvironmentProfile(apiFetch, view.name)}
          fallbackError={t.deleteFailed}
          onClose={() => setConfirming(false)}
          onDone={() => {
            setConfirming(false);
            toast.show({ kind: 'success', message: fmt(t.deletedToast, { name: view.name }) });
            onDeleted();
          }}
        />
      )}
    </div>
  );
}
