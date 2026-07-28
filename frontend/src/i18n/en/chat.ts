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
} as const;
