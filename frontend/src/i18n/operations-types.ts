import type { OperationsErrorCode } from '@/lib/api/operations';

/**
 * The `/operations` workspace.
 *
 * Two conventions run through it. **Enum records are exhaustive** — every
 * `Record<..., string>` below is keyed by the exact wire vocabulary the backend
 * can return, so a value that reaches the UI always has a name (the parity test
 * asserts both languages carry the same keys). And **error copy is keyed by the
 * stable backend CODE, never by the backend's message**: the message is written
 * for an operator, is untranslated, and may name internal machinery, so it never
 * enters the DOM.
 */
export interface OperationsContent {
  /** document.title for the route. */
  metaTitle: string;
  /** Topbar navigation label. */
  nav: string;
  title: string;
  /** Accessible name for the route-level loading skeleton. */
  loading: string;

  /** Cold sign-in gate (the route is authenticated-only). */
  gateTitle: string;
  gateBody: string;
  gateAction: string;
  /** Shown when no API base URL is configured for this build. */
  unconfiguredTitle: string;
  unconfiguredBody: string;
  /** Involuntary session loss, keeping the workspace mounted. */
  expiredTitle: string;
  expiredBody: string;
  expiredAction: string;

  tabsAria: string;
  tabActivity: string;
  tabSandboxes: string;

  /** Segmented scope control (rendered only when the server says the caller may
   *  select the global scope). */
  scopeAria: string;
  scopeAll: string;
  scopeMine: string;
  scopeAccessible: string;
  /** Eyebrow preceding the effective-scope name. */
  effectiveScope: string;
  headingActivityMine: string;
  headingActivityAll: string;
  headingSandboxAccessible: string;
  headingSandboxAll: string;

  filtersAria: string;
  refresh: string;
  refreshing: string;
  resetFilters: string;
  retry: string;
  anyOption: string;
  /** `{names}` = the ignored search-parameter names. */
  ignoredParams: string;
  /** Shown after a denied global request normalized the view. */
  scopeReset: string;

  /** Activity filter labels. */
  fRange: string;
  rangePreset: Record<'1h' | '24h' | '7d' | '30d' | 'custom', string>;
  fFrom: string;
  fTo: string;
  rangeHint: string;
  rangeInvalid: string;
  fRecordKind: string;
  recordKindFilter: Record<'api_request' | 'sandbox_lifecycle' | 'all', string>;
  fActorId: string;
  fActorLogin: string;
  fOperation: string;
  operationGroup: Record<
    'canvas' | 'sessions' | 'environments' | 'auth' | 'operations' | 'system',
    string
  >;
  fMethod: string;
  fStatusClass: string;
  fStatusCode: string;
  fOutcome: string;
  fRepo: string;
  fTriggerIssue: string;
  fSessionId: string;
  fRequestId: string;
  /** Blocking state when a personal lifecycle query names no session. */
  sessionRequiredTitle: string;
  sessionRequiredBody: string;

  /** Sandbox filter labels. */
  fStatus: string;
  fBackend: string;
  fCreatorId: string;
  fCreatorLogin: string;
  fAttribution: string;
  /** States plainly that filters narrow, never widen, what the server allows. */
  filterScopeNote: string;

  /** Activity table. */
  activityTableAria: string;
  colTime: string;
  colRecordKind: string;
  colActor: string;
  colPrincipal: string;
  colMethod: string;
  colOperation: string;
  colArguments: string;
  colStatus: string;
  colDuration: string;
  colCorrelation: string;
  colDelivery: string;
  colRequestId: string;

  /** Sandbox table. */
  sandboxTableAria: string;
  colSandboxStatus: string;
  colSandboxId: string;
  colCreator: string;
  colRepository: string;
  colCreated: string;
  colAge: string;
  colLifetime: string;
  colIdle: string;
  colRestarts: string;
  colTransition: string;

  loadOlder: string;
  loadingOlder: string;
  noMore: string;
  pollPaused: string;

