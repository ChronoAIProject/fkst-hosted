import type { SiteContent } from './types';

// ---------------------------------------------------------------------------
// Domain-slice aliases.
//
// The `en` / `zh` catalogs are authored as per-domain modules under `en/` and
// `zh/` and composed back into the single `SiteContent` shape by `en.ts` /
// `zh.ts` — so consumers keep the same key paths (`dashboard.canvas.*`, …) with
// zero call-site changes. Splitting the string data by domain removes the
// merge bottleneck: parallel work items each edit a different module.
//
// These aliases are the contract each module implements. `Pick` forces a module
// to carry EXACTLY its assigned keys (missing → error, excess → error), and the
// final `: SiteContent` annotation on `en`/`zh` guarantees the composed union is
// complete. The `dashboard.canvas` and `dashboard.repos` objects are split
// across modules (graph vs. sidebar vs. modals vs. environments) precisely so
// the later-wave clusters never contend on one file; each slice below owns a
// disjoint set of those keys.
export type DashboardContent = SiteContent['dashboard'];
export type CanvasContent = DashboardContent['canvas'];
export type ReposContent = DashboardContent['repos'];

/** Top-nav labels. */
export type NavSlice = SiteContent['nav'];
/** Shared chrome shared by every page: language toggle, auth, footer, and the
 *  app-shell error/404/toast strings. */
export type CommonSlice = Pick<SiteContent, 'toggle' | 'auth' | 'footer' | 'shell'>;
/** Marketing / get-started long-form pages. */
export type PagesSlice = Pick<SiteContent, 'intro' | 'gs'>;
/** Guided product-tour strings (its own top-level domain). */
export type TourSlice = SiteContent['tour'];
/** Per-session detail drawer (session-detail cluster owns this). */
export type DetailSlice = DashboardContent['detail'];

/** Dashboard page shell scalars (everything not itself a nested surface). */
export type DashboardScalars = Omit<
  DashboardContent,
  'canvas' | 'repos' | 'detail' | 'environment'
>;
/** The `dashboard.environment` scalar — grouped with the environment strings. */
export type EnvironmentScalarSlice = Pick<DashboardContent, 'environment'>;

/** Canvas graph itself: cards, breadcrumb, legend, charts (dashboard module). */
export type CanvasGraphSlice = Pick<
  CanvasContent,
  | 'canvasAria'
  | 'repoWorkspaceAria'
  | 'breadcrumbAria'
  | 'breadcrumbRoot'
  | 'back'
  | 'backAria'
  | 'escHint'
  | 'loadingCanvas'
  | 'legendTitle'
  | 'legendNone'
  | 'legendInstalled'
  | 'legendActive'
  | 'ownerBadge'
  | 'statusNone'
  | 'statusInstalled'
  | 'statusActiveCount'
  | 'repoCount'
  | 'moreRepos'
  | 'openAccountAria'
  | 'openRepoAria'
  | 'countsIncomplete'
  | 'filterAccountsPlaceholder'
  | 'filterReposPlaceholder'
  | 'noAccountsMatch'
  | 'noReposMatch'
  | 'noAccounts'
  | 'chartSessionsTitle'
  | 'chartPackagesTitle'
  | 'chartScopeAllAccounts'
  | 'chartScopeAllRepos'
  | 'chartScopeAriaAccounts'
  | 'chartScopeAriaRepos'
  | 'chartEmpty'
  | 'chartOther'
>;
/** Canvas details panel: level ledes, sessions/PRs listing (sidebar cluster). */
export type CanvasSidebarSlice = Pick<
  CanvasContent,
  | 'sidebarAria'
  | 'loadingSidebar'
  | 'viewRoot'
  | 'viewAccount'
  | 'viewRepo'
  | 'sessionsTitle'
  | 'pollNote'
  | 'sessionsFreshness'
  | 'sessionsRetry'
  | 'sessionsRefreshing'
  | 'sessionsLoadFailed'
  | 'sessionsStaleNotice'
  | 'notInstalledNote'
  | 'livenessStarting'
  | 'livenessLive'
  | 'livenessTerminating'
  | 'logDownload'
  | 'prsTitle'
  | 'prMerged'
  | 'prForIssue'
  | 'createdWord'
  | 'updatedWord'
  | 'closedWord'
  | 'firstRunTitle'
  | 'firstRunBody'
  | 'firstRunInstall'
  | 'firstRunGuide'
  | 'needsInstall'
  | 'needsInstallHint'
  | 'addWorkItem'
>;
/** Canvas dialogs: create-session + stop-session (modals cluster). */
export type CanvasModalsSlice = Pick<
  CanvasContent,
  | 'newTrigger'
  | 'stop'
  | 'stopAria'
  | 'stopConfirmTitle'
  | 'stopConfirmBody'
  | 'stopConfirm'
  | 'stopPending'
  | 'stopFailed'
  | 'createTitle'
  | 'createNameLabel'
  | 'createNameHint'
  | 'createPackagesLabel'
  | 'createPackagePlaceholder'
  | 'addPackage'
  | 'removePackageAria'
  | 'createWorkLabelLabel'
  | 'createAutoMergeLabel'
  | 'createLogAccessLabel'
  | 'createLogAccessHint'
  | 'createOutputLangLabel'
  | 'createOutputLangHint'
  | 'createSubmit'
  | 'createPending'
  | 'createFailed'
  | 'createEnvironmentNone'
  | 'createEnvironmentNote'
  | 'createEnvironmentLoadFailed'
  | 'createWorkLabelHint'
  | 'createWorkLabelHintLink'
  | 'createdToast'
  | 'workItemTitle'
  | 'workItemTitleLabel'
  | 'workItemTitleHint'
  | 'workItemBodyLabel'
  | 'workItemBodyHint'
  | 'workItemLabelNote'
  | 'workItemSubmit'
  | 'workItemPending'
  | 'workItemFailed'
  | 'workItemCreatedToast'
>;
/** Environment picker string in the create-session form (environments cluster). */
export type CanvasEnvSlice = Pick<CanvasContent, 'createEnvironmentLabel'>;

/** Repository list rows + group headers (dashboard module). */
export type ReposBaseSlice = Pick<
  ReposContent,
  | 'refresh'
  | 'refreshing'
  | 'loadFailed'
  | 'refreshFailedStale'
  | 'private'
  | 'public'
  | 'org'
  | 'installed'
  | 'install'
  | 'nonAdminHint'
  | 'appNotConfigured'
  | 'personalGroup'
  | 'orgGroup'
  | 'groupCounts'
  | 'groupEmpty'
  | 'connect'
  | 'connectHint'
  | 'manage'
  | 'manageRepoHint'
>;
/** Repository dialogs: create-repo + uninstall-confirm (modals cluster). */
export type ReposModalsSlice = Pick<
  ReposContent,
  | 'newRepo'
  | 'createTitle'
  | 'ownerLabel'
  | 'ownerPersonal'
  | 'nameLabel'
  | 'nameHint'
  | 'privateLabel'
  | 'descriptionLabel'
  | 'submit'
  | 'creating'
  | 'cancel'
  | 'createFailed'
  | 'createdNextStep'
  | 'createdToast'
  | 'uninstall'
  | 'uninstallConfirmTitle'
  | 'uninstallConfirmBody'
  | 'uninstallConfirm'
  | 'uninstallPending'
  | 'uninstallFailed'
>;
