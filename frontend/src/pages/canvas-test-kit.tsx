import { vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { Dashboard } from './dashboard';
import { AuthProvider } from '@/lib/auth/github-auth';
import { BroaderOAuthProvider } from '@/lib/auth/broader-oauth';
import { ToastProvider } from '@/components/ui/toast';
import type { AccountOverview, OverviewResponse, RepoOverview } from '@/lib/api/types';

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

/** Stub global fetch: GET /api/v1/overview gets the given body/status. */
export function stubApi(body: OverviewResponse | null, status = 200) {
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

export function renderDashboard() {
  return render(
    <ToastProvider>
      <AuthProvider>
        <BroaderOAuthProvider>
          <Dashboard />
        </BroaderOAuthProvider>
      </AuthProvider>
    </ToastProvider>
  );
}

/** Drill into an account (canvas node or sidebar affordance — same label).
 *  fireEvent, not user-event: the full pointer sequence trips d3-zoom's
 *  jsdom-null event.view. */
export async function openAccount(login: string) {
  const buttons = await screen.findAllByRole('button', { name: `Open account ${login}` });
  fireEvent.click(buttons[0]!);
}
