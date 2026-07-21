import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { jsonResponse, renderDashboard } from './canvas-test-kit';
import type {
  AccountOverview,
  OverviewResponse,
  RepoSessionsResponse,
} from '@/lib/api/types';

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
        active_sessions: 1,
        packages: ['o/p@main:pkg/base'],
      },
    ],
  },
];

const overviewBody: OverviewResponse = {
  app_slug: 'chronoai-fkst',
  viewer: { login: 'shining' },
  global_admin: false,
  accounts,
  totals: { sessions: 1, packages: [{ package: 'o/p@main:pkg/base', count: 1 }] },
  broader_oauth_available: false,
};

const sessionsBody: RepoSessionsResponse = {
  owner: 'shining',
  name: 'lab',
  installed: true,
  sessions: [
    {
      session_id: 'aaaa1111-2222-3333-4444-555566667777',
      name: 'nightly',
      work_label: 'lab-work',
      auto_merge: null,
      environment: null,
      packages: ['o/p@main:pkg/base'],
      invalid_reason: null,
      status_labels: [],
      trigger: {
        number: 7,
        title: 'nightly',
        state: 'open',
        author: 'shining',
        labels: [],
        html_url: 'https://github.com/shining/lab/issues/7',
        created_at: '2026-07-01T00:00:00Z',
        updated_at: '2026-07-02T00:00:00Z',
        closed_at: null,
      },
      work_issues: [],
      log_url: null,
      liveness: 'live',
      prs: [],
    },
  ],
};

