import type { SessionHealth, SessionPhase, WorkItemState } from '@/lib/api/derive';
import type {
  SessionRecoveryReason,
  SessionRecoveryState,
  SessionRuntimeState,
} from '@/lib/api/types';

/**
 * The per-session detail drawer (status / packages / logs / outcomes).
 *
 * Its enum records are keyed by the DERIVED vocabularies in `@/lib/api/derive`
 * and the wire states in `@/lib/api/types`, so a phase, health, or recovery
 * state that reaches the UI always has a localized name.
 */
export interface SessionDetailContent {
  /** "Details" action on a session card + its aria label (`{name}`). */
  open: string;
  openAria: string;
  /** Drawer heading + close affordance. */
  title: string;
  close: string;
  closeAria: string;
  tabsAria: string;
  tabStatus: string;
  tabPackages: string;
  tabLogs: string;
  tabOutcomes: string;
  tabHealth: string;
  /** This session's scheduled workflows: a schedule is assigned to a session
   *  creator and runs inside that session's pod, so it is looked at here. */
  tabWorkflows: string;
  /** Live runtime-observation tab, split out of Status so the lifecycle view
   *  issues no pod exec. */
  tabEngine: string;
  /** Copy-affordance label on the full session id in the drawer header. */
  sessionIdCopy: string;
  creatorLabel: string;
  sourceBranchLabel: string;
  targetBranchLabel: string;
  repoDefault: string;
  // ---- Status tab ----
  lifecycle: string;
  /** Decoded lifecycle phase labels — one per `SessionPhase`. */
  phase: Record<SessionPhase, string>;
  /** Stage-strip label shown at the 'active' node when the session is idle
   *  (paused): it ran but its pod was reaped for lack of work. */
  stagePaused: string;
  healthLabel: string;
  /** Decoded health labels — one per `SessionHealth`. */
  health: Record<SessionHealth, string>;
  recoveryDiagnostics: string;
  recoveryState: Record<SessionRecoveryState, string>;
  recoveryReason: Record<SessionRecoveryReason, string>;
  recoveryOpenWork: string;
  recoveryRuntime: string;
  runtimeState: Record<SessionRuntimeState, string>;
  workItems: string;
  noWorkItems: string;
  /** Decoded work-item state labels — one per `WorkItemState`. */
  work: Record<WorkItemState, string>;
  // ---- Status tab: overview cards (progress + distribution donut) ----
  /** Progress card eyebrow. */
  overviewProgress: string;
  /** Work-distribution donut card eyebrow + aria label. */
  overviewDistribution: string;
  /** Aggregate stat label for the in-progress group (thinking + implementing
   *  + claimed) — no single `work.*` label spans the group. */
  statInProgress: string;
  /** Caption under the donut's centered total. */
  donutTotalLabel: string;
  /** Friendly note shown in the donut card when there are no work items
   *  (distinct from `noWorkItems` so the two never collide on screen). */
  donutEmpty: string;
  // ---- Status tab: session timeline ----
  /** Timeline card eyebrow. */
  timeline: string;
  /** First timeline node — the trigger issue's creation. */
  timelineStarted: string;
  /** A work issue entering the queue (its creation). */
  timelineWorkQueued: string;
  /** A work issue closing (finished / no longer worked). */
  timelineWorkFinished: string;
  /** A pull request being opened. */
  timelinePrOpened: string;
  /** A pull request merging. */
  timelinePrMerged: string;
  /** A pull request closing without a merge. */
  timelinePrClosed: string;
  /** The terminal "current state" node prefix (e.g. "Now — Idle"). */
  timelineNow: string;
  liveEngine: string;
  liveEngineLoading: string;
  /** Note that observe is a slow pod exec. */
  liveEngineSlow: string;
  liveEngineEmpty: string;
  /** Calm note shown in place of the observe fetch when the pod is NOT live
   *  (the Status tab gates the live-engine fetch on `liveness === 'live'`). */
  liveEnginePaused: string;
  /** Runtime-specific note while durable work is waiting for recovery. */
  liveEngineRecovering: string;
  /** Defensive observe-error fallback (non-409): the live-engine read is
   *  only available while the pod runs. */
  liveEngineNotLive: string;
  /** Observe-error message for HTTP 409 — the session has no durable
   *  delivery store to observe. */
  liveEngineErrorNoStore: string;
  queues: string;
  queueDepth: string;
  queuePending: string;
  queueInFlight: string;
  queueRetrying: string;
  /** Pending-delivery count line; `{n}` placeholder. */
  deliveries: string;
  /** Dead-letter count line; `{n}` placeholder. */
  deadLetters: string;
  // ---- Packages tab ----
  packagesNone: string;
  packageRefAria: string;
  /** Copy-affordance label on each package `<code>` ref. */
  packageRefCopy: string;
  queueActivity: string;
  // ---- Packages tab: frozen-config panel ----
  configLabel: string;
  /** Note that the config below is frozen at registration. */
  configFrozenNote: string;
  configWorkLabel: string;
  configEnvironment: string;
  configAutoMerge: string;
  configOutputLang: string;
  /** fkst-manifest references (`### Manifest`) frozen at registration. */
  configManifest: string;
  /** Rendered when the manifest list is empty. */
  configManifestNone: string;
  configLogAccess: string;
  /** Work-item authority list (`### Session Collaborators`); distinct from
   *  the log-access allowlist. */
  configCollaborators: string;
  /** Placeholder for a scalar the session did not carry. */
  configUnset: string;
  configYes: string;
  configNo: string;
  /** Rendered when the log-access allowlist is empty. */
  configLogAccessNone: string;
  /** Rendered when the collaborators list is empty. */
  configCollaboratorsNone: string;
  // ---- Logs tab ----
  logsUnavailable: string;
  logsLoading: string;
  logsError: string;
  /** Logs manifest error for HTTP 503 — log storage isn't configured for
   *  this deployment. */
  logsErrorNoStorage: string;
  logsEmpty: string;
  logsFilesAria: string;
  /** aria-label on the selected-file detail pane of the Logs master/detail
   *  split (mirrors `healthDetailAria`). */
  logsDetailAria: string;
  logsSelectFile: string;
  logsFileLoading: string;
  logsFileError: string;
  logsSearchPlaceholder: string;
  /** Match count for the in-file find; `{n}` placeholder. */
  logsSearchCount: string;
  logsRefresh: string;
  /** Logs Refresh button label while the re-fetch is in flight. */
  logsRefreshing: string;
  /** Retry affordance on a failed manifest load. */
  logsRetry: string;
  /** Tail notice; `{shown}` / `{total}` placeholders (already formatted). */
  logsTruncated: string;
  /** Match count when the shown content is only a tail; `{n}` placeholder.
   *  Nudges the reader to load the full file to search everything. */
  logsSearchCountTail: string;
  /** Action that fetches the whole file (drops the tail window). */
  logsLoadFull: string;
  /** In-flight label while the full file loads. */
  logsLoadingFull: string;
  /** Shown when a failed Refresh left the last-good content on screen. */
  logsStale: string;
  /** Copy-affordance label on the shown log file's name. */
  logsFilenameCopy: string;
  logsDownloadBundle: string;
  // ---- Logs tab: per-run picker ----
  /** Eyebrow / accessible name for the run picker (a session is served by a
   *  sequence of pod incarnations — "runs"). */
  runPicker: string;
  /** Label for the current, still-running incarnation; `{start}` = its SGT
   *  start time. */
  runRunning: string;
  /** Compact label for a legacy session's single synthetic run (no window). */
  runLatest: string;
  /** Non-blocking notice when the run list failed to load and the tab fell
   *  back to the latest bundle. */
  runsError: string;
  // ---- Outcomes tab ----
  outcomesLoading: string;
  outcomesError: string;
  outcomesEmpty: string;
  outcomesFilesError: string;
  outcomesNoFiles: string;
  /** File status chip labels — known GitHub statuses; unknown falls back. */
  fileStatus: Record<'added' | 'modified' | 'removed' | 'renamed', string>;
  /** `{n}` placeholder. */
  additionsAria: string;
  /** `{n}` placeholder. */
  deletionsAria: string;
  /** `{from}` placeholder. */
  renamedFrom: string;
  /** Per-file changed-line count shown next to the row. `{n}` placeholder. */
  sizeLines: string;
  preview: string;
  previewClose: string;
  /** Explicit "fetch and preview this file's bytes" affordance — the fetch
   *  is deferred behind this click so a row never streams media blind. */
  previewLoad: string;
  previewLoading: string;
  previewError: string;
  previewTooLarge: string;
  previewBinary: string;
  openOnGithub: string;
  download: string;
  /** Outcome-file download button label while the blob is being fetched. */
  downloadPending: string;
  /** `{name}` placeholder. */
  downloadAria: string;
  logsNone: string;
  /** Match count for the in-file find; `{n}` placeholder. */
  /** Retry affordance on a failed manifest load. */
  /** Tail notice; `{shown}` / `{total}` placeholders (already formatted). */
  /** Match count when the shown content is only a tail; `{n}` placeholder.
   *  Nudges the reader to load the full file to search everything. */
  /** Action that fetches the whole file (drops the tail window). */
  /** In-flight label while the full file loads. */
  /** Shown when a failed Refresh left the last-good content on screen. */
  /** Copy-affordance label on the shown log file's name. */
  // ---- Logs tab: per-run picker ----
  /** Eyebrow / accessible name for the run picker (a session is served by a
   *  sequence of pod incarnations — "runs"). */
  /** Label for the current, still-running incarnation; `{start}` = its SGT
   *  start time. */
  /** Compact label for a legacy session's single synthetic run (no window). */
  /** Non-blocking notice when the run list failed to load and the tab fell
   *  back to the latest bundle. */
  // ---- Health tab ----
  /** Report status labels, keyed by the v1 taxonomy. */
  healthStatus: Record<
    'working' | 'idle' | 'blocked' | 'stalled' | 'failing' | 'unknown',
    string
  >;
  /** Heartbeat verdict labels, keyed by the API's staleness state. */
  healthStaleness: Record<'not_running' | 'never_reported' | 'fresh' | 'stale', string>;
  /** Header chip label when the heartbeat is stale (overrides the status). */
  healthStaleChip: string;
  healthCurrent: string;
  healthEvidence: string;
  healthHistory: string;
  healthBody: string;
  /** aria-label on the rendered (untrusted) report body region. */
  healthBodyAria: string;
  healthProducer: string;
  healthConfidence: string;
  /** `{n}` placeholder — whole minutes since the newest report. */
  healthLastReport: string;
  /** Shown instead when the age could not be determined. */
  healthLastReportUnknown: string;
  /** The stale callout: heading + body with `{expected}` and `{age}` minutes. */
  healthStaleTitle: string;
  healthStaleBody: string;
  healthLoading: string;
  /** Calm empty state: the first report has not landed yet. */
  healthNeverReported: string;
  /** Calm empty state: the session is not running, so it is not reporting. */
  healthNotRunning: string;
  /** 503 — health reporting is not configured for this deployment. */
  healthUnavailable: string;
  healthError: string;
  healthRetry: string;
  /** aria-label on the report-history list. */
  healthHistoryAria: string;
  /** aria-label on the selected-report detail pane. */
  healthDetailAria: string;
}