  /** Enumerated vocabularies. */
  recordKind: Record<'api_request' | 'sandbox_lifecycle', string>;
  outcome: Record<
    | 'success'
    | 'redirect'
    | 'client_error'
    | 'server_error'
    | 'timeout'
    | 'rejected'
    | 'incomplete',
    string
  >;
  delivery: Record<
    | 'verified_in_posthog'
    | 'accepted_pending_verification'
    | 'queued'
    | 'incomplete'
    | 'dead_letter',
    string
  >;
  actorKind: Record<
    'github_user' | 'github_webhook_sender' | 'anonymous' | 'service' | 'system',
    string
  >;
  principalKind: Record<
    | 'github_user_token'
    | 'oauth_session'
    | 'github_app_installation'
    | 'webhook_hmac'
    | 'reconciler'
    | 'anonymous'
    | 'none',
    string
  >;
  lifecycleAction: Record<
    | 'create_requested'
    | 'created'
    | 'create_failed'
    | 'delete_requested'
    | 'deleted'
    | 'delete_failed'
    | 'identity_backfilled'
    | 'identity_conflict',
    string
  >;
  sandboxStatus: Record<
    | 'pending'
    | 'running'
    | 'paused'
    | 'transitioning'
    | 'succeeded'
    | 'failed'
    | 'terminating'
    | 'terminated'
    | 'unknown',
    string
  >;
  backendKind: Record<'kubernetes' | 'opensandbox', string>;
  attribution: Record<
    | 'launch_metadata'
    | 'backfilled_current_trigger'
    | 'partial_metadata'
    | 'unknown_legacy'
    | 'conflict',
    string
  >;
  warning: Record<
    | 'missing_session_id'
    | 'malformed_correlation'
    | 'malformed_identity'
    | 'attribution_conflict'
    | 'missing_created_at'
    | 'malformed_created_at'
    | 'malformed_last_pending'
    | 'clock_skew'
    | 'lifetime_overflow'
    | 'unknown_status'
    | 'warnings_incomplete',
    string
  >;
  sourceHealth: Record<'healthy' | 'degraded' | 'unavailable' | 'not_configured', string>;
  sourceMessage: Record<
    'posthog_unavailable' | 'relay_unavailable' | 'activity_rows_dropped',
    string
  >;
  metadataState: Record<'complete' | 'partial' | 'malformed', string>;
  argumentsParseStatus: Record<'parsed' | 'invalid' | 'not_applicable' | 'unavailable', string>;

  /** Freshness / source status. */
  sourcesLabel: string;
  posthogLabel: string;
  relayLabel: string;
  /** `{time}` = the localized instant. */
  queriedAt: string;
  observedAt: string;
  partialNotice: string;
  staleNotice: string;
  inventoryWarnings: string;

  /** Empty and failure states. */
  emptyActivity: string;
  emptySandboxes: string;
  errorTitle: string;
  /** `{id}` = the propagated request id, when one was exposed. */
  errorRequestId: string;
  /** Keyed by the STABLE error code — exhaustively, so every failure the client
   *  can produce has copy and no backend message is ever rendered. */
  errorMessage: Record<OperationsErrorCode, string>;

  /** Values. */
  unknownCreator: string;
  notReported: string;
  unlimited: string;
  expired: string;
  /** `{duration}` = a formatted span. */
  remaining: string;
  neverPending: string;
  systemActor: string;
  noArguments: string;

  /** Row details. */
  detailsAria: string;
  openDetails: string;
  closeDetails: string;
  detailIdentity: string;
  detailCorrelation: string;
  detailArguments: string;
  detailDelivery: string;
  detailRuntime: string;
  detailLifetime: string;
  detailStatus: string;
  dActorId: string;
  dActorLogin: string;
  dActorKind: string;
  dPrincipal: string;
  dEventId: string;
  dRequestId: string;
  dSessionId: string;
  dRepo: string;
  dInstallation: string;
  dTriggerIssue: string;
  dWebhookDelivery: string;
  dStartedAt: string;
  dCompletedAt: string;
  dOccurredAt: string;
  dRoute: string;
  dOperation: string;
  dErrorCode: string;
  dReasonCode: string;
  dSource: string;
  dDeliveryState: string;
  dRuntimeId: string;
  dRuntimeName: string;
  dRuntimeUid: string;
  dLocation: string;
  dRawStatus: string;
  dStatusReason: string;
  dStatusMessage: string;
  dCreatedAt: string;
  dExpiresAt: string;
  dMinLifetime: string;
  dIdleGrace: string;
  dLastPending: string;
  dLastTransition: string;
  dDeletionAt: string;
  dRestarts: string;
  dMetadataState: string;
  dAttribution: string;
  dTriggerAuthor: string;
  dManaged: string;
  yes: string;
  no: string;

  /** Copy affordances + the cross-link. */
  copyEventId: string;
  copyRequestId: string;
  copySessionId: string;
  copyRuntimeId: string;
  viewActivity: string;
  viewActivityAria: string;
}
