/** The FKST Orchestrator: launcher, panel chrome, transcript, and composer. Every
 *  string the surface renders lives here — the components hold no literals. */
export interface ChatContent {
  launcherLabel: string;
  launcherAria: string;
  launcherCloseAria: string;
  panelTitle: string;
  panelAria: string;
  /** Idle status chip text (paired with a tone, never colour alone). */
  linkActive: string;
  /** Streaming status chip text. */
  streaming: string;
  clear: string;
  clearAria: string;
  /** Download the whole session, including every step's parameters and response. */
  export: string;
  /** Window controls: drag-resize, pin-open, and full screen. */
  resizeAria: string;
  pinAria: string;
  fullScreenAria: string;
  fullScreenExitAria: string;
  exportAria: string;
  close: string;
  closeAria: string;
  welcomeTitle: string;
  welcomeBody: string;
  /** Starter prompts that prefill the composer from the empty state. */
  starters: {
    running: string;
    unrouted: string;
    start: string;
  };
  transcriptAria: string;
  jumpToLatest: string;
  assistantRole: string;
  userRole: string;
  copyAnswer: string;
  answerAria: string;
  activityToggle: string;
  toolRunning: string;
  toolOk: string;
  toolDenied: string;
  /** A 404/409 — the thing is absent, which is not the same as denied. */
  toolNone: string;
  toolError: string;
  toolTruncated: string;
  /** Marks an answer the user cut short by asking something else. */
  interrupted: string;
  sendInterruptAria: string;
  /** The orchestration timeline: one row per model round and per tool call,
   *  each expandable to the exact parameters and response, plus the
   *  CLEAN/VERBOSE switch that decides how much of it a turn renders. */
  stepRound: string;
  stepRoundOpen: string;
  stepRoundCalls: string;
  /** Shown for a finished round in which the model produced no prose. */
  stepRoundSilent: string;
  stepParameters: string;
  stepResponse: string;
  stepTruncated: string;
  stepSummaryOne: string;
  stepSummaryMany: string;
  viewLevelAria: string;
  viewLevelClean: string;
  viewLevelVerbose: string;
  viewLevelNoteVerbose: string;
  viewLevelNoteClean: string;
  /** Human labels keyed by backend tool name; an unlisted name renders raw. */
  toolNames: Record<string, string>;
  /** Structured data cards projected from a tool result. Placeholders as named. */
  cardEnvironments: string;
  cardEnvironment: string;
  cardNoEnvironments: string;
  cardEnvCounts: string;
  cardOutcomes: string;
  cardOutcomeSummary: string;
  cardMerged: string;
  cardFilesChanged: string;
  cardLogRuns: string;
  cardRunLive: string;
  cardLogFiles: string;
  cardOmitted: string;
  /** Session rich-cards. */
  triggerPrefix: string;
  sessionChip: string;
  openInDashboard: string;
  openTrigger: string;
  placeholder: string;
  inputAria: string;
  send: string;
  sendAria: string;
  stop: string;
  stopAria: string;
  /** `{used}` / `{max}` character counter. */
  charCount: string;
  signInTitle: string;
  signInBody: string;
  /** Confirm-gated action cards. */
  kindNewSession: string;
  kindWorkItem: string;
  kindStopSession: string;
  kindNewRepo: string;
  kindSaveEnv: string;
  kindDeleteEnv: string;
  kindUninstallApp: string;
  /** Scope line for a proposal that belongs to the user, not a repository. */
  scopeYourAccount: string;
  scopePersonal: string;
  /** New-repository card. */
  repoPrivate: string;
  repoPublic: string;
  repoInstallNote: string;
  /** Environment card. */
  envInstall: string;
  envVariables: string;
  envSecrets: string;
  envNoVariables: string;
  envCreateNote: string;
  envReplaceNote: string;
  envUnknownNote: string;
  envSecretHint: string;
  envSecretPlaceholder: string;
  envSecretsRequired: string;
  envValidateNote: string;
  /** Carries `{name}`. */
  deleteEnvLine: string;
  deleteEnvConfirmTitle: string;
  /** Carries `{name}`. */
  deleteEnvConfirmBody: string;
  deleteEnvConfirmAction: string;
  /** Uninstall-App card. Each carries `{owner}`. */
  uninstallLine: string;
  uninstallConfirmTitle: string;
  uninstallConfirmBody: string;
  uninstallConfirmAction: string;
  previewToggle: string;
  fieldWorkLabel: string;
  fieldAutoDiscovered: string;
  fieldPackages: string;
  fieldBranches: string;
  fieldDefault: string;
  fieldAutoMerge: string;
  fieldEnvironment: string;
  on: string;
  off: string;
  workItemBodyAria: string;
  /** Carries `{number}`. */
  stopTriggerLine: string;
  finalChecksNote: string;
  confirmExecute: string;
  dismiss: string;
  executing: string;
  executeFailed: string;
  unreadableProposal: string;
  restoredUnknown: string;
  outcomeChipCreated: string;
  outcomeChipStopped: string;
  outcomeChipSaved: string;
  outcomeChipDeleted: string;
  outcomeChipRemoved: string;
  openIssue: string;
  openRepo: string;
  /** Each carries `{number}` and `{repo}`. */
  outcomeSession: string;
  outcomeWorkItem: string;
  outcomeStopped: string;
  /** Carries `{repo}`. */
  outcomeRepo: string;
  /** Each carries `{name}`. */
  outcomeEnvSaved: string;
  outcomeEnvDeleted: string;
  /** Carries `{owner}`. */
  outcomeUninstalled: string;
  stopConfirmTitle: string;
  /** Carries `{number}` and `{repo}`. */
  stopConfirmBody: string;
  stopConfirmAction: string;
  /** Error copy keyed by the stream's stable error code, plus an `unknown`
   *  fallback and a `rate_limited_after` variant carrying `{seconds}`. */
  errors: Record<string, string>;
}
