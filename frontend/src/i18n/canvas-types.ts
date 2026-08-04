/**
 * The zoomable canvas surface (levels 0–2), its sidebar, and the dialogs it
 * launches. Split out of `types.ts` so the dashboard's largest surface does not
 * hold that file over the 500-line limit — the shape is unchanged, and
 * `slices.ts` still derives every per-module alias from `SiteContent`.
 */
export interface CanvasContent {
  canvasAria: string;
  repoWorkspaceAria: string;
  sidebarAria: string;
  breadcrumbAria: string;
  /** Root crumb label (level 0). */
  breadcrumbRoot: string;
  back: string;
  backAria: string;
  /** Small keyboard hint next to the breadcrumb. */
  escHint: string;
  loadingCanvas: string;
  loadingSidebar: string;
  legendTitle: string;
  legendNone: string;
  legendInstalled: string;
  legendActive: string;
  /** Sidebar lede stating what level 0 represents. */
  viewRoot: string;
  /** Sidebar lede for level 1; `{login}` placeholder. */
  viewAccount: string;
  /** Sidebar lede for level 2; `{repo}` placeholder. */
  viewRepo: string;
  /** Badge on accounts where the viewer is owner/admin. */
  ownerBadge: string;
  statusNone: string;
  statusInstalled: string;
  /** Active badge; `{n}` placeholder. */
  statusActiveCount: string;
  /** `{n}` placeholder. */
  repoCount: string;
  /** Overflow marker for the in-card repo dots; `{n}` placeholder. */
  moreRepos: string;
  /** `{login}` placeholder. */
  openAccountAria: string;
  /** `{repo}` placeholder. */
  openRepoAria: string;
  /** Warning when the backend flagged counts_complete=false. */
  countsIncomplete: string;
  filterAccountsPlaceholder: string;
  filterReposPlaceholder: string;
  noAccountsMatch: string;
  noReposMatch: string;
  noAccounts: string;
  chartSessionsTitle: string;
  chartPackagesTitle: string;
  chartScopeAllAccounts: string;
  chartScopeAllRepos: string;
  chartScopeAriaAccounts: string;
  chartScopeAriaRepos: string;
  chartEmpty: string;
  /** Aggregate row label when the chart tail is folded. */
  chartOther: string;
  // ---- Broader-visibility (full repo/org) connect affordance ----
  /** Connect-banner heading: invites authorizing the broader credential. */
  broaderConnectTitle: string;
  /** One-line explanation under the connect heading (what it unlocks). */
  broaderConnectHint: string;
  /** Connect CTA button label. */
  broaderConnect: string;
  /** Connected-state status text (broader token active). */
  broaderShowingAll: string;
  /** Inline disconnect action in the connected state. */
  broaderDisconnect: string;
  sessionsTitle: string;
  /** Poll cadence, now surfaced as the freshness line's tooltip. */
  pollNote: string;
  /** Live freshness line replacing the static poll note; `{time}` holds a
   *  relative "2 min ago" rendered by the formatter. */
  sessionsFreshness: string;
  /** Retry affordance on the no-data sessions load error. */
  sessionsRetry: string;
  /** Spinner label while a manual sessions refresh is in flight. */
  sessionsRefreshing: string;
  sessionsLoadFailed: string;
  /** Non-blocking notice when a refresh failed but last-good data shows. */
  sessionsStaleNotice: string;
  notInstalledNote: string;
  newTrigger: string;
  livenessStarting: string;
  livenessLive: string;
  livenessTerminating: string;
  logDownload: string;
  prsTitle: string;
  prMerged: string;
  /** PR → work-issue link text; `{n}` placeholder. */
  prForIssue: string;
  createdWord: string;
  updatedWord: string;
  closedWord: string;
  /** First-run empty state (level 0): heading shown when the viewer has
   *  connected the App to no account yet. */
  firstRunTitle: string;
  /** One-line explanation under the first-run heading. */
  firstRunBody: string;
  /** Primary "Install the GitHub App" call-to-action in the first-run state. */
  firstRunInstall: string;
  /** Secondary link from the first-run state to the get-started guide. */
  firstRunGuide: string;
  /** Persistent badge on a level-1 repo row that still needs the App. */
  needsInstall: string;
  /** Tooltip on the needs-install badge. */
  needsInstallHint: string;
  /** Per-session affordance that opens the queue-work-item dialog. */
  addWorkItem: string;
  stop: string;
  /** `{name}` placeholder. */
  stopAria: string;
  /** `{name}` placeholder. */
  stopConfirmTitle: string;
  /** `{number}` placeholder (the trigger issue number). */
  stopConfirmBody: string;
  stopConfirm: string;
  stopPending: string;
  stopFailed: string;
  createTitle: string;
  createNameLabel: string;
  createNameHint: string;
  createPackagesLabel: string;
  createPackagePlaceholder: string;
  addPackage: string;
  /** `{n}` placeholder (1-based row index). */
  removePackageAria: string;
  /** Manifest textarea (one `owner/repo@ref:path` per line) — a fkst-manifest
   *  the server expands into packages (`### Manifest`). */
  createManifestsLabel: string;
  createManifestsHint: string;
  createWorkLabelLabel: string;
  createEnvironmentLabel: string;
  createAdvancedLabel: string;
  createSourceBranchLabel: string;
  createSourceBranchPlaceholder: string;
  createTargetBranchLabel: string;
  createTargetBranchPlaceholder: string;
  createBranchInvalid: string;
  createAutoMergeLabel: string;
  createLogAccessLabel: string;
  createLogAccessHint: string;
  /** Collaborators (work-item authority) input — distinct from log access. */
  createCollaboratorsLabel: string;
  createCollaboratorsHint: string;
  createOutputLangLabel: string;
  /** Hint naming what the value drives (the '### Output Language' section). */
  createOutputLangHint: string;
  createSubmit: string;
  createPending: string;
  createFailed: string;
  /** Modals-cluster additions for the create-trigger environment picker +
   *  work-label hint (owned by the create-trigger modal item). */
  createEnvironmentNone: string;
  createEnvironmentSaved: string;
  createEnvironmentDisposable: string;
  createSavedEnvironmentLabel: string;
  createEnvironmentNote: string;
  createEnvironmentLoadFailed: string;
  createDisposablePrivateNote: string;
  createDisposableInstallLabel: string;
  createDisposableInstallPlaceholder: string;
  createDisposableAddInstall: string;
  /** `{n}` placeholder (1-based row index). */
  createDisposableRemoveInstall: string;
  createDisposableVariablesLabel: string;
  createDisposableSecretsLabel: string;
  createDisposableNamePlaceholder: string;
  createDisposableValuePlaceholder: string;
  createDisposableSecretPlaceholder: string;
  createDisposableAddVariable: string;
  createDisposableAddSecret: string;
  /** `{n}` placeholder (1-based row index). */
  createDisposableRemoveVariable: string;
  /** `{n}` placeholder (1-based row index). */
  createDisposableRemoveSecret: string;
  createDisposableImmutableNote: string;
  createDisposableEmpty: string;
  createDisposableConfirmTitle: string;
  createDisposableConfirmBody: string;
  createDisposableConfirmInstall: string;
  createDisposableConfirmVariables: string;
  createDisposableConfirmSecrets: string;
  createDisposableConfirmWarning: string;
  createDisposableConfirmBack: string;
  createDisposableConfirmSubmit: string;
  createDisposableConfirmPending: string;
  createWorkLabelHint: string;
  createWorkLabelHintLink: string;
  /** Inline collision warning: the typed work label is already claimed by an
   *  open session on this repo, so the backend would reject it. */
  createWorkLabelCollision: string;
  createdToast: string;
  /** Modals-cluster additions for the queue-work-item dialog (owned by the
   *  create-work-item modal item). */
  workItemTitle: string;
  workItemTitleLabel: string;
  workItemTitleHint: string;
  workItemLabelLabel: string;
  workItemBodyLabel: string;
  workItemBodyHint: string;
  workItemBodyModeAria: string;
  workItemWrite: string;
  workItemPreview: string;
  workItemPreviewAria: string;
  workItemPreviewEmpty: string;
  /** Note naming the session's work label the issue is stamped with;
   *  `{label}` and `{creator}` placeholders. */
  workItemLabelNote: string;
  workItemSubmit: string;
  workItemPending: string;
  workItemFailed: string;
  workItemCreatedToast: string;
}
