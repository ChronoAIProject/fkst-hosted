import { useEffect, useState } from 'react';
import type React from 'react';
import { Link } from 'react-router-dom';
import { cn } from '@/lib/utils';
import { useAuth } from '@/lib/auth/github-auth';
import { useContent } from '@/i18n';
import { createTrigger } from '@/lib/api/canvas';
import { listEnvironmentProfiles } from '@/lib/api/environments';
import type { CreateSessionRequest, CreateSessionResponse } from '@/lib/api/types';
import { ModalShell } from './modal-shell';
import { ErrorNote } from '@/components/ui/error-note';
import { FIELD_INPUT, FIELD_LABEL } from '@/components/ui/field';
import { useToast } from '@/components/ui/toast';

/** The `<form>` element id, referenced by the sticky-footer submit button so it
 *  can live outside the form subtree yet still submit it. */
const FORM_ID = 'create-trigger-form';

/** Split a free-text allowlist into entries (whitespace/comma separated). */
function parseAllowlist(raw: string): string[] {
  return raw
    .split(/[\s,]+/)
    .map((s) => s.trim())
    .filter(Boolean);
}

/** Build the POST body from the form state — optional sections are omitted
 *  entirely when blank, mirroring how the backend renders the trigger body. */
export function buildCreateRequest(form: {
  name: string;
  packages: string[];
  workLabel: string;
  environment: string;
  autoMerge: boolean;
  logAccess: string;
  outputLang: string;
}): CreateSessionRequest {
  const request: CreateSessionRequest = {
    name: form.name.trim(),
    packages: form.packages.map((p) => p.trim()).filter(Boolean),
  };
  const workLabel = form.workLabel.trim();
  if (workLabel) request.work_label = workLabel;
  const environment = form.environment.trim();
  if (environment) request.environment = environment;
  if (form.autoMerge) request.auto_merge = true;
  const logAccess = parseAllowlist(form.logAccess);
  if (logAccess.length > 0) request.log_access = logAccess;
  const outputLang = form.outputLang.trim();
  if (outputLang) request.output_lang = outputLang;
  return request;
}

/** Load state for the environment picker. `profiles === null` is "still
 *  loading"; `error` flips the field to the free-text fallback so a failed
 *  fetch never blocks the whole dialog. */
interface EnvLoad {
  profiles: string[] | null;
  error: boolean;
}

/** Environment field: a <select> over the caller's saved profiles (plus a
 *  blank "none"), degrading to a free-text input when the profile list cannot
 *  be loaded — the design closes the parity gap (a session may only reference a
 *  profile that exists) without ever trapping the user behind a failed fetch. */
function EnvironmentField({
  cc,
  value,
  onChange,
  load,
}: {
  cc: ReturnType<typeof useContent>['dashboard']['canvas'];
  value: string;
  onChange: (v: string) => void;
  load: EnvLoad;
}) {
  const label = (
    <label htmlFor="trigger-environment" className={FIELD_LABEL}>
      {cc.createEnvironmentLabel}
    </label>
  );

  // Fetch failed: keep the modal usable with a free-text input + a note.
  if (load.error) {
    return (
      <div className="flex flex-col gap-1.5">
        {label}
        <input
          id="trigger-environment"
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          spellCheck={false}
          autoComplete="off"
          className={cn(FIELD_INPUT, 'font-mono')}
        />
        <p className="font-mono text-[11px] text-ghost">{cc.createEnvironmentLoadFailed}</p>
      </div>
    );
  }

  const profiles = load.profiles ?? [];
  return (
    <div className="flex flex-col gap-1.5">
      {label}
      <select
        id="trigger-environment"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        // Disabled only while the very first fetch is in flight (profiles null).
        disabled={load.profiles === null}
        className={cn(FIELD_INPUT, 'font-mono cursor-pointer')}
      >
        <option value="">{cc.createEnvironmentNone}</option>
        {profiles.map((p) => (
          <option key={p} value={p}>
            {p}
          </option>
        ))}
      </select>
      <p className="font-mono text-[11px] text-ghost">{cc.createEnvironmentNote}</p>
    </div>
  );
}

