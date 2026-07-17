import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {
  account,
  openAccount,
  overviewBody,
  overviewGetCalls,
  renderDashboard,
  repo,
  stubApi,
} from './canvas-test-kit';

// The repository-browsing half of the old flat dashboard's 15 scenarios,
// ported to the canvas UI: accounts live at level 0 (sidebar rows + canvas
// nodes), repositories at level 1 (drill in via an "Open account"
// affordance), everything served by GET /api/v1/overview. The admin flows
// (create / connect / uninstall) live in dashboard.repo-admin.test.tsx.

describe('Dashboard — repository browsing on the canvas', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('renders a row per repo with visibility/org badges and a GitHub link', async () => {
    stubApi(
      overviewBody([
        account({ login: 'shining' }),
        account({
          login: 'acme',
          kind: 'org',
          repos: [
            repo({ owner: 'acme', name: 'widgets', private: true, installed: true }),
            repo({ owner: 'acme', name: 'gears' }),
          ],
        }),
      ])
    );
    renderDashboard();
    await openAccount('acme');

    const link = await screen.findByRole('link', { name: 'acme/widgets' });
    expect(link).toHaveAttribute('href', 'https://github.com/acme/widgets');
    expect(link).toHaveAttribute('target', '_blank');
    expect(link).toHaveAttribute('rel', 'noreferrer');
    // Visibility chips ride both the sidebar rows and the canvas nodes.
    expect(screen.getAllByText('private').length).toBeGreaterThan(0);
    expect(screen.getAllByText('public').length).toBeGreaterThan(0);
    expect(screen.getAllByText('org').length).toBeGreaterThan(0);
  });

  it('shows Installed for installed repos and an Install link (with admin hint) otherwise', async () => {
    stubApi(
      overviewBody([
        account({
          login: 'acme',
          kind: 'org',
          installed: true,
          installation_id: 22,
          repos: [
            repo({ owner: 'acme', name: 'widgets', private: true, installed: true }),
            repo({ owner: 'acme', name: 'gears', private: true, admin: false }),
          ],
        }),
      ])
    );
    renderDashboard();
    await openAccount('acme');

    expect(await screen.findByText('✓ Installed')).toBeInTheDocument();
    const install = screen.getByRole('link', { name: 'Install' });
    expect(install).toHaveAttribute(
      'href',
      'https://github.com/apps/chronoai-fkst/installations/new'
    );
    expect(install).toHaveAttribute('target', '_blank');
    expect(install).toHaveAttribute('rel', 'noreferrer');
    // admin=false → the approval-request hint rides on the link's title.
    expect(install).toHaveAttribute(
      'title',
      'You are not an admin of this repository — GitHub may send an approval request to its owner.'
    );
  });

  it('re-fetches the overview when Refresh is clicked', async () => {
    const user = userEvent.setup();
    const fetchMock = stubApi(overviewBody([account({ login: 'shining' })]));
    renderDashboard();

    expect(await screen.findByText('Legend')).toBeInTheDocument();
    expect(overviewGetCalls(fetchMock)).toBe(1);

    await user.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(overviewGetCalls(fetchMock)).toBe(2));
  });

  it('shows a compact error line when the overview endpoint fails', async () => {
    stubApi(null, 500);
    renderDashboard();

    expect(
      await screen.findByText('Could not load your repositories. Please try again.')
    ).toBeInTheDocument();
  });

  it('shows the not-configured note and no Install/Connect links when app_slug is null', async () => {
    stubApi(
      overviewBody(
        [account({ login: 'shining', repos: [repo({ owner: 'shining', name: 'gears' })] })],
        null
      )
    );
    renderDashboard();

    expect(
      await screen.findByText(
        'The GitHub App is not configured for this deployment yet, so install links are unavailable.'
      )
    ).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Connect' })).not.toBeInTheDocument();

    await openAccount('shining');
    expect(await screen.findByRole('link', { name: 'shining/gears' })).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Install' })).not.toBeInTheDocument();
  });

  it('lists accounts personal-first then orgs, with counts and empty org groups', async () => {
    stubApi(
      overviewBody([
        // The backend contract orders personal first, then orgs sorted; the
        // UI must preserve that order.
        account({ login: 'shining', repos: [repo({ owner: 'shining', name: 'lab' })] }),
        account({
          login: 'acme',
          kind: 'org',
          repos: [
            repo({ owner: 'acme', name: 'widgets', installed: true }),
            repo({ owner: 'acme', name: 'gears' }),
          ],
        }),
        account({ login: 'zeta', kind: 'org' }), // org with no repos
      ])
    );
    renderDashboard();

    const owners = await screen.findAllByRole('heading', { level: 3 });
    expect(owners.map((h) => h.textContent)).toEqual(['shining', 'acme', 'zeta']);
    expect(screen.getAllByText('Personal').length).toBeGreaterThan(0);
    expect(screen.getAllByText('Organization').length).toBeGreaterThan(0);
    // Per-account installed/total counts.
    expect(screen.getByText('0/1 installed')).toBeInTheDocument(); // shining
    expect(screen.getByText('1/2 installed')).toBeInTheDocument(); // acme
    expect(screen.getByText('0/0 installed')).toBeInTheDocument(); // zeta (empty)
    // The repo-less org still renders as a (labelled) creation target.
    expect(screen.getByText('No repositories yet.')).toBeInTheDocument();
  });

  it('filters accounts by name substring (case-insensitive) at level 0', async () => {
    const user = userEvent.setup();
    stubApi(
      overviewBody([
        account({ login: 'shining', repos: [repo({ owner: 'shining', name: 'lab' })] }),
        account({
          login: 'acme',
          kind: 'org',
          repos: [repo({ owner: 'acme', name: 'widgets' })],
        }),
      ])
    );
    renderDashboard();

    const box = await screen.findByLabelText('Filter accounts…');
    await user.type(box, 'ACM');
    const owners = screen.getAllByRole('heading', { level: 3 });
    expect(owners.map((h) => h.textContent)).toEqual(['acme']);
    expect(screen.queryAllByRole('button', { name: 'Open account shining' })).toHaveLength(0);

    await user.clear(box);
    expect(screen.getAllByRole('heading', { level: 3 })).toHaveLength(2);

    await user.type(box, 'zzz');
    expect(screen.getByText('No accounts match your filter.')).toBeInTheDocument();
  });

  it('filters repos by name substring at level 1', async () => {
    const user = userEvent.setup();
    stubApi(
      overviewBody([
        account({
          login: 'acme',
          kind: 'org',
          repos: [
            repo({ owner: 'acme', name: 'widgets' }),
            repo({ owner: 'acme', name: 'gears' }),
          ],
        }),
      ])
    );
    renderDashboard();
    await openAccount('acme');

    const box = await screen.findByLabelText('Filter repositories…');
    await user.type(box, 'WIDG');
    expect(screen.getByRole('link', { name: 'acme/widgets' })).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'acme/gears' })).not.toBeInTheDocument();

    await user.clear(box);
    await user.type(box, 'zzz');
    expect(screen.getByText('No repositories match your filter.')).toBeInTheDocument();
  });
});
