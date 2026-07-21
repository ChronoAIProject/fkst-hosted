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
  createManifestsLabel: 'Manifests (optional)',
  createManifestsHint:
    'fkst-manifest references, one owner/repo@ref:path per line. A manifest is a bundle the server expands into packages — enough on its own, so a session can reference only a manifest.',
  createWorkLabelLabel: 'Work label (optional)',
  createAutoMergeLabel: 'Auto-merge',
  createLogAccessLabel: 'Log access allowlist (optional)',
  createLogAccessHint: 'Extra GitHub logins or ids, comma or space separated.',
  createCollaboratorsLabel: 'Collaborators (optional)',
  createCollaboratorsHint:
    'GitHub logins granted work-item authority — they can raise, label and comment on this session’s work issues. Comma or space separated. Distinct from log access.',
  createOutputLangLabel: 'Output language (optional)',
  createOutputLangHint: 'The language the session writes its output in, e.g. `English` or `中文`.',
  createSubmit: 'Create trigger issue',
  createPending: 'Creating…',
  createFailed: 'Could not create the trigger issue. Please try again.',
  createEnvironmentNone: 'None',
  createEnvironmentNote: 'Only a saved environment can be referenced. Create one in the Environments manager in the top bar.',
  createEnvironmentLoadFailed: 'Could not load your environments — enter a name manually.',
  createWorkLabelHint: 'One work label per trigger — the session claims every issue that carries it.',
  createWorkLabelHintLink: 'Learn more in Get started.',
  createWorkLabelCollision:
    'A session on this repo already uses this work label — it will be rejected. Choose a different label.',
  createdToast: 'Session created',
  workItemTitle: 'Queue work',
  workItemTitleLabel: 'Title',
  workItemTitleHint: 'A short summary of the task, like a GitHub issue title.',
  workItemBodyLabel: 'Details (optional)',
  workItemBodyHint: 'Markdown is supported.',
  workItemLabelNote: 'Opens an issue labeled `{label}`, which this session claims and works.',
  workItemSubmit: 'Queue work item',
  workItemPending: 'Queuing…',
  workItemFailed: 'Could not queue the work item. Please try again.',
  workItemCreatedToast: 'Work item queued',
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
  createdToast: 'Repository created',
  uninstall: 'Uninstall',
  uninstallConfirmTitle: 'Uninstall from {owner}?',
  uninstallConfirmBody:
    'This uninstalls the GitHub App from {owner}. Everything the App covers in this account — repository creation and every fkst session — stops working immediately.',
  uninstallConfirm: 'Uninstall',
  uninstallPending: 'Uninstalling…',
  uninstallFailed: 'Could not uninstall the App. Please try again.',
};
