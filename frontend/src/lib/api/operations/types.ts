// The wire contract of the two operations endpoints, transcribed from the
// backend DTOs (`backend/src/routes/operations/{dto,sandbox_dto}.rs`).
//
// Every closed vocabulary is declared as a `const` tuple rather than a bare
// union type: the tuple is what the filter selects iterate, what the URL codec
// validates against, and what the i18n catalogs are keyed by — so a value the
// server can return but the UI cannot name becomes a compile error rather than
// a blank cell.
//
// Nothing here is optional-by-omission on purpose: the sandbox contract
// serializes every unknown as an explicit `null` so "the backend cannot know
// this" and "this happens to be absent" stay distinguishable, and the row
// renderers depend on that distinction (a `null` restart count renders
// "Not reported", never `0`).

/** Activity scope words. `mine` is a regular caller's only option. */
export const ACTIVITY_SCOPES = ['mine', 'all'] as const;
export type ActivityScope = (typeof ACTIVITY_SCOPES)[number];

/** Sandbox scope words. `accessible` is a regular caller's only option. */
export const SANDBOX_SCOPES = ['accessible', 'all'] as const;
export type SandboxScope = (typeof SANDBOX_SCOPES)[number];

/** Which record kinds one activity query asks for. */
export const RECORD_KINDS = ['api_request', 'sandbox_lifecycle', 'all'] as const;
export type RecordKindFilter = (typeof RECORD_KINDS)[number];

/** The discriminator each returned activity row carries. */
export const ROW_KINDS = ['api_request', 'sandbox_lifecycle'] as const;
export type RowKind = (typeof ROW_KINDS)[number];

export const METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE'] as const;
export type Method = (typeof METHODS)[number];

export const STATUS_CLASSES = ['2xx', '3xx', '4xx', '5xx'] as const;
export type StatusClass = (typeof STATUS_CLASSES)[number];

export const OUTCOMES = [
  'success',
  'redirect',
  'client_error',
  'server_error',
  'timeout',
  'rejected',
  'incomplete',
] as const;
export type Outcome = (typeof OUTCOMES)[number];

export const DELIVERY_STATES = [
  'verified_in_posthog',
  'accepted_pending_verification',
  'queued',
  'incomplete',
  'dead_letter',
] as const;
export type DeliveryState = (typeof DELIVERY_STATES)[number];

export const ACTIVITY_SOURCES = ['posthog', 'relay'] as const;
export type ActivitySource = (typeof ACTIVITY_SOURCES)[number];

export const SOURCE_HEALTHS = ['healthy', 'degraded', 'unavailable', 'not_configured'] as const;
export type SourceHealth = (typeof SOURCE_HEALTHS)[number];

/** Bounded codes explaining why an activity page is partial. */
export const SOURCE_MESSAGES = [
  'posthog_unavailable',
  'relay_unavailable',
  'activity_rows_dropped',
] as const;
export type SourceMessage = (typeof SOURCE_MESSAGES)[number];

export const ACTOR_KINDS = [
  'github_user',
  'github_webhook_sender',
  'anonymous',
  'service',
  'system',
] as const;
export type ActorKind = (typeof ACTOR_KINDS)[number];

export const PRINCIPAL_KINDS = [
  'github_user_token',
  'oauth_session',
  'github_app_installation',
  'webhook_hmac',
  'reconciler',
  'anonymous',
  'none',
] as const;
export type PrincipalKind = (typeof PRINCIPAL_KINDS)[number];

export const LIFECYCLE_ACTIONS = [
  'create_requested',
  'created',
  'create_failed',
  'delete_requested',
  'deleted',
  'delete_failed',
  'identity_backfilled',
  'identity_conflict',
] as const;
export type LifecycleAction = (typeof LIFECYCLE_ACTIONS)[number];

export const SANDBOX_STATUSES = [
  'pending',
  'running',
  'paused',
  'transitioning',
  'succeeded',
  'failed',
  'terminating',
  'terminated',
  'unknown',
] as const;
export type SandboxStatus = (typeof SANDBOX_STATUSES)[number];

export const SANDBOX_BACKENDS = ['kubernetes', 'opensandbox'] as const;
export type SandboxBackend = (typeof SANDBOX_BACKENDS)[number];

export const ATTRIBUTION_SOURCES = [
  'launch_metadata',
  'backfilled_current_trigger',
  'partial_metadata',
  'unknown_legacy',
  'conflict',
] as const;
export type AttributionSource = (typeof ATTRIBUTION_SOURCES)[number];

export const SANDBOX_WARNINGS = [
  'missing_session_id',
  'malformed_correlation',
  'malformed_identity',
  'attribution_conflict',
  'missing_created_at',
  'malformed_created_at',
  'malformed_last_pending',
  'clock_skew',
  'lifetime_overflow',
  'unknown_status',
  'warnings_incomplete',
] as const;
export type SandboxWarning = (typeof SANDBOX_WARNINGS)[number];

