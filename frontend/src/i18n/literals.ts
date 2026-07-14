// Locale-independent literals. These are GitHub syntax, code, commands, and
// identifiers that MUST be byte-identical in every language — they live here,
// outside the translation catalogs, so they can never drift between locales.

export const REPO_URL = 'https://github.com/ChronoAIProject';
export const REPO = 'https://github.com/ChronoAIProject/fkst-hosted';
export const MANUAL_URL = `${REPO}/blob/main/skills/fkst-control-plane-manual/SKILL.md`;

/** The package-reference grammar shown in the Package-references step heading. */
export const PKG_GRAMMAR = 'owner/repo@ref:path';

export const TRIGGER_EXAMPLE = `### Session Name
sitebuilder

### Packages
ChronoAIProject/fkst-packages@dev:packages/github-devloop
ChronoAIProject/fkst-packages@dev:packages/github-devloop-pr
ChronoAIProject/fkst-packages@dev:packages/github-devloop-ops
ChronoAIProject/fkst-packages@dev:packages/consensus

### Work Label
site-build

### Auto-merge
true`;

export const GH_CREATE = `gh issue create \\
  --repo <owner>/<repo> \\
  --title "[session] sitebuilder" \\
  --body-file body.md \\
  --label fkst-substrate-trigger`;

export const CURL_LOGS = `curl -L \\
  -H "Authorization: Bearer $GITHUB_TOKEN" \\
  https://<host>/api/v1/logs/<session_id> \\
  -o logs.tar.gz`;

export const PKG_REF_EXAMPLE = 'ChronoAIProject/fkst-packages@dev:packages/github-devloop';

export type FieldKey =
  | 'sessionName'
  | 'packages'
  | 'workLabel'
  | 'environment'
  | 'autoMerge'
  | 'logAllowlist';

/** Trigger-body sections: heading token + required flag are literal; the rule
 *  text is translated (keyed by `key`). */
export const TRIGGER_FIELDS: { key: FieldKey; name: string; required: boolean }[] = [
  { key: 'sessionName', name: '### Session Name', required: true },
  { key: 'packages', name: '### Packages', required: true },
  { key: 'workLabel', name: '### Work Label', required: true },
  { key: 'environment', name: '### Environment', required: false },
  { key: 'autoMerge', name: '### Auto-merge', required: false },
  { key: 'logAllowlist', name: '### FKST Contributors', required: false },
];

export type GrammarKey = 'ownerRepo' | 'ref' | 'path';

export const GRAMMAR_PARTS: { key: GrammarKey; part: string }[] = [
  { key: 'ownerRepo', part: 'owner/repo' },
  { key: 'ref', part: 'ref' },
  { key: 'path', part: 'path' },
];

export type SignalKey =
  | 'registered'
  | 'pickup'
  | 'pr'
  | 'degraded'
  | 'retired'
  | 'invalid'
  | 'configRejected';

export type SignalTone = 'green' | 'neutral' | 'amber' | 'red';

/** Status signals: the name (comment pattern / label identifier) and tone are
 *  literal; kind / where / meaning are translated (keyed by `key`). */
export const SIGNALS: { key: SignalKey; name: string; tone: SignalTone }[] = [
  { key: 'registered', name: 'session … registered', tone: 'green' },
  { key: 'pickup', name: 'pick-up', tone: 'neutral' },
  { key: 'pr', name: 'PR by the App bot', tone: 'neutral' },
  { key: 'degraded', name: 'fkst-degraded', tone: 'amber' },
  { key: 'retired', name: 'fkst-session-retired', tone: 'red' },
  { key: 'invalid', name: 'fkst-substrate-invalid', tone: 'red' },
  { key: 'configRejected', name: 'fkst-config-rejected', tone: 'red' },
];

export type StepId =
  | 'install'
  | 'start'
  | 'parameters'
  | 'packages'
  | 'queue'
  | 'status'
  | 'logs'
  | 'lifecycle';

/** Canonical step order (drives the TOC and the numbered step headings). */
export const STEP_ORDER: StepId[] = [
  'install',
  'start',
  'parameters',
  'packages',
  'queue',
  'status',
  'logs',
  'lifecycle',
];

export type FlowKey = 'trigger' | 'session' | 'work' | 'pr' | 'merge';
export const FLOW_ORDER: FlowKey[] = ['trigger', 'session', 'work', 'pr', 'merge'];

export type MentalKey = 'session' | 'trigger' | 'work';
export const MENTAL_ORDER: MentalKey[] = ['session', 'trigger', 'work'];
