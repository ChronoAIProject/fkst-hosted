import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AuthProvider, useAuth } from './github-auth';

function Probe() {
  const { isAuthenticated, sessionExpired, error, signOut, apiFetch } = useAuth();
  return (
    <div>
      <span>{isAuthenticated ? 'authed' : 'anon'}</span>
      <span>{sessionExpired ? 'expired' : 'live'}</span>
      <span>error:{error ?? 'none'}</span>
      <button type="button" onClick={signOut}>
        out
      </button>
      <button
        type="button"
        // apiFetch drives the reactive-refresh path we assert on; swallow the
        // returned Response so an unhandled rejection can't fail the test.
        onClick={() => {
          void apiFetch('/api/v1/anything').catch(() => undefined);
        }}
      >
        call
      </button>
    </div>
  );
}

describe('AuthProvider / useAuth', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('is anonymous with no stored token', () => {
    render(
      <AuthProvider>
        <Probe />
      </AuthProvider>
    );
    expect(screen.getByText('anon')).toBeInTheDocument();
  });

  it('reports authenticated when an access token is stored', () => {
    window.localStorage.setItem('fkst-gh-access', 'ghu_stored');
    render(
      <AuthProvider>
        <Probe />
      </AuthProvider>
    );
    expect(screen.getByText('authed')).toBeInTheDocument();
  });

  it('captures the token set from the callback fragment and clears the hash', () => {
    window.location.hash = '#gh_token=ghu_frag&gh_refresh=ghr_frag&gh_expires_in=28800';
    render(
      <AuthProvider>
        <Probe />
      </AuthProvider>
    );
    expect(screen.getByText('authed')).toBeInTheDocument();
    expect(window.localStorage.getItem('fkst-gh-access')).toBe('ghu_frag');
    expect(window.localStorage.getItem('fkst-gh-refresh')).toBe('ghr_frag');
    expect(window.location.hash).toBe('');
  });

  it('signOut forgets the token set without flagging an expiry', async () => {
    const user = userEvent.setup();
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
    window.localStorage.setItem('fkst-gh-refresh', 'ghr_y');
    render(
      <AuthProvider>
        <Probe />
      </AuthProvider>
    );
    expect(screen.getByText('authed')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'out' }));
    expect(screen.getByText('anon')).toBeInTheDocument();
    // An explicit sign-out is a clean exit — never the involuntary-expiry state.
    expect(screen.getByText('live')).toBeInTheDocument();
    expect(window.localStorage.getItem('fkst-gh-access')).toBeNull();
    expect(window.localStorage.getItem('fkst-gh-refresh')).toBeNull();
  });

  it('exposes the real OAuth error slug from the callback fragment', () => {
    window.location.hash = '#gh_error=access_denied';
    render(
      <AuthProvider>
        <Probe />
      </AuthProvider>
    );
    expect(screen.getByText('error:access_denied')).toBeInTheDocument();
    expect(screen.getByText('anon')).toBeInTheDocument();
    expect(window.location.hash).toBe('');
  });

  it('a fresh sign-in fragment leaves no error slug and no expiry', () => {
    window.location.hash = '#gh_token=ghu_new&gh_refresh=ghr_new&gh_expires_in=28800';
    render(
      <AuthProvider>
        <Probe />
      </AuthProvider>
    );
    // The token branch actively clears error + sessionExpired on sign-in.
    expect(screen.getByText('error:none')).toBeInTheDocument();
    expect(screen.getByText('live')).toBeInTheDocument();
    expect(screen.getByText('authed')).toBeInTheDocument();
  });

  it('sets sessionExpired when a 401 refresh is rejected (401)', async () => {
    const user = userEvent.setup();
    // Every fetch — the API call and the refresh exchange — returns 401, so the
    // refresh token is rejected and the session cannot be recovered.
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => ({ status: 401, ok: false, json: async () => ({}) }) as unknown as Response)
    );
    window.localStorage.setItem('fkst-gh-access', 'ghu_expired');
    window.localStorage.setItem('fkst-gh-refresh', 'ghr_dead');
    render(
      <AuthProvider>
        <Probe />
      </AuthProvider>
    );
    expect(screen.getByText('authed')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'call' }));
    expect(await screen.findByText('expired')).toBeInTheDocument();
    expect(screen.getByText('anon')).toBeInTheDocument();
    // Tokens are cleared on an involuntary expiry, same as a sign-out.
    expect(window.localStorage.getItem('fkst-gh-access')).toBeNull();
    expect(window.localStorage.getItem('fkst-gh-refresh')).toBeNull();
  });

  it('sets sessionExpired when a 401 has no refresh token to recover with', async () => {
    const user = userEvent.setup();
    // API call 401s; with no refresh token stored, refresh() cannot recover.
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => ({ status: 401, ok: false, json: async () => ({}) }) as unknown as Response)
    );
    window.localStorage.setItem('fkst-gh-access', 'ghu_only');
    render(
      <AuthProvider>
        <Probe />
      </AuthProvider>
    );
    await user.click(screen.getByRole('button', { name: 'call' }));
    expect(await screen.findByText('expired')).toBeInTheDocument();
    expect(screen.getByText('anon')).toBeInTheDocument();
  });

  it('keeps the session on a transient (5xx) refresh failure', async () => {
    const user = userEvent.setup();
    // First fetch (API) 401s to trigger a refresh; the refresh 500s (transient).
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce({ status: 401, ok: false, json: async () => ({}) })
      .mockResolvedValueOnce({ status: 500, ok: false, json: async () => ({}) });
    vi.stubGlobal('fetch', fetchMock as unknown as typeof fetch);
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
    window.localStorage.setItem('fkst-gh-refresh', 'ghr_y');
    render(
      <AuthProvider>
        <Probe />
      </AuthProvider>
    );
    await user.click(screen.getByRole('button', { name: 'call' }));
    // A 5xx is transient: the session must NOT be flagged expired, tokens kept.
    expect(screen.getByText('live')).toBeInTheDocument();
    expect(screen.getByText('authed')).toBeInTheDocument();
    expect(window.localStorage.getItem('fkst-gh-access')).toBe('ghu_x');
  });
});
