import React from 'react';
import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { SettingsScreen } from './settings-screen';
import { useHealth, UseHealthResult } from '@/lib/hooks/useHealth';
import { usePackagesList } from '@/lib/hooks/usePackages';
import { useSession, useStopSession } from '@/lib/hooks/useSessions';
import { useSessionRegistry, SessionRegistryProvider } from '@/lib/hooks/session-registry';
import { ApiError } from '@/lib/api/client';
import { NyxIDProvider } from '@/lib/auth';
import { useGitHubAccounts } from '@/lib/hooks/useGitHubAccounts';

// Mock the hooks
vi.mock('@/lib/hooks/useHealth');
vi.mock('@/lib/hooks/usePackages');
vi.mock('@/lib/hooks/useSessions');
vi.mock('@/lib/hooks/useGitHubAccounts');

function renderWithProviders(ui: React.ReactNode, { sessionIdToRegister }: { sessionIdToRegister?: string } = {}) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
      },
    },
  });

  function TestWrapper({ children }: { children: React.ReactNode }) {
    return (
      <SessionRegistryProvider>
        <QueryClientProvider client={queryClient}>
          <MemoryRouter>
            <NyxIDProvider
              baseUrl="http://localhost"
              clientId="test-client"
              redirectUri="http://localhost/auth/callback"
            >
              {sessionIdToRegister ? (
                <RegistryInit packageName="fkst-substrate" sessionId={sessionIdToRegister}>
                  {children}
                </RegistryInit>
              ) : (
                children
              )}
            </NyxIDProvider>
          </MemoryRouter>
        </QueryClientProvider>
      </SessionRegistryProvider>
    );
  }

  return render(ui, { wrapper: TestWrapper });
}

function RegistryInit({
  packageName,
  sessionId,
  children,
}: {
  packageName: string;
  sessionId: string;
  children: React.ReactNode;
}) {
  const { registerSession } = useSessionRegistry();
  React.useEffect(() => {
    registerSession(packageName, sessionId);
  }, [packageName, sessionId, registerSession]);
  return <>{children}</>;
}

