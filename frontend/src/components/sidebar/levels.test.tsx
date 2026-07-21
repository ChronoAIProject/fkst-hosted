import { describe, it, expect, beforeEach, vi } from 'vitest';
import type { ReactNode } from 'react';
import { render, screen, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AuthProvider } from '@/lib/auth/github-auth';
import type {
  AccountOverview,
  IssueDetail,
  OverviewResponse,
  RepoOverview,
  SessionDetail,
} from '@/lib/api/types';
import { formatAbsolute } from '@/lib/format';
import { Level0Sidebar } from './level0';
import { Level1Sidebar } from './level1';
import { SessionCard } from './session-card';

// The sidebars are intentionally router-free (the first-run get-started link is
// a plain anchor), matching the dashboard test harness that mounts them without
// a Router — so the auth store is the only provider needed.
function wrap(ui: ReactNode) {
  return <AuthProvider>{ui}</AuthProvider>;
}

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
    broader_oauth_available: false,
  }) satisfies OverviewResponse;

describe('Level0Sidebar', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });

  it('states what the view is, shows the legend, charts and account rows', () => {
    render(
      wrap(
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
      )
    );

    // Plain statement of what the view represents + the three-status legend.
    expect(screen.getByText(/every GitHub account you can reach/)).toBeInTheDocument();
    // The status legend is collapsed by default (to lift the primary content
    // higher in the height-constrained panel); expand it to read the color key.
    fireEvent.click(screen.getByText('Legend'));
    expect(screen.getByText('Grey — App not installed')).toBeInTheDocument();
    expect(screen.getByText('Blue — App installed, no active sessions')).toBeInTheDocument();
    expect(screen.getByText('Blinking blue — active sessions running')).toBeInTheDocument();

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

    // At least one installation exists → the first-run CTA stays hidden.
    expect(screen.queryByText('Get started with fkst')).not.toBeInTheDocument();
  });

  it('shows the first-run Install CTA when no installation exists anywhere', () => {
    render(
      wrap(
        <Level0Sidebar
          // Accounts are reachable but none is installed — the viewer still
          // needs the prominent primary path, not just per-row Connect links.
          overview={overview([account({ login: 'shining' }), account({ login: 'acme', kind: 'org' })])}
          query=""
          onQueryChange={() => {}}
          onOpenAccount={() => {}}
          onRepoCreated={() => {}}
          onChanged={() => {}}
        />
      )
    );

    expect(screen.getByText('Get started with fkst')).toBeInTheDocument();
    expect(
      screen.getByText(/Install the GitHub App on your account or an organization/)
    ).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Install the GitHub App' })).toHaveAttribute(
      'href',
      'https://github.com/apps/chronoai-fkst/installations/new'
    );
    // The get-started link routes to the guide page.
    expect(screen.getByRole('link', { name: 'How it works →' })).toHaveAttribute(
      'href',
      '/get-started'
    );
    // Account rows still render below the callout so specific accounts can be
    // connected individually.
    expect(screen.getAllByRole('link', { name: 'Connect' })).toHaveLength(2);
  });

  it('replaces the muted no-accounts line with the first-run CTA at zero accounts', () => {
    render(
      wrap(
        <Level0Sidebar
          overview={overview([])}
          query=""
          onQueryChange={() => {}}
          onOpenAccount={() => {}}
          onRepoCreated={() => {}}
          onChanged={() => {}}
        />
      )
    );

    // The callout carries the CTA; the bare "No accounts found." line is gone.
    expect(screen.getByText('Get started with fkst')).toBeInTheDocument();
    expect(screen.queryByText('No accounts found.')).not.toBeInTheDocument();
  });

  it('falls back to the muted no-accounts line when the App is not configured', () => {
    render(
      wrap(
        <Level0Sidebar
          overview={overview([], null)}
          query=""
          onQueryChange={() => {}}
          onOpenAccount={() => {}}
          onRepoCreated={() => {}}
          onChanged={() => {}}
        />
      )
    );

    // No app_slug → no install URL to point at → no callout, muted line only.
    expect(screen.queryByText('Get started with fkst')).not.toBeInTheDocument();
    expect(screen.getByText('No accounts found.')).toBeInTheDocument();
    expect(screen.getByText(/The GitHub App is not configured for this deployment/)).toBeInTheDocument();
  });

  it('filters account rows by the query and shows the empty-filter note', async () => {
    const user = userEvent.setup();
    const onQueryChange = vi.fn();
    const { rerender } = render(
      wrap(
        <Level0Sidebar
          overview={overview([
            account({ login: 'shining', installation_id: 11 }),
            account({ login: 'acme', kind: 'org', installation_id: 12 }),
          ])}
          query=""
          onQueryChange={onQueryChange}
          onOpenAccount={() => {}}
          onRepoCreated={() => {}}
          onChanged={() => {}}
        />
      )
    );

    await user.type(screen.getByLabelText('Filter accounts…'), 'z');
    expect(onQueryChange).toHaveBeenCalledWith('z');

    rerender(
      wrap(
        <Level0Sidebar
          overview={overview([
            account({ login: 'shining', installation_id: 11 }),
            account({ login: 'acme', kind: 'org', installation_id: 12 }),
          ])}
          query="zzz"
          onQueryChange={onQueryChange}
          onOpenAccount={() => {}}
          onRepoCreated={() => {}}
          onChanged={() => {}}
        />
      )
    );
    expect(screen.getByText('No accounts match your filter.')).toBeInTheDocument();
  });

  it('resets the chart scope to All when the filter removes the scoped account', async () => {
    const user = userEvent.setup();
    // Give both accounts an installation so the first-run CTA does not steal
    // focus from the scope-clamp behavior under test.
    const data = [
      account({ login: 'shining', installation_id: 11 }),
      account({ login: 'acme', kind: 'org', installation_id: 12 }),
    ];
    const props = {
      onQueryChange: () => {},
      onOpenAccount: () => {},
      onRepoCreated: () => {},
      onChanged: () => {},
    };
    const { rerender } = render(
      wrap(<Level0Sidebar overview={overview(data)} query="" {...props} />)
    );

    const scope = screen.getByLabelText('Scope charts to an account');
    await user.selectOptions(scope, 'acme');
    expect(scope).toHaveValue('acme');

    // Filtering 'acme' away must not leave the select on an invisible value.
    rerender(wrap(<Level0Sidebar overview={overview(data)} query="shin" {...props} />));
    expect(scope).toHaveValue('');
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
      wrap(
        <Level1Sidebar
          account={acc}
          appSlug="chronoai-fkst"
          query=""
          onQueryChange={() => {}}
          createdKey={null}
          onOpenRepo={() => {}}
        />
      )
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

  it('shows a persistent needs-install badge on not-yet-installed repos only', () => {
    render(
      wrap(
        <Level1Sidebar
          account={acc}
          appSlug="chronoai-fkst"
          query=""
          onQueryChange={() => {}}
          // No createdKey: the badge is derived purely from repo state, so it
          // must appear even without the transient freshly-created callout.
          createdKey={null}
          onOpenRepo={() => {}}
        />
      )
    );

    // Exactly one not-installed repo (gears) → exactly one badge, carrying the
    // explanatory tooltip; the installed repo (widgets) has none.
    const badges = screen.getAllByText('Needs install');
    expect(badges).toHaveLength(1);
    expect(badges[0]!.closest('[title]')).toHaveAttribute(
      'title',
      'Install the App on this repository so its sessions can run here.'
    );
  });

  it('omits the needs-install badge when the App is not configured', () => {
    render(
      wrap(
        <Level1Sidebar
          account={acc}
          appSlug={null}
          query=""
          onQueryChange={() => {}}
          createdKey={null}
          onOpenRepo={() => {}}
        />
      )
    );

    // With no app_slug there is no install path, so the badge is suppressed.
    expect(screen.queryByText('Needs install')).not.toBeInTheDocument();
  });

  it('resets the chart scope to All when the filter removes the scoped repo', async () => {
    const user = userEvent.setup();
    const props = {
      account: acc,
      appSlug: 'chronoai-fkst',
      onQueryChange: () => {},
      createdKey: null,
      onOpenRepo: () => {},
    };
    const { rerender } = render(wrap(<Level1Sidebar {...props} query="" />));

    const scope = screen.getByLabelText('Scope charts to a repository');
    await user.selectOptions(scope, 'gears');
    expect(scope).toHaveValue('gears');

    rerender(wrap(<Level1Sidebar {...props} query="widg" />));
    expect(scope).toHaveValue('');
  });

  it('opens a repo from the row affordance', async () => {
    const user = userEvent.setup();
    const onOpenRepo = vi.fn();
    render(
      wrap(
        <Level1Sidebar
          account={acc}
          appSlug="chronoai-fkst"
          query=""
          onQueryChange={() => {}}
          createdKey={null}
          onOpenRepo={onOpenRepo}
        />
      )
    );
    await user.click(screen.getByRole('button', { name: 'Open repository acme/gears' }));
    expect(onOpenRepo).toHaveBeenCalledWith('acme', 'gears');
  });
});

