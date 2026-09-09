import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render, screen } from '@testing-library/react';
import { AuthProvider, useAuth } from './github-auth';

// `identityGeneration` is what per-viewer caches key on, so its contract needs
// its own coverage: it must move whenever the token set is REPLACED, and stay
// put when the same viewer merely gets a fresher access token.

function Probe() {
  const { identityGeneration, isAuthenticated, signOut } = useAuth();
  return (
    <div>
      <span data-testid="generation">{identityGeneration}</span>
      <span data-testid="authenticated">{String(isAuthenticated)}</span>
      <button type="button" onClick={signOut}>
        sign out
      </button>
    </div>
  );
}

const generation = () => Number(screen.getByTestId('generation').textContent);

beforeEach(() => {
  window.localStorage.clear();
  window.location.hash = '';
});

afterEach(() => {
  vi.unstubAllGlobals();
  window.location.hash = '';
});

describe('identityGeneration', () => {
  it('starts at zero for whatever token set was on disk at mount', () => {
    window.localStorage.setItem('fkst-gh-access', 'token-a');
    render(
      <AuthProvider>
        <Probe />
      </AuthProvider>
    );
    expect(generation()).toBe(0);
    expect(screen.getByTestId('authenticated')).toHaveTextContent('true');
  });

  it('moves on sign-out', () => {
    window.localStorage.setItem('fkst-gh-access', 'token-a');
    render(
      <AuthProvider>
        <Probe />
      </AuthProvider>
    );
    act(() => {
      screen.getByRole('button', { name: 'sign out' }).click();
    });
    expect(generation()).toBe(1);
    expect(screen.getByTestId('authenticated')).toHaveTextContent('false');
  });

  it('moves when a login callback swaps one account for another', () => {
    // Already signed in as one account; the callback fragment carries another.
    // `isAuthenticated` stays true throughout, which is exactly why a cache
    // keyed only on that flag would keep the previous person's rows.
    window.localStorage.setItem('fkst-gh-access', 'token-a');
    window.location.hash = '#gh_token=token-b&gh_refresh=refresh-b';
    render(
      <AuthProvider>
        <Probe />
      </AuthProvider>
    );
    expect(generation()).toBe(1);
    expect(screen.getByTestId('authenticated')).toHaveTextContent('true');
    expect(window.localStorage.getItem('fkst-gh-access')).toBe('token-b');
  });

  it('does NOT move for a transparent refresh of the same identity', async () => {
    window.localStorage.setItem('fkst-gh-access', 'token-a');
    window.localStorage.setItem('fkst-gh-refresh', 'refresh-a');
    // An expired access token forces the reactive refresh path.
    window.localStorage.setItem('fkst-gh-expires', String(Date.now() - 1000));
    const fetchMock = vi.fn(async () => ({
      ok: true,
      status: 200,
      json: async () => ({ access_token: 'token-a2', expires_in: 3600 }),
    }));
    vi.stubGlobal('fetch', fetchMock);

    let apiFetch!: (path: string) => Promise<Response>;
    function Capture() {
      apiFetch = useAuth().apiFetch;
      return null;
    }
    render(
      <AuthProvider>
        <Probe />
        <Capture />
      </AuthProvider>
    );

    await act(async () => {
      await apiFetch('/api/v1/anything');
    });
    // Same viewer, newer token: nothing per-viewer needs invalidating.
    expect(generation()).toBe(0);
    expect(window.localStorage.getItem('fkst-gh-access')).toBe('token-a2');
  });

  it('moves when a 401 cannot be recovered and the session expires', async () => {
    window.localStorage.setItem('fkst-gh-access', 'token-a');
    // No refresh token: the 401 path has nothing to recover with.
    const fetchMock = vi.fn(async () => ({ ok: false, status: 401, json: async () => ({}) }));
    vi.stubGlobal('fetch', fetchMock);

    let apiFetch!: (path: string) => Promise<Response>;
    function Capture() {
      apiFetch = useAuth().apiFetch;
      return null;
    }
    render(
      <AuthProvider>
        <Probe />
        <Capture />
      </AuthProvider>
    );

    await act(async () => {
      await apiFetch('/api/v1/anything');
    });
    expect(generation()).toBe(1);
    expect(screen.getByTestId('authenticated')).toHaveTextContent('false');
  });
});
