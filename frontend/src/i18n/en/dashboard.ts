import type { CanvasGraphSlice, DashboardScalars, ReposBaseSlice } from '../slices';

// Dashboard page shell + the canvas graph itself + the repository list rows.
// The canvas details panel (sidebar), its dialogs (modals), the environment
// strings and the per-session drawer (detail) each live in a sibling module so
// later-wave clusters never contend on this file; index composes every slice
// back into the single `dashboard.canvas` / `dashboard.repos` key paths.

export const dashboardScalars: DashboardScalars = {
  metaTitle: 'FKST — Dashboard',
  loading: 'Loading the dashboard…',
  eyebrow: 'Dashboard',
  title: 'Your fkst sessions',
  lede: 'Across the repositories where the fkst-hosted App is installed, see every trigger issue and its work issues — grouped session by session.',
  signInTitle: 'Sign in to view your dashboard',
  signInBody: 'Connect your GitHub account to load your fkst sessions and issues. You stay signed in — the token refreshes automatically.',
  notConfigured: 'The dashboard backend is not configured for this deployment yet.',
  authError: 'Sign-in was cancelled or failed. Please try again.',
  authErrorBySlug: {
    access_denied:
      'You cancelled or denied the GitHub authorization. Sign in again whenever you are ready.',
    redirect_uri_mismatch:
      'The sign-in redirect did not match this deployment. Check the GitHub App callback URLs and try again.',
    application_suspended:
      'This GitHub App is suspended, so sign-in cannot complete. Contact the deployment owner.',
  },
  retry: 'Retry',
  sessionExpiredTitle: 'Your session expired',
  sessionExpiredBody:
    'You were signed out because your GitHub token could not be refreshed. Sign in again to pick up right where you left off — your place on the dashboard is kept.',
  sessionExpiredAction: 'Sign in again',
  noSessions: 'No fkst sessions in this repository.',
  installed: 'installed',
  workLabel: 'Work label',
  packages: 'Packages',
  autoMerge: 'auto-merge',
  invalidTrigger: 'Invalid trigger',
  trigger: 'Trigger',
  workIssues: 'Work issues',
  open: 'open',
  closed: 'closed',
};

export const canvasGraph: CanvasGraphSlice = {
  canvasAria: 'Accounts and repositories canvas',
  repoWorkspaceAria: 'Repository sessions workspace',
  breadcrumbAria: 'Canvas level',
  breadcrumbRoot: 'Accounts',
  back: '← Back',
  backAria: 'Back to the previous level',
  escHint: 'Esc — back',
  loadingCanvas: 'Loading canvas…',
  legendTitle: 'Legend',
  legendNone: 'Grey — App not installed',
  legendInstalled: 'Amber — App installed, no active sessions',
  legendActive: 'Blinking amber — active sessions running',
  ownerBadge: 'owner',
  statusNone: 'no App',
  statusInstalled: 'installed',
  statusActiveCount: '{n} active',
  repoCount: '{n} repositories',
  moreRepos: '+{n} more',
  openAccountAria: 'Open account {login}',
  openRepoAria: 'Open repository {repo}',
  countsIncomplete: 'Some session counts could not be read — totals may be low.',
  filterAccountsPlaceholder: 'Filter accounts…',
  filterReposPlaceholder: 'Filter repositories…',
  noAccountsMatch: 'No accounts match your filter.',
  noReposMatch: 'No repositories match your filter.',
  noAccounts: 'No accounts found.',
  chartSessionsTitle: 'Running sessions',
  chartPackagesTitle: 'Packages in use',
  chartScopeAllAccounts: 'All accounts',
  chartScopeAllRepos: 'All repositories',
  chartScopeAriaAccounts: 'Scope charts to an account',
  chartScopeAriaRepos: 'Scope charts to a repository',
  chartEmpty: 'Nothing to chart yet.',
  chartOther: 'Other',
};

export const reposBase: ReposBaseSlice = {
  refresh: 'Refresh',
  refreshing: 'Refreshing…',
  loadFailed: 'Could not load your repositories. Please try again.',
  refreshFailedStale: 'Refresh failed — showing the last loaded data.',
  private: 'private',
  public: 'public',
  org: 'org',
  installed: 'Installed',
  install: 'Install',
  nonAdminHint:
    'You are not an admin of this repository — GitHub may send an approval request to its owner.',
  appNotConfigured:
    'The GitHub App is not configured for this deployment yet, so install links are unavailable.',
  personalGroup: 'Personal',
  orgGroup: 'Organization',
  groupCounts: '{installed}/{total} installed',
  groupEmpty: 'No repositories yet.',
  connect: 'Connect',
  connectHint: 'Connect to enable repository creation and fkst sessions.',
  manage: 'Manage',
  manageRepoHint: 'Manage this repository on GitHub (add or remove it there).',
};