/** Create-trigger form: session name + ≥1 package rows + the optional knobs.
 *  Server-side validation failures (the trigger parser's 400) are surfaced
 *  verbatim inside the dialog. */
export function CreateTriggerModal({
  owner,
  name,
  onClose,
  onCreated,
}: {
  owner: string;
  name: string;
  onClose: () => void;
  onCreated: (created: CreateSessionResponse) => void;
}) {
  const c = useContent().dashboard;
  const cc = c.canvas;
  const { apiFetch } = useAuth();
  const toast = useToast();
  const [sessionName, setSessionName] = useState('');
  const [packages, setPackages] = useState<string[]>(['']);
  const [workLabel, setWorkLabel] = useState('');
  const [environment, setEnvironment] = useState('');
  const [autoMerge, setAutoMerge] = useState(false);
  const [logAccess, setLogAccess] = useState('');
  const [outputLang, setOutputLang] = useState('');
  const [pending, setPending] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);
  const [envLoad, setEnvLoad] = useState<EnvLoad>({ profiles: null, error: false });

  // Populate the environment picker once on open. A failed fetch is NOT fatal:
  // it flips the field to a free-text fallback rather than blocking the dialog.
  useEffect(() => {
    let alive = true;
    listEnvironmentProfiles(apiFetch)
      .then((summaries) => {
        if (alive) setEnvLoad({ profiles: summaries.map((s) => s.name), error: false });
      })
      .catch(() => {
        if (alive) setEnvLoad({ profiles: null, error: true });
      });
    return () => {
      alive = false;
    };
  }, [apiFetch]);

  const valid = sessionName.trim() !== '' && packages.some((p) => p.trim() !== '');

  const setPackageAt = (i: number, value: string) =>
    setPackages((rows) => rows.map((row, j) => (j === i ? value : row)));
  const removePackageAt = (i: number) =>
    setPackages((rows) => (rows.length > 1 ? rows.filter((_, j) => j !== i) : rows));

  const onSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!valid || pending) return;
    setPending(true);
    setServerError(null);
    try {
      const result = await createTrigger(
        apiFetch,
        owner,
        name,
        buildCreateRequest({
          name: sessionName,
          packages,
          workLabel,
          environment,
          autoMerge,
          logAccess,
          outputLang,
        })
      );
      if (result.ok) {
        toast.show({ kind: 'success', message: cc.createdToast });
        onCreated(result.data);
        return;
      }
      setServerError(result.message ?? cc.createFailed);
    } catch {
      setServerError(cc.createFailed);
    } finally {
      setPending(false);
    }
  };

  const footer = (
    <div className="flex items-center justify-end gap-2">
      <button
        type="button"
        onClick={onClose}
        className="font-ui font-semibold text-[12.5px] border border-line rounded-control px-4 py-2 text-dim hover:text-fg transition-colors cursor-pointer"
      >
        {c.repos.cancel}
      </button>
      <button
        type="submit"
        form={FORM_ID}
        disabled={!valid || pending}
        className={cn(
          'font-ui font-semibold text-[12.5px] rounded-control px-4 py-2 transition-colors',
          !valid || pending
            ? 'bg-amber/50 text-amber-ink/60 cursor-not-allowed'
            : 'bg-amber text-amber-ink hover:brightness-[1.06] cursor-pointer'
        )}
      >
        {pending ? cc.createPending : cc.createSubmit}
      </button>
    </div>
  );

  return (
    <ModalShell
      titleId="create-trigger-title"
      title={cc.createTitle}
      onClose={onClose}
      footer={footer}
    >
      <form id={FORM_ID} onSubmit={onSubmit} className="flex flex-col gap-4">
        <div className="flex flex-col gap-1.5">
          <label htmlFor="trigger-name" className={FIELD_LABEL}>
            {cc.createNameLabel}
          </label>
          <input
            id="trigger-name"
            type="text"
            value={sessionName}
            onChange={(e) => setSessionName(e.target.value)}
            spellCheck={false}
            autoComplete="off"
            className={cn(FIELD_INPUT, 'font-mono')}
          />
          <p className="font-mono text-[11px] text-ghost">{cc.createNameHint}</p>
        </div>

        <fieldset className="flex flex-col gap-1.5 border-0 p-0 m-0">
          <legend className={FIELD_LABEL}>{cc.createPackagesLabel}</legend>
          {packages.map((value, i) => (
            // Index keys are correct here: rows are positional form slots.
            <div key={`package-row-${i}`} className="flex items-center gap-2">
              <input
                type="text"
                value={value}
                onChange={(e) => setPackageAt(i, e.target.value)}
                placeholder={cc.createPackagePlaceholder}
                aria-label={`${cc.createPackagesLabel} ${i + 1}`}
                spellCheck={false}
                autoComplete="off"
                className={cn(FIELD_INPUT, 'font-mono')}
              />
              {packages.length > 1 && (
                <button
                  type="button"
                  onClick={() => removePackageAt(i)}
                  aria-label={cc.removePackageAria.replace('{n}', String(i + 1))}
                  className="font-mono text-[13px] text-dim hover:text-red transition-colors cursor-pointer px-2 py-1 border border-line rounded-control flex-none"
                >
                  ×
                </button>
              )}
            </div>
          ))}
          <button
            type="button"
            onClick={() => setPackages((rows) => [...rows, ''])}
            className="self-start font-ui font-semibold text-[11.5px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg transition-colors cursor-pointer"
          >
            {cc.addPackage}
          </button>
        </fieldset>

        <div className="flex flex-col gap-1.5">
          <label htmlFor="trigger-work-label" className={FIELD_LABEL}>
            {cc.createWorkLabelLabel}
          </label>
          <input
            id="trigger-work-label"
            type="text"
            value={workLabel}
            onChange={(e) => setWorkLabel(e.target.value)}
            spellCheck={false}
            autoComplete="off"
            className={cn(FIELD_INPUT, 'font-mono')}
          />
          <p className="font-mono text-[11px] text-ghost">
            {cc.createWorkLabelHint}{' '}
            <Link
              to="/get-started"
              className="text-dim hover:text-fg underline underline-offset-2 transition-colors"
            >
              {cc.createWorkLabelHintLink}
            </Link>
          </p>
        </div>

        <EnvironmentField cc={cc} value={environment} onChange={setEnvironment} load={envLoad} />

        <label className="flex items-center gap-2 text-[13px] text-fg cursor-pointer select-none">
          <input
            type="checkbox"
            checked={autoMerge}
            onChange={(e) => setAutoMerge(e.target.checked)}
            className="w-3.5 h-3.5 accent-amber"
          />
          {cc.createAutoMergeLabel}
        </label>

        <div className="flex flex-col gap-1.5">
          <label htmlFor="trigger-log-access" className={FIELD_LABEL}>
            {cc.createLogAccessLabel}
          </label>
          <input
            id="trigger-log-access"
            type="text"
            value={logAccess}
            onChange={(e) => setLogAccess(e.target.value)}
            spellCheck={false}
            autoComplete="off"
            className={cn(FIELD_INPUT, 'font-mono')}
          />
          <p className="font-mono text-[11px] text-ghost">{cc.createLogAccessHint}</p>
        </div>

        <div className="flex flex-col gap-1.5">
          <label htmlFor="trigger-output-lang" className={FIELD_LABEL}>
            {cc.createOutputLangLabel}
          </label>
          <input
            id="trigger-output-lang"
            type="text"
            value={outputLang}
            onChange={(e) => setOutputLang(e.target.value)}
            spellCheck={false}
            autoComplete="off"
            className={cn(FIELD_INPUT, 'font-mono')}
          />
          <p className="font-mono text-[11px] text-ghost">{cc.createOutputLangHint}</p>
        </div>

        {serverError && <ErrorNote message={serverError} />}
      </form>
    </ModalShell>
  );
}
