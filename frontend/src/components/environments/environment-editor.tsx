import { useMemo, useState } from 'react';
import type React from 'react';
import { cn } from '@/lib/utils';
import { useAuth } from '@/lib/auth/github-auth';
import { useToast } from '@/components/ui/toast';
import { FIELD_INPUT, FIELD_LABEL } from '@/components/ui/field';
import { ErrorNote } from '@/components/ui/error-note';
import { putEnvironmentProfile } from '@/lib/api/environments';
import {
  ENV_MAX_NAME_LEN,
  ENV_NAME_REGEX,
  type EnvironmentProfileSpec,
  type EnvironmentProfileView,
  type InstallValidationError,
} from '@/lib/api/types';
import type { EnvManagerStrings } from '@/i18n/en/environments';
import { Note, SectionLabel, Spinner, fmt } from './environments-drawer';

/** A NAME/value pair row in the variables/secrets editors. */
interface KvRow {
  name: string;
  value: string;
}

/** Seed the KV editor from an existing map (edit mode) or a single blank row. */
function kvRowsFrom(map: Record<string, string>): KvRow[] {
  const rows = Object.entries(map).map(([name, value]) => ({ name, value }));
  return rows.length > 0 ? rows : [{ name: '', value: '' }];
}

/** Seed secret rows from existing KEY names (values are never returned, so they
 *  start blank) or a single blank row. */
function secretRowsFrom(keys: string[]): KvRow[] {
  const rows = keys.map((name) => ({ name, value: '' }));
  return rows.length > 0 ? rows : [{ name: '', value: '' }];
}

/** Fold KV rows into a wire map. `keepEmptyValues` distinguishes variables
 *  (empty value is a legitimate value) from secrets (a blank value means "not
 *  provided", so it is dropped — since PUT replaces, a blank secret is removed). */
function foldKv(rows: KvRow[], keepEmptyValues: boolean): Record<string, string> {
  const out: Record<string, string> = {};
  for (const row of rows) {
    const name = row.name.trim();
    if (!name) continue;
    if (!keepEmptyValues && row.value === '') continue;
    out[name] = row.value;
  }
  return out;
}

/** Inline mono report of an install-validation `422` (nothing was persisted). */
function ValidationReport({
  t,
  error,
}: {
  t: EnvManagerStrings;
  error: InstallValidationError;
}) {
  const rows: Array<[string, string]> = [
    [t.validationIndex, String(error.failed_command_index)],
    [t.validationCommand, error.failed_command || '—'],
    [t.validationExitCode, String(error.exit_code)],
    [t.validationTimedOut, error.timed_out ? t.yes : t.no],
  ];
  return (
    // Elevated failure surface: translucent glass with a red left-edge accent,
    // lifted on card depth + a soft red glow so a failed install validation reads
    // as a distinct, raised alert. Enters on the shared row-in curve.
    <div className="anim-row-in border border-line border-l-2 border-l-red rounded-card bg-glass backdrop-blur-glass px-3.5 py-3 flex flex-col gap-2.5 shadow-[var(--shadow-2),var(--glow-red)]">
      <span className="font-mono text-eyebrow text-red uppercase tracking-[0.14em]">
        {t.validationTitle}
      </span>
      <p className="font-mono text-[11.5px] text-dim break-words">{error.message}</p>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="font-mono text-[11px] text-ghost">{label}</dt>
            <dd className="font-mono text-[11px] text-dim break-all">{value}</dd>
          </div>
        ))}
      </dl>
      <div className="flex flex-col gap-1">
        <SectionLabel>{t.validationStderr}</SectionLabel>
        <pre className="font-mono text-[11px] text-dim bg-bg border border-[color-mix(in_oklab,var(--red)_25%,var(--line))] rounded-control px-2.5 py-2 overflow-x-auto whitespace-pre-wrap break-words shadow-highlight-top">
          {error.stderr_tail || '—'}
        </pre>
      </div>
    </div>
  );
}

