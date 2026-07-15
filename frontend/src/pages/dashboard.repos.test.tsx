import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Dashboard } from './dashboard';
import { AuthProvider } from '@/lib/auth/github-auth';

// The wire shape of GET /api/v1/repos (mirrors the component's DTO).
interface RepoSpec {
  owner: string;
  name: string;
  private: boolean;
  org: boolean;
  admin: boolean;
  installed: boolean;
}
interface ReposBody {
  app_slug: string | null;
  viewer: { login: string };
  orgs: string[];
  repos: RepoSpec[];
}

function jsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  } as Response;
}

/** Stub global fetch: the mount-time cached-dashboard load gets an empty
 *  payload; GET /api/v1/repos gets the given body/status. Returns the mock so
 *  tests can count per-endpoint calls. */
function stubApi(reposBody: ReposBody | null, reposStatus = 200) {
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url.endsWith('/api/v1/dashboard')) {
      return jsonResponse({ last_pulled_at_ms: null, dashboard: null });
    }
    if (url.endsWith('/api/v1/repos') && init?.method !== 'POST') {
      return jsonResponse(reposBody, reposStatus);
    }
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

/** Stub for the create flow: GET /api/v1/repos serves `initial` until a
 *  successful POST, then `afterCreate`; POST answers with the given
 *  status/body (201 repo view or an error envelope). */
function stubCreateApi(opts: {
  initial: ReposBody;
  afterCreate: ReposBody;
  post: { status: number; body: unknown };
}) {
  let created = false;
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url.endsWith('/api/v1/dashboard')) {
      return jsonResponse({ last_pulled_at_ms: null, dashboard: null });
    }
    if (url.endsWith('/api/v1/repos')) {
      if (init?.method === 'POST') {
        if (opts.post.status < 300) created = true;
        return jsonResponse(opts.post.body, opts.post.status);
      }
      return jsonResponse(created ? opts.afterCreate : opts.initial);
    }
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

const repoGetCalls = (fetchMock: ReturnType<typeof stubApi>) =>
  fetchMock.mock.calls.filter(
    ([input, init]) => String(input).endsWith('/api/v1/repos') && init?.method !== 'POST'
  ).length;

const repoPostCall = (fetchMock: ReturnType<typeof stubApi>) =>
  fetchMock.mock.calls.find(
    ([input, init]) => String(input).endsWith('/api/v1/repos') && init?.method === 'POST'
  );

function renderDashboard() {
  return render(
    <AuthProvider>
      <Dashboard />
    </AuthProvider>
  );
}

const repo = (over: Partial<RepoSpec> & Pick<RepoSpec, 'owner' | 'name'>): RepoSpec => ({
  private: false,
  org: false,
  admin: true,
  installed: false,
  ...over,
});

