import type { CanvasModalsSlice, ReposModalsSlice } from '../slices';

// Dialog strings. `canvasModals` covers the create-session and stop-session
// dialogs (composed into `dashboard.canvas`); `reposModals` covers the
// create-repository and uninstall-confirm dialogs (composed into
// `dashboard.repos`). Owned by the modals cluster.

export const canvasModals: CanvasModalsSlice = {
  newTrigger: 'New session',
  stop: 'Stop',
  stopAria: 'Stop session {name}',
  stopConfirmTitle: 'Stop session {name}?',
  stopConfirmBody:
    'This closes trigger issue #{number}. The session retires permanently — a closed trigger is never re-registered. Open a new trigger issue to start again.',
  stopConfirm: 'Stop session',
  stopPending: 'Stopping…',
  stopFailed: 'Could not stop the session. Please try again.',
  createTitle: 'Start a new session',
  createNameLabel: 'Session name',
  createNameHint: 'Lowercase letters, digits and dashes.',
  createPackagesLabel: 'Packages',
  createPackagePlaceholder: 'owner/repo@ref:path',
  addPackage: 'Add package',
  removePackageAria: 'Remove package {n}',
  createWorkLabelLabel: 'Work label (optional)',
  createAutoMergeLabel: 'Auto-merge',
  createLogAccessLabel: 'Log access allowlist (optional)',
  createLogAccessHint: 'Extra GitHub logins or ids, comma or space separated.',
  createOutputLangLabel: 'Output language (optional)',
  createOutputLangHint: 'The language the session writes its output in, e.g. `English` or `中文`.',
  createSubmit: 'Create trigger issue',
  createPending: 'Creating…',
  createFailed: 'Could not create the trigger issue. Please try again.',
};

export const reposModals: ReposModalsSlice = {
  newRepo: 'New repository',
  createTitle: 'Create a repository',
  ownerLabel: 'Owner',
  ownerPersonal: 'Personal ({login})',
  nameLabel: 'Repository name',
  nameHint: 'Letters, digits, and . _ - only.',
  privateLabel: 'Private',
  descriptionLabel: 'Description (optional)',
  submit: 'Create repository',
  creating: 'Creating…',
  cancel: 'Cancel',
  createFailed: 'Could not create the repository. Please try again.',
  createdNextStep: 'Next: install the App on this repo',
  uninstall: 'Uninstall',
  uninstallConfirmTitle: 'Uninstall from {owner}?',
  uninstallConfirmBody:
    'This uninstalls the GitHub App from {owner}. Everything the App covers in this account — repository creation and every fkst session — stops working immediately.',
  uninstallConfirm: 'Uninstall',
  uninstallPending: 'Uninstalling…',
  uninstallFailed: 'Could not uninstall the App. Please try again.',
};
