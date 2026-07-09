import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AuthProvider, useAuth } from './github-auth';

function Probe() {
  const { isAuthenticated, signOut } = useAuth();
  return (
    <div>
      <span>{isAuthenticated ? 'authed' : 'anon'}</span>
      <button type="button" onClick={signOut}>
        out
      </button>
    </div>
  );
}

describe('AuthProvider / useAuth', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
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

  it('signOut forgets the token set', async () => {
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
    expect(window.localStorage.getItem('fkst-gh-access')).toBeNull();
    expect(window.localStorage.getItem('fkst-gh-refresh')).toBeNull();
  });
});