describe('Dashboard — Repositories section', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('renders a row per repo with visibility/org badges and a GitHub link', async () => {
    stubApi({
      app_slug: 'chronoai-fkst',
      viewer: { login: 'shining' },
      orgs: ['acme'],
      repos: [
        { owner: 'acme', name: 'widgets', private: true, org: true, admin: true, installed: true },
        { owner: 'shining', name: 'lab', private: false, org: false, admin: true, installed: false },
      ],
    });
    renderDashboard();

    expect(await screen.findByText('Repositories')).toBeInTheDocument();
    const link = await screen.findByRole('link', { name: 'acme/widgets' });
    expect(link).toHaveAttribute('href', 'https://github.com/acme/widgets');
    expect(link).toHaveAttribute('target', '_blank');
    expect(link).toHaveAttribute('rel', 'noreferrer');
    expect(screen.getByText('private')).toBeInTheDocument();
    expect(screen.getByText('org')).toBeInTheDocument();
    expect(screen.getByText('public')).toBeInTheDocument();
  });

  it('shows Installed for installed repos and an Install link (with admin hint) otherwise', async () => {
    stubApi({
      app_slug: 'chronoai-fkst',
      viewer: { login: 'shining' },
      orgs: ['acme'],
      repos: [
        { owner: 'acme', name: 'widgets', private: true, org: true, admin: true, installed: true },
        { owner: 'acme', name: 'gears', private: true, org: true, admin: false, installed: false },
      ],
    });
    renderDashboard();

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

  it('re-fetches the list when Refresh is clicked', async () => {
    const user = userEvent.setup();
    const fetchMock = stubApi({
      app_slug: 'chronoai-fkst',
      viewer: { login: 'shining' },
      orgs: [],
      repos: [],
    });
    renderDashboard();

    expect(await screen.findByText('No repositories found on your account.')).toBeInTheDocument();
    expect(repoGetCalls(fetchMock)).toBe(1);

    await user.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(repoGetCalls(fetchMock)).toBe(2));
  });

  it('shows a compact error line when the endpoint fails', async () => {
    stubApi(null, 500);
    renderDashboard();

    expect(
      await screen.findByText('Could not load your repositories. Please try again.')
    ).toBeInTheDocument();
  });

  it('shows the not-configured note and no Install buttons when app_slug is null', async () => {
    stubApi({
      app_slug: null,
      viewer: { login: 'acme' },
      orgs: [],
      repos: [
        { owner: 'acme', name: 'gears', private: false, org: false, admin: true, installed: false },
      ],
    });
    renderDashboard();

    expect(await screen.findByRole('link', { name: 'acme/gears' })).toBeInTheDocument();
    expect(
      screen.getByText(
        'The GitHub App is not configured for this deployment yet, so install links are unavailable.'
      )
    ).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Install' })).not.toBeInTheDocument();
  });

  it('groups repos personal-first then orgs alphabetically, with counts and empty org groups', async () => {
    stubApi({
      app_slug: 'chronoai-fkst',
      viewer: { login: 'shining' },
      orgs: ['zeta', 'acme'], // deliberately out of order + zeta has no repos
      repos: [
        repo({ owner: 'acme', name: 'widgets', org: true, installed: true }),
        repo({ owner: 'acme', name: 'gears', org: true }),
        repo({ owner: 'shining', name: 'lab' }),
      ],
    });
    renderDashboard();

    const owners = await screen.findAllByRole('heading', { level: 3 });
    expect(owners.map((h) => h.textContent)).toEqual(['shining', 'acme', 'zeta']);
    expect(screen.getByText('Personal')).toBeInTheDocument();
    expect(screen.getAllByText('Organization')).toHaveLength(2);
    // Per-group installed/total counts.
    expect(screen.getByText('0/1 installed')).toBeInTheDocument(); // shining
    expect(screen.getByText('1/2 installed')).toBeInTheDocument(); // acme
    expect(screen.getByText('0/0 installed')).toBeInTheDocument(); // zeta (empty)
    // The repo-less org still renders as a (labelled) creation target.
    expect(screen.getByText('No repositories yet.')).toBeInTheDocument();
  });

  it('filters rows by owner/name substring (case-insensitive) and collapses empty groups', async () => {
    const user = userEvent.setup();
    stubApi({
      app_slug: 'chronoai-fkst',
      viewer: { login: 'shining' },
      orgs: ['acme'],
      repos: [
        repo({ owner: 'shining', name: 'lab' }),
        repo({ owner: 'acme', name: 'widgets', org: true }),
      ],
    });
    renderDashboard();

    const box = await screen.findByRole('searchbox');
    await user.type(box, 'WIDG');
    expect(screen.getByRole('link', { name: 'acme/widgets' })).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'shining/lab' })).not.toBeInTheDocument();
    // The personal group has no match, so it collapses away entirely.
    expect(screen.queryByText('Personal')).not.toBeInTheDocument();

    await user.clear(box);
    expect(screen.getByRole('link', { name: 'shining/lab' })).toBeInTheDocument();

    await user.type(box, 'zzz');
    expect(screen.getByText('No repositories match your search.')).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'acme/widgets' })).not.toBeInTheDocument();
  });

  it('creates a personal repo then highlights it and points at the Install step', async () => {
    const user = userEvent.setup();
    const initial: ReposBody = {
      app_slug: 'chronoai-fkst',
      viewer: { login: 'shining' },
      orgs: ['acme'],
      repos: [repo({ owner: 'shining', name: 'lab', installed: true })],
    };
    const created = repo({ owner: 'shining', name: 'rocket', private: true });
    const fetchMock = stubCreateApi({
      initial,
      afterCreate: { ...initial, repos: [...initial.repos, created] },
      post: { status: 201, body: created },
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

    // Modal closes, the list re-fetches, the new row shows up highlighted
    // with the guided next-step callout linking to the App install page.
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
    const initial: ReposBody = {
      app_slug: 'chronoai-fkst',
      viewer: { login: 'shining' },
      orgs: ['acme'],
      repos: [repo({ owner: 'acme', name: 'widgets', org: true })],
    };
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
    stubApi({
      app_slug: 'chronoai-fkst',
      viewer: { login: 'shining' },
      orgs: [],
      repos: [repo({ owner: 'shining', name: 'lab' })],
    });
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
});
