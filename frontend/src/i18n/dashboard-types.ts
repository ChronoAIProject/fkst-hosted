import type { CanvasContent } from './canvas-types';
import type { SessionDetailContent } from './session-detail-types';

/** Repository list rows, group headers, and the create/uninstall dialogs. */
export interface ReposContent {
  refresh: string;
  /** Refresh button label while the overview re-fetch is in flight. */
  refreshing: string;
  loadFailed: string;
  /** Non-blocking notice when a refresh failed but last-good data shows. */
  refreshFailedStale: string;
  private: string;
  public: string;
  org: string;
  installed: string;
  install: string;
  nonAdminHint: string;
  appNotConfigured: string;
  /** Group label for the viewer's own account. */
  personalGroup: string;
  /** Group label for an organization account. */
  orgGroup: string;
  /** Per-group counts template; `{installed}` / `{total}` placeholders. */
  groupCounts: string;
  /** Body line for a group (org creation target) with no repositories. */
  groupEmpty: string;
  /** "New repository" button in the section header. */
  newRepo: string;
  createTitle: string;
  ownerLabel: string;
  /** Personal option in the owner select; `{login}` placeholder. */
  ownerPersonal: string;
  nameLabel: string;
  /** Hint under the name input (allowed characters). */
  nameHint: string;
  privateLabel: string;
  descriptionLabel: string;
  submit: string;
  creating: string;
  cancel: string;
  /** Generic creation failure (no server message available). */
  createFailed: string;
  /** Callout under a freshly created repo pointing at the Install step. */
  createdNextStep: string;
  /** Success toast raised when a repository is created. */
  createdToast: string;
  /** Group-header CTA for an account without an App installation. */
  connect: string;
  /** Short hint next to the Connect CTA (why connecting matters). */
  connectHint: string;
  /** Group-header link to the installation's GitHub settings page. */
  manage: string;
  manageRepoHint: string;
  /** Group-header danger action for a connected account. */
  uninstall: string;
  /** Uninstall confirm dialog title; `{owner}` placeholder. */
  uninstallConfirmTitle: string;
  /** Uninstall confirm dialog body; `{owner}` placeholder. */
  uninstallConfirmBody: string;
  /** Uninstall confirm dialog: explicit confirm button. */
  uninstallConfirm: string;
  /** Uninstall confirm button while the DELETE is in flight. */
  uninstallPending: string;
  /** Generic uninstall failure (no server message available). */
  uninstallFailed: string;
}

/** The authenticated dashboard route: its own scalars plus the three surfaces
 *  it hosts. */
export interface DashboardContent {
  metaTitle: string;
  /** Route-level skeleton label while the lazy dashboard chunk downloads. */
  loading: string;
  eyebrow: string;
  title: string;
  lede: string;
  /** Compact authenticated-mode marker shown for deployment-wide admins. */
  globalAdmin: string;
  signInTitle: string;
  signInBody: string;
  notConfigured: string;
  /** Generic OAuth-callback error (fallback when the slug is unrecognized). */
  authError: string;
  /** Known OAuth-callback error slugs → specific copy; `authError` is the
   *  fallback for any slug not listed here. */
  authErrorBySlug: Record<string, string>;
  /** Retry action on the in-panel overview load error. */
  retry: string;
  /** Involuntary-expiry re-authenticate prompt: shown in place of the cold
   *  sign-in card so the user's level/selection is preserved. */
  sessionExpiredTitle: string;
  sessionExpiredBody: string;
  sessionExpiredAction: string;
  noSessions: string;
  installed: string;
  workLabel: string;
  packages: string;
  autoMerge: string;
  environment: string;
  invalidTrigger: string;
  trigger: string;
  workIssues: string;
  open: string;
  closed: string;
  canvas: CanvasContent;
  repos: ReposContent;
  detail: SessionDetailContent;
}
