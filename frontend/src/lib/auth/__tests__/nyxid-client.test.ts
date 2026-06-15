import { describe, it, expect, vi } from 'vitest';
import { NyxIDClient, StorageLike } from '../nyxid-client';

class MockStorage implements StorageLike {
  private data = new Map<string, string>();
  getItem(key: string): string | null {
    return this.data.get(key) ?? null;
  }
  setItem(key: string, value: string): void {
    this.data.set(key, value);
  }
  removeItem(key: string): void {
    this.data.delete(key);
  }
}

describe('NyxIDClient PKCE Flow', () => {
  const config = {
    baseUrl: 'https://iam.example.com',
    clientId: 'test-client',
    redirectUri: 'https://app.example.com/auth/callback',
    scope: 'openid profile',
  };

  it('generates a valid authorization URL with state and PKCE challenges', async () => {
    const storage = new MockStorage();
    const client = new NyxIDClient({ ...config, storage });

    const authorizeUrlStr = await client.buildAuthorizeUrl({ state: 'fixed-state' });
    const authorizeUrl = new URL(authorizeUrlStr);

    expect(authorizeUrl.origin).toBe('https://iam.example.com');
    expect(authorizeUrl.pathname).toBe('/oauth/authorize');
    expect(authorizeUrl.searchParams.get('response_type')).toBe('code');
    expect(authorizeUrl.searchParams.get('client_id')).toBe('test-client');
    expect(authorizeUrl.searchParams.get('redirect_uri')).toBe('https://app.example.com/auth/callback');
    expect(authorizeUrl.searchParams.get('scope')).toBe('openid profile');
    expect(authorizeUrl.searchParams.get('state')).toBe('fixed-state');
    expect(authorizeUrl.searchParams.get('code_challenge_method')).toBe('S256');
    expect(authorizeUrl.searchParams.get('code_challenge')).toBeTruthy();

    // Check that pending state was saved to storage
    const rawPending = storage.getItem('nyxid:pending:test-client');
    expect(rawPending).toBeTruthy();
    const pending = JSON.parse(rawPending!);
    expect(pending.state).toBe('fixed-state');
    expect(pending.codeVerifier).toBeTruthy();
  });

  it('rejects the callback if state is mismatched', async () => {
    const storage = new MockStorage();
    const client = new NyxIDClient({ ...config, storage });

    // Build auth URL to set state in storage
    await client.buildAuthorizeUrl({ state: 'expected-state' });

    // Call redirect with mismatched state
    const callbackUrl = 'https://app.example.com/auth/callback?code=123&state=mismatched-state';
    
    await expect(client.handleRedirectCallback(callbackUrl)).rejects.toThrow('State mismatch');
  });

  it('rejects the callback if pending state is missing from storage', async () => {
    const storage = new MockStorage();
    const client = new NyxIDClient({ ...config, storage });

    const callbackUrl = 'https://app.example.com/auth/callback?code=123&state=some-state';
    
    await expect(client.handleRedirectCallback(callbackUrl)).rejects.toThrow('Missing PKCE state in storage');
  });

  it('successfully exchanges code for tokens and saves them to storage', async () => {
    const storage = new MockStorage();
    const mockFetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        access_token: 'mock-access-token',
        token_type: 'Bearer',
        expires_in: 3600,
        refresh_token: 'mock-refresh-token',
        id_token: 'mock-id-token',
        scope: 'openid profile',
      }),
    });

    const client = new NyxIDClient({ ...config, storage, fetchFn: mockFetch });

    // Start PKCE flow
    await client.buildAuthorizeUrl({ state: 'valid-state' });

    // Complete PKCE flow
    const callbackUrl = 'https://app.example.com/auth/callback?code=auth-code-123&state=valid-state';
    const tokens = await client.handleRedirectCallback(callbackUrl);

    expect(tokens.accessToken).toBe('mock-access-token');
    expect(tokens.tokenType).toBe('Bearer');
    expect(tokens.expiresIn).toBe(3600);
    expect(tokens.refreshToken).toBe('mock-refresh-token');
    expect(tokens.idToken).toBe('mock-id-token');

    // Tokens should be stored in the storage
    const stored = client.getStoredTokens();
    expect(stored).toEqual(tokens);

    const call = mockFetch.mock.calls[0]!;
    const url = call[0];
    const init = call[1];
    expect(url).toBe('https://iam.example.com/oauth/token');
    expect(init.method).toBe('POST');
    expect(init.headers['Content-Type']).toBe('application/x-www-form-urlencoded');

    const params = new URLSearchParams(init.body);
    expect(params.get('grant_type')).toBe('authorization_code');
    expect(params.get('code')).toBe('auth-code-123');
    expect(params.get('redirect_uri')).toBe('https://app.example.com/auth/callback');
    expect(params.get('client_id')).toBe('test-client');
    expect(params.get('code_verifier')).toBeTruthy();
  });
});