describe('Dashboard — canvas levels and loading', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('renders shimmer skeletons for canvas and sidebar before any data', () => {
    // A fetch that never settles: the page must show its loading state, not
    // a blank canvas.
    vi.stubGlobal(
      'fetch',
      vi.fn(() => new Promise<Response>(() => {}))
    );
    renderDashboard();

    expect(screen.getByTestId('canvas-skeleton')).toBeInTheDocument();
    expect(screen.getByTestId('sidebar-skeleton')).toBeInTheDocument();
    expect(screen.getByLabelText('Loading canvas…')).toBeInTheDocument();
    expect(screen.getByLabelText('Loading details…')).toBeInTheDocument();
  });

  it('marks the App-wide global-administrator view in the dashboard header', async () => {
    const adminBody: OverviewResponse = {
      ...overviewBody,
      global_admin: true,
      broader_oauth_available: false,
    };
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        if (String(input).endsWith('/api/v1/overview')) return jsonResponse(adminBody);
        throw new Error(`unexpected fetch: ${String(input)}`);
      })
    );

    renderDashboard();

    expect(await screen.findByText('Global admin')).toBeInTheDocument();
    expect(screen.queryByText('See all your repositories')).not.toBeInTheDocument();
  });

  it('drills root → account → repo, fetching level-2 sessions, and Escape walks back up', async () => {
    let resolveSessions: ((r: Response) => void) | null = null;
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith('/api/v1/overview')) return jsonResponse(overviewBody);
      if (url.endsWith('/api/v1/repos/shining/lab/sessions')) {
        return new Promise<Response>((resolve) => {
          resolveSessions = resolve;
        });
      }
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    renderDashboard();

    // Level 0 → level 1.
    const openAccountButtons = await screen.findAllByRole('button', {
      name: 'Open account shining',
    });
    fireEvent.click(openAccountButtons[0]!);
    expect(await screen.findByText(/repositories of shining/)).toBeInTheDocument();

    // Level 1 → level 2: the sessions endpoint is hit, skeleton first.
    const openRepoButtons = await screen.findAllByRole('button', {
      name: 'Open repository shining/lab',
    });
    fireEvent.click(openRepoButtons[0]!);
    await waitFor(() =>
      expect(
        fetchMock.mock.calls.some(([input]) =>
          String(input).endsWith('/api/v1/repos/shining/lab/sessions')
        )
      ).toBe(true)
    );
    expect(await screen.findByTestId('canvas-skeleton')).toBeInTheDocument();

    // Data lands → the repo workspace replaces the skeleton: the session shows
    // in the rail AND as the inline detail (its name in the rail card + the
    // detail heading), and the detail's Status tab confirms the inline view.
    resolveSessions!(jsonResponse(sessionsBody));
    expect((await screen.findAllByText('nightly')).length).toBeGreaterThan(0);
    expect(screen.getByRole('tab', { name: 'Status' })).toBeInTheDocument();

    // Breadcrumb shows the full path (the crumb is the aria-current element;
    // the canvas detail node repeats the name); Escape mirrors Back, one
    // level at a time.
    const crumbs = screen.getAllByText('shining/lab');
    expect(crumbs.some((el) => el.getAttribute('aria-current') === 'page')).toBe(true);
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(await screen.findByText(/repositories of shining/)).toBeInTheDocument();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(await screen.findByText(/every GitHub account you can reach/)).toBeInTheDocument();
    // At root, Escape is a no-op.
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.getByText(/every GitHub account you can reach/)).toBeInTheDocument();
  });

  it('drops an out-of-order sessions response for the same repo', async () => {
    const staleBody: RepoSessionsResponse = {
      ...sessionsBody,
      sessions: [{ ...sessionsBody.sessions[0]!, name: 'stale-old' }],
    };
    const pending: ((r: Response) => void)[] = [];
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith('/api/v1/overview')) return jsonResponse(overviewBody);
      if (url.endsWith('/api/v1/repos/shining/lab/sessions')) {
        return new Promise<Response>((resolve) => {
          pending.push(resolve);
        });
      }
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    renderDashboard();

    // Enter the repo: request 1 goes out and stays in flight.
    fireEvent.click((await screen.findAllByRole('button', { name: 'Open account shining' }))[0]!);
    fireEvent.click(
      (await screen.findAllByRole('button', { name: 'Open repository shining/lab' }))[0]!
    );
    await waitFor(() => expect(pending).toHaveLength(1));

    // Leave and re-enter the SAME repo: request 2 with the same level key.
    fireEvent.keyDown(window, { key: 'Escape' });
    fireEvent.click(
      (await screen.findAllByRole('button', { name: 'Open repository shining/lab' }))[0]!
    );
    await waitFor(() => expect(pending).toHaveLength(2));

    // The newer request resolves first; the older one straggles in after.
    pending[1]!(jsonResponse(sessionsBody));
    expect((await screen.findAllByText('nightly')).length).toBeGreaterThan(0);
    pending[0]!(jsonResponse(staleBody));

    // The stale payload must never land: 'nightly' stays, 'stale-old' never shows.
    await waitFor(() => expect(screen.queryByText('stale-old')).not.toBeInTheDocument());
    expect(screen.getAllByText('nightly').length).toBeGreaterThan(0);
  });

  it('ignores Escape pressed inside an editable field (filter inputs)', async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith('/api/v1/overview')) return jsonResponse(overviewBody);
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    renderDashboard();

    const openAccountButtons = await screen.findAllByRole('button', {
      name: 'Open account shining',
    });
    fireEvent.click(openAccountButtons[0]!);
    expect(await screen.findByText(/repositories of shining/)).toBeInTheDocument();

    // Escape inside the repo filter clears the field natively (WebKit/Blink);
    // it must NOT also walk the canvas up a level.
    const filter = screen.getByLabelText('Filter repositories…');
    filter.focus();
    fireEvent.keyDown(filter, { key: 'Escape' });
    expect(screen.getByText(/repositories of shining/)).toBeInTheDocument();

    // Outside the field the shortcut still works.
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(await screen.findByText(/every GitHub account you can reach/)).toBeInTheDocument();
  });

  it('navigates via the breadcrumb Back button', async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith('/api/v1/overview')) return jsonResponse(overviewBody);
      if (url.endsWith('/sessions')) return jsonResponse(sessionsBody);
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    renderDashboard();

    const openAccountButtons = await screen.findAllByRole('button', {
      name: 'Open account shining',
    });
    fireEvent.click(openAccountButtons[0]!);
    expect(await screen.findByText(/repositories of shining/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Back to the previous level' }));
    expect(await screen.findByText(/every GitHub account you can reach/)).toBeInTheDocument();
  });

  it('fills its region: the canvas section uses h-full, not a fixed magic height', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        if (String(input).endsWith('/api/v1/overview')) return jsonResponse(overviewBody);
        throw new Error(`unexpected fetch: ${String(input)}`);
      })
    );
    const { container } = renderDashboard();

    await screen.findAllByRole('button', { name: 'Open account shining' });
    const section = container.querySelector('section[aria-label="Accounts and repositories canvas"]');
    expect(section).not.toBeNull();
    expect(section!.className).toContain('h-full');
    // The former viewport-overflowing magic heights are gone.
    expect(section!.className).not.toContain('h-[640px]');
    expect(section!.className).not.toContain('h-[440px]');
  });

  it('shows an in-canvas empty message when there are zero accounts', async () => {
    const emptyBody: OverviewResponse = {
      app_slug: 'chronoai-fkst',
      viewer: { login: 'shining' },
      global_admin: false,
      accounts: [],
      totals: { sessions: 0, packages: [] },
      broader_oauth_available: false,
    };
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        if (String(input).endsWith('/api/v1/overview')) return jsonResponse(emptyBody);
        throw new Error(`unexpected fetch: ${String(input)}`);
      })
    );
    renderDashboard();

    // The empty-accounts message renders in the canvas (and possibly the
    // sidebar); at least one instance must appear rather than a blank canvas.
    expect((await screen.findAllByText('No accounts found.')).length).toBeGreaterThan(0);
  });

  it('renders an in-panel error with a working Retry when the overview load fails', async () => {
    let calls = 0;
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith('/api/v1/overview')) {
        calls += 1;
        // First load fails (no data); the Retry re-fetch succeeds.
        return calls === 1 ? jsonResponse(null, 500) : jsonResponse(overviewBody);
      }
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    renderDashboard();

    expect(
      await screen.findByText('Could not load your repositories. Please try again.')
    ).toBeInTheDocument();
    // The sidebar must NOT be stuck on a skeleton behind a blank canvas.
    expect(screen.queryByTestId('sidebar-skeleton')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));

    // The retry re-fetch lands and the canvas replaces the error panel.
    expect(
      (await screen.findAllByRole('button', { name: 'Open account shining' })).length
    ).toBeGreaterThan(0);
    expect(
      screen.queryByText('Could not load your repositories. Please try again.')
    ).not.toBeInTheDocument();
  });

  it('refreshes the visible session list (not only counts) on Refresh at repo level', async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith('/api/v1/overview')) return jsonResponse(overviewBody);
      if (url.endsWith('/api/v1/repos/shining/lab/sessions')) return jsonResponse(sessionsBody);
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    renderDashboard();

    fireEvent.click((await screen.findAllByRole('button', { name: 'Open account shining' }))[0]!);
    fireEvent.click(
      (await screen.findAllByRole('button', { name: 'Open repository shining/lab' }))[0]!
    );
    await screen.findAllByText('nightly');

    const sessionCalls = () =>
      fetchMock.mock.calls.filter(([input]) => String(input).endsWith('/sessions')).length;
    const before = sessionCalls();

    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    // Refresh at level 2 re-hits the sessions endpoint, not just /overview.
    await waitFor(() => expect(sessionCalls()).toBeGreaterThan(before));
  });

  it('keeps the dashboard body and prompts re-auth (not the cold card) on involuntary expiry', async () => {
    // An expired access token with no refresh token → the first apiFetch cannot
    // recover the session, so the auth context flips to sessionExpired.
    window.localStorage.setItem('fkst-gh-expires', String(Date.now() - 1000));
    window.localStorage.removeItem('fkst-gh-refresh');
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        if (String(input).endsWith('/api/v1/overview')) return jsonResponse(overviewBody);
        throw new Error(`unexpected fetch: ${String(input)}`);
      })
    );
    renderDashboard();

    expect(await screen.findByText('Your session expired')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Sign in again' })).toBeInTheDocument();
    // The cold sign-in card must NOT replace the (context-preserving) body.
    expect(screen.queryByText('Sign in to view your dashboard')).not.toBeInTheDocument();
  });
});