/**
 * Create or edit one named environment. The name is validated client-side
 * against the backend's regex/length before the (slow) PUT; on save the backend
 * runs the install commands in a throwaway pod, so a spinner + note stand in for
 * the round-trip, a `422` renders the detailed install-validation report inline,
 * and success raises a toast and returns to the list.
 */
export function EnvironmentEditor({
  t,
  initial,
  onCancel,
  onSaved,
}: {
  t: EnvManagerStrings;
  /** Present in edit mode; absent = create mode. */
  initial?: EnvironmentProfileView;
  onCancel: () => void;
  onSaved: () => void;
}) {
  const { apiFetch } = useAuth();
  const toast = useToast();
  const editing = initial !== undefined;

  const [name, setName] = useState(initial?.name ?? '');
  const [install, setInstall] = useState<string[]>(
    initial && initial.install.length > 0 ? initial.install : ['']
  );
  const [variables, setVariables] = useState<KvRow[]>(
    initial ? kvRowsFrom(initial.variables) : [{ name: '', value: '' }]
  );
  const [secrets, setSecrets] = useState<KvRow[]>(
    initial ? secretRowsFrom(initial.secret_keys) : [{ name: '', value: '' }]
  );

  const [pending, setPending] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);
  const [validation, setValidation] = useState<InstallValidationError | null>(null);

  // Client-side name validation mirrors the backend's rules so a bad name never
  // costs a slow round-trip. The message is only shown once the user has typed.
  const nameError = useMemo(() => {
    const trimmed = name.trim();
    if (trimmed.length === 0) return null; // required-ness gates the button, not a message
    if (trimmed.length > ENV_MAX_NAME_LEN) return fmt(t.nameErrorLength, { max: ENV_MAX_NAME_LEN });
    if (!ENV_NAME_REGEX.test(trimmed)) return t.nameErrorFormat;
    return null;
  }, [name, t]);

  const nameValid = name.trim().length > 0 && nameError === null;
  const canSave = nameValid && !pending;

  // --- ordered install-command row helpers ---
  const setInstallAt = (i: number, value: string) =>
    setInstall((rows) => rows.map((row, j) => (j === i ? value : row)));
  const removeInstallAt = (i: number) =>
    setInstall((rows) => (rows.length > 1 ? rows.filter((_, j) => j !== i) : rows));

  // --- generic KV row helpers (variables + secrets share them) ---
  const setKv = (
    setter: React.Dispatch<React.SetStateAction<KvRow[]>>,
    i: number,
    patch: Partial<KvRow>
  ) => setter((rows) => rows.map((row, j) => (j === i ? { ...row, ...patch } : row)));
  const removeKv = (setter: React.Dispatch<React.SetStateAction<KvRow[]>>, i: number) =>
    setter((rows) => (rows.length > 1 ? rows.filter((_, j) => j !== i) : rows));

  const onSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSave) return;
    setPending(true);
    setServerError(null);
    setValidation(null);

    const spec: EnvironmentProfileSpec = {
      install: install.map((c) => c.trim()).filter(Boolean),
      variables: foldKv(variables, true),
      secrets: foldKv(secrets, false),
    };

    try {
      const result = await putEnvironmentProfile(apiFetch, name.trim(), spec);
      if (result.ok) {
        toast.show({ kind: 'success', message: fmt(t.savedToast, { name: name.trim() }) });
        onSaved();
        return;
      }
      if ('validation' in result) {
        setValidation(result.validation);
        return;
      }
      setServerError(result.message ?? t.saveFailed);
    } catch {
      // Network/parse failure — the API layer only rejects on unexpected throws.
      setServerError(t.saveFailed);
    } finally {
      setPending(false);
    }
  };

  return (
    <form onSubmit={onSubmit} className="flex flex-col gap-5">
      <h3 className="grad-text-fg font-display font-semibold text-display-sm">
        {editing ? t.editorEditTitle : t.editorCreateTitle}
      </h3>

      {/* Name */}
      <div className="flex flex-col gap-1.5">
        <label htmlFor="env-name" className={FIELD_LABEL}>
          {t.nameLabel}
        </label>
        <input
          id="env-name"
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder={t.namePlaceholder}
          spellCheck={false}
          autoComplete="off"
          // The name is the object identity; PUT to a different name would create
          // a second environment, so it is locked once the env exists.
          disabled={editing}
          className={cn(FIELD_INPUT, 'font-mono', editing && 'opacity-60 cursor-not-allowed')}
        />
        {nameError ? (
          <p className="font-mono text-[11px] text-red">{nameError}</p>
        ) : (
          <p className="font-mono text-[11px] text-ghost">
            {editing ? t.nameLockedHint : t.nameHint}
          </p>
        )}
      </div>

      {/* Install commands (ordered) */}
      <fieldset className="flex flex-col gap-1.5 border-0 p-0 m-0">
        <legend className={FIELD_LABEL}>{t.installLegend}</legend>
        {install.map((value, i) => (
          <div key={`install-row-${i}`} className="flex items-center gap-2">
            <span className="font-mono text-[11px] text-ghost w-5 flex-none text-right">
              {i + 1}.
            </span>
            <input
              type="text"
              value={value}
              onChange={(e) => setInstallAt(i, e.target.value)}
              placeholder={t.installPlaceholder}
              aria-label={`${t.installLegend} ${i + 1}`}
              spellCheck={false}
              autoComplete="off"
              className={cn(FIELD_INPUT, 'font-mono')}
            />
            {install.length > 1 && (
              <button
                type="button"
                onClick={() => removeInstallAt(i)}
                aria-label={fmt(t.removeInstallAria, { n: i + 1 })}
                className="font-mono text-[13px] text-dim hover:text-red hover:border-[color-mix(in_oklab,var(--red)_45%,var(--line))] hover:shadow-glow-red transition-[color,border-color,box-shadow] cursor-pointer px-2 py-1 border border-line rounded-control flex-none"
              >
                ×
              </button>
            )}
          </div>
        ))}
        <button
          type="button"
          onClick={() => setInstall((rows) => [...rows, ''])}
          className="self-start font-ui font-semibold text-[11.5px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg hover:border-line-2 hover:shadow-glow-amber transition-[color,border-color,box-shadow] cursor-pointer"
        >
          {t.addInstall}
        </button>
        <Note>{t.installHint}</Note>
      </fieldset>

      {/* Variables */}
      <fieldset className="flex flex-col gap-1.5 border-0 p-0 m-0">
        <legend className={FIELD_LABEL}>{t.variablesLegend}</legend>
        {variables.map((row, i) => (
          <div key={`var-row-${i}`} className="flex items-center gap-2">
            <input
              type="text"
              value={row.name}
              onChange={(e) => setKv(setVariables, i, { name: e.target.value })}
              placeholder={t.variableNamePlaceholder}
              aria-label={`${t.variablesLegend} ${i + 1} ${t.variableNamePlaceholder}`}
              spellCheck={false}
              autoComplete="off"
              className={cn(FIELD_INPUT, 'font-mono w-2/5 flex-none')}
            />
            <input
              type="text"
              value={row.value}
              onChange={(e) => setKv(setVariables, i, { value: e.target.value })}
              placeholder={t.variableValuePlaceholder}
              aria-label={`${t.variablesLegend} ${i + 1} ${t.variableValuePlaceholder}`}
              spellCheck={false}
              autoComplete="off"
              className={cn(FIELD_INPUT, 'font-mono')}
            />
            {variables.length > 1 && (
              <button
                type="button"
                onClick={() => removeKv(setVariables, i)}
                aria-label={fmt(t.removeVariableAria, { n: i + 1 })}
                className="font-mono text-[13px] text-dim hover:text-red hover:border-[color-mix(in_oklab,var(--red)_45%,var(--line))] hover:shadow-glow-red transition-[color,border-color,box-shadow] cursor-pointer px-2 py-1 border border-line rounded-control flex-none"
              >
                ×
              </button>
            )}
          </div>
        ))}
        <button
          type="button"
          onClick={() => setVariables((rows) => [...rows, { name: '', value: '' }])}
          className="self-start font-ui font-semibold text-[11.5px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg hover:border-line-2 hover:shadow-glow-amber transition-[color,border-color,box-shadow] cursor-pointer"
        >
          {t.addVariable}
        </button>
      </fieldset>

      {/* Secrets (write-only) */}
      <fieldset className="flex flex-col gap-1.5 border-0 p-0 m-0">
        <legend className={FIELD_LABEL}>{t.secretsLegend}</legend>
        {secrets.map((row, i) => (
          <div key={`secret-row-${i}`} className="flex items-center gap-2">
            <input
              type="text"
              value={row.name}
              onChange={(e) => setKv(setSecrets, i, { name: e.target.value })}
              placeholder={t.secretNamePlaceholder}
              aria-label={`${t.secretsLegend} ${i + 1} ${t.secretNamePlaceholder}`}
              spellCheck={false}
              autoComplete="off"
              className={cn(FIELD_INPUT, 'font-mono w-2/5 flex-none')}
            />
            <input
              // Write-only: masked input, never pre-filled with a real value.
              type="password"
              value={row.value}
              onChange={(e) => setKv(setSecrets, i, { value: e.target.value })}
              placeholder={t.secretValuePlaceholder}
              aria-label={`${t.secretsLegend} ${i + 1} ${t.secretValuePlaceholder}`}
              spellCheck={false}
              autoComplete="new-password"
              className={cn(FIELD_INPUT, 'font-mono')}
            />
            {secrets.length > 1 && (
              <button
                type="button"
                onClick={() => removeKv(setSecrets, i)}
                aria-label={fmt(t.removeSecretAria, { n: i + 1 })}
                className="font-mono text-[13px] text-dim hover:text-red hover:border-[color-mix(in_oklab,var(--red)_45%,var(--line))] hover:shadow-glow-red transition-[color,border-color,box-shadow] cursor-pointer px-2 py-1 border border-line rounded-control flex-none"
              >
                ×
              </button>
            )}
          </div>
        ))}
        <button
          type="button"
          onClick={() => setSecrets((rows) => [...rows, { name: '', value: '' }])}
          className="self-start font-ui font-semibold text-[11.5px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg hover:border-line-2 hover:shadow-glow-amber transition-[color,border-color,box-shadow] cursor-pointer"
        >
          {t.addSecret}
        </button>
        <Note>{editing ? t.secretsEditHint : t.secretsHint}</Note>
      </fieldset>

      {validation && <ValidationReport t={t} error={validation} />}
      {serverError && <ErrorNote message={serverError} />}

      {pending && (
        // The PUT runs the install commands in a throwaway pod — a slow round-trip.
        // A glass pill with a breathing amber glow signals live, in-progress work.
        <div className="anim-glow-pulse anim-row-in flex items-center gap-2.5 rounded-card bg-glass backdrop-blur-glass border border-[color-mix(in_oklab,var(--amber)_28%,var(--line))] px-3.5 py-2.5">
          <Spinner />
          <Note>{t.validatingNote}</Note>
        </div>
      )}

      <div className="flex items-center justify-end gap-2 pt-1">
        <button
          type="button"
          onClick={onCancel}
          disabled={pending}
          className={cn(
            'font-ui font-semibold text-[12.5px] border border-line rounded-control px-4 py-2 text-dim transition-[color,border-color,box-shadow]',
            pending ? 'opacity-60 cursor-not-allowed' : 'hover:text-fg hover:border-line-2 hover:shadow-glow-amber cursor-pointer'
          )}
        >
          {t.cancel}
        </button>
        <button
          type="submit"
          disabled={!canSave}
          className={cn(
            'font-ui font-semibold text-[12.5px] rounded-control px-4 py-2 transition-[filter,box-shadow,background-color]',
            !canSave
              ? 'bg-amber/50 text-amber-ink/60 cursor-not-allowed'
              : // Primary CTA: brand gradient fill + amber bloom, brightening on hover.
                'anim-sheen bg-grad-accent text-amber-ink shadow-[var(--shadow-2),var(--glow-amber)] hover:brightness-110 cursor-pointer'
          )}
        >
          {pending ? t.saving : t.save}
        </button>
      </div>
    </form>
  );
}
