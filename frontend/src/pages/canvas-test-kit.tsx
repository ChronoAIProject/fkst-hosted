import { vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { BrowserRouter } from 'react-router-dom';
import { Dashboard } from './dashboard';
import { AuthProvider } from '@/lib/auth/github-auth';
import { BroaderOAuthProvider } from '@/lib/auth/broader-oauth';
import { ToastProvider } from '@/components/ui/toast';
import type {
  AccountOverview,
  OverviewResponse,
  RepoOverview,
  RepoSessionsResponse,
} from '@/lib/api/types';

// Shared fixtures + fetch stubs for the canvas dashboard page tests
// (dashboard.repos / dashboard.repo-admin / dashboard.canvas suites).

export function jsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  } as Response;
}

let nextRepoId = 1;
export const repo = (
  over: Partial<RepoOverview> & Pick<RepoOverview, 'owner' | 'name'>
): RepoOverview => ({
  id: nextRepoId++,
  private: false,
  admin: true,
  installed: false,
  viewer_visible: true,
  active_sessions: 0,
  packages: [],
  ...over,
});

export const account = (
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

export const overviewBody = (
  accounts: AccountOverview[],
  appSlug: string | null = 'chronoai-fkst'
): OverviewResponse => ({
  app_slug: appSlug,
  viewer: { login: 'shining' },
  global_admin: false,
  accounts,
  totals: { sessions: 0, packages: [] },
  broader_oauth_available: false,
});

/** An empty per-repo sessions payload for `owner/name`. */
export const repoSessionsBody = (
  owner: string,
  name: string,
  over: Partial<RepoSessionsResponse> = {}
): RepoSessionsResponse => ({
  owner,
  name,
  installed: true,
  sessions: [],
  ...over,
});

/** Stub global fetch: GET /api/v1/overview gets the given body/status.
 *
 *  `sessions` additionally serves `GET /api/v1/repos/{owner}/{name}/sessions`, which
 *  a deep link straight to a repository reaches on mount — without it the stub's
 *  deliberate `unexpected fetch` guard would throw. */
export function stubApi(
  body: OverviewResponse | null,
  status = 200,
  sessions?: RepoSessionsResponse
) {
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url.endsWith('/api/v1/overview') && init?.method === undefined) {
      return jsonResponse(body, status);
    }
    if (sessions != null && /\/api\/v1\/repos\/[^/]+\/[^/]+\/sessions$/.test(url)) {
      return jsonResponse(sessions);
    }
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

/** Stub for the create flow: the overview serves `initial` until a successful
 *  POST /api/v1/repos, then `afterCreate`. */
export function stubCreateApi(opts: {
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
export function stubDeleteApi(opts: {
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

export type FetchMock = ReturnType<typeof stubApi>;

export const overviewGetCalls = (fetchMock: FetchMock) =>
  fetchMock.mock.calls.filter(([input]) => String(input).endsWith('/api/v1/overview')).length;

export const repoPostCall = (fetchMock: FetchMock) =>
  fetchMock.mock.calls.find(
    ([input, init]) => String(input).endsWith('/api/v1/repos') && init?.method === 'POST'
  );

export const deleteCall = (fetchMock: FetchMock) =>
  fetchMock.mock.calls.find(([, init]) => init?.method === 'DELETE');

/** Render the dashboard inside a real router.
 *
 *  `BrowserRouter`, not `MemoryRouter`, deliberately: the dashboard writes its
 *  location into the query string, and only a browser router puts that on
 *  `window.location.search` where a test can assert it meaningfully. */
export function renderDashboard() {
  return render(
    <ToastProvider>
      <AuthProvider>
        <BroaderOAuthProvider>
          <BrowserRouter>
            <Dashboard />
          </BrowserRouter>
        </BroaderOAuthProvider>
      </AuthProvider>
    </ToastProvider>
  );
}

/** Point the browser at a dashboard URL before rendering, for deep-link cases.
 *  Call {@link resetDashboardUrl} afterwards so suites stay independent. */
export function seedDashboardUrl(search: string) {
  window.history.replaceState(null, '', `/dashboard${search}`);
}

export function resetDashboardUrl() {
  window.history.replaceState(null, '', '/dashboard');
}

/** Drill into an account (canvas node or sidebar affordance — same label).
 *  fireEvent, not user-event: the full pointer sequence trips d3-zoom's
 *  jsdom-null event.view. */
export async function openAccount(login: string) {
  const buttons = await screen.findAllByRole('button', { name: `Open account ${login}` });
  fireEvent.click(buttons[0]!);
}
