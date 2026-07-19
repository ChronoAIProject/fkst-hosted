import type { CanvasEnvSlice, EnvironmentScalarSlice } from '../slices';

// Environment strings. Two audiences share this domain file:
//  1. The tiny "Environment" chip/field strings that compose INTO the shared
//     `SiteContent` catalog (`canvasEnv` / `environmentScalar`, consumed by
//     `en.ts`). Those must keep their exact slice shape.
//  2. The full environment-manager drawer (`environmentsManager`), which is a
//     self-contained dictionary the manager components read directly via
//     `useLang()` — it is NOT wired into `SiteContent`, so adding to it never
//     forces a change to the shared `types.ts` / `en.ts` / `zh.ts` files another
//     work item owns. The `EnvManagerStrings` interface below is the contract the
//     zh sibling implements, so TypeScript keeps the two locales in lock-step.

/** Composed into the dashboard scalars (`dashboard.environment`). */
export const environmentScalar: EnvironmentScalarSlice = {
  environment: 'Environment',
};

/** Composed into `dashboard.canvas` alongside the create-session dialog. */
export const canvasEnv: CanvasEnvSlice = {
  createEnvironmentLabel: 'Environment (optional)',
};

/** Every string the environment-manager drawer renders. Templated strings use
 *  `{name}` / `{n}` / `{max}` / `{time}` placeholders substituted at the call
 *  site (see `fmt` in `environments-drawer.tsx`). */
export interface EnvManagerStrings {
  // Drawer chrome
  title: string;
  close: string;
  closeAria: string;
  back: string;
  backAria: string;
  newEnvironment: string;

  // List view
  listLoading: string;
  listLoadFailed: string;
  listEmpty: string;
  listEmptyHint: string;
  retry: string;
  validatedAt: string; // "Validated {time}"
  neverValidated: string;
  installCount: string; // "{n} install"
  variableCount: string; // "{n} variable"
  secretCount: string; // "{n} secret"
  openAria: string; // "Open environment {name}"

  // Editor view
  editorCreateTitle: string;
  editorEditTitle: string;
  nameLabel: string;
  namePlaceholder: string;
  nameHint: string;
  nameLockedHint: string;
  nameErrorFormat: string;
  nameErrorLength: string; // "… {max} …"
  installLegend: string;
  installPlaceholder: string;
  installHint: string;
  addInstall: string;
  removeInstallAria: string; // "Remove install command {n}"
  variablesLegend: string;
  variableNamePlaceholder: string;
  variableValuePlaceholder: string;
  addVariable: string;
  removeVariableAria: string; // "Remove variable {n}"
  secretsLegend: string;
  secretNamePlaceholder: string;
  secretValuePlaceholder: string;
  secretsHint: string;
  secretsEditHint: string;
  addSecret: string;
  removeSecretAria: string; // "Remove secret {n}"
  validatingNote: string;
  save: string;
  saving: string;
  cancel: string;
  saveFailed: string;
  validationTitle: string;
  validationCommand: string;
  validationIndex: string;
  validationExitCode: string;
  validationTimedOut: string;
  validationStderr: string;
  savedToast: string; // "Environment “{name}” saved."

  // Detail view
  detailLoading: string;
  detailLoadFailed: string;
  statusLabel: string;
  validatedLabel: string;
  installTitle: string;
  installEmpty: string;
  variablesTitle: string;
  variablesEmpty: string;
  secretsTitle: string;
  secretsEmpty: string;
  secretsValueNote: string;
  edit: string;
  deleteButton: string;
  deleteConfirmTitle: string;
  deleteConfirmBody: string; // "… “{name}” …"
  deleteConfirm: string;
  deletePending: string;
  deleteCancel: string;
  deleteFailed: string;
  deletedToast: string; // "Environment “{name}” deleted."

  // Shared
  yes: string;
  no: string;
}

/** The English environment-manager dictionary. */
export const environmentsManager: EnvManagerStrings = {
  title: 'Environments',
  close: 'Close',
  closeAria: 'Close environments',
  back: 'Back',
  backAria: 'Back to environment list',
  newEnvironment: 'New environment',

  listLoading: 'Loading environments…',
  listLoadFailed: 'Could not load your environments.',
  listEmpty: 'No environments yet.',
  listEmptyHint: 'Create one to reuse install steps, variables, and secrets across sessions.',
  retry: 'Retry',
  validatedAt: 'Validated {time}',
  neverValidated: 'Not validated',
  installCount: '{n} install',
  variableCount: '{n} variable',
  secretCount: '{n} secret',
  openAria: 'Open environment {name}',

  editorCreateTitle: 'New environment',
  editorEditTitle: 'Edit environment',
  nameLabel: 'Name',
  namePlaceholder: 'my-environment',
  nameHint: 'Lowercase letters, digits, and hyphens — used to build the stored object name.',
  nameLockedHint: 'The name cannot be changed after creation.',
  nameErrorFormat: 'Use lowercase letters, digits, and hyphens (not at the ends).',
  nameErrorLength: 'Name must be at most {max} characters.',
  installLegend: 'Install commands',
  installPlaceholder: 'pip install -r requirements.txt',
  installHint: 'Run in order in a throwaway pod when you save.',
  addInstall: 'Add command',
  removeInstallAria: 'Remove install command {n}',
  variablesLegend: 'Variables',
  variableNamePlaceholder: 'NAME',
  variableValuePlaceholder: 'value',
  addVariable: 'Add variable',
  removeVariableAria: 'Remove variable {n}',
  secretsLegend: 'Secrets',
  secretNamePlaceholder: 'NAME',
  secretValuePlaceholder: 'value (write-only)',
  secretsHint: 'Secret values are write-only — they are never shown again after saving.',
  secretsEditHint: 'Re-enter every secret value; secrets left blank are removed when you save.',
  addSecret: 'Add secret',
  removeSecretAria: 'Remove secret {n}',
  validatingNote: 'Validating install commands in an isolated pod… this can take a while.',
  save: 'Save',
  saving: 'Saving…',
  cancel: 'Cancel',
  saveFailed: 'Could not save the environment.',
  validationTitle: 'Install validation failed',
  validationCommand: 'Failed command',
  validationIndex: 'Command index',
  validationExitCode: 'Exit code',
  validationTimedOut: 'Timed out',
  validationStderr: 'stderr (tail)',
  savedToast: 'Environment “{name}” saved.',

  detailLoading: 'Loading environment…',
  detailLoadFailed: 'Could not load the environment.',
  statusLabel: 'Status',
  validatedLabel: 'Validated',
  installTitle: 'Install commands',
  installEmpty: 'No install commands.',
  variablesTitle: 'Variables',
  variablesEmpty: 'No variables.',
  secretsTitle: 'Secrets',
  secretsEmpty: 'No secrets.',
  secretsValueNote: 'Values are hidden and never returned.',
  edit: 'Edit',
  deleteButton: 'Delete',
  deleteConfirmTitle: 'Delete environment?',
  deleteConfirmBody:
    'Delete “{name}”? Sessions that reference it will no longer find it. This cannot be undone.',
  deleteConfirm: 'Delete',
  deletePending: 'Deleting…',
  deleteCancel: 'Cancel',
  deleteFailed: 'Could not delete the environment.',
  deletedToast: 'Environment “{name}” deleted.',

  yes: 'Yes',
  no: 'No',
};
