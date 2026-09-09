import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { jsonResponse, renderDashboard, seedDashboardUrl, resetDashboardUrl } from './canvas-test-kit';
import type { AccountOverview, OverviewResponse, RepoSessionsResponse } from '@/lib/api/types';

// URL-addressable dashboard locations: `/dashboard?owner=&repo=&session=`.
//
// `BrowserRouter` (via the shared test kit) is what makes these assertions
// meaningful — `window.location.search` reflects real navigation, so a test can
// check the URL the same way a user would read the address bar.

const accounts: AccountOverview[] = [
  {
    login: 'shining',
    kind: 'personal',
    owner: true,
    installed: true,
    installation_id: 11,
    repository_selection: 'all',
    counts_complete: true,
    repos: [
      {
        id: 1,
        owner: 'shining',
        name: 'lab',
        private: false,
        admin: true,
        installed: true,
        viewer_visible: true,
        active_sessions: 1,
        packages: [],
      },
      {
        id: 2,
        owner: 'shining',
        name: 'site',
        private: false,
        admin: true,
        installed: true,
        viewer_visible: true,
        active_sessions: 0,
        packages: [],
      },
    ],
  },
];

const overviewBody: OverviewResponse = {
  app_slug: 'chronoai-fkst',
  viewer: { login: 'shining' },
  global_admin: false,
  accounts,
  totals: { sessions: 1, packages: [] },
  broader_oauth_available: false,
};

const session = (id: string | null, trigger: number, name: string) => ({
  session_id: id,
  name,
  creator: 'shining',
  work_label: 'lab-work',
  work_labels: ['lab-work'],
  auto_merge: null,
  environment: null,
  source_branch: null,
  target_branch: 'fkst-hosted-default',
  packages: [],
  manifests: [],
  log_access: [],
  collaborators: [],
  output_lang: null,
  invalid_reason: null,
  status_labels: [],
  trigger: {
    number: trigger,
    title: name,
    state: 'open',
    author: 'shining',
    labels: [],
    html_url: `https://github.com/shining/lab/issues/${trigger}`,
    created_at: '2026-07-01T00:00:00Z',
    updated_at: '2026-07-02T00:00:00Z',
    closed_at: null,
  },
  work_issues: [],
  log_url: null,
  liveness: 'live',
  prs: [],
});

const sessionsBody = {
  owner: 'shining',
  name: 'lab',
  installed: true,
  sessions: [session('aaaa1111', 7, 'nightly'), session(null, 9, 'pending-session')],
} as unknown as RepoSessionsResponse;

/** Serve the overview plus any repo's sessions. */
function stubAll(overview: OverviewResponse = overviewBody) {
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.endsWith('/api/v1/overview')) return jsonResponse(overview);
    if (/\/api\/v1\/repos\/[^/]+\/[^/]+\/sessions$/.test(url)) return jsonResponse(sessionsBody);
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

