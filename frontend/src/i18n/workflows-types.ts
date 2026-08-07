/**
 * A session's scheduled workflows: what they are, when they fire next, and what
 * they are doing or last did.
 *
 * The surface lives inside the session detail's Workflows tab — a schedule is
 * assigned to a session creator and runs inside that session's pod, so there is
 * no route and no repository picker to name here.
 *
 * Two conventions, both shared with `/operations`. **Enum records are
 * exhaustive** — each `Record<..., string>` below is keyed by the exact wire
 * vocabulary the backend can return, so a value that reaches the UI always has a
 * name (the parity test asserts both languages carry the same keys). And **no
 * cadence arithmetic lives here**: every firing time comes from the API, because
 * a second implementation in TypeScript would eventually disagree with the clock
 * and the UI would confidently show a time the schedule does not honour.
 */
export interface WorkflowsContent {
  /** Accessible name for the tab-level loading state. */
  loading: string;

  /** Empty and error states. */
  emptyTitle: string;
  emptyBody: string;
  emptyAction: string;
  notInstalled: string;
  loadFailed: string;
  retry: string;

  /** The rail: this session's schedules, and the ones routed to no session. */
  railTitle: string;
  railAria: string;
  unroutedTitle: string;
  unroutedBody: string;
  /** Shown in the detail pane when the rail holds only unrouted schedules, none
   *  of which is selectable. */
  unroutedOnly: string;

  /** Detail-pane field labels. */
  cadenceLabel: string;
  successLabel: string;

  /** Lifecycle badges, keyed by the API's `state` vocabulary. */
  lifecycle: Record<'idle' | 'running' | 'paused' | 'invalid', string>;
  /** Run statuses, keyed by the `fkst-cron-run:v1` status vocabulary. */
  runStatus: Record<'dispatched' | 'ok' | 'failed' | 'timeout' | 'skipped-overlap', string>;
  /** Per-step statuses. */
  stepStatus: Record<'ok' | 'failed' | 'skipped', string>;

  /** Relative next-run rendering. `{d}`/`{h}`/`{m}` are substituted. */
  inDays: string;
  inHours: string;
  inMinutes: string;
  imminent: string;
  overdue: string;
  never: string;

  /** Detail view. */
  upcoming: string;
  argumentsTitle: string;
  noArguments: string;
  latestRunTitle: string;
  earlierRunsTitle: string;
  noRuns: string;
  noSteps: string;
  /** What a run still in flight can honestly say about its steps: the runner
   *  posts one record at the end, so there is nothing finer to report yet. */
  awaitingSteps: string;
  /** A run's live age. `{d}` is substituted with a formatted duration. */
  runningFor: string;
  openOnGithub: string;
  openRunIssue: string;
  /** The one line that explains why there is no inline cadence editor. */
  editHint: string;

  /** Actions. */
  actionRunNow: string;
  actionPause: string;
  actionResume: string;
  actionBusy: string;
  actionFailed: string;

  /** Labels inside the run list and the stepper. */
  manual: string;
  stepperAria: string;
  runsAria: string;
}
