import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Dashboard } from './dashboard';
import { AuthProvider } from '@/lib/auth/github-auth';
import type {
  AccountOverview,
  OverviewResponse,
  RepoSessionsResponse,
} from '@/lib/api/types';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

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
  accounts,
  totals: { sessions: 1, packages: [{ package: 'o/p@main:pkg/base', count: 1 }] },
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

function renderDashboard() {
  return render(
    <AuthProvider>
      <Dashboard />
    </AuthProvider>
  );
}

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
    expect(await screen.findByTestId('sidebar-skeleton')).toBeInTheDocument();

    // Data lands → the session list replaces the skeleton ('nightly' shows
    // both on the canvas detail node and in the sidebar session card).
    resolveSessions!(jsonResponse(sessionsBody));
    expect((await screen.findAllByText('nightly')).length).toBeGreaterThan(0);
    expect(screen.getByText(/fkst sessions of shining\/lab/)).toBeInTheDocument();

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
});
