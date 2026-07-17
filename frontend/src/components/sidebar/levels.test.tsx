import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AuthProvider } from '@/lib/auth/github-auth';
import type { AccountOverview, OverviewResponse, RepoOverview } from '@/lib/api/types';
import { Level0Sidebar } from './level0';
import { Level1Sidebar } from './level1';

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

const overview = (accounts: AccountOverview[], appSlug: string | null = 'chronoai-fkst') =>
  ({
    app_slug: appSlug,
    viewer: { login: 'shining' },
    accounts,
    totals: { sessions: 0, packages: [] },
  }) satisfies OverviewResponse;

describe('Level0Sidebar', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });

  it('states what the view is, shows the legend, charts and account rows', () => {
    render(
      <AuthProvider>
        <Level0Sidebar
          overview={overview([
            account({
              login: 'shining',
              installed: true,
              installation_id: 11,
              repos: [repo({ name: 'lab', installed: true, active_sessions: 2 })],
            }),
            account({ login: 'acme', kind: 'org', owner: false }),
          ])}
          query=""
          onQueryChange={() => {}}
          onOpenAccount={() => {}}
          onRepoCreated={() => {}}
          onChanged={() => {}}
        />
      </AuthProvider>
    );

    // Plain statement of what the view represents + the three-status legend.
    expect(screen.getByText(/every GitHub account you can reach/)).toBeInTheDocument();
    expect(screen.getByText('Legend')).toBeInTheDocument();
    expect(screen.getByText('Grey — App not installed')).toBeInTheDocument();
    expect(screen.getByText('Amber — App installed, no active sessions')).toBeInTheDocument();
    expect(screen.getByText('Blinking amber — active sessions running')).toBeInTheDocument();

    // Both charts render as labeled figures with the scope filter above them.
    expect(screen.getByRole('figure', { name: 'Running sessions' })).toBeInTheDocument();
    expect(screen.getByRole('figure', { name: 'Packages in use' })).toBeInTheDocument();
    expect(screen.getByLabelText('Scope charts to an account')).toBeInTheDocument();

    // Account rows: connected account has Manage+Uninstall, the other Connect.
    expect(screen.getByRole('link', { name: 'Manage' })).toHaveAttribute(
      'href',
      'https://github.com/settings/installations/11'
    );
    expect(screen.getByRole('button', { name: 'Uninstall' })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Connect' })).toBeInTheDocument();
  });

  it('filters account rows by the query and shows the empty-filter note', async () => {
    const user = userEvent.setup();
    const onQueryChange = vi.fn();
    const { rerender } = render(
      <AuthProvider>
        <Level0Sidebar
          overview={overview([account({ login: 'shining' }), account({ login: 'acme', kind: 'org' })])}
          query=""
          onQueryChange={onQueryChange}
          onOpenAccount={() => {}}
          onRepoCreated={() => {}}
          onChanged={() => {}}
        />
      </AuthProvider>
    );

    await user.type(screen.getByLabelText('Filter accounts…'), 'z');
    expect(onQueryChange).toHaveBeenCalledWith('z');

    rerender(
      <AuthProvider>
        <Level0Sidebar
          overview={overview([account({ login: 'shining' }), account({ login: 'acme', kind: 'org' })])}
          query="zzz"
          onQueryChange={onQueryChange}
          onOpenAccount={() => {}}
          onRepoCreated={() => {}}
          onChanged={() => {}}
        />
      </AuthProvider>
    );
    expect(screen.getByText('No accounts match your filter.')).toBeInTheDocument();
  });
});

describe('Level1Sidebar', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });

  const acc = account({
    login: 'acme',
    kind: 'org',
    installed: true,
    installation_id: 22,
    repos: [
      repo({ owner: 'acme', name: 'widgets', private: true, installed: true, active_sessions: 1 }),
      repo({ owner: 'acme', name: 'gears', admin: false }),
    ],
  });

  it('describes the account view and carries the install affordances', () => {
    render(
      <AuthProvider>
        <Level1Sidebar
          account={acc}
          appSlug="chronoai-fkst"
          query=""
          onQueryChange={() => {}}
          createdKey={null}
          onOpenRepo={() => {}}
        />
      </AuthProvider>
    );

    expect(screen.getByText(/repositories of acme/)).toBeInTheDocument();
    expect(screen.getByText('Legend')).toBeInTheDocument();

    // Installed row: green mark with the manage-on-GitHub hint (the account
    // has an installation), no per-repo Remove anywhere.
    const installed = screen.getByText('✓ Installed');
    expect(installed).toHaveAttribute(
      'title',
      'Manage this repository on GitHub (add or remove it there).'
    );
    expect(screen.queryByRole('button', { name: 'Remove' })).not.toBeInTheDocument();

    // Not-installed row: Install link with the non-admin hint.
    const install = screen.getByRole('link', { name: 'Install' });
    expect(install).toHaveAttribute(
      'href',
      'https://github.com/apps/chronoai-fkst/installations/new'
    );
    expect(install).toHaveAttribute(
      'title',
      'You are not an admin of this repository — GitHub may send an approval request to its owner.'
    );

    // Charts scoped to the account.
    expect(screen.getByRole('figure', { name: 'Running sessions' })).toBeInTheDocument();
    expect(screen.getByLabelText('Scope charts to a repository')).toBeInTheDocument();
  });

  it('opens a repo from the row affordance', async () => {
    const user = userEvent.setup();
    const onOpenRepo = vi.fn();
    render(
      <AuthProvider>
        <Level1Sidebar
          account={acc}
          appSlug="chronoai-fkst"
          query=""
          onQueryChange={() => {}}
          createdKey={null}
          onOpenRepo={onOpenRepo}
        />
      </AuthProvider>
    );
    await user.click(screen.getByRole('button', { name: 'Open repository acme/gears' }));
    expect(onOpenRepo).toHaveBeenCalledWith('acme', 'gears');
  });
});
