import { render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { App } from './index';
import { nextCondensed } from './shell';
import type { ReactNode } from 'react';
import { NyxIDProvider } from '../lib/auth';

function renderApp() {
  return render(
    <NyxIDProvider
      baseUrl="http://localhost"
      clientId="test-client"
      redirectUri="http://localhost/auth/callback"
    >
      <App />
    </NyxIDProvider>
  );
}

vi.mock('@/lib/auth', () => ({
  authRequired: () => false,
  NyxIDProvider: ({ children }: { children: ReactNode }) => children,
  useAuthSession: () => ({
    isAuthenticated: false,
    accessToken: null,
    login: async () => {},
    logout: () => {},
    handleRedirectCallback: async () => ({ accessToken: '', idToken: '', refreshToken: '', tokenType: '', expiresIn: 0 }),
    getUserInfo: async () => ({ sub: '', name: '', email: '' }),
  }),
}));

vi.mock('@/lib/hooks/useGitHubAccounts', () => ({
  useGitHubAccounts: () => ({
    data: undefined,
    isLoading: false,
    isError: false,
  }),
}));

const mockFetch: typeof fetch = (input: RequestInfo | URL, init?: RequestInit) => {
  const href = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
  void init;

  if (href.includes('/api/v1/health')) {
    return Promise.resolve(
      new Response(
        JSON.stringify({
          status: 'ok',
          mongo: 'up',
          version: '1.0.0-test',
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      )
    );
  }
  if (href.includes('/api/v1/packages')) {
    if (href.endsWith('/api/v1/packages')) {
      return Promise.resolve(
        new Response(
          JSON.stringify(['package-example']),
          { status: 200, headers: { 'Content-Type': 'application/json' } }
        )
      );
    }
    return Promise.resolve(
      new Response(
        JSON.stringify({
          name: 'package-example',
          files: [],
          composed_deps: [],
          created_at: '2026-06-13T00:00:00Z',
          updated_at: '2026-06-13T00:00:00Z',
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      )
    );
  }
  if (href.includes('/api/v1/sessions')) {
    return Promise.resolve(
      new Response(
        JSON.stringify({
          id: 'session-123',
          package_name: 'package-example',
          status: 'running',
          pod_id: 'pod-123',
          fencing_token: 1,
          pid: 1234,
          runtime_dir: '/tmp',
          error: null,
          created_at: '2026-06-13T00:00:00Z',
          started_at: '2026-06-13T00:00:00Z',
          stopped_at: null,
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      )
    );
  }
  if (href.includes('/api/v1/goals')) {
    if (href.endsWith('/api/v1/goals/152')) {
      return Promise.resolve(
        new Response(
          JSON.stringify({
            id: '152',
            title: 'Mock Goal 152',
            description: 'Mock Description',
            package_names: ['package-example'],
            repo: { owner: 'foo', name: 'bar' },
            status: 'running',
            owner_user_id: 'user-123',
            org_id: null,
            active_session_id: 'session-123',
            created_at: '2026-06-13T00:00:00Z',
            updated_at: '2026-06-13T00:00:00Z'
          }),
          { status: 200, headers: { 'Content-Type': 'application/json' } }
        )
      );
    }
    return Promise.resolve(
      new Response(
        JSON.stringify([]),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      )
    );
  }
  return Promise.reject(new Error(`Unhandled request to ${href}`));
};

describe('App Smoke Test', () => {
  let originalFetch: typeof globalThis.fetch;
  let consoleErrorSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    // NOTE: Layout overflow and container dimensions are not verified in these unit tests.
    // We explicitly defer overflow assertions to the W3.N Playwright smoke tests,
    // as JSDOM does not measure layout or perform CSS container rendering.
    window.history.pushState({}, '', '/');
    originalFetch = globalThis.fetch;
    vi.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);
    consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    expect(consoleErrorSpy).not.toHaveBeenCalled();
    vi.restoreAllMocks();
  });

  it('redirects to /overview and renders the Overview screen', async () => {
    renderApp();
    
    await waitFor(() => {
      expect(screen.getByText('Design')).toBeInTheDocument();
      expect(window.location.pathname).toBe('/overview');
      // Honesty regression guard: assert Overview vitals cells render default unknown/—
      expect(screen.getAllByText('unknown')[0]).toBeInTheDocument();
    });
  });

  it('renders Overview screen on /overview', async () => {
    window.history.pushState({}, '', '/overview');
    renderApp();
    await waitFor(() => {
      expect(screen.getByText('Design')).toBeInTheDocument();
      expect(screen.getByText('Build')).toBeInTheDocument();
      expect(screen.getByText('Review')).toBeInTheDocument();
      expect(screen.getByText('Ship')).toBeInTheDocument();
      expect(screen.getByText('Merged')).toBeInTheDocument();
    });
  });

  it('renders Goals screen on /goals (issues view)', async () => {
    window.history.pushState({}, '', '/goals');
    renderApp();
    await waitFor(() => {
      expect(screen.getByText(/no goals found/i)).toBeInTheDocument();
    });
  });

  it('renders Goals screen on /goals?view=activity (activity view)', async () => {
    window.history.pushState({}, '', '/goals?view=activity');
    renderApp();
    await waitFor(() => {
      expect(screen.getByText(/host telemetry not connected/i)).toBeInTheDocument();
      // Assert the Activity segment carries the active treatment
      const activityButton = screen.getByRole('button', { name: 'Activity' });
      expect(activityButton).toHaveClass('bg-amber');
      expect(activityButton).toHaveClass('text-amber-ink');
      expect(activityButton).toHaveClass('font-semibold');
    });
  });

  it('renders Goal screen on /goals/:id', async () => {
    window.history.pushState({}, '', '/goals/152');
    renderApp();
    await waitFor(() => {
      expect(screen.getByText(/lifecycle timeline not exposed by the v1 API/i)).toBeInTheDocument();
      expect(screen.getByText('#152')).toBeInTheDocument();
    });
  });

  it('renders Packages screen on /packages', async () => {
    window.history.pushState({}, '', '/packages');
    renderApp();
    await waitFor(() => {
      expect(screen.getByText(/Packages are the/i)).toBeInTheDocument();
    });
  });

  it('renders Settings screen on /settings', async () => {
    window.history.pushState({}, '', '/settings');
    renderApp();
    await waitFor(() => {
      expect(screen.getByText('Hosted engine — ChronoAI cloud')).toBeInTheDocument();
    });
  });

  it('redirects /runs to /goals?view=activity', async () => {
    window.history.pushState({}, '', '/runs');
    renderApp();
    await waitFor(() => {
      expect(window.location.pathname).toBe('/goals');
      expect(window.location.search).toBe('?view=activity');
      expect(screen.getByText(/host telemetry not connected/i)).toBeInTheDocument();
    });
  });

  it('renders topbar logo and nav links correctly, excluding Settings from primary nav', async () => {
    renderApp();

    await waitFor(() => {
      // 1. Logo text "FKST"
      const logoLink = screen.getByRole('link', { name: (name) => name.replace(/\s+/g, '') === 'FKST' });
      expect(logoLink).toBeInTheDocument();
      expect(logoLink.getAttribute('href')).toBe('/overview');

      // 2. Primary nav links exist
      const overviewLink = screen.getByRole('link', { name: 'Overview' });
      const goalsLink = screen.getByRole('link', { name: 'Goals' });
      const packagesLink = screen.getByRole('link', { name: 'Packages' });
      
      expect(overviewLink).toBeInTheDocument();
      expect(goalsLink).toBeInTheDocument();
      expect(packagesLink).toBeInTheDocument();

      // 3. Settings is NOT in the primary nav list (nav role)
      const navElement = screen.getByRole('navigation');
      const settingsInNav = navElement.querySelector('a[href="/settings"]');
      expect(settingsInNav).toBeNull();

      // 4. Avatar (outside nav) links to settings
      const avatarLink = screen.getByRole('link', { name: /sign-in pending/i });
      expect(avatarLink).toBeInTheDocument();
      expect(avatarLink.getAttribute('href')).toBe('/settings');
    });
  });
});

describe('Hysteresis Unit Tests (nextCondensed)', () => {
  it('handles standard transition triggers', () => {
    // y = 0 -> false
    expect(nextCondensed(false, 0)).toBe(false);
    expect(nextCondensed(true, 0)).toBe(false);

    // y = 100 from false -> false
    expect(nextCondensed(false, 100)).toBe(false);

    // y = 141 -> true
    expect(nextCondensed(false, 141)).toBe(true);
    expect(nextCondensed(true, 141)).toBe(true);

    // y = 100 from true -> true (remains true due to hysteresis)
    expect(nextCondensed(true, 100)).toBe(true);

    // y = 39 -> false
    expect(nextCondensed(true, 39)).toBe(false);
    expect(nextCondensed(false, 39)).toBe(false);
  });

  it('handles boundary values 40/140 exactly per semantics', () => {
    // 140 is NOT > 140, so state remains unchanged
    expect(nextCondensed(false, 140)).toBe(false);
    expect(nextCondensed(true, 140)).toBe(true);

    // 40 is NOT < 40, so state remains unchanged
    expect(nextCondensed(true, 40)).toBe(true);
    expect(nextCondensed(false, 40)).toBe(false);
  });
});
