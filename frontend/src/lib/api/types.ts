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
  /** True when this deployment offers the broader (classic-OAuth) credential
   *  that unlocks repos/orgs where the fkst App is NOT installed. Drives the
   *  connect affordance on the canvas; when false the feature is off and the
   *  affordance is never rendered. */
  broader_oauth_available: boolean;
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
  /** GitHub logins allowed to download this session's logs. Optional: a later
   *  backend/UI item populates it, so it is harmless/undefined until then. */
  log_access?: string[] | null;
  /** GitHub logins granted WORK-ITEM authority over this session (they may
   *  raise/label/comment on its work issues). A DISTINCT list from
   *  {@link log_access} (log-download access). Optional: undefined until the
   *  backend populates it. */
  collaborators?: string[] | null;
  /** Preferred natural-language for the session's output. Optional: populated
   *  by a later item, undefined until then. */
  output_lang?: string | null;
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
  /** GitHub logins granted work-item authority (`### Session Collaborators`);
   *  distinct from `log_access`. */
  collaborators?: string[];
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

// ---- /api/v1/users/me/environment-profiles ----------------------------------
// The named-environment REST API (backend `routes/environments.rs`). Each named
// environment bundles ordered install commands, non-secret variables, and
// write-only secrets. Secret VALUES never cross the wire in any direction that
// echoes them back — a view exposes secret KEY names only. Names are snake_case
// exactly as the Rust `#[derive(Serialize/Deserialize)]` DTOs emit them.

/** Anchored environment-NAME pattern (mirrors the backend's `env_name_regex`),
 *  so the composed `fkst-env-<id>-<name>` object stays a valid k8s name. Used by
 *  the UI to validate a name before the (slow) PUT round-trip. */
export const ENV_NAME_REGEX = /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/;
/** The backend's `MAX_NAME_LEN`: the ceiling on an environment `name`. */
export const ENV_MAX_NAME_LEN = 40;

/** A compact list-view of one named environment: counts only, no contents. */
export interface EnvironmentProfileSummary {
  name: string;
  status: string;
  validated_at: string;
  install_command_count: number;
  variable_count: number;
  secret_count: number;
}

/** The full view of one named environment. Secret VALUES are deliberately
 *  absent — only their key NAMES appear (`secret_keys`), locked server-side. */
export interface EnvironmentProfileView {
  name: string;
  status: string;
  validated_at: string;
  install: string[];
  variables: Record<string, string>;
  /** Secret key NAMES only — values are write-only and never returned. */
  secret_keys: string[];
}

/** PUT body: the desired environment contents. Every field is required on the
 *  wire but the backend defaults each to empty, so an empty map/array is valid.
 *  `secrets` VALUES are write-only (accepted here, never echoed back). */
export interface EnvironmentProfileSpec {
  install: string[];
  variables: Record<string, string>;
  secrets: Record<string, string>;
}

/** The detailed `422` body PUT returns when the install commands fail
 *  validation in the isolated pod (nothing is persisted). Distinct from a plain
 *  `ErrorEnvelope` 422 by its fixed `error: "install_validation_failed"`. */
export interface InstallValidationError {
  error: string;
  message: string;
  /** Zero-based index of the failing command (0 when the run timed out). */
  failed_command_index: number;
  /** The exact command that failed (empty when the run timed out). */
  failed_command: string;
  /** The command's exit code (`-1` when unknown / timed out). */
  exit_code: number;
  /** Whether the sequence exceeded the validation deadline. */
  timed_out: boolean;
  /** Trailing bytes of the failing command's stderr (bounded by the backend). */
  stderr_tail: string;
}

// ---- Shared -----------------------------------------------------------------

/** The backend's uniform error body. */
export interface ErrorEnvelope {
  error: string;
  message: string;
}
