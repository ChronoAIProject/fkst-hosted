import { describe, it, expect } from 'vitest';
import {
  accountStatus,
  filterAccounts,
  filterRepos,
  foldTail,
  packageShortLabel,
  packagesByAccount,
  packagesByRepo,
  repoStatus,
  sessionsByAccount,
  sessionsByRepo,
} from './derive';
import type { AccountOverview, RepoOverview } from './types';

let nextId = 1;
const repo = (over: Partial<RepoOverview> & Pick<RepoOverview, 'name'>): RepoOverview => ({
  id: nextId++,
  owner: 'shining',
  private: false,
  admin: true,
  installed: false,
  active_sessions: 0,
  packages: [],
  ...over,
});

const account = (
  over: Partial<AccountOverview> & Pick<AccountOverview, 'login'>
): AccountOverview => ({
  kind: 'personal',
  owner: true,
  installed: false,
  installation_id: null,
  repository_selection: null,
  counts_complete: true,
  repos: [],
  ...over,
});

describe('status derivation', () => {
  it('maps a repo to none / installed / active', () => {
    expect(repoStatus(repo({ name: 'a' }))).toBe('none');
    expect(repoStatus(repo({ name: 'a', installed: true }))).toBe('installed');
    expect(repoStatus(repo({ name: 'a', installed: true, active_sessions: 1 }))).toBe('active');
    // Not installed wins even with a (stale) session count.
    expect(repoStatus(repo({ name: 'a', active_sessions: 2 }))).toBe('none');
  });

  it('marks an account active when any repo is active', () => {
    const a = account({
      login: 'acme',
      installed: true,
      repos: [
        repo({ name: 'quiet', installed: true }),
        repo({ name: 'busy', installed: true, active_sessions: 3 }),
      ],
    });
    expect(accountStatus(a)).toBe('active');
  });

  it('marks an installed account with no active repos as installed', () => {
    const a = account({
      login: 'acme',
      installed: true,
      repos: [repo({ name: 'quiet', installed: true })],
    });
    expect(accountStatus(a)).toBe('installed');
  });

  it('marks an account without an installation as none — even with repos', () => {
    expect(accountStatus(account({ login: 'acme', repos: [repo({ name: 'x' })] }))).toBe('none');
    // Installation present but zero repos still reads as installed.
    expect(accountStatus(account({ login: 'acme', installed: true }))).toBe('installed');
  });
});

describe('name filters', () => {
  const accounts = [
    account({ login: 'shining' }),
    account({ login: 'acme', kind: 'org' }),
    account({ login: 'Zeta-Labs', kind: 'org' }),
  ];

  it('filters accounts by case-insensitive substring and keeps order', () => {
    expect(filterAccounts(accounts, '').map((a) => a.login)).toEqual([
      'shining',
      'acme',
      'Zeta-Labs',
    ]);
    expect(filterAccounts(accounts, 'ZETA').map((a) => a.login)).toEqual(['Zeta-Labs']);
    expect(filterAccounts(accounts, '  in ').map((a) => a.login)).toEqual(['shining']);
    expect(filterAccounts(accounts, 'nope')).toEqual([]);
  });

  it('filters repos on the owner/name pair', () => {
    const repos = [
      repo({ owner: 'acme', name: 'widgets' }),
      repo({ owner: 'acme', name: 'gears' }),
    ];
    expect(filterRepos(repos, 'WIDG').map((r) => r.name)).toEqual(['widgets']);
    expect(filterRepos(repos, 'acme/').map((r) => r.name)).toEqual(['widgets', 'gears']);
    expect(filterRepos(repos, 'zzz')).toEqual([]);
  });
});

describe('chart row builders', () => {
  const pkgA = 'ChronoAIProject/fkst-packages@fkst-hosted:codex/base';
  const pkgB = 'ChronoAIProject/fkst-packages@fkst-hosted:codex/triage';
  const accounts = [
    account({
      login: 'shining',
      installed: true,
      repos: [
        repo({ name: 'lab', installed: true, active_sessions: 2, packages: [pkgA] }),
        repo({ name: 'idle', installed: true }),
      ],
    }),
    account({
      login: 'acme',
      kind: 'org',
      installed: true,
      repos: [
        repo({ owner: 'acme', name: 'widgets', installed: true, active_sessions: 1, packages: [pkgA, pkgB] }),
      ],
    }),
  ];

  it('sums sessions per account, sorted descending', () => {
    expect(sessionsByAccount(accounts)).toEqual([
      { key: 'shining', label: 'shining', value: 2 },
      { key: 'acme', label: 'acme', value: 1 },
    ]);
  });

  it('scopes the account chart to a single login', () => {
    expect(sessionsByAccount(accounts, 'acme')).toEqual([
      { key: 'acme', label: 'acme', value: 1 },
    ]);
  });

  it('lists sessions per repo of one account, zero rows included', () => {
    expect(sessionsByRepo(accounts[0]!)).toEqual([
      { key: 'lab', label: 'lab', value: 2 },
      { key: 'idle', label: 'idle', value: 0 },
    ]);
    expect(sessionsByRepo(accounts[0]!, 'idle')).toEqual([{ key: 'idle', label: 'idle', value: 0 }]);
  });

  it('counts package usage across accounts and scopes by account', () => {
    expect(packagesByAccount(accounts)).toEqual([
      { key: pkgA, label: 'base', value: 2 },
      { key: pkgB, label: 'triage', value: 1 },
    ]);
    expect(packagesByAccount(accounts, 'acme')).toEqual([
      { key: pkgA, label: 'base', value: 1 },
      { key: pkgB, label: 'triage', value: 1 },
    ]);
  });

  it('counts package usage within an account and scopes by repo', () => {
    expect(packagesByRepo(accounts[1]!)).toEqual([
      { key: pkgA, label: 'base', value: 1 },
      { key: pkgB, label: 'triage', value: 1 },
    ]);
    expect(packagesByRepo(accounts[0]!, 'idle')).toEqual([]);
  });

  it('shortens package refs to the path tail, falling back to the full ref', () => {
    expect(packageShortLabel('o/r@ref:skills/codex-triage')).toBe('codex-triage');
    expect(packageShortLabel('o/r@ref:base')).toBe('base');
    expect(packageShortLabel('not-a-ref')).toBe('not-a-ref');
    expect(packageShortLabel('o/r@ref:')).toBe('o/r@ref:');
  });

  it('folds the tail past the cap into a single Other row', () => {
    const rows = [5, 4, 3, 2, 1].map((v, i) => ({ key: `k${i}`, label: `l${i}`, value: v }));
    expect(foldTail(rows, 7, 'Other')).toEqual(rows); // under the cap: untouched
    const folded = foldTail(rows, 3, 'Other');
    expect(folded).toHaveLength(3);
    expect(folded[2]).toEqual({ key: '__other__', label: 'Other', value: 3 + 2 + 1 });
  });
});
