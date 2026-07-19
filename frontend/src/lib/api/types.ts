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

// ---- GET /api/v1/repos/{owner}/{name}/sessions/{issue}/outcomes -------------
// A session's committed outcome files, grouped by devloop PR (spec B2). The
// wire is snake_case; these mirror the backend DTOs field for field.

/** How a committed file previews — guessed from the extension by the backend. */
export type OutcomeFileKind = 'text' | 'image' | 'video' | 'audio' | 'binary';

export interface OutcomeFile {
  filename: string;
  /** GitHub file status: `added` | `modified` | `removed` | `renamed`. */
  status: string;
  additions: number;
  deletions: number;
  /** Blob SHA — the handle the `/blob/{sha}` endpoint streams. */
  sha: string;
  previous_filename: string | null;
  kind: OutcomeFileKind;
  /** additions+deletions for text, null for binary. */
  size_hint: number | null;
}

export interface PrOutcome {
  number: number;
  title: string;
  html_url: string;
  state: string;
  merged: boolean;
  work_issue: number | null;
  files: OutcomeFile[];
  /** True when this PR's file list could not be fetched (best-effort per PR). */
  files_error: boolean;
}

export interface SessionOutcomes {
  owner: string;
  name: string;
  trigger_issue: number;
  prs: PrOutcome[];
}

// ---- GET /api/v1/logs/{session_id}/manifest & /file -------------------------
// The in-bundle log viewer (spec B4). Bundle files are already redacted.

export interface LogFileEntry {
  /** In-bundle path, e.g. `fkst-substrate/codex/codex.log`. */
  path: string;
  size: number;
  /** Fixed classification: `Driver` | `Supervise` | `Codex` | `Misc` | `README` | `Meta`. */
  label: string;
}

export interface LogManifest {
  session_id: string;
  files: LogFileEntry[];
  generated_at: string;
}

export interface LogFileContent {
  session_id: string;
  path: string;
  content: string;
  total_bytes: number;
  returned_bytes: number;
  truncated: boolean;
}

// ---- GET /api/v1/sessions/{session_id}/observe (raw engine JSON) ------------
// The live engine read-model. The backend returns the engine's own JSON
// verbatim (spec B5 — no server-side shaping), so every field is optional and
// the UI renders only what is present. NEVER assume a field exists.

export interface ObserveQueue {
  // The engine names the queue in a `queue` field (e.g.
  // "workflow-writer.workflow_writer_materialization_tick").
  queue?: string;
  depth?: number;
  pending?: number;
  in_flight?: number;
  retrying?: number;
  oldest_pending_age_ms?: number | null;
}

export interface ObserveSnapshot {
  queues?: ObserveQueue[];
  // The delivery-queue snapshot (pending/retrying deliveries); the engine emits
  // it as `deliveries`, not `codex_runs`.
  deliveries?: unknown[];
  dead_letters?: unknown[];
  [k: string]: unknown;
}

// ---- Shared -----------------------------------------------------------------

/** The backend's uniform error body. */
export interface ErrorEnvelope {
  error: string;
  message: string;
}