describe('SettingsScreen', () => {
  beforeEach(() => {
    vi.mocked(useGitHubAccounts).mockReturnValue({
      data: [],
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof useGitHubAccounts>);
  });

  it('posture never asserts and write posture controls are disabled', () => {
    vi.mocked(useHealth).mockReturnValue({
      healthStatus: 'ok',
      mongo: 'up',
      version: '1.0.0',
      isSuccess: true,
      isError: false,
      isLoading: false,
      error: null,
      refetch: vi.fn(),
    } as unknown as UseHealthResult);

    vi.mocked(usePackagesList).mockReturnValue({
      data: ['fkst-substrate'],
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof usePackagesList>);

    renderWithProviders(<SettingsScreen />);

    // Verifies PostureChip renders the unknown state
    expect(screen.getByText('posture unknown (deploy-time)')).toBeInTheDocument();

    // Verifies Arm REAL is disabled
    const armRealLabel = screen.getByText('Arm REAL');
    expect(armRealLabel).toBeInTheDocument();

    // Verifies Enable REAL writes is disabled
    const enableRealBtn = screen.getByRole('button', { name: /enable real writes/i });
    expect(enableRealBtn).toBeDisabled();

    // Verifies that the grounding note is present
    expect(screen.getAllByText(/global FKST_GITHUB_WRITE is deploy-time env/i).length).toBeGreaterThan(0);
  });

  it('renders unknown instead of 0 or live when engine is unreachable', () => {
    vi.mocked(useHealth).mockReturnValue({
      healthStatus: 'unknown',
      mongo: undefined,
      version: undefined,
      isSuccess: false,
      isError: true,
      isLoading: false,
      error: new Error('Unreachable'),
      refetch: vi.fn(),
    } as unknown as UseHealthResult);

    vi.mocked(usePackagesList).mockReturnValue({
      data: [],
      isLoading: false,
      isError: true,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof usePackagesList>);

    renderWithProviders(<SettingsScreen />);

    // Verify connection status, MongoDB, version, etc. should render "unknown"
    const unknownElements = screen.getAllByText('unknown');
    expect(unknownElements.length).toBeGreaterThan(5);

    // Verify "live" does not exist
    expect(screen.queryByText('live')).not.toBeInTheDocument();

    // Deployment knobs should render "unknown"
    const envVars = [
      'FKST_GITHUB_REPO',
      'FKST_DEVLOOP_INTEGRATION_BRANCH',
      'FKST_DEVLOOP_UPSTREAM_BRANCH',
      'FKST_DEVLOOP_ROLLUP_MERGE',
      'FKST_GITHUB_BOT_LOGIN',
      'FKST_DURABLE_ROOT',
      'FKST_PACKAGE_ROOTS',
      'FKST_CODEX_PERMIT_SLOTS',
      'FKST_RETRY_DEFAULT_MAX_ATTEMPTS',
    ];
    envVars.forEach((env) => {
      expect(screen.getAllByText(env).length).toBeGreaterThan(0);
    });
  });

  it('stop flow fires mutation and displays ack-not-success copy while polling', async () => {
    vi.mocked(useHealth).mockReturnValue({
      healthStatus: 'ok',
      mongo: 'up',
      version: '1.0.0',
      isSuccess: true,
      isError: false,
      isLoading: false,
      error: null,
      refetch: vi.fn(),
    } as unknown as UseHealthResult);

    vi.mocked(usePackagesList).mockReturnValue({
      data: ['fkst-substrate'],
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof usePackagesList>);

    vi.mocked(useSession).mockReturnValue({
      data: {
        id: 'session-123',
        package_name: 'fkst-substrate',
        status: 'running',
        created_at: '2026-06-13T02:00:00Z',
        started_at: '2026-06-13T02:00:02Z',
        stopped_at: null,
      },
      isLoading: false,
      isError: false,
    } as unknown as ReturnType<typeof useSession>);

    const mockMutate = vi.fn((_id, options) => {
      if (options?.onSuccess) {
        options.onSuccess();
      }
    });

    const mockStopMutation = {
      mutate: mockMutate,
      isPending: false,
      isSuccess: false,
      isError: false,
      error: null,
      reset: vi.fn(),
      data: undefined,
      mutateAsync: vi.fn(),
      variables: undefined,
      context: undefined,
      status: 'idle' as const,
      failureCount: 0,
      failureReason: null,
      isIdle: true,
      isPaused: false,
    };

    vi.mocked(useStopSession).mockReturnValue(mockStopMutation as unknown as ReturnType<typeof useStopSession>);

    renderWithProviders(<SettingsScreen />, { sessionIdToRegister: 'session-123' });

    // Verify session details are shown asynchronously
    const sessionIdEl = await screen.findByText(/id: session-123/i);
    expect(sessionIdEl).toBeInTheDocument();
    expect(await screen.findByText('running')).toBeInTheDocument();

    // Click "Stop session" button to trigger Dialog
    const stopBtn = screen.getByRole('button', { name: /stop session/i });
    fireEvent.click(stopBtn);

    // Dialog opens
    expect(screen.getByText('Confirm Stop Session')).toBeInTheDocument();

    // Click "Confirm Stop" button inside Dialog
    const confirmBtn = screen.getByRole('button', { name: /confirm stop/i });

    // Setup mutated state for subsequent render
    mockStopMutation.isSuccess = true;

    fireEvent.click(confirmBtn);

    // Verify stop mutation was fired with the correct session ID
    expect(mockMutate).toHaveBeenCalledWith('session-123', expect.any(Object));

    // Verify the ack-not-success copy is displayed
    expect(
      await screen.findByText('stop requested · waiting for stopped — 202 is an ack, truth is the poll')
    ).toBeInTheDocument();
  });

  it('disables all gap controls with appropriate grounding notes', () => {
    vi.mocked(useHealth).mockReturnValue({
      healthStatus: 'ok',
      mongo: 'up',
      version: '1.0.0',
      isSuccess: true,
      isError: false,
      isLoading: false,
      error: null,
      refetch: vi.fn(),
    } as unknown as UseHealthResult);

    vi.mocked(usePackagesList).mockReturnValue({
      data: ['fkst-substrate'],
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof usePackagesList>);

    renderWithProviders(<SettingsScreen />);

    // Verify buttons are disabled
    expect(screen.getByRole('button', { name: 'Sign out' })).toBeDisabled();
    const connectBtns = screen.getAllByRole('button', { name: 'Connect GitHub' });
    expect(connectBtns.length).toBeGreaterThan(0);
    connectBtns.forEach(btn => expect(btn).toBeDisabled());
    expect(screen.getByRole('button', { name: 'Delete account' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Enable REAL writes' })).toBeDisabled();

    // Verify exact notes
    expect(screen.getAllByText('NyxID integration pending · no active identity').length).toBeGreaterThan(0);
    expect(
      screen.getByText(
        'Disabled — global FKST_GITHUB_WRITE is deploy-time env; no API to read or change it in v1; applied via session restart'
      )
    ).toBeInTheDocument();
  });

  it('does not render fkst-substrate when package list is empty or errored', () => {
    vi.mocked(useHealth).mockReturnValue({
      healthStatus: 'ok',
      mongo: 'up',
      version: '1.0.0',
      isSuccess: true,
      isError: false,
      isLoading: false,
      error: null,
      refetch: vi.fn(),
    } as unknown as UseHealthResult);

    // Case 1: Errored list
    vi.mocked(usePackagesList).mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: true,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof usePackagesList>);

    const { rerender } = renderWithProviders(<SettingsScreen />);

    expect(screen.getByText(/Package list unavailable — session controls require a package name/i)).toBeInTheDocument();
    expect(screen.queryByText('fkst-substrate')).not.toBeInTheDocument();

    // Case 2: Genuinely empty list
    vi.mocked(usePackagesList).mockReturnValue({
      data: [],
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof usePackagesList>);

    rerender(<SettingsScreen />);

    expect(screen.getByText(/No packages returned by the hosted backend./i)).toBeInTheDocument();
    expect(screen.queryByText('fkst-substrate')).not.toBeInTheDocument();
  });

  it('displays stop request failure inline and keeps the dialog mounted', async () => {
    vi.mocked(useHealth).mockReturnValue({
      healthStatus: 'ok',
      mongo: 'up',
      version: '1.0.0',
      isSuccess: true,
      isError: false,
      isLoading: false,
      error: null,
      refetch: vi.fn(),
    } as unknown as UseHealthResult);

    vi.mocked(usePackagesList).mockReturnValue({
      data: ['fkst-substrate'],
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof usePackagesList>);

    vi.mocked(useSession).mockReturnValue({
      data: {
        id: 'session-123',
        package_name: 'fkst-substrate',
        status: 'running',
        created_at: '2026-06-13T02:00:00Z',
        started_at: '2026-06-13T02:00:02Z',
        stopped_at: null,
      },
      isLoading: false,
      isError: false,
    } as unknown as ReturnType<typeof useSession>);

    const mockMutate = vi.fn();
    const mockStopMutation = {
      mutate: mockMutate,
      isPending: false,
      isSuccess: false,
      isError: true,
      error: new Error('Network timeout'),
      reset: vi.fn(),
      data: undefined,
      mutateAsync: vi.fn(),
      variables: undefined,
      context: undefined,
      status: 'error' as const,
      failureCount: 1,
      failureReason: null,
      isIdle: false,
      isPaused: false,
    };
    vi.mocked(useStopSession).mockReturnValue(mockStopMutation as unknown as ReturnType<typeof useStopSession>);

    renderWithProviders(<SettingsScreen />, { sessionIdToRegister: 'session-123' });

    // Open confirm dialog
    const stopBtn = screen.getByRole('button', { name: /stop session/i });
    fireEvent.click(stopBtn);

    // Verify dialog is open
    expect(screen.getByText('Confirm Stop Session')).toBeInTheDocument();

    // Verify error is displayed inline
    expect(screen.getByText(/stop request failed: Network timeout — session unchanged/i)).toBeInTheDocument();

    // Dialog stays open (confirm button is still visible)
    expect(screen.getByRole('button', { name: /confirm stop/i })).toBeInTheDocument();
  });

  it('stale session 404 displays stale copy and clears session from registry', async () => {
    vi.mocked(useHealth).mockReturnValue({
      healthStatus: 'ok',
      mongo: 'up',
      version: '1.0.0',
      isSuccess: true,
      isError: false,
      isLoading: false,
      error: null,
      refetch: vi.fn(),
    } as unknown as UseHealthResult);

    vi.mocked(usePackagesList).mockReturnValue({
      data: ['fkst-substrate'],
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof usePackagesList>);

    // Mock useSession to fail with 404 using a real ApiError instance
    const apiError = new ApiError(404, null, 'Session not found');
    vi.mocked(useSession).mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: true,
      error: apiError,
    } as unknown as ReturnType<typeof useSession>);

    vi.mocked(useStopSession).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
      isSuccess: false,
      isError: false,
      error: null,
    } as unknown as ReturnType<typeof useStopSession>);

    renderWithProviders(<SettingsScreen />, { sessionIdToRegister: 'session-123' });

    // Verify stale copy is shown
    expect(await screen.findByText(/session no longer found — stale registry entry from this tab/i)).toBeInTheDocument();
  });

  it('renders loading state for github accounts', () => {
    vi.mocked(useHealth).mockReturnValue({
      healthStatus: 'ok',
      mongo: 'up',
      version: '1.0.0',
      isSuccess: true,
      refetch: vi.fn(),
    } as unknown as UseHealthResult);
    vi.mocked(usePackagesList).mockReturnValue({
      data: [],
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof usePackagesList>);
    vi.mocked(useGitHubAccounts).mockReturnValue({
      data: undefined,
      isLoading: true,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof useGitHubAccounts>);

    renderWithProviders(<SettingsScreen />);
    expect(screen.getByText('Loading connected accounts...')).toBeInTheDocument();
  });

  it('renders error state for github accounts', () => {
    vi.mocked(useHealth).mockReturnValue({
      healthStatus: 'ok',
      mongo: 'up',
      version: '1.0.0',
      isSuccess: true,
      refetch: vi.fn(),
    } as unknown as UseHealthResult);
    vi.mocked(usePackagesList).mockReturnValue({
      data: [],
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof usePackagesList>);
    vi.mocked(useGitHubAccounts).mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: true,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof useGitHubAccounts>);

    renderWithProviders(<SettingsScreen />);
    expect(screen.getByText("couldn't reach the connection service — unknown")).toBeInTheDocument();
  });

  it('renders empty state for github accounts', () => {
    vi.mocked(useHealth).mockReturnValue({
      healthStatus: 'ok',
      mongo: 'up',
      version: '1.0.0',
      isSuccess: true,
      refetch: vi.fn(),
    } as unknown as UseHealthResult);
    vi.mocked(usePackagesList).mockReturnValue({
      data: [],
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof usePackagesList>);
    vi.mocked(useGitHubAccounts).mockReturnValue({
      data: [],
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof useGitHubAccounts>);

    renderWithProviders(<SettingsScreen />);
    expect(screen.getByText('no GitHub accounts connected')).toBeInTheDocument();
    // Connect GitHub CTA is rendered (disabled here since VITE_NYXID_CONNECT_GITHUB_URL is not set by default in tests)
    expect(screen.getAllByRole('button', { name: 'Connect GitHub' }).length).toBeGreaterThan(0);
    expect(screen.getAllByText('GitHub connection URL is not configured').length).toBeGreaterThan(0);
  });

  it('renders populated list of github accounts', () => {
    vi.mocked(useHealth).mockReturnValue({
      healthStatus: 'ok',
      mongo: 'up',
      version: '1.0.0',
      isSuccess: true,
      refetch: vi.fn(),
    } as unknown as UseHealthResult);
    vi.mocked(usePackagesList).mockReturnValue({
      data: [],
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof usePackagesList>);
    vi.mocked(useGitHubAccounts).mockReturnValue({
      data: [
        { connection_id: 'conn-1', login: 'user-primary', primary: true },
        { connection_id: 'conn-2', login: 'user-secondary', primary: false },
      ],
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof useGitHubAccounts>);

    renderWithProviders(<SettingsScreen />);

    // Verifies details rendered
    expect(screen.getByText('user-primary')).toBeInTheDocument();
    expect(screen.getByText('user-secondary')).toBeInTheDocument();
    expect(screen.getByText('primary')).toBeInTheDocument(); // Primary badge
    expect(screen.getByText('connection_id: conn-1')).toBeInTheDocument();
    expect(screen.getByText('connection_id: conn-2')).toBeInTheDocument();
  });

  it('refetches accounts query on mount and window focus', () => {
    const refetchMock = vi.fn();
    vi.mocked(useHealth).mockReturnValue({
      healthStatus: 'ok',
      mongo: 'up',
      version: '1.0.0',
      isSuccess: true,
      refetch: vi.fn(),
    } as unknown as UseHealthResult);
    vi.mocked(usePackagesList).mockReturnValue({
      data: [],
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof usePackagesList>);
    vi.mocked(useGitHubAccounts).mockReturnValue({
      data: [],
      isLoading: false,
      isError: false,
      refetch: refetchMock,
    } as unknown as ReturnType<typeof useGitHubAccounts>);

    renderWithProviders(<SettingsScreen />);

    // Called on mount
    expect(refetchMock).toHaveBeenCalled();

    // Trigger window focus
    fireEvent.focus(window);
    expect(refetchMock).toHaveBeenCalledTimes(2);
  });

  it('renders ConnectGitHub CTA as enabled link when VITE_NYXID_CONNECT_GITHUB_URL is set', () => {
    vi.mocked(useHealth).mockReturnValue({
      healthStatus: 'ok',
      mongo: 'up',
      version: '1.0.0',
      isSuccess: true,
      refetch: vi.fn(),
    } as unknown as UseHealthResult);
    vi.mocked(usePackagesList).mockReturnValue({
      data: [],
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof usePackagesList>);
    vi.mocked(useGitHubAccounts).mockReturnValue({
      data: [],
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    } as unknown as ReturnType<typeof useGitHubAccounts>);

    const originalUrl = import.meta.env.VITE_NYXID_CONNECT_GITHUB_URL;
    import.meta.env.VITE_NYXID_CONNECT_GITHUB_URL = 'https://nyx.chrono-ai.fun/api/v1/github/connect';

    try {
      renderWithProviders(<SettingsScreen />);
      
      const connectLinks = screen.getAllByRole('link', { name: 'Connect GitHub' });
      expect(connectLinks.length).toBeGreaterThan(0);
      connectLinks.forEach(link => {
        expect(link).toHaveAttribute('href', 'https://nyx.chrono-ai.fun/api/v1/github/connect');
      });
    } finally {
      import.meta.env.VITE_NYXID_CONNECT_GITHUB_URL = originalUrl;
    }
  });
});