/** The initiating identity. `id` is the only ownership proof; `login` is a
 *  historical snapshot and is display-only. */
export interface ActorView {
  kind?: string | null;
  id?: number | null;
  login?: string | null;
}

/** The executing identity. Never a credential or a token fingerprint. */
export interface PrincipalView {
  kind?: string | null;
  id?: string | null;
}

/** The correlation keys a record carries. */
export interface CorrelationView {
  session_id?: string | null;
  repo_full_name?: string | null;
  installation_id?: number | null;
  trigger_issue?: number | null;
  request_id?: string | null;
  webhook_delivery_id?: string | null;
}

/** One recorded API request. */
export interface ApiRequestRow {
  record_kind: 'api_request';
  event_id: string;
  request_id?: string | null;
  started_at?: string | null;
  completed_at: string;
  method: string;
  /** The normalized route template. NEVER a raw URI — the server refuses to
   *  record one, and the renderers refuse to invent one. */
  route_template: string;
  operation_id: string;
  actor: ActorView;
  principal: PrincipalView;
  /** The operation's own allowlisted safe arguments, verbatim. */
  arguments: Record<string, unknown>;
  arguments_parse_status?: string | null;
  status_code?: number | null;
  outcome: string;
  error_code?: string | null;
  duration_ms?: number | null;
  correlation: CorrelationView;
  delivery_state: string;
  source: string;
}

/** One recorded sandbox lifecycle transition. It has no HTTP method, status, or
 *  duration, and the renderers must never fabricate them. */
export interface SandboxLifecycleRow {
  record_kind: 'sandbox_lifecycle';
  event_id: string;
  occurred_at: string;
  lifecycle_action: string;
  actor: ActorView;
  principal: PrincipalView;
  session_id: string;
  backend?: string | null;
  runtime_id?: string | null;
  creator: ActorView;
  trigger_author: ActorView;
  correlation: CorrelationView;
  created_at?: string | null;
  reason_code?: string | null;
  delivery_state: string;
  source: string;
}

export type ActivityRow = ApiRequestRow | SandboxLifecycleRow;

/** Bounded per-source deployment health for one page. Never a statistic about
 *  records the caller may not see. */
export interface SourceStatus {
  posthog: string;
  relay: string;
  partial: boolean;
  message_code?: string | null;
}

/** One keyset page of activity. */
export interface ActivityPage {
  queried_at: string;
  from: string;
  to: string;
  effective_scope: ActivityScope;
  can_view_all: boolean;
  items: ActivityRow[];
  next_cursor?: string | null;
  source_status: SourceStatus;
}

/** The caller's own normalized sandbox filters, echoed back. */
export interface SandboxFiltersView {
  status: string | null;
  backend: string | null;
  creator_id: number | null;
  creator_login: string | null;
  repo_full_name: string | null;
  session_id: string | null;
  trigger_issue: number | null;
  attribution_source: string | null;
}

/** One live FKST-managed runtime the caller is authorized to see. */
export interface SandboxRow {
  backend: string;
  runtime_id: string;
  runtime_name: string | null;
  runtime_uid: string | null;
  backend_location: string | null;
  session_id: string | null;
  managed: boolean;
  metadata_state: string;
  creator_id: number | null;
  creator_login: string | null;
  trigger_author_id: number | null;
  trigger_author_login: string | null;
  attribution_source: string;
  repo_full_name: string | null;
  installation_id: number | null;
  trigger_issue: number | null;
  status: string;
  raw_status: string;
  status_reason: string | null;
  status_message: string | null;
  created_at: string | null;
  age_seconds: number | null;
  /** `null` means the deployment configured an UNLIMITED lifetime. It is never
   *  the same thing as `0`. */
  max_lifetime_seconds: number | null;
  expires_at: string | null;
  remaining_seconds: number | null;
  minimum_lifetime_seconds: number;
  minimum_lifetime_remaining_seconds: number | null;
  idle_grace_seconds: number;
  last_pending_at: string | null;
  idle_for_seconds: number | null;
  /** `null` — never zero — when the backend has no restart concept. */
  restart_count: number | null;
  last_transition_at: string | null;
  deletion_timestamp: string | null;
  warning_codes: string[];
}

/** One complete authorized live snapshot. */
export interface SandboxInventory {
  observed_at: string;
  backend: string;
  effective_scope: SandboxScope;
  can_view_all: boolean;
  item_count: number;
  filters_applied: SandboxFiltersView;
  items: SandboxRow[];
  warning_codes: string[];
}

/** Narrow an arbitrary string to a member of a closed vocabulary, or `null`.
 *  Used everywhere a server value drives a localized label: an unrecognized
 *  value falls back to a stable raw rendering instead of a blank cell. */
export function asMember<T extends string>(
  vocabulary: readonly T[],
  value: unknown
): T | null {
  return typeof value === 'string' && (vocabulary as readonly string[]).includes(value)
    ? (value as T)
    : null;
}
