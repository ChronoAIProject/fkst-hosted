import type { WorkflowsSlice } from '../slices';

export const workflows: WorkflowsSlice = {
  loading: 'Loading scheduled workflows',

  emptyTitle: 'This session has no scheduled workflows',
  emptyBody:
    'A scheduled workflow is one GitHub issue: open it from the **FKST scheduled workflow** template, name a workflow and a run mode, and assign this session’s creator. There is nothing else to install.',
  emptyAction: 'Open the template on GitHub',
  notInstalled: 'The FKST app is not installed on this repository, or you cannot see it.',
  loadFailed: 'Could not load the scheduled workflows for this repository.',
  detailFailed: 'Could not load this scheduled workflow.',
  retry: 'Try again',

  railTitle: 'Schedules',
  railAria: 'Scheduled workflows this session owns',
  unroutedTitle: 'Routed to no session',
  unroutedBody:
    'A schedule runs only when exactly one of its assignees is a session creator. These have none, or several, so nothing will run them.',
  unroutedOnly: 'Nothing here is routed to this session yet.',

  cadenceLabel: 'Cadence',
  successLabel: '30-day success',

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

  upcoming: 'Next firings',
  argumentsTitle: 'Arguments',
  noArguments: 'This workflow takes no arguments.',
  latestRunTitle: 'Most recent run',
  earlierRunsTitle: 'Earlier runs',
  noRuns: 'This workflow has not run yet.',
  noSteps: 'This run recorded no per-step outcomes.',
  awaitingSteps: 'Awaiting the first step record — a run reports its steps when it finishes.',
  runningFor: 'running for {d}',
  openOnGithub: 'Open the definition on GitHub',
  openRunIssue: 'Run issue',
  editHint:
    'There is no editor here on purpose: the schedule lives on its GitHub issue and stays editable there. Change the cadence or the arguments by editing that issue.',

  actionRunNow: 'Run now',
  actionPause: 'Pause',
  actionResume: 'Resume',
  actionBusy: 'Working…',
  actionFailed: 'That did not work.',

  manual: 'Manual',
  stepperAria: 'Steps of this run',
  runsAria: 'Earlier runs',
};
