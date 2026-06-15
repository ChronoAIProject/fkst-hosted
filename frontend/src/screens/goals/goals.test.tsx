import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { Goals } from './goals';

vi.mock('@/lib/auth', () => ({
  authRequired: () => true,
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

describe('Goals Screen Unit Tests', () => {
  beforeEach(() => {
    vi.stubEnv('VITE_NYXID_CONNECT_GITHUB_URL', 'https://example.com/connect');
  });

  afterEach(() => {
    vi.unstubAllEnvs();
  });
  it('renders counts as — or unknown, never 0, in default empty state', () => {
    render(<Goals />);

    // Check that goal counts and age render as "—" or "unknown"
    const dashes = screen.getAllByText('—');
    expect(dashes.length).toBeGreaterThan(0);

    // Verify "0" is not rendered as a count
    const allZeroElements = screen.queryAllByText('0');
    expect(allZeroElements.length).toBe(0);

    // Verify empty state text is rendered
    expect(screen.getByText(/no GitHub plane connected — sign-in pending/i)).toBeInTheDocument();
  });

  it('proves the forbidden string "Nothing needs you" is absent', () => {
    render(<Goals />);
    expect(screen.queryByText(/Nothing needs you/i)).toBeNull();
  });

  it('renders the Activity view empty state when view="activity" is passed', () => {
    render(<Goals view="activity" />);

    // Vitals and run lists should read "—" or "unknown"
    const dashes = screen.getAllByText('—');
    expect(dashes.length).toBeGreaterThan(0);

    // Verify Activity empty state note is present
    expect(screen.getByText(/host telemetry not connected/i)).toBeInTheDocument();
  });

  it('renders custom populated data in both views', () => {
    // 1. Populated Issues
    const { rerender } = render(
      <Goals
        view="issues"
        authSessionOverride={{ isAuthenticated: true }}
        accountsOverride={[{ connection_id: 'c1', login: 'octocat', primary: true }]}
        goals={[
          {
            id: '152',
            title: 'Composed conformance suite',
            stage: 'Ship',
            state: 'merging',
            age: '3m',
            repo: 'fkst-substrate',
            pr: '#29',
            ci: 'passing',
          },
        ]}
      />
    );

    expect(screen.getByText('Composed conformance suite')).toBeInTheDocument();
    expect(screen.getByText('152')).toBeInTheDocument();

    // 2. Populated Activity
    rerender(
      <Goals
        view="activity"
        vitals={{
          runsDispatched: '10',
          successRate: '90%',
          medianDuration: '30s',
          inDlq: 'unknown',
        }}
        runs={[
          {
            id: 'run_1',
            goalId: '205',
            goalTitle: 'State label set-exclusive',
            action: 'implement',
            attempt: '1',
            duration: '38s',
            exitCode: 0,
            when: 'just now',
            lease: '1',
            status: 'ok',
          },
        ]}
      />
    );

    expect(screen.getByText('10')).toBeInTheDocument();
    expect(screen.getByText('90%')).toBeInTheDocument();
    expect(screen.getByText('State label set-exclusive')).toBeInTheDocument();
  });

  describe('Goals Screen Empty States', () => {
    it('renders state (a) when auth required and no session exists', () => {
      render(
        <Goals
          authSessionOverride={{ isAuthenticated: false }}
        />
      );
      expect(screen.getByText(/no GitHub plane connected — sign-in pending/i)).toBeInTheDocument();
    });

    it('renders state (b) Connect GitHub CTA when signed in but ZERO linked GitHub accounts', () => {
      render(
        <Goals
          authSessionOverride={{ isAuthenticated: true }}
          accountsOverride={[]}
        />
      );
      expect(screen.getByText(/no GitHub accounts connected/i)).toBeInTheDocument();
      
      const ctaLink = screen.getByRole('link', { name: /Connect GitHub/i });
      expect(ctaLink).toBeInTheDocument();
      expect(ctaLink.getAttribute('href')).toBeTruthy();
    });

    it('renders state (c) empty goals list when signed in and >=1 accounts linked, but goals is empty', () => {
      render(
        <Goals
          authSessionOverride={{ isAuthenticated: true }}
          accountsOverride={[{ connection_id: 'c1', login: 'octocat', primary: true }]}
        />
      );
      expect(screen.getByText(/no goals found/i)).toBeInTheDocument();
      expect(screen.queryByText(/no GitHub accounts connected/i)).not.toBeInTheDocument();
    });
  });

  describe('Honesty Contract and Gating Revisions', () => {
    const mockGoals = [
      {
        id: '152',
        title: 'Composed conformance suite',
        stage: 'Ship' as const,
        state: 'merging' as const,
        age: '3m',
        repo: 'fkst-substrate',
        pr: '#29',
        ci: 'passing' as const,
      },
    ];

    it('ensures gate wins and hides rows when non-empty goals but unauthenticated', () => {
      render(
        <Goals
          authSessionOverride={{ isAuthenticated: false }}
          goals={mockGoals}
        />
      );
      expect(screen.queryByText('Composed conformance suite')).toBeNull();
      expect(screen.getByText(/no GitHub plane connected — sign-in pending/i)).toBeInTheDocument();
    });

    it('ensures gate wins and hides rows when non-empty goals but accounts loading', () => {
      render(
        <Goals
          authSessionOverride={{ isAuthenticated: true }}
          accountsLoadingOverride={true}
          goals={mockGoals}
        />
      );
      expect(screen.queryByText('Composed conformance suite')).toBeNull();
      expect(screen.getByText(/loading GitHub accounts\.\.\./i)).toBeInTheDocument();
    });

    it('ensures gate wins and hides rows when non-empty goals but accounts undefined (unknown)', () => {
      render(
        <Goals
          authSessionOverride={{ isAuthenticated: true }}
          accountsOverride={undefined}
          goals={mockGoals}
        />
      );
      expect(screen.queryByText('Composed conformance suite')).toBeNull();
      expect(screen.getByText(/GitHub status unknown — couldn't reach the connection service/i)).toBeInTheDocument();
    });

    it('ensures gate wins and hides rows when non-empty goals but accounts error', () => {
      render(
        <Goals
          authSessionOverride={{ isAuthenticated: true }}
          accountsErrorOverride={true}
          goals={mockGoals}
        />
      );
      expect(screen.queryByText('Composed conformance suite')).toBeNull();
      expect(screen.getByText(/GitHub status unknown — couldn't reach the connection service/i)).toBeInTheDocument();
    });

    it('ensures gate wins and hides rows when non-empty goals but empty accounts', () => {
      render(
        <Goals
          authSessionOverride={{ isAuthenticated: true }}
          accountsOverride={[]}
          goals={mockGoals}
        />
      );
      expect(screen.queryByText('Composed conformance suite')).toBeNull();
      expect(screen.getByText(/no GitHub accounts connected/i)).toBeInTheDocument();
    });

    it('distinguishes undefined/error (unknown) from success empty [] (Connect CTA)', () => {
      // 1. undefined accounts -> unknown state
      const { rerender } = render(
        <Goals
          authSessionOverride={{ isAuthenticated: true }}
          accountsOverride={undefined}
        />
      );
      expect(screen.getByText(/GitHub status unknown — couldn't reach the connection service/i)).toBeInTheDocument();
      expect(screen.queryByText(/no GitHub accounts connected/i)).toBeNull();

      // 2. error accounts -> unknown state
      rerender(
        <Goals
          authSessionOverride={{ isAuthenticated: true }}
          accountsErrorOverride={new Error('proxy error')}
        />
      );
      expect(screen.getByText(/GitHub status unknown — couldn't reach the connection service/i)).toBeInTheDocument();
      expect(screen.queryByText(/no GitHub accounts connected/i)).toBeNull();

      // 3. empty accounts [] -> Connect CTA
      rerender(
        <Goals
          authSessionOverride={{ isAuthenticated: true }}
          accountsOverride={[]}
        />
      );
      expect(screen.getByText(/no GitHub accounts connected/i)).toBeInTheDocument();
      expect(screen.queryByText(/GitHub status unknown/i)).toBeNull();
    });

    it('renders rows when authenticated and >=1 linked accounts', () => {
      render(
        <Goals
          authSessionOverride={{ isAuthenticated: true }}
          accountsOverride={[{ connection_id: 'c1', login: 'octocat', primary: true }]}
          goals={mockGoals}
        />
      );
      expect(screen.getByText('Composed conformance suite')).toBeInTheDocument();
    });

    it('renders disabled CTA with an honest note when VITE_NYXID_CONNECT_GITHUB_URL is unset', () => {
      vi.stubEnv('VITE_NYXID_CONNECT_GITHUB_URL', '');
      render(
        <Goals
          authSessionOverride={{ isAuthenticated: true }}
          accountsOverride={[]}
        />
      );
      expect(screen.getByText(/no GitHub accounts connected/i)).toBeInTheDocument();
      
      // The CTA should be a disabled button, not a link, and should have an honest note
      expect(screen.queryByRole('link', { name: /Connect GitHub/i })).toBeNull();
      
      const disabledButton = screen.getByRole('button', { name: /Connect GitHub/i });
      expect(disabledButton).toBeInTheDocument();
      expect(disabledButton).toBeDisabled();
      expect(screen.getByText(/GitHub connection URL is not configured/i)).toBeInTheDocument();
    });
  });
});