describe('SessionCard', () => {
  const issue = (over: Partial<IssueDetail> = {}): IssueDetail => ({
    number: 7,
    title: 'trigger',
    state: 'open',
    author: 'shining',
    labels: [],
    html_url: 'https://github.com/acme/lab/issues/7',
    created_at: '2026-07-19T10:00:00Z',
    updated_at: '2026-07-19T10:05:00Z',
    closed_at: null,
    ...over,
  });

  const session = (over: Partial<SessionDetail> = {}): SessionDetail => ({
    session_id: 'abcdef1234567890',
    name: 'my-session',
    work_label: 'sa:demo',
    auto_merge: false,
    environment: null,
    packages: [],
    invalid_reason: null,
    status_labels: [],
    trigger: issue(),
    work_issues: [],
    log_url: null,
    liveness: null,
    prs: [],
    ...over,
  });

  it('copies the full session id while showing only the 8-char prefix', () => {
    render(wrap(<SessionCard owner="acme" name="lab" session={session()} onStop={() => {}} />));

    // Visible text is the truncated prefix; the copy button carries the full id.
    expect(screen.getByText('abcdef12')).toBeInTheDocument();
    expect(screen.queryByText('abcdef1234567890')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Copy session ID' })).toBeInTheDocument();
  });

  it('renders created/updated relative time with the absolute value in a tooltip', () => {
    render(wrap(<SessionCard owner="acme" name="lab" session={session()} onStop={() => {}} />));

    // The created stamp shows the localized word and backs it with the full,
    // zone-qualified absolute value as a title tooltip.
    const created = screen.getByText(/created/);
    expect(created).toHaveAttribute('title', formatAbsolute('2026-07-19T10:00:00Z', 'en'));
    expect(screen.getByText(/updated/)).toBeInTheDocument();
    // An open trigger has no closed_at → no closed stamp is rendered.
    expect(screen.queryByText(/closed/)).not.toBeInTheDocument();
  });

  it('shows a closed timestamp only when the trigger carries closed_at', () => {
    render(
      wrap(
        <SessionCard
          owner="acme"
          name="lab"
          session={session({
            trigger: issue({ state: 'closed', closed_at: '2026-07-19T11:00:00Z' }),
          })}
          onStop={() => {}}
        />
      )
    );

    // "closed" appears both as the issue-line state and the timestamp word;
    // the timestamp is the one carrying the absolute-value tooltip.
    const stamp = screen
      .getAllByText(/closed/)
      .find((el) => el.getAttribute('title') === formatAbsolute('2026-07-19T11:00:00Z', 'en'));
    expect(stamp).toBeTruthy();
  });

  it('mount-animates the liveness and status chips so a poll-tick change reads as motion', () => {
    render(
      wrap(
        <SessionCard
          owner="acme"
          name="lab"
          session={session({ liveness: 'live', status_labels: ['degraded'] })}
          onStop={() => {}}
        />
      )
    );

    // Both the liveness and each status chip sit inside an anim-chip-in wrapper
    // (the CSS animation replays on mount when the chip newly appears).
    expect(screen.getByText('live').closest('.anim-chip-in')).not.toBeNull();
    expect(screen.getByText('degraded').closest('.anim-chip-in')).not.toBeNull();
  });
});
