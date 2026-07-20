// Pure derivations over the overview payload: canvas status classes, name
// filters, and the chart row builders the sidebar charts consume. Everything
// here is a plain function of its inputs — no fetching, no React — so the
// status/filter/chart logic is unit-testable in isolation.

import type {
  AccountOverview,
  IssueDetail,
  RepoOverview,
  RepoSessionsResponse,
  SessionDetail,
  SessionLiveness,
} from './types';

/** The three visual status classes of the canvas (contract §frontend):
 *  - `none`       grey — App not installed
 *  - `installed`  static amber highlight — App installed, 0 active sessions
 *  - `active`     amber highlight + blinking glow — installed AND ≥1 active */
export type CanvasStatus = 'none' | 'installed' | 'active';

export function repoStatus(repo: RepoOverview): CanvasStatus {
  if (!repo.installed) return 'none';
  return repo.active_sessions >= 1 ? 'active' : 'installed';
}

/** An account is `active` when any of its repos qualifies; `installed` when
 *  the account-level App installation exists (even with zero covered repos). */
export function accountStatus(account: AccountOverview): CanvasStatus {
  if (account.repos.some((r) => repoStatus(r) === 'active')) return 'active';
  return account.installed ? 'installed' : 'none';
}

/** A level-2 session row counts as active when its trigger issue is open and
 *  parsed OK — the same registration-level notion the overview counts. */
export function sessionActive(session: SessionDetail): boolean {
  return session.trigger.state === 'open' && session.invalid_reason == null;
}

/** Level-2 card status from the live sessions payload — same contract as
 *  `repoStatus`: blinking `active` requires the App installed, so a repo the
 *  App was removed from reads grey even while a trigger issue is still open. */
export function repoDetailStatus(
  installed: boolean,
  sessions: RepoSessionsResponse | null
): CanvasStatus {
  if (!installed) return 'none';
  return sessions != null && sessions.sessions.some(sessionActive) ? 'active' : 'installed';
}

// ---- Name filters -----------------------------------------------------------

const norm = (s: string) => s.trim().toLowerCase();

/** Case-insensitive substring match on the account login. An empty query
 *  keeps everything (order preserved — the backend already sorts). */
export function filterAccounts(accounts: AccountOverview[], query: string): AccountOverview[] {
  const q = norm(query);
  if (!q) return accounts;
  return accounts.filter((a) => a.login.toLowerCase().includes(q));
}

/** Case-insensitive substring match on `owner/name`. */
export function filterRepos(repos: RepoOverview[], query: string): RepoOverview[] {
  const q = norm(query);
  if (!q) return repos;
  return repos.filter((r) => `${r.owner}/${r.name}`.toLowerCase().includes(q));
}

// ---- Chart rows -------------------------------------------------------------

export interface ChartRow {
  /** Stable identity for React keys. */
  key: string;
  /** Axis label (may be shortened; `key` stays full). */
  label: string;
  value: number;
}

/** Shorten a package ref for the category axis: the `path` tail identifies a
 *  package to a human better than the full `owner/repo@ref:path` string. */
export function packageShortLabel(ref: string): string {
  const at = ref.indexOf('@');
  const colon = at >= 0 ? ref.indexOf(':', at) : -1;
  if (colon >= 0 && colon + 1 < ref.length) {
    const path = ref.slice(colon + 1);
    const tail = path.split('/').filter(Boolean).pop();
    if (tail) return tail;
  }
  return ref;
}

const byValueDesc = (a: ChartRow, b: ChartRow) =>
  b.value - a.value || a.label.localeCompare(b.label);

/** Active sessions per account (level-0 chart), optionally scoped to one
 *  account login. Accounts with zero sessions still get a row so the reader
 *  sees the flat baseline, not a missing category. */
export function sessionsByAccount(
  accounts: AccountOverview[],
  accountLogin?: string | null
): ChartRow[] {
  return accounts
    .filter((a) => !accountLogin || a.login === accountLogin)
    .map((a) => ({
      key: a.login,
      label: a.login,
      value: a.repos.reduce((n, r) => n + r.active_sessions, 0),
    }))
    .sort(byValueDesc);
}

/** Active sessions per repository (level-1 chart). Takes the already
 *  name-filtered repo set so the chart describes exactly the listed
 *  population — mirroring how level 0 feeds `shown` into its builders. */
export function sessionsByRepo(repos: RepoOverview[], repoName?: string | null): ChartRow[] {
  return repos
    .filter((r) => !repoName || r.name === repoName)
    .map((r) => ({ key: r.name, label: r.name, value: r.active_sessions }))
    .sort(byValueDesc);
}

/** Package usage across accounts: for each package ref, the number of repos
 *  whose active sessions use it. Optional scoping to one account. */
