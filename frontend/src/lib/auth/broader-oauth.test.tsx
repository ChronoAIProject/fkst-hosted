import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { BroaderOAuthProvider, useBroaderOAuth } from './broader-oauth';

function Probe() {
  const { connected, token, disconnectBroader } = useBroaderOAuth();
  return (
    <div>
      <span>{connected ? 'connected' : 'disconnected'}</span>
      <span>token:{token ?? 'none'}</span>
      <button type="button" onClick={disconnectBroader}>
        disconnect
      </button>
    </div>
  );
}

const STORAGE_KEY = 'fkst-gh-broader';

describe('BroaderOAuthProvider / useBroaderOAuth', () => {
  beforeEach(() => {
    window.sessionStorage.clear();
    window.localStorage.clear();
    window.location.hash = '';
  });

  it('is disconnected with no stored token and no fragment', () => {
    render(
      <BroaderOAuthProvider>
        <Probe />
      </BroaderOAuthProvider>
    );
    expect(screen.getByText('disconnected')).toBeInTheDocument();
    expect(screen.getByText('token:none')).toBeInTheDocument();
  });

  it('reports connected when a broader token is already stored', () => {
    window.sessionStorage.setItem(STORAGE_KEY, 'brd_stored');
    render(
      <BroaderOAuthProvider>
        <Probe />
      </BroaderOAuthProvider>
    );
    expect(screen.getByText('connected')).toBeInTheDocument();
    expect(screen.getByText('token:brd_stored')).toBeInTheDocument();
  });

  it('captures the broader token from the return fragment and strips the hash', () => {
    window.location.hash = '#broader_token=brd_frag';
    render(
      <BroaderOAuthProvider>
        <Probe />
      </BroaderOAuthProvider>
    );
    expect(screen.getByText('connected')).toBeInTheDocument();
    expect(screen.getByText('token:brd_frag')).toBeInTheDocument();
    // Persisted under the distinct sessionStorage key (never localStorage).
    expect(window.sessionStorage.getItem(STORAGE_KEY)).toBe('brd_frag');
    expect(window.localStorage.getItem(STORAGE_KEY)).toBeNull();
    // Fragment stripped so the token never lingers in history / a shared URL.
    expect(window.location.hash).toBe('');
  });

  it('leaves a non-broader fragment intact and stays disconnected', () => {
    // A primary-login fragment must survive for AuthProvider's own handler —
    // this provider only ever touches its own `broader_token` param.
    window.location.hash = '#gh_token=ghu_x';
    render(
      <BroaderOAuthProvider>
        <Probe />
      </BroaderOAuthProvider>
    );
    expect(screen.getByText('disconnected')).toBeInTheDocument();
    expect(window.location.hash).toBe('#gh_token=ghu_x');
    expect(window.sessionStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it('disconnectBroader clears the stored token and flips back to disconnected', async () => {
    const user = userEvent.setup();
    window.sessionStorage.setItem(STORAGE_KEY, 'brd_live');
    render(
      <BroaderOAuthProvider>
        <Probe />
      </BroaderOAuthProvider>
    );
    expect(screen.getByText('connected')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'disconnect' }));
    expect(screen.getByText('disconnected')).toBeInTheDocument();
    expect(screen.getByText('token:none')).toBeInTheDocument();
    expect(window.sessionStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it('throws when used outside its provider', () => {
    // Silence the expected React error boundary console noise for this render.
    const Bare = () => {
      useBroaderOAuth();
      return null;
    };
    expect(() => render(<Bare />)).toThrow(/must be used within a <BroaderOAuthProvider>/);
  });
});
