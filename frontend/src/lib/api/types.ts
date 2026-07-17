// Typed DTOs for the canvas dashboard endpoints. These mirror the pinned
// backend contract (canvas-contract.md) field by field — the backend is built
// against the same document, so any drift is a bug on one side, not a style
// choice. Keep names snake_case exactly as they appear on the wire.

// ---- GET /api/v1/overview ---------------------------------------------------

export interface PackageCount {
  package: string;
  count: number;
}

export interface OverviewTotals {
  sessions: number;
  packages: PackageCount[];
}

export interface RepoOverview {
  id: number;
  owner: string;
  name: string;
  private: boolean;
  admin: boolean;
  installed: boolean;
  /** Open trigger issues that parse OK (registration-level active). */
  active_sessions: number;
  /** Union of package refs across this repo's active sessions. */
  packages: string[];
}

export type AccountKind = 'personal' | 'org';

export interface AccountOverview {
  login: string;
  kind: AccountKind;
  /** Personal account: always true. Org: role == admin. */
  owner: boolean;
  installed: boolean;
  installation_id: number | null;
  repository_selection: 'all' | 'selected' | null;
  /** False when any repo's trigger read failed (the call still succeeds). */
  counts_complete: boolean;
  repos: RepoOverview[];
}

export interface OverviewResponse {
  app_slug: string | null;
  viewer: { login: string };
  accounts: AccountOverview[];
  totals: OverviewTotals;
}

// ---- GET /api/v1/repos/{owner}/{name}/sessions ------------------------------

export interface IssueDetail {
  number: number;
  title: string;
  state: 'open' | 'closed';
  author: string;
  labels: string[];
  html_url: string;
  created_at: string;
  updated_at: string;
  closed_at: string | null;
}

export interface PrDetail {
  number: number;
  title: string;
  html_url: string;
  state: 'open' | 'closed';
  merged: boolean;
  work_issue: number | null;
}

export type SessionLiveness = 'starting' | 'live' | 'terminating';

export interface SessionDetail {
  session_id: string | null;
  name: string | null;
  work_label: string | null;
  auto_merge: boolean | null;
  environment: string | null;
  packages: string[];
  invalid_reason: string | null;
  status_labels: string[];
  trigger: IssueDetail;
  work_issues: IssueDetail[];
  /** Identity-gated log download URL; null when unavailable. */
  log_url: string | null;
  /** Pod liveness from the session backend; null when backend absent/error. */
  liveness: SessionLiveness | null;
  prs: PrDetail[];
}

export interface RepoSessionsResponse {
  owner: string;
  name: string;
  installed: boolean;
  sessions: SessionDetail[];
}

// ---- POST /api/v1/repos/{owner}/{name}/sessions -----------------------------

export interface CreateSessionRequest {
  name: string;
  /** At least one `owner/repo@ref:path` reference. */
  packages: string[];
  work_label?: string;
  environment?: string;
  auto_merge?: boolean;
  log_access?: string[];
  output_lang?: string;
}

export interface CreateSessionResponse {
  issue_number: number;
  html_url: string;
}

// ---- Shared -----------------------------------------------------------------

/** The backend's uniform error body. */
export interface ErrorEnvelope {
  error: string;
  message: string;
}
