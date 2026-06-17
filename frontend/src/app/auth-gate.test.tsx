/* eslint-disable @typescript-eslint/no-explicit-any */
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { App } from './index';
import { authRequired, useAuthSession } from '@/lib/auth';

vi.mock('@/lib/auth', () => {
  if (!(globalThis as any).__mockAuthRequired) {
    (globalThis as any).__mockAuthRequired = vi.fn();
    (globalThis as any).__mockUseAuthSession = vi.fn();
  }
  return {
    authRequired: (globalThis as any).__mockAuthRequired,
    useAuthSession: (globalThis as any).__mockUseAuthSession,
  };
});

vi.mock('../lib/auth', () => {
  if (!(globalThis as any).__mockAuthRequired) {
    (globalThis as any).__mockAuthRequired = vi.fn();
    (globalThis as any).__mockUseAuthSession = vi.fn();
  }
  return {
    authRequired: (globalThis as any).__mockAuthRequired,
    useAuthSession: (globalThis as any).__mockUseAuthSession,
  };
});

// Mock fetch to avoid unhandled network requests
const mockFetch: typeof fetch = (input: RequestInfo | URL) => {
  const href = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
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
    return Promise.resolve(
      new Response(
        JSON.stringify([]),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      )
    );
  }
  if (href.includes('/api/v1/goals')) {
    return Promise.resolve(
      new Response(
        JSON.stringify([]),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      )
    );
  }
  return Promise.resolve(new Response(JSON.stringify({}), { status: 200, headers: { 'Content-Type': 'application/json' } }));
};

describe('Env-driven NyxID Auth Gate', () => {
  let originalFetch: typeof globalThis.fetch;

  beforeEach(() => {
    originalFetch = globalThis.fetch;
    vi.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);
    // Suppress console error output for expected errors in tests
    vi.spyOn(console, 'error').mockImplementation(() => {});
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  it('gated: authRequired true + unauthenticated -> gate/login triggered, protected content not rendered', async () => {
    // Mock authentication: required, but not authenticated
    vi.mocked(authRequired).mockReturnValue(true);
    const mockLogin = vi.fn().mockResolvedValue(undefined);
    vi.mocked(useAuthSession).mockReturnValue({
      isAuthenticated: false,
      accessToken: null,
      login: mockLogin,
      logout: vi.fn(),
      handleRedirectCallback: vi.fn(),
      getUserInfo: vi.fn(),
    });

    window.history.pushState({}, '', '/overview');
    render(<App />);

    // Assert that protected content (like overview cells or titles) is not rendered
    await waitFor(() => {
      expect(screen.queryByText('Design')).not.toBeInTheDocument();
      expect(screen.queryByText('Build')).not.toBeInTheDocument();
      // Assert that sign in gate is visible
      expect(screen.getByText('Authentication Required')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: /Sign in with NyxID/i })).toBeInTheDocument();
    });

    // Click Sign in button to verify login trigger
    const signInButton = screen.getByRole('button', { name: /Sign in with NyxID/i });
    fireEvent.click(signInButton);
    expect(mockLogin).toHaveBeenCalled();
  });

  it('ungated: authRequired false -> app renders normally, no gate', async () => {
    // Mock authentication: not required
    vi.mocked(authRequired).mockReturnValue(false);
    vi.mocked(useAuthSession).mockReturnValue({
      isAuthenticated: false,
      accessToken: null,
      login: vi.fn(),
      logout: vi.fn(),
      handleRedirectCallback: vi.fn(),
      getUserInfo: vi.fn(),
    });

    window.history.pushState({}, '', '/overview');
    render(<App />);

    // Assert that the app renders normally
    await waitFor(() => {
      expect(screen.getByText('Design')).toBeInTheDocument();
      expect(screen.getByText('Build')).toBeInTheDocument();
      expect(screen.queryByText('Authentication Required')).not.toBeInTheDocument();
    });
  });

  it('callback: callback route remains reachable without being gated', async () => {
    // Mock authentication: required, but unauthenticated (which would gate normal routes)
    vi.mocked(authRequired).mockReturnValue(true);
    const mockHandleRedirectCallback = vi.fn().mockResolvedValue({
      accessToken: 'test-token',
      tokenType: 'Bearer',
      expiresIn: 3600,
    });
    vi.mocked(useAuthSession).mockReturnValue({
      isAuthenticated: false,
      accessToken: null,
      login: vi.fn(),
      logout: vi.fn(),
      handleRedirectCallback: mockHandleRedirectCallback,
      getUserInfo: vi.fn(),
    });

    window.history.pushState({}, '', '/auth/callback');
    render(<App />);

    // Assert callback content is rendered (which shows "Completing login...") rather than the gate
    await waitFor(() => {
      expect(screen.getByText(/Completing login\.\.\./i)).toBeInTheDocument();
      expect(screen.queryByText('Authentication Required')).not.toBeInTheDocument();
    });
  });
});
