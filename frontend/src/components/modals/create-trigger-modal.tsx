import { useEffect, useRef, useState } from 'react';
import type React from 'react';
import { Link } from 'react-router-dom';
import { cn } from '@/lib/utils';
import { useAuth } from '@/lib/auth/github-auth';
import { useContent } from '@/i18n';
import { createTrigger } from '@/lib/api/canvas';
import { listEnvironmentProfiles } from '@/lib/api/environments';
import type {
  CreateSessionRequest,
  CreateSessionResponse,
  DisposableEnvironmentSpec,
} from '@/lib/api/types';
import { validateOptionalBranchName } from '@/lib/branch';
import { ModalShell } from './modal-shell';
import { ErrorNote } from '@/components/ui/error-note';
import { FIELD_INPUT, FIELD_LABEL } from '@/components/ui/field';
import { useToast } from '@/components/ui/toast';
import {
  DisposableEnvironmentFields,
  disposableEnvironmentCounts,
  disposableSpecFromDraft,
  emptyDisposableEnvironmentDraft,
  hasDisposableEnvironmentContents,
} from './disposable-environment-fields';
import { DisposableEnvironmentConfirm } from './disposable-environment-confirm';

/** The `<form>` element id, referenced by the sticky-footer submit button so it
 *  can live outside the form subtree yet still submit it. */
const FORM_ID = 'create-trigger-form';

type EnvironmentMode = 'none' | 'saved' | 'disposable';

/** Split a free-text allowlist into entries (whitespace/comma separated). */
function parseAllowlist(raw: string): string[] {
  return raw
    .split(/[\s,]+/)
    .map((s) => s.trim())
    .filter(Boolean);
}

/** Split a free-text manifest textarea into references — one per line, trimmed,
 *  blanks dropped. Each surviving line is sent as its own `### Manifest` entry
 *  (a line carrying more than one ref fails the backend's round-trip check). */
function parseLines(raw: string): string[] {
  return raw
    .split(/\r?\n/)
    .map((s) => s.trim())
    .filter(Boolean);
}

/** Build the POST body from the form state — optional sections are omitted
 *  entirely when blank, mirroring how the backend renders the trigger body. */
