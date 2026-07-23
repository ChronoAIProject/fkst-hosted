import { cn } from '@/lib/utils';
import { ErrorNote } from '@/components/ui/error-note';
import type { DashboardContent } from '@/i18n/slices';
import type { DisposableEnvironmentCounts } from './disposable-environment-fields';
import { ModalShell } from './modal-shell';

export function DisposableEnvironmentConfirm({
  t,
  counts,
  pending,
  serverError,
  onBack,
  onConfirm,
}: {
  t: DashboardContent['canvas'];
  counts: DisposableEnvironmentCounts;
  pending: boolean;
  serverError: string | null;
  onBack: () => void;
  onConfirm: () => void;
}) {
  return (
    <ModalShell
      titleId="disposable-environment-confirm-title"
      title={t.createDisposableConfirmTitle}
      onClose={onBack}
    >
      <div className="flex flex-col gap-4">
        <p className="text-[13.5px] leading-relaxed text-dim">{t.createDisposableConfirmBody}</p>

        <dl className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-4 gap-y-2 border-y border-line py-3">
          {[
            [t.createDisposableConfirmInstall, counts.install],
            [t.createDisposableConfirmVariables, counts.variables],
            [t.createDisposableConfirmSecrets, counts.secrets],
          ].map(([label, count]) => (
            <div key={String(label)} className="contents">
              <dt className="font-mono text-[11.5px] text-dim">{label}</dt>
              <dd className="font-mono text-[12px] text-fg tabular-nums text-right">{count}</dd>
            </div>
          ))}
        </dl>

        <p className="border border-line border-l-2 border-l-amber rounded-card bg-glass px-3 py-2.5 font-mono text-[11.5px] leading-relaxed text-dim">
          {t.createDisposableConfirmWarning}
        </p>

        {serverError && (
          <div key={serverError} className="anim-notice-in">
            <ErrorNote message={serverError} />
          </div>
        )}

        <div className="flex items-center justify-end gap-2 pt-1">
          <button
            type="button"
            onClick={onBack}
            disabled={pending}
            className="font-ui font-semibold text-[12.5px] bg-glass border border-line rounded-control px-4 py-2 text-dim hover:text-fg hover:border-line-2 transition-colors cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {t.createDisposableConfirmBack}
          </button>
          <button
            type="button"
            onClick={onConfirm}
            disabled={pending}
            className={cn(
              'font-ui font-semibold text-[12.5px] rounded-control px-4 py-2 transition-[filter,box-shadow]',
              pending
                ? 'bg-amber/40 text-amber-ink/50 cursor-not-allowed'
                : 'bg-grad-accent text-amber-ink shadow-[var(--shadow-1),var(--glow-amber)] anim-sheen hover:brightness-110 cursor-pointer'
            )}
          >
            {pending ? t.createDisposableConfirmPending : t.createDisposableConfirmSubmit}
          </button>
        </div>
      </div>
    </ModalShell>
  );
}