describe('Dashboard — URL-addressable locations', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
    resetDashboardUrl();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    resetDashboardUrl();
  });

  // ---- reading the URL ----------------------------------------------------

  it('opens the repository workspace straight from a deep link', async () => {
    stubAll();
    seedDashboardUrl('?owner=shining&repo=lab');
    renderDashboard();

    // The whole point: no clicking through root → account → repo first.
    expect(await screen.findByTestId('repo-workspace')).toBeInTheDocument();
    // The name appears in both the rail card and the detail pane.
    expect((await screen.findAllByText('nightly')).length).toBeGreaterThan(0);
  });

  it('opens an account level from a deep link', async () => {
    stubAll();
    seedDashboardUrl('?owner=shining');
    renderDashboard();

    // The account level lists the account's repositories.
    expect(
      (await screen.findAllByRole('button', { name: 'Open repository shining/lab' })).length
    ).toBeGreaterThan(0);
  });

  it('selects the session named by the URL', async () => {
    stubAll();
    seedDashboardUrl('?owner=shining&repo=lab&session=trigger-9');
    renderDashboard();

    const detail = await screen.findByTestId('session-detail');
    // The second session, not the default first one.
    expect(detail).toHaveTextContent('pending-session');
  });

  it('accepts the trigger-<n> alias for a session that already has an id', async () => {
    // A chat card can only mint `trigger-<n>` before the session acquires a
    // session_id; the link must keep working afterwards.
    stubAll();
    seedDashboardUrl('?owner=shining&repo=lab&session=trigger-7');
    renderDashboard();

    const detail = await screen.findByTestId('session-detail');
    expect(detail).toHaveTextContent('nightly');
  });

  it('falls back to the first session for an unknown session key', async () => {
    stubAll();
    seedDashboardUrl('?owner=shining&repo=lab&session=not-a-session');
    renderDashboard();

    const detail = await screen.findByTestId('session-detail');
    expect(detail).toHaveTextContent('nightly');
  });

  // ---- invalid locations --------------------------------------------------

  it('falls back to the root and clears the URL for an unknown owner', async () => {
    stubAll();
    seedDashboardUrl('?owner=ghost&repo=nope');
    renderDashboard();

    // Root shows the account cards, and the bad parameters are gone so a refresh
    // does not re-open the broken level.
    await screen.findAllByRole('button', { name: 'Open account shining' });
    await waitFor(() => expect(window.location.search).toBe(''));
  });

  it('falls back to the root for a known owner with an unknown repo', async () => {
    stubAll();
    seedDashboardUrl('?owner=shining&repo=ghost');
    renderDashboard();

    await screen.findAllByRole('button', { name: 'Open account shining' });
    await waitFor(() => expect(window.location.search).toBe(''));
  });

  // ---- writing the URL ----------------------------------------------------

  it('writes the level as the user drills in', async () => {
    stubAll();
    renderDashboard();

    fireEvent.click((await screen.findAllByRole('button', { name: 'Open account shining' }))[0]!);
    await waitFor(() => expect(window.location.search).toBe('?owner=shining'));

    fireEvent.click((await screen.findAllByRole('button', { name: 'Open repository shining/lab' }))[0]!);
    await waitFor(() => expect(window.location.search).toBe('?owner=shining&repo=lab'));
  });

  it('writes the session as the user selects one', async () => {
    stubAll();
    seedDashboardUrl('?owner=shining&repo=lab');
    renderDashboard();

    await screen.findByTestId('repo-workspace');
    fireEvent.click((await screen.findAllByRole('button', { name: /pending-session/ }))[0]!);
    await waitFor(() =>
      expect(window.location.search).toBe('?owner=shining&repo=lab&session=trigger-9')
    );
  });

  it('rewrites the URL when Escape walks up a level', async () => {
    stubAll();
    seedDashboardUrl('?owner=shining&repo=lab');
    renderDashboard();

    await screen.findByTestId('repo-workspace');
    fireEvent.keyDown(window, { key: 'Escape' });
    await waitFor(() => expect(window.location.search).toBe('?owner=shining'));

    fireEvent.keyDown(window, { key: 'Escape' });
    await waitFor(() => expect(window.location.search).toBe(''));
  });

  it('does not carry one repository session selection into another', async () => {
    stubAll();
    seedDashboardUrl('?owner=shining&repo=lab&session=trigger-9');
    renderDashboard();

    await screen.findByTestId('repo-workspace');
    // Walk up to the account, then into the OTHER repository.
    fireEvent.keyDown(window, { key: 'Escape' });
    await waitFor(() => expect(window.location.search).toBe('?owner=shining'));
    fireEvent.click((await screen.findAllByRole('button', { name: 'Open repository shining/site' }))[0]!);
    await waitFor(() => expect(window.location.search).toBe('?owner=shining&repo=site'));
  });

  it('does not fill the back stack while browsing levels', async () => {
    stubAll();
    renderDashboard();

    await screen.findAllByRole('button', { name: 'Open account shining' });
    const before = window.history.length;
    fireEvent.click((await screen.findAllByRole('button', { name: 'Open account shining' }))[0]!);
    await waitFor(() => expect(window.location.search).toBe('?owner=shining'));
    // Back must leave `/dashboard`, matching the behaviour before deep links.
    expect(window.history.length).toBe(before);
  });

  // ---- poll safety --------------------------------------------------------

  it('keeps the deep-linked level across an overview refetch', async () => {
    // Params are written only from explicit navigation, and the URL is read once,
    // so a poll landing new overview data must not move the user.
    stubAll();
    seedDashboardUrl('?owner=shining&repo=lab');
    renderDashboard();

    await screen.findByTestId('repo-workspace');
    fireEvent.click(await screen.findByRole('button', { name: /Refresh/i }));
    await waitFor(() => expect(window.location.search).toBe('?owner=shining&repo=lab'));
    expect(screen.getByTestId('repo-workspace')).toBeInTheDocument();
  });
});
