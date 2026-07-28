/** Copy for the chat concierge: launcher, panel chrome, transcript, composer. */
export const chat = {
  /** Launcher label + accessible name. */
  launcherLabel: 'CONCIERGE',
  launcherAria: 'Open the fkst concierge',
  launcherCloseAria: 'Close the fkst concierge',

  /** Panel chrome. */
  panelTitle: 'FKST // CONCIERGE',
  panelAria: 'fkst concierge',
  linkActive: 'LINK ACTIVE',
  streaming: 'STREAMING',
  clear: 'Clear',
  clearAria: 'Clear the conversation',
  close: 'Close',
  closeAria: 'Close the concierge panel',

  /** Empty state. */
  welcomeTitle: 'Ask about your sessions',
  welcomeBody:
    'I can look up what is running, read a failing log, and explain how the platform works — using only what you have access to.',
  starters: {
    running: 'What sessions are running?',
    unrouted: 'Why is my issue unrouted?',
    start: 'How do I start a session?',
  },

  /** Transcript. */
  transcriptAria: 'Conversation',
  jumpToLatest: 'JUMP TO LATEST',
  assistantRole: 'CONCIERGE',
  userRole: 'YOU',
  copyAnswer: 'Copy',
  answerAria: 'Assistant answer',
  activityToggle: 'activity',
  toolRunning: 'RUNNING',
  toolOk: 'OK',
  toolDenied: 'DENIED',
  toolError: 'ERR',
  toolTruncated: 'TRUNCATED',
  /** Human labels for the backend's tool names; an unlisted name renders raw. */
  toolNames: {
    get_overview: 'accounts & repos',
    list_repo_sessions: 'repo sessions',
    get_session_outcomes: 'session outcomes',
    observe_session: 'live engine state',
    list_log_runs: 'log runs',
    get_log_manifest: 'log files',
    tail_log_file: 'log tail',
    list_environment_profiles: 'environments',
    search_manual: 'manual',
  } as Record<string, string>,

  /** Session rich-cards. */
  triggerPrefix: 'trigger #',
  sessionChip: 'SESSION',
  openInDashboard: 'OPEN IN DASHBOARD',
  openTrigger: 'TRIGGER',

  /** Composer. */
  placeholder: 'Ask about a session, a log, or how something works…',
  inputAria: 'Message the concierge',
  send: 'Send',
  sendAria: 'Send message',
  stop: 'Stop',
  stopAria: 'Stop the current answer',
  charCount: '{used} / {max}',

  /** Sign-in gate inside the panel. */
  signInTitle: 'Sign in to use the concierge',
  signInBody:
    'The concierge answers using your own GitHub access, so it needs you signed in first.',

  /** Confirm-gated action cards. */
  kindNewSession: 'NEW SESSION',
  kindWorkItem: 'WORK ITEM',
  kindStopSession: 'STOP SESSION',
  previewToggle: 'issue body',
  fieldWorkLabel: 'work label',
  fieldAutoDiscovered: 'auto-discovered',
  fieldPackages: 'package refs',
  fieldBranches: 'branches',
  fieldDefault: 'default',
  fieldAutoMerge: 'auto-merge',
  fieldEnvironment: 'environment',
  on: 'on',
  off: 'off',
  workItemBodyAria: 'Work item body',
  stopTriggerLine: 'Closes trigger #{number} — this is permanent.',
  finalChecksNote: 'Final permission and collision checks run when you confirm.',
  confirmExecute: 'CONFIRM & EXECUTE',
  dismiss: 'DISMISS',
  executing: 'EXECUTING…',
  executeFailed: 'That action could not be completed.',
  unreadableProposal: 'The assistant produced an unreadable action draft — ask it to try again.',
  restoredUnknown:
    'This action was still running when the page closed, so its outcome is unknown — check the dashboard before retrying.',
  outcomeChipCreated: 'CREATED',
  outcomeChipStopped: 'STOPPED',
  openIssue: 'ISSUE',
  outcomeSession: 'Created trigger #{number} in {repo}.',
  outcomeWorkItem: 'Created work item #{number} in {repo}.',
  outcomeStopped: 'Closed trigger #{number} in {repo}; the session is retired.',
  stopConfirmTitle: 'Stop this session?',
  stopConfirmBody:
    'Closing trigger #{number} in {repo} retires the session permanently — it never revives. Open work issues stay open but are no longer worked.',
  stopConfirmAction: 'Stop session',

  /** Error copy, keyed by the stream's stable error code. */
  errors: {
    deadline_exceeded: 'That took too long to answer. Try a narrower question.',
    tool_budget_exhausted:
      'That needed too many lookups to answer. Try asking about one thing at a time.',
    rate_limited: 'One question at a time — try again in a moment.',
    rate_limited_after: 'One question at a time — try again in {seconds}s.',
    llm_error: 'The language model could not be reached. Please try again.',
    protocol: 'The answer could not be read. Please try again.',
    unavailable: 'The concierge is not available on this deployment right now.',
    unauthorized: 'Your session expired. Sign in again to keep asking.',
    network: 'The connection dropped before the answer finished.',
    request: 'That request was rejected.',
    unknown: 'Something went wrong answering that.',
  } as Record<string, string>,
} as const;
