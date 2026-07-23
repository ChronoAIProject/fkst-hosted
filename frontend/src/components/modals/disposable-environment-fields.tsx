import { cn } from '@/lib/utils';
import { FIELD_INPUT, FIELD_LABEL } from '@/components/ui/field';
import type { DisposableEnvironmentSpec } from '@/lib/api/types';
import type { DashboardContent } from '@/i18n/slices';

export interface EnvironmentKvRow {
  name: string;
  value: string;
}

export interface DisposableEnvironmentDraft {
  install: string[];
  variables: EnvironmentKvRow[];
  secrets: EnvironmentKvRow[];
}

export interface DisposableEnvironmentCounts {
  install: number;
  variables: number;
  secrets: number;
}

export function emptyDisposableEnvironmentDraft(): DisposableEnvironmentDraft {
  return {
    install: [''],
    variables: [{ name: '', value: '' }],
    secrets: [{ name: '', value: '' }],
  };
}

function foldRows(rows: EnvironmentKvRow[], keepEmptyValues: boolean): Record<string, string> {
  const result: Record<string, string> = {};
  for (const row of rows) {
    const name = row.name.trim();
    if (!name || (!keepEmptyValues && row.value === '')) continue;
    result[name] = row.value;
  }
  return result;
}

/** Build the exact write-only API payload. Commands and names are trimmed;
 * variable values are preserved verbatim, while an empty secret value means
 * the row is incomplete and is omitted. */
export function disposableSpecFromDraft(
  draft: DisposableEnvironmentDraft
): DisposableEnvironmentSpec {
  return {
    install: draft.install.map((command) => command.trim()).filter(Boolean),
    variables: foldRows(draft.variables, true),
    secrets: foldRows(draft.secrets, false),
  };
}

export function disposableEnvironmentCounts(
  spec: DisposableEnvironmentSpec
): DisposableEnvironmentCounts {
  return {
    install: spec.install.length,
    variables: Object.keys(spec.variables).length,
    secrets: Object.keys(spec.secrets).length,
  };
}

export function hasDisposableEnvironmentContents(spec: DisposableEnvironmentSpec): boolean {
  const counts = disposableEnvironmentCounts(spec);
  return counts.install + counts.variables + counts.secrets > 0;
}

type CanvasStrings = DashboardContent['canvas'];