export function buildCreateRequest(form: {
  name: string;
  packages: string[];
  manifests: string;
  workLabel: string;
  environment: string;
  disposableEnvironment?: DisposableEnvironmentSpec;
  sourceBranch: string;
  targetBranch: string;
  autoMerge: boolean;
  logAccess: string;
  collaborators: string;
  outputLang: string;
}): CreateSessionRequest {
  const request: CreateSessionRequest = {
    name: form.name.trim(),
    packages: form.packages.map((p) => p.trim()).filter(Boolean),
  };
  const manifests = parseLines(form.manifests);
  if (manifests.length > 0) request.manifests = manifests;
  const workLabel = form.workLabel.trim();
  if (workLabel) request.work_label = workLabel;
  if (form.disposableEnvironment) {
    request.disposable_environment = form.disposableEnvironment;
  } else {
    const environment = form.environment.trim();
    if (environment) request.environment = environment;
  }
  const sourceBranch = form.sourceBranch.trim();
  if (sourceBranch) request.source_branch = sourceBranch;
  const targetBranch = form.targetBranch.trim();
  if (targetBranch) request.target_branch = targetBranch;
  if (form.autoMerge) request.auto_merge = true;
  const logAccess = parseAllowlist(form.logAccess);
  if (logAccess.length > 0) request.log_access = logAccess;
  const collaborators = parseAllowlist(form.collaborators);
  if (collaborators.length > 0) request.collaborators = collaborators;
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
      {cc.createSavedEnvironmentLabel}
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

function EnvironmentSection({
  cc,
  mode,
  onModeChange,
  savedEnvironment,
  onSavedEnvironmentChange,
  load,
  disposableDraft,
  onDisposableDraftChange,
  disposableHasContents,
}: {
  cc: ReturnType<typeof useContent>['dashboard']['canvas'];
  mode: EnvironmentMode;
  onModeChange: (mode: EnvironmentMode) => void;
  savedEnvironment: string;
  onSavedEnvironmentChange: (value: string) => void;
  load: EnvLoad;
  disposableDraft: ReturnType<typeof emptyDisposableEnvironmentDraft>;
  onDisposableDraftChange: (draft: ReturnType<typeof emptyDisposableEnvironmentDraft>) => void;
  disposableHasContents: boolean;
}) {
  const modes: Array<[EnvironmentMode, string]> = [
    ['none', cc.createEnvironmentNone],
    ['saved', cc.createEnvironmentSaved],
    ['disposable', cc.createEnvironmentDisposable],
  ];

  return (
    <section className="flex flex-col gap-3" aria-labelledby="trigger-environment-mode-label">
      <span id="trigger-environment-mode-label" className={FIELD_LABEL}>
        {cc.createEnvironmentLabel}
      </span>
      <div
        role="group"
        aria-label={cc.createEnvironmentLabel}
        className="grid grid-cols-3 border border-line rounded-control overflow-hidden"
      >
        {modes.map(([value, label]) => {
          const selected = mode === value;
          return (
            <button
              key={value}
              type="button"
              aria-pressed={selected}
              onClick={() => onModeChange(value)}
              className={cn(
                'min-h-9 px-2 py-1.5 font-ui font-semibold text-[11.5px] border-r border-line last:border-r-0 transition-[color,background,border-color]',
                selected
                  ? 'bg-amber text-amber-ink'
                  : 'bg-glass text-dim hover:text-fg hover:bg-glass-2'
              )}
            >
              {label}
            </button>
          );
        })}
      </div>

      {mode === 'saved' && (
        <EnvironmentField
          cc={cc}
          value={savedEnvironment}
          onChange={onSavedEnvironmentChange}
          load={load}
        />
      )}
      {mode === 'disposable' && (
        <>
          <DisposableEnvironmentFields
            t={cc}
            draft={disposableDraft}
            onChange={onDisposableDraftChange}
          />
          {!disposableHasContents && (
            <p className="font-mono text-[11px] text-amber" role="status">
              {cc.createDisposableEmpty}
            </p>
          )}
        </>
      )}
    </section>
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
  inUseWorkLabels = [],
}: {
  owner: string;
  name: string;
  onClose: () => void;
  onCreated: (created: CreateSessionResponse) => void;
  /** Work labels already claimed by an OPEN session on this repo. A typed label
   *  that exactly matches one of these is flagged early and blocks submit — an
   *  advisory that mirrors, but does not replace, the backend pre-flight 409. */
  inUseWorkLabels?: readonly string[];
}) {
  const c = useContent().dashboard;
  const cc = c.canvas;
  const { apiFetch } = useAuth();
  const toast = useToast();
  const [sessionName, setSessionName] = useState('');
  const [packages, setPackages] = useState<string[]>(['']);
  const [manifests, setManifests] = useState('');
  const [workLabel, setWorkLabel] = useState('');
  const [environmentMode, setEnvironmentMode] = useState<EnvironmentMode>('none');
  const [environment, setEnvironment] = useState('');
  const [disposableDraft, setDisposableDraft] = useState(emptyDisposableEnvironmentDraft);
  const [sourceBranch, setSourceBranch] = useState('');
  const [targetBranch, setTargetBranch] = useState('');
  const [autoMerge, setAutoMerge] = useState(false);
  const [logAccess, setLogAccess] = useState('');
  const [collaborators, setCollaborators] = useState('');
  const [outputLang, setOutputLang] = useState('');
  const [pending, setPending] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);
  const [envLoad, setEnvLoad] = useState<EnvLoad>({ profiles: null, error: false });
  const [confirmationRequest, setConfirmationRequest] = useState<CreateSessionRequest | null>(null);
  const requestInFlight = useRef(false);

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

  // A session needs SOME package source: at least one explicit package OR a
  // manifest reference (a manifest supplies the packages — epic #594 I7). The
  // work label stays optional and never gates submit.
  const hasPackageSource =
    packages.some((p) => p.trim() !== '') || parseLines(manifests).length > 0;
  const disposableSpec = disposableSpecFromDraft(disposableDraft);
  const disposableHasContents = hasDisposableEnvironmentContents(disposableSpec);
  const environmentValid = environmentMode !== 'disposable' || disposableHasContents;
  const valid = sessionName.trim() !== '' && hasPackageSource && environmentValid;

  // Early collision advisory: the trimmed work label exactly matches one an open
  // session on this repo already claims. This blocks submit client-side, but the
  // backend pre-flight (409) stays authoritative — a label freed between poll
  // and submit is still caught server-side, and the server error is shown below.
  const trimmedWorkLabel = workLabel.trim();
  const workLabelCollision = trimmedWorkLabel !== '' && inUseWorkLabels.includes(trimmedWorkLabel);
  const sourceBranchError = validateOptionalBranchName(sourceBranch);
  const targetBranchError = validateOptionalBranchName(targetBranch);
  const submitBlocked =
    !valid ||
    pending ||
    workLabelCollision ||
    sourceBranchError != null ||
    targetBranchError != null;

  const setPackageAt = (i: number, value: string) =>
    setPackages((rows) => rows.map((row, j) => (j === i ? value : row)));
  const removePackageAt = (i: number) =>
    setPackages((rows) => (rows.length > 1 ? rows.filter((_, j) => j !== i) : rows));

  const clearDisposableState = () => {
    setDisposableDraft(emptyDisposableEnvironmentDraft());
    setConfirmationRequest(null);
  };

  const closeAndClear = () => {
    clearDisposableState();
    onClose();
  };

  const createSession = async (request: CreateSessionRequest) => {
    if (requestInFlight.current) return;
    requestInFlight.current = true;
    setPending(true);
    setServerError(null);
    try {
      const result = await createTrigger(apiFetch, owner, name, request);
      if (result.ok) {
        clearDisposableState();
        toast.show({ kind: 'success', message: cc.createdToast });
        onCreated(result.data);
        return;
      }
      setServerError(result.message ?? cc.createFailed);
    } catch {
      setServerError(cc.createFailed);
    } finally {
      requestInFlight.current = false;
      setPending(false);
    }
  };

  const onSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (submitBlocked) return;
    const request = buildCreateRequest({
      name: sessionName,
      packages,
      manifests,
      workLabel,
      environment: environmentMode === 'saved' ? environment : '',
      disposableEnvironment: environmentMode === 'disposable' ? disposableSpec : undefined,
      sourceBranch,
      targetBranch,
      autoMerge,
      logAccess,
      collaborators,
      outputLang,
    });
    setServerError(null);
    if (environmentMode === 'disposable') {
      setConfirmationRequest(request);
      return;
    }
    void createSession(request);
  };

  if (confirmationRequest?.disposable_environment) {
    return (
      <DisposableEnvironmentConfirm
        t={cc}
        counts={disposableEnvironmentCounts(confirmationRequest.disposable_environment)}
        pending={pending}
        serverError={serverError}
        onBack={() => {
          setServerError(null);
          setConfirmationRequest(null);
        }}
        onConfirm={() => void createSession(confirmationRequest)}
      />
    );
  }

  const footer = (
    <div className="flex items-center justify-end gap-2">
      <button
        type="button"
        onClick={closeAndClear}
        className="font-ui font-semibold text-[12.5px] bg-glass border border-line rounded-control px-4 py-2 text-dim transition-[color,border-color,box-shadow,background] hover:text-fg hover:border-line-2 hover:bg-glass-2 hover:shadow-glow-amber cursor-pointer"
      >
        {c.repos.cancel}
      </button>
      {/* Primary CTA: the brand accent-gradient fill, seated on a soft accent
          glow with a one-shot sheen sweep on mount; hover lifts brightness. */}
      <button
        type="submit"
        form={FORM_ID}
        disabled={submitBlocked}
        className={cn(
          'font-ui font-semibold text-[12.5px] rounded-control px-4 py-2 transition-[filter,box-shadow]',
          submitBlocked
            ? 'bg-amber/40 text-amber-ink/50 cursor-not-allowed'
            : 'bg-grad-accent text-amber-ink shadow-[var(--shadow-1),var(--glow-amber)] anim-sheen hover:brightness-110 cursor-pointer'
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
      onClose={closeAndClear}
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
            className="self-start font-ui font-semibold text-[11.5px] bg-glass border border-line rounded-control px-2.5 py-1 text-dim transition-[color,border-color,box-shadow,background] hover:text-fg hover:border-line-2 hover:bg-glass-2 hover:shadow-glow-amber cursor-pointer"
          >
            {cc.addPackage}
          </button>
        </fieldset>

        <div className="flex flex-col gap-1.5">
          <label htmlFor="trigger-manifests" className={FIELD_LABEL}>
            {cc.createManifestsLabel}
          </label>
          <textarea
            id="trigger-manifests"
            value={manifests}
            onChange={(e) => setManifests(e.target.value)}
            placeholder={cc.createPackagePlaceholder}
            spellCheck={false}
            autoComplete="off"
            rows={2}
            className={cn(FIELD_INPUT, 'font-mono resize-y')}
          />
          <p className="font-mono text-[11px] text-ghost">{cc.createManifestsHint}</p>
        </div>

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
            aria-invalid={workLabelCollision}
            aria-describedby={workLabelCollision ? 'trigger-work-label-collision' : undefined}
            className={cn(FIELD_INPUT, 'font-mono')}
          />
          {/* Advisory only: the backend pre-flight (409) is authoritative and its
              message still renders in the server-error note below. Keyed on the
              label so a fresh collision re-triggers the reduced-motion-safe
              entrance rather than popping. */}
          {workLabelCollision && (
            <div key={trimmedWorkLabel} className="anim-notice-in">
              <p
                id="trigger-work-label-collision"
                role="alert"
                className="border border-line border-l-2 border-l-amber rounded-card bg-glass backdrop-blur-glass px-3 py-2 font-mono text-[11.5px] text-dim shadow-[var(--shadow-1),var(--glow-amber)]"
              >
                {cc.createWorkLabelCollision}
              </p>
            </div>
          )}
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

        <EnvironmentSection
          cc={cc}
          mode={environmentMode}
          onModeChange={setEnvironmentMode}
          savedEnvironment={environment}
          onSavedEnvironmentChange={setEnvironment}
          load={envLoad}
          disposableDraft={disposableDraft}
          onDisposableDraftChange={setDisposableDraft}
          disposableHasContents={disposableHasContents}
        />

        <fieldset className="flex flex-col gap-3 border-0 border-t border-line pt-4 p-0 m-0">
          <legend className="font-ui font-semibold text-[12px] text-dim pr-2">
            {cc.createAdvancedLabel}
          </legend>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <div className="flex flex-col gap-1.5 min-w-0">
              <label htmlFor="trigger-source-branch" className={FIELD_LABEL}>
                {cc.createSourceBranchLabel}
              </label>
              <input
                id="trigger-source-branch"
                type="text"
                value={sourceBranch}
                onChange={(e) => setSourceBranch(e.target.value)}
                placeholder={cc.createSourceBranchPlaceholder}
                spellCheck={false}
                autoComplete="off"
                aria-invalid={sourceBranchError != null}
                aria-describedby={sourceBranchError ? 'trigger-source-branch-error' : undefined}
                className={cn(FIELD_INPUT, 'font-mono')}
              />
              {sourceBranchError && (
                <p
                  id="trigger-source-branch-error"
                  role="alert"
                  className="font-mono text-[11px] text-red"
                >
                  {cc.createBranchInvalid}
                </p>
              )}
            </div>
            <div className="flex flex-col gap-1.5 min-w-0">
              <label htmlFor="trigger-target-branch" className={FIELD_LABEL}>
                {cc.createTargetBranchLabel}
              </label>
              <input
                id="trigger-target-branch"
                type="text"
                value={targetBranch}
                onChange={(e) => setTargetBranch(e.target.value)}
                placeholder={cc.createTargetBranchPlaceholder}
                spellCheck={false}
                autoComplete="off"
                aria-invalid={targetBranchError != null}
                aria-describedby={targetBranchError ? 'trigger-target-branch-error' : undefined}
                className={cn(FIELD_INPUT, 'font-mono')}
              />
              {targetBranchError && (
                <p
                  id="trigger-target-branch-error"
                  role="alert"
                  className="font-mono text-[11px] text-red"
                >
                  {cc.createBranchInvalid}
                </p>
              )}
            </div>
          </div>
        </fieldset>

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
          <label htmlFor="trigger-collaborators" className={FIELD_LABEL}>
            {cc.createCollaboratorsLabel}
          </label>
          <input
            id="trigger-collaborators"
            type="text"
            value={collaborators}
            onChange={(e) => setCollaborators(e.target.value)}
            spellCheck={false}
            autoComplete="off"
            className={cn(FIELD_INPUT, 'font-mono')}
          />
          <p className="font-mono text-[11px] text-ghost">{cc.createCollaboratorsHint}</p>
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
