import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
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
 *  payload; /api/v1/repos gets the given body/status. Returns the mock so
 *  tests can count per-endpoint calls. */
function stubApi(reposBody: ReposBody | null, reposStatus = 200) {
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.endsWith('/api/v1/dashboard')) {
      return jsonResponse({ last_pulled_at_ms: null, dashboard: null });
    }
    if (url.endsWith('/api/v1/repos')) {
      return jsonResponse(reposBody, reposStatus);
    }
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

const repoCalls = (fetchMock: ReturnType<typeof stubApi>) =>
  fetchMock.mock.calls.filter(([input]) => String(input).endsWith('/api/v1/repos')).length;

function renderDashboard() {
  return render(
    <AuthProvider>
      <Dashboard />
    </AuthProvider>
  );
}

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
    const fetchMock = stubApi({ app_slug: 'chronoai-fkst', repos: [] });
    renderDashboard();

    expect(await screen.findByText('No repositories found on your account.')).toBeInTheDocument();
    expect(repoCalls(fetchMock)).toBe(1);

    await user.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(repoCalls(fetchMock)).toBe(2));
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
});