export function DisposableEnvironmentFields({
  t,
  draft,
  onChange,
}: {
  t: CanvasStrings;
  draft: DisposableEnvironmentDraft;
  onChange: (draft: DisposableEnvironmentDraft) => void;
}) {
  const setInstallAt = (index: number, value: string) =>
    onChange({
      ...draft,
      install: draft.install.map((row, i) => (i === index ? value : row)),
    });
  const removeInstallAt = (index: number) =>
    onChange({
      ...draft,
      install:
        draft.install.length > 1 ? draft.install.filter((_, i) => i !== index) : draft.install,
    });

  const updateRows = (
    key: 'variables' | 'secrets',
    index: number,
    patch: Partial<EnvironmentKvRow>
  ) =>
    onChange({
      ...draft,
      [key]: draft[key].map((row, i) => (i === index ? { ...row, ...patch } : row)),
    });
  const removeRow = (key: 'variables' | 'secrets', index: number) =>
    onChange({
      ...draft,
      [key]: draft[key].length > 1 ? draft[key].filter((_, i) => i !== index) : draft[key],
    });

  const removeButton = (label: string, onClick: () => void) => (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      title={label}
      className="w-8 h-8 flex-none border border-line rounded-control text-dim hover:text-red hover:border-red/50 transition-colors cursor-pointer"
    >
      ×
    </button>
  );

  return (
    <div className="flex flex-col gap-4 border-l-2 border-l-amber pl-3.5 py-1">
      <p className="font-mono text-[11px] leading-relaxed text-dim">
        {t.createDisposablePrivateNote}
      </p>

      <fieldset className="flex flex-col gap-1.5 border-0 p-0 m-0">
        <legend className={FIELD_LABEL}>{t.createDisposableInstallLabel}</legend>
        {draft.install.map((value, index) => (
          <div key={`disposable-install-${index}`} className="flex items-center gap-2 min-w-0">
            <span className="w-5 flex-none text-right font-mono text-[11px] text-ghost">
              {index + 1}.
            </span>
            <input
              type="text"
              value={value}
              onChange={(event) => setInstallAt(index, event.target.value)}
              placeholder={t.createDisposableInstallPlaceholder}
              aria-label={`${t.createDisposableInstallLabel} ${index + 1}`}
              spellCheck={false}
              autoComplete="off"
              className={cn(FIELD_INPUT, 'font-mono min-w-0')}
            />
            {draft.install.length > 1 &&
              removeButton(t.createDisposableRemoveInstall.replace('{n}', String(index + 1)), () =>
                removeInstallAt(index)
              )}
          </div>
        ))}
        <button
          type="button"
          onClick={() => onChange({ ...draft, install: [...draft.install, ''] })}
          className="self-start font-ui font-semibold text-[11.5px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg hover:border-line-2 transition-colors cursor-pointer"
        >
          {t.createDisposableAddInstall}
        </button>
      </fieldset>

      <fieldset className="flex flex-col gap-1.5 border-0 p-0 m-0">
        <legend className={FIELD_LABEL}>{t.createDisposableVariablesLabel}</legend>
        {draft.variables.map((row, index) => (
          <div
            key={`disposable-variable-${index}`}
            className="grid grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)_auto] gap-2 items-center"
          >
            <input
              type="text"
              value={row.name}
              onChange={(event) => updateRows('variables', index, { name: event.target.value })}
              placeholder={t.createDisposableNamePlaceholder}
              aria-label={`${t.createDisposableVariablesLabel} ${index + 1} ${t.createDisposableNamePlaceholder}`}
              spellCheck={false}
              autoComplete="off"
              className={cn(FIELD_INPUT, 'font-mono min-w-0')}
            />
            <input
              type="text"
              value={row.value}
              onChange={(event) => updateRows('variables', index, { value: event.target.value })}
              placeholder={t.createDisposableValuePlaceholder}
              aria-label={`${t.createDisposableVariablesLabel} ${index + 1} ${t.createDisposableValuePlaceholder}`}
              spellCheck={false}
              autoComplete="off"
              className={cn(FIELD_INPUT, 'font-mono min-w-0')}
            />
            {draft.variables.length > 1 ? (
              removeButton(t.createDisposableRemoveVariable.replace('{n}', String(index + 1)), () =>
                removeRow('variables', index)
              )
            ) : (
              <span className="w-8" aria-hidden="true" />
            )}
          </div>
        ))}
        <button
          type="button"
          onClick={() =>
            onChange({
              ...draft,
              variables: [...draft.variables, { name: '', value: '' }],
            })
          }
          className="self-start font-ui font-semibold text-[11.5px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg hover:border-line-2 transition-colors cursor-pointer"
        >
          {t.createDisposableAddVariable}
        </button>
      </fieldset>

      <fieldset className="flex flex-col gap-1.5 border-0 p-0 m-0">
        <legend className={FIELD_LABEL}>{t.createDisposableSecretsLabel}</legend>
        {draft.secrets.map((row, index) => (
          <div
            key={`disposable-secret-${index}`}
            className="grid grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)_auto] gap-2 items-center"
          >
            <input
              type="text"
              value={row.name}
              onChange={(event) => updateRows('secrets', index, { name: event.target.value })}
              placeholder={t.createDisposableNamePlaceholder}
              aria-label={`${t.createDisposableSecretsLabel} ${index + 1} ${t.createDisposableNamePlaceholder}`}
              spellCheck={false}
              autoComplete="off"
              className={cn(FIELD_INPUT, 'font-mono min-w-0')}
            />
            <input
              type="password"
              value={row.value}
              onChange={(event) => updateRows('secrets', index, { value: event.target.value })}
              placeholder={t.createDisposableSecretPlaceholder}
              aria-label={`${t.createDisposableSecretsLabel} ${index + 1} ${t.createDisposableSecretPlaceholder}`}
              spellCheck={false}
              autoComplete="new-password"
              className={cn(FIELD_INPUT, 'font-mono min-w-0')}
            />
            {draft.secrets.length > 1 ? (
              removeButton(t.createDisposableRemoveSecret.replace('{n}', String(index + 1)), () =>
                removeRow('secrets', index)
              )
            ) : (
              <span className="w-8" aria-hidden="true" />
            )}
          </div>
        ))}
        <button
          type="button"
          onClick={() =>
            onChange({
              ...draft,
              secrets: [...draft.secrets, { name: '', value: '' }],
            })
          }
          className="self-start font-ui font-semibold text-[11.5px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg hover:border-line-2 transition-colors cursor-pointer"
        >
          {t.createDisposableAddSecret}
        </button>
      </fieldset>

      <p className="font-mono text-[11px] leading-relaxed text-ghost">
        {t.createDisposableImmutableNote}
      </p>
    </div>
  );
}
