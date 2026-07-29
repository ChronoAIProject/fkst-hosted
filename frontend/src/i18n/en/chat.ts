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
    get_environment_profile: 'environment detail',
    search_manual: 'manual',
    draft_trigger_session: 'drafting a session',
    draft_work_item: 'drafting a work item',
    propose_stop_session: 'drafting a stop',
    propose_create_repository: 'drafting a repository',
    draft_environment_profile: 'drafting an environment',
    propose_delete_environment_profile: 'drafting an environment delete',
    propose_uninstall_app: 'drafting an uninstall',
  } as Record<string, string>,

  /** Structured data cards projected from a tool result. */
  cardEnvironments: 'ENVIRONMENTS',
  cardEnvironment: 'ENVIRONMENT',
  cardNoEnvironments: 'No saved environment profiles yet.',
  cardEnvCounts: '{install} install · {vars} vars · {secrets} secrets',
  cardOutcomes: 'OUTCOMES',
  cardOutcomeSummary: '{total} pull request(s) · {merged} merged',
  cardMerged: 'MERGED',
  cardFilesChanged: '{count} file(s)',
  cardLogRuns: 'LOG RUNS',
  cardRunLive: 'RUNNING',
  cardLogFiles: 'LOG FILES',
  cardOmitted: '+{count} more — open the dashboard for the full list.',

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
  kindNewRepo: 'NEW REPOSITORY',
  kindSaveEnv: 'ENVIRONMENT',
  kindDeleteEnv: 'DELETE ENVIRONMENT',
  kindUninstallApp: 'UNINSTALL APP',
  /** Scope line for a proposal that belongs to the user, not a repository. */
  scopeYourAccount: 'your account',
  scopePersonal: 'personal account',

  /** New-repository card. */
  repoPrivate: 'PRIVATE',
  repoPublic: 'PUBLIC',
  repoInstallNote:
    'A new repository has no fkst App installed yet — install it before starting a session there.',

  /** Environment card. */
  envInstall: 'install',
  envVariables: 'variables',
  envSecrets: 'secrets',
  envNoVariables: 'none',
  envCreateNote: 'This creates a new environment profile.',
  envReplaceNote: 'This REPLACES the existing profile — everything not listed here is dropped.',
  envUnknownNote:
    'Whether a profile with this name already exists could not be checked; confirming replaces it if it does.',
  envSecretHint:
    'Secret values stay in this browser until you confirm — the assistant never sees them, and they are not saved to the transcript.',
  envSecretPlaceholder: 'value',
  envSecretsRequired: 'Enter every secret value before confirming.',
  envValidateNote:
    'Confirming runs the install commands in an isolated pod before saving, which can take a minute.',
  deleteEnvLine: 'Deletes `{name}` permanently — its secret values cannot be recovered.',
  deleteEnvConfirmTitle: 'Delete this environment?',
  deleteEnvConfirmBody:
    'Deleting `{name}` is permanent. Its secret values cannot be recovered, and any trigger that names it in its Environment section will stop resolving.',
  deleteEnvConfirmAction: 'Delete environment',

  /** Uninstall-App card. */
  uninstallLine: 'Removes fkst from EVERY repository on {owner}.',
  uninstallConfirmTitle: 'Uninstall the fkst App?',
  uninstallConfirmBody:
    'This removes the fkst App from every repository on {owner} at once and stops every session running there. Re-installing later does not resume retired sessions.',
  uninstallConfirmAction: 'Uninstall App',
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
  outcomeChipSaved: 'SAVED',
  outcomeChipDeleted: 'DELETED',
  outcomeChipRemoved: 'REMOVED',
  openIssue: 'ISSUE',
  openRepo: 'REPOSITORY',
  outcomeSession: 'Created trigger #{number} in {repo}.',
  outcomeWorkItem: 'Created work item #{number} in {repo}.',
  outcomeStopped: 'Closed trigger #{number} in {repo}; the session is retired.',
  outcomeRepo: 'Created the repository {repo}.',
  outcomeEnvSaved: 'Saved the environment profile {name}.',
  outcomeEnvDeleted: 'Deleted the environment profile {name}.',
  outcomeUninstalled: 'Uninstalled the fkst App from {owner}.',
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
