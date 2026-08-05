import type { WorkflowsSlice } from '../slices';

export const workflows: WorkflowsSlice = {
  metaTitle: 'Scheduled workflows — FKST',
  nav: 'Workflows',
  title: 'Scheduled workflows',
  loading: 'Loading scheduled workflows',

  gateTitle: 'Sign in to see your scheduled workflows',
  gateBody:
    'A scheduled workflow runs a workflow from your repository on a cadence — once, or on a cron schedule. Sign in with GitHub to see the ones you can reach.',
  gateAction: 'Sign in with GitHub',
  unconfiguredTitle: 'No API configured',
  unconfiguredBody:
    'This build has no API base URL, so no request can be made. Set `VITE_FKST_API_BASE` at build time.',

  repoLabel: 'Repository',
  repoPlaceholder: 'owner/name',
  repoHint: 'The repository whose scheduled workflows you want to see.',

  emptyTitle: 'No scheduled workflows yet',
  emptyBody:
    'A scheduled workflow is one GitHub issue: open it from the **FKST scheduled workflow** template, name a workflow, a run mode, and assign the session creator. There is nothing else to install.',
  emptyAction: 'Open the template on GitHub',
  notInstalled: 'The FKST app is not installed on this repository, or you cannot see it.',
  loadFailed: 'Could not load the scheduled workflows for this repository.',
  retry: 'Try again',

  colWorkflow: 'Workflow',
  colCadence: 'Cadence',
  colNextRun: 'Next run',
  colState: 'State',
  colLastRun: 'Last run',
  colSuccess: '30-day success',

  lifecycle: {
    idle: 'Idle',
    running: 'Running',
    paused: 'Paused',
    invalid: 'Invalid',
  },
  runStatus: {
    dispatched: 'Running',
    ok: 'Succeeded',
    failed: 'Failed',
    timeout: 'Timed out',
    'skipped-overlap': 'Skipped',
  },
  stepStatus: {
    ok: 'Succeeded',
    failed: 'Failed',
    skipped: 'Not run',
  },

  inDays: 'in {d}d',
  inHours: 'in {h}h',
  inMinutes: 'in {m}m',
  imminent: 'due now',
  overdue: 'overdue',
  never: '—',

  detailBack: 'Back to all workflows',
  upcoming: 'Next firings',
  argumentsTitle: 'Arguments',
  noArguments: 'This workflow takes no arguments.',
  runsTitle: 'Runs',
  noRuns: 'This workflow has not run yet.',
  stepsTitle: 'Steps',
  noSteps: 'This run recorded no per-step outcomes.',
  openOnGithub: 'Open the definition on GitHub',
  runIssue: 'Run issue',
  editHint:
    'There is no editor here on purpose: the schedule lives on its GitHub issue and stays editable there. Change the cadence or the arguments by editing that issue.',

  actionRunNow: 'Run now',
  actionPause: 'Pause',
  actionResume: 'Resume',
  actionBusy: 'Working…',
  actionFailed: 'That did not work.',
  runNowStarted: 'Started. The run appears once the session picks it up.',

  slot: 'Slot',
  duration: 'Duration',
  manual: 'Manual',
  detailColumn: 'Detail',
  stepperAria: 'Steps of this run',
  runsAria: 'Run history',
  schedulesAria: "This repository's scheduled workflows",
};
