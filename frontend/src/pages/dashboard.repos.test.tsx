import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Dashboard } from './dashboard';
import { AuthProvider } from '@/lib/auth/github-auth';
import type { AccountOverview, OverviewResponse, RepoOverview } from '@/lib/api/types';

// The 15 repository-management scenarios of the old flat dashboard, ported to
// the canvas UI: accounts live at level 0 (sidebar rows + canvas nodes),
// repositories at level 1 (drill in via an "Open account" affordance), and
// everything is served by GET /api/v1/overview.

function jsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  } as Response;
}

let nextRepoId = 1;
const repo = (
  over: Partial<RepoOverview> & Pick<RepoOverview, 'owner' | 'name'>
): RepoOverview => ({
  id: nextRepoId++,
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

const overviewBody = (
  accounts: AccountOverview[],
  appSlug: string | null = 'chronoai-fkst'
): OverviewResponse => ({
  app_slug: appSlug,
  viewer: { login: 'shining' },
  accounts,
  totals: { sessions: 0, packages: [] },
});

/** Stub global fetch: GET /api/v1/overview gets the given body/status. */
function stubApi(body: OverviewResponse | null, status = 200) {
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url.endsWith('/api/v1/overview') && init?.method === undefined) {
      return jsonResponse(body, status);
    }
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

/** Stub for the create flow: the overview serves `initial` until a successful
 *  POST /api/v1/repos, then `afterCreate`. */
function stubCreateApi(opts: {
  initial: OverviewResponse;
  afterCreate: OverviewResponse;
  post: { status: number; body: unknown };
}) {
  let created = false;
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url.endsWith('/api/v1/repos') && init?.method === 'POST') {
      if (opts.post.status < 300) created = true;
      return jsonResponse(opts.post.body, opts.post.status);
    }
    if (url.endsWith('/api/v1/overview')) {
      return jsonResponse(created ? opts.afterCreate : opts.initial);
    }
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

/** Stub for the danger flows: the overview serves `initial` until a
 *  successful DELETE (any path), then `after`. */
function stubDeleteApi(opts: {
  initial: OverviewResponse;
  after: OverviewResponse;
  del: { status: number; body?: unknown };
}) {
  let deleted = false;
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (init?.method === 'DELETE') {
      if (opts.del.status < 300) deleted = true;
      return jsonResponse(opts.del.body ?? null, opts.del.status);
    }
    if (url.endsWith('/api/v1/overview')) {
      return jsonResponse(deleted ? opts.after : opts.initial);
    }
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

type FetchMock = ReturnType<typeof stubApi>;

const overviewGetCalls = (fetchMock: FetchMock) =>
  fetchMock.mock.calls.filter(([input]) => String(input).endsWith('/api/v1/overview')).length;

const repoPostCall = (fetchMock: FetchMock) =>
  fetchMock.mock.calls.find(
    ([input, init]) => String(input).endsWith('/api/v1/repos') && init?.method === 'POST'
  );

const deleteCall = (fetchMock: FetchMock) =>
  fetchMock.mock.calls.find(([, init]) => init?.method === 'DELETE');

function renderDashboard() {
  return render(
    <AuthProvider>
      <Dashboard />
    </AuthProvider>
  );
}

/** Drill into an account (canvas node or sidebar affordance — same label).
 *  fireEvent, not user-event: the full pointer sequence trips d3-zoom's
 *  jsdom-null event.view. */
async function openAccount(login: string) {
  const buttons = await screen.findAllByRole('button', { name: `Open account ${login}` });
  fireEvent.click(buttons[0]!);
}

describe('Dashboard — repository management on the canvas', () => {
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
