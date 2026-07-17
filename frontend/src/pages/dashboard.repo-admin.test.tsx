import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {
  account,
  deleteCall,
  openAccount,
  overviewBody,
  overviewGetCalls,
  renderDashboard,
  repo,
  repoPostCall,
  stubApi,
  stubCreateApi,
  stubDeleteApi,
} from './canvas-test-kit';

// The repository-administration half of the ported dashboard scenarios:
// repo creation, and the account-level Connect / Manage / Uninstall flows.

describe('Dashboard — repository administration on the canvas', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('creates a personal repo then highlights it and points at the Install step', async () => {
    const user = userEvent.setup();
    const initial = overviewBody([
      account({
        login: 'shining',
        installed: true,
        installation_id: 11,
        repos: [repo({ owner: 'shining', name: 'lab', installed: true })],
      }),
      account({ login: 'acme', kind: 'org' }),
    ]);
    const created = repo({ owner: 'shining', name: 'rocket', private: true });
    const afterCreate = overviewBody([
      {
        ...initial.accounts[0]!,
        repos: [...initial.accounts[0]!.repos, created],
      },
      initial.accounts[1]!,
    ]);
    const fetchMock = stubCreateApi({
      initial,
      afterCreate,
      post: { status: 201, body: { ...created, org: false } },
    });
    renderDashboard();

    await user.click(await screen.findByRole('button', { name: 'New repository' }));
    const dialog = await screen.findByRole('dialog');
    // Private defaults to ON.
    expect(within(dialog).getByRole('checkbox')).toBeChecked();
    await user.type(within(dialog).getByLabelText('Repository name'), 'rocket');
    await user.click(within(dialog).getByRole('button', { name: 'Create repository' }));

    // Personal owner is sent as null; no description key when left empty.
    await waitFor(() => expect(repoPostCall(fetchMock)).toBeDefined());
    const [, postInit] = repoPostCall(fetchMock)!;
    expect(JSON.parse(String(postInit!.body))).toEqual({
      owner: null,
      name: 'rocket',
      private: true,
    });

    // Modal closes, the page zooms into the owner account, the new row shows
    // up highlighted with the guided next-step callout.
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(await screen.findByRole('link', { name: 'shining/rocket' })).toBeInTheDocument();
    expect(screen.getByText('Next: install the App on this repo')).toBeInTheDocument();
    const installs = screen.getAllByRole('link', { name: 'Install' });
    expect(installs).toHaveLength(2); // the row's own button + the callout's
    for (const a of installs) {
      expect(a).toHaveAttribute('href', 'https://github.com/apps/chronoai-fkst/installations/new');
    }
    expect(document.querySelector('.anim-repo-pulse')).not.toBeNull();
  });

  it('shows the server error message verbatim and keeps the modal open on create failure', async () => {
    const user = userEvent.setup();
    const initial = overviewBody([
      account({ login: 'shining' }),
      account({
        login: 'acme',
        kind: 'org',
        repos: [repo({ owner: 'acme', name: 'widgets' })],
      }),
    ]);
    const message = 'The GitHub App is missing the Administration permission on acme.';
    const fetchMock = stubCreateApi({
      initial,
      afterCreate: initial,
      post: { status: 403, body: { error: 'app_permission', message } },
    });
    renderDashboard();

    await user.click(await screen.findByRole('button', { name: 'New repository' }));
    const dialog = await screen.findByRole('dialog');
    await user.selectOptions(within(dialog).getByLabelText('Owner'), 'acme');
    await user.type(within(dialog).getByLabelText('Repository name'), 'rocket');
    await user.type(within(dialog).getByLabelText('Description (optional)'), 'lift off');
    await user.click(within(dialog).getByRole('button', { name: 'Create repository' }));

    // The envelope's message is displayed verbatim inside the (still open) form.
    expect(await within(dialog).findByText(message)).toBeInTheDocument();
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    // Org owner + description were carried in the POST body.
    const [, postInit] = repoPostCall(fetchMock)!;
    expect(JSON.parse(String(postInit!.body))).toEqual({
      owner: 'acme',
      name: 'rocket',
      private: true,
      description: 'lift off',
    });
    // The submit button is usable again for a retry.
    expect(within(dialog).getByRole('button', { name: 'Create repository' })).toBeEnabled();
  });

  it('disables Create until the repo name is valid', async () => {
    const user = userEvent.setup();
    stubApi(
      overviewBody([
        account({ login: 'shining', repos: [repo({ owner: 'shining', name: 'lab' })] }),
      ])
    );
    renderDashboard();

    await user.click(await screen.findByRole('button', { name: 'New repository' }));
    const dialog = await screen.findByRole('dialog');
    const submit = within(dialog).getByRole('button', { name: 'Create repository' });
    expect(submit).toBeDisabled(); // empty name

    const name = within(dialog).getByLabelText('Repository name');
    await user.type(name, 'bad name!');
    expect(submit).toBeDisabled();

    await user.clear(name);
    await user.type(name, 'Good-name.1_x');
    expect(submit).toBeEnabled();
  });

  it('shows a Connect CTA (with hint) on an account without an installation', async () => {
    stubApi(
      overviewBody([
        account({ login: 'shining', repos: [repo({ owner: 'shining', name: 'lab' })] }),
      ])
    );
    renderDashboard();

    const connect = await screen.findByRole('link', { name: 'Connect' });
    expect(connect).toHaveAttribute(
      'href',
      'https://github.com/apps/chronoai-fkst/installations/new'
    );
    expect(connect).toHaveAttribute('target', '_blank');
    expect(connect).toHaveAttribute('rel', 'noreferrer');
    expect(
      screen.getByText('Connect to enable repository creation and fkst sessions.')
    ).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Manage' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Uninstall' })).not.toBeInTheDocument();
  });

  it('shows Manage (exact per-account settings URL) and Uninstall on connected accounts', async () => {
    stubApi(
      overviewBody([
        account({
          login: 'shining',
          installed: true,
          installation_id: 11,
          repository_selection: 'all',
          repos: [repo({ owner: 'shining', name: 'lab', installed: true })],
        }),
        account({
          login: 'acme',
          kind: 'org',
          installed: true,
          installation_id: 22,
          repository_selection: 'selected',
          repos: [repo({ owner: 'acme', name: 'widgets', installed: true })],
        }),
      ])
    );
    renderDashboard();

    const manages = await screen.findAllByRole('link', { name: 'Manage' });
    // Personal account renders first, then orgs — each with its own settings page.
    expect(manages.map((a) => a.getAttribute('href'))).toEqual([
      'https://github.com/settings/installations/11',
      'https://github.com/organizations/acme/settings/installations/22',
    ]);
    for (const a of manages) {
      expect(a).toHaveAttribute('target', '_blank');
      expect(a).toHaveAttribute('rel', 'noreferrer');
    }
    expect(screen.getAllByRole('button', { name: 'Uninstall' })).toHaveLength(2);
    expect(screen.queryByRole('link', { name: 'Connect' })).not.toBeInTheDocument();
  });

  it('uninstalls an account after confirmation and re-fetches the overview', async () => {
    const user = userEvent.setup();
    const initial = overviewBody([
      account({
        login: 'shining',
        installed: true,
        installation_id: 11,
        repository_selection: 'all',
        repos: [repo({ owner: 'shining', name: 'lab', installed: true })],
      }),
    ]);
    const fetchMock = stubDeleteApi({
      initial,
      after: overviewBody([
        account({ login: 'shining', repos: [repo({ owner: 'shining', name: 'lab' })] }),
      ]),
      del: { status: 204 },
    });
    renderDashboard();

    await user.click(await screen.findByRole('button', { name: 'Uninstall' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('Uninstall from shining?')).toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: 'Uninstall' }));

    await waitFor(() => expect(deleteCall(fetchMock)).toBeDefined());
    expect(String(deleteCall(fetchMock)![0])).toMatch(/\/api\/v1\/installations\/shining$/);
    // Dialog closes and the overview re-fetches; the account is not connected.
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    await waitFor(() => expect(overviewGetCalls(fetchMock)).toBe(2));
    expect(await screen.findByRole('link', { name: 'Connect' })).toBeInTheDocument();
  });

  it('keeps the uninstall dialog open and shows the envelope message on failure', async () => {
    const user = userEvent.setup();
    const initial = overviewBody([
      account({
        login: 'shining',
        installed: true,
        installation_id: 11,
        repos: [repo({ owner: 'shining', name: 'lab', installed: true })],
      }),
    ]);
    const message = 'No installation found for this account.';
    const fetchMock = stubDeleteApi({
      initial,
      after: initial,
      del: { status: 404, body: { error: 'not_found', message } },
    });
    renderDashboard();

    await user.click(await screen.findByRole('button', { name: 'Uninstall' }));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByRole('button', { name: 'Uninstall' }));

    expect(await within(dialog).findByText(message)).toBeInTheDocument();
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(overviewGetCalls(fetchMock)).toBe(1); // no re-fetch on failure
  });

  it('shows no in-app per-repo Remove — installed rows carry the manage-on-GitHub hint', async () => {
    // GitHub only allows per-repo selection changes on the installation
    // settings page (not via our App user-to-server token), so an installed
    // row has no Remove action; the account-level Manage link is the entry
    // point for repo selection.
    stubApi(
      overviewBody([
        account({
          login: 'shining',
          installed: true,
          installation_id: 11,
          repository_selection: 'selected',
          repos: [repo({ owner: 'shining', name: 'lab', installed: true })],
        }),
      ])
    );
    renderDashboard();

    expect(await screen.findByRole('link', { name: 'Manage' })).toHaveAttribute(
      'href',
      'https://github.com/settings/installations/11'
    );

    await openAccount('shining');
    const installedMark = await screen.findByText('✓ Installed');
    expect(installedMark).toHaveAttribute(
      'title',
      'Manage this repository on GitHub (add or remove it there).'
    );
    expect(screen.queryByRole('button', { name: 'Remove' })).not.toBeInTheDocument();
  });
});