export function packagesByAccount(
  accounts: AccountOverview[],
  accountLogin?: string | null
): ChartRow[] {
  const counts = new Map<string, number>();
  for (const account of accounts) {
    if (accountLogin && account.login !== accountLogin) continue;
    for (const repo of account.repos) {
      for (const ref of repo.packages) {
        counts.set(ref, (counts.get(ref) ?? 0) + 1);
      }
    }
  }
  return [...counts.entries()]
    .map(([ref, value]) => ({ key: ref, label: packageShortLabel(ref), value }))
    .sort(byValueDesc);
}

/** Package usage across the given (already name-filtered) repos, optionally
 *  scoped to a single repo — same population rule as `sessionsByRepo`. */
export function packagesByRepo(repos: RepoOverview[], repoName?: string | null): ChartRow[] {
  const counts = new Map<string, number>();
  for (const repo of repos) {
    if (repoName && repo.name !== repoName) continue;
    for (const ref of repo.packages) {
      counts.set(ref, (counts.get(ref) ?? 0) + 1);
    }
  }
  return [...counts.entries()]
    .map(([ref, value]) => ({ key: ref, label: packageShortLabel(ref), value }))
    .sort(byValueDesc);
}

/** Fold the tail past `max` rows into a single aggregate row (dataviz rule:
 *  never more than ~7 meaningful classes — the tail becomes "Other"). */
export function foldTail(rows: ChartRow[], max: number, otherLabel: string): ChartRow[] {
  if (rows.length <= max) return rows;
  const head = rows.slice(0, max - 1);
  const tail = rows.slice(max - 1);
  return [
    ...head,
    {
      key: '__other__',
      label: otherLabel,
      value: tail.reduce((n, r) => n + r.value, 0),
    },
  ];
}

// ---- Session-detail decoders (drawer) ---------------------------------------
// Pure classifiers over the control-plane status markers. The label strings
// are the ones the reconciler latches on the trigger issue (see
// `backend/src/reconcile/mod.rs`); the `fkst-dev:*` work-item labels are set by
// the running session's devloop package.

/** The lifecycle phase of a session, decoded from its status labels + trigger
 *  state + liveness. Ordered here roughly as the lifecycle progresses. */
export type SessionPhase =
  | 'registered'
  | 'active'
  | 'picked-up'
  | 'degraded'
  | 'retired'
  | 'invalid'
  | 'idle';

/** A coarse health signal separate from the phase: `ok` needs positive
 *  liveness, `degraded` is latched by the reconciler, everything else is
 *  `unknown` (we have no positive signal, not necessarily bad). */
export type SessionHealth = 'ok' | 'degraded' | 'unknown';

export interface DecodedSessionStatus {
  phase: SessionPhase;
  health: SessionHealth;
  liveness: SessionLiveness | null;
}

/** Trigger-issue status labels the reconciler latches. */
const SESSION_LABELS = {
  invalid: 'fkst-substrate-invalid',
  active: 'fkst-substrate-active',
  pickedUp: 'fkst-picked-up',
  retired: 'fkst-session-retired',
  degraded: 'fkst-degraded',
  configRejected: 'fkst-config-rejected',
} as const;

/** Decode a session's lifecycle phase + health from its labels, invalid reason,
 *  trigger state and pod liveness. Precedence is terminal-first: an invalid /
 *  config-rejected session overrides everything, then a retired/closed one,
 *  then degraded, then a LIVE pod (active), then the paused (idle) / reviving
 *  (picked-up) / fresh (registered) open-trigger states.
 *
 *  The `fkst-substrate-active` label is a DURABLE one-way latch the reconciler
 *  sets ONCE at registration (backend/src/reconcile/mod.rs) — it records "this
 *  session was announced", NOT "a pod is running right now" (there is no removal
 *  path). So a latched active label alone is NOT enough to read "active": that
 *  requires a live pod (`liveness === 'live'`). A session whose pod was reaped
 *  for lack of work still carries the label but is IDLE (paused), and
 *  auto-revives when a new work issue opens. */
