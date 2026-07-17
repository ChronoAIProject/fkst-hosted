// Pure derivations over the overview payload: canvas status classes, name
// filters, and the chart row builders the sidebar charts consume. Everything
// here is a plain function of its inputs — no fetching, no React — so the
// status/filter/chart logic is unit-testable in isolation.

import type {
  AccountOverview,
  RepoOverview,
  RepoSessionsResponse,
  SessionDetail,
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
