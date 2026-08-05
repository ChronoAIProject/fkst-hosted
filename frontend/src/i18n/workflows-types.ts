/**
 * The `/workflows` workspace: a repository's scheduled workflows, their next
 * firings, and their run history.
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

  /** Repository picker. */
  repoLabel: string;
  repoPlaceholder: string;
  repoHint: string;

  /** Empty and error states. */
  emptyTitle: string;
  emptyBody: string;
  emptyAction: string;
  notInstalled: string;
  loadFailed: string;
  retry: string;

  /** List columns. */
  colWorkflow: string;
  colCadence: string;
  colNextRun: string;
  colState: string;
  colLastRun: string;
  colSuccess: string;

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
  detailBack: string;
  upcoming: string;
  argumentsTitle: string;
  noArguments: string;
  runsTitle: string;
  noRuns: string;
  stepsTitle: string;
  noSteps: string;
  openOnGithub: string;
  runIssue: string;
  /** The one line that explains why there is no inline cadence editor. */
  editHint: string;

  /** Actions. */
  actionRunNow: string;
  actionPause: string;
  actionResume: string;
  actionBusy: string;
  actionFailed: string;
  runNowStarted: string;

  /** Column/aria labels inside the run list and the stepper. */
  slot: string;
  duration: string;
  manual: string;
  detailColumn: string;
  stepperAria: string;
  runsAria: string;
  schedulesAria: string;
}