export function decodeSessionStatus(session: SessionDetail): DecodedSessionStatus {
  const labels = new Set(session.status_labels);
  const has = (label: string) => labels.has(label);
  const degraded = has(SESSION_LABELS.degraded);
  const liveness = session.liveness ?? null;
  const live = liveness === 'live';
  // The announce latch records that the session ran at least once — it is the
  // signal that separates a paused (idle) session from a never-activated
  // (registered) one, but only ever in combination with the live-pod check.
  const announced = has(SESSION_LABELS.active);
  // Open work items are what keep a session's pod alive; with none pending the
  // reconciler idles/reaps the pod to save resources (the manual's IDLE state).
  const hasOpenWork = session.work_issues.some((issue) => issue.state === 'open');

  let phase: SessionPhase;
  if (session.invalid_reason != null || has(SESSION_LABELS.invalid) || has(SESSION_LABELS.configRejected)) {
    phase = 'invalid';
  } else if (has(SESSION_LABELS.retired) || session.trigger.state === 'closed') {
    phase = 'retired';
  } else if (degraded) {
    phase = 'degraded';
  } else if (live) {
    // A live pod is the ONLY signal that reads as "active"; a latched announce
    // label whose pod has since been reaped resolves to idle (below), not active.
    phase = 'active';
  } else if (announced && !hasOpenWork) {
    // Announced before, no live pod, nothing queued: paused to save resources.
    // A new work issue auto-revives it.
    phase = 'idle';
  } else if (has(SESSION_LABELS.pickedUp) || (announced && hasOpenWork)) {
    // Claimed / reviving: picked up, or announced with pending work, but the pod
    // is not live yet (e.g. `liveness === 'starting'`).
    phase = 'picked-up';
  } else if (session.trigger.state === 'open') {
    phase = 'registered';
  } else {
    phase = 'idle';
  }

  const health: SessionHealth = degraded ? 'degraded' : live ? 'ok' : 'unknown';
  return { phase, health, liveness };
}

/** A decoded work-item (queue issue) state + a semantic tone for the chip. */
export type WorkItemState =
  | 'queued'
  | 'thinking'
  | 'implementing'
  | 'ready'
  | 'failed'
  | 'done'
  | 'claimed'
  | 'other';

export type WorkItemTone = 'neutral' | 'progress' | 'good' | 'bad';

export interface DecodedWorkItem {
  state: WorkItemState;
  tone: WorkItemTone;
}

/** The devloop marks its progress on a work issue with `fkst-dev:<suffix>`
 *  labels. */
const DEV_LABEL_PREFIX = 'fkst-dev:';

/** Decode one work issue's state from its `fkst-dev:*` labels and open/closed
 *  state. A closed issue is `done` regardless of any stale in-flight marker —
 *  the devloop closes it when its PR merges. Open issues resolve highest-signal
 *  first: a terminal failure, then a ready PR, then the in-flight phases, then
 *  the pre-work latches, falling back to `queued` (waiting) or `other`. */
export function decodeWorkItemStatus(issue: IssueDetail): DecodedWorkItem {
  if (issue.state === 'closed') return { state: 'done', tone: 'good' };

  const suffixes = new Set(
    issue.labels
      .filter((label) => label.startsWith(DEV_LABEL_PREFIX))
      .map((label) => label.slice(DEV_LABEL_PREFIX.length))
  );
  const has = (suffix: string) => suffixes.has(suffix);

  if (has('impl-failed')) return { state: 'failed', tone: 'bad' };
  if (has('ready')) return { state: 'ready', tone: 'good' };
  if (has('implementing')) return { state: 'implementing', tone: 'progress' };
  if (has('thinking')) return { state: 'thinking', tone: 'progress' };
  if (has('claimed')) return { state: 'claimed', tone: 'progress' };
  if (has('enabled')) return { state: 'queued', tone: 'neutral' };
  if (suffixes.size === 0) return { state: 'queued', tone: 'neutral' };
  return { state: 'other', tone: 'neutral' };
}

/** A package reference decoded into a short handle (the path tail) and a
 *  friendly role name. */
export interface PackageRole {
  short: string;
  role: string;
}

/** Ordered role rules: the first whose matcher hits the (lower-cased) path tail
 *  wins. Mapping the role from the tail keeps the busy `owner/repo@ref:path`
 *  string out of the primary UI while the full ref stays available in a
 *  tooltip / `<code>`. Falls back to the short handle itself. */
const ROLE_RULES: ReadonlyArray<{ match: (tail: string) => boolean; role: string }> = [
  { match: (t) => t.startsWith('workflow-dev'), role: 'Dev workflow' },
  { match: (t) => t.startsWith('github-devloop'), role: 'Devloop' },
  { match: (t) => t.includes('consensus'), role: 'Consensus' },
  { match: (t) => t.includes('triage'), role: 'Triage' },
  { match: (t) => t.includes('review'), role: 'Review' },
  { match: (t) => t.includes('security'), role: 'Security' },
  { match: (t) => t.includes('intake'), role: 'Intake' },
  { match: (t) => t === 'base' || t.endsWith('/base'), role: 'Base' },
];

export function packageRole(ref: string): PackageRole {
  const short = packageShortLabel(ref);
  const tail = short.toLowerCase();
  const rule = ROLE_RULES.find((candidate) => candidate.match(tail));
  return { short, role: rule ? rule.role : short };
}
