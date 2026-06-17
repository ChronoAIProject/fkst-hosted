import { describe, it, expect, vi, beforeEach, afterEach, beforeAll, afterAll, MockInstance } from 'vitest';
import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import { NyxIDClient } from '../nyxid-client';
import {
  handleUnauthorized,
  resetRedirectInFlight,
  registerAuthErrorListener,
} from '../token';
import { getHealth } from '../../api/client';

const server = setupServer();

describe('token auth handling and loop guard', () => {
  let clearSessionSpy: MockInstance;
  let loginWithRedirectSpy: MockInstance;

  beforeAll(() => {
    server.listen({ onUnhandledRequest: 'bypass' });
  });

  beforeEach(() => {
    // Save original env values
    (import.meta.env as unknown as Record<string, string>).VITE_AUTH_REQUIRED = 'true';
    (import.meta.env as unknown as Record<string, string>).VITE_NYXID_BASE_URL = 'https://iam.example.com';
    (import.meta.env as unknown as Record<string, string>).VITE_NYXID_CLIENT_ID = 'test-client';

    // Reset loop guard state
    resetRedirectInFlight();

    // Spies on NyxIDClient prototype
    clearSessionSpy = vi.spyOn(NyxIDClient.prototype, 'clearSession');
    loginWithRedirectSpy = vi
      .spyOn(NyxIDClient.prototype, 'loginWithRedirect')
      .mockResolvedValue(undefined);

    if (typeof window !== 'undefined') {
      sessionStorage.clear();
    }
  });

  afterEach(() => {
    server.resetHandlers();
    vi.restoreAllMocks();
    if (typeof window !== 'undefined') {
      sessionStorage.clear();
    }
  });

  afterAll(() => {
    server.close();
  });

  it('calls clearSession and loginWithRedirect when unauthorized and auth is required', () => {
    handleUnauthorized();

    expect(clearSessionSpy).toHaveBeenCalledTimes(1);
    expect(loginWithRedirectSpy).toHaveBeenCalledTimes(1);
    expect(sessionStorage.getItem('nyxid:auto_login_attempts')).toBe('1');
    expect(sessionStorage.getItem('nyxid:auth_error')).toBeNull();
  });

  it('prevents redirects and triggers auth-error state if 401 happens shortly after callback', () => {
    const errorMsgListener = vi.fn();
    const cleanup = registerAuthErrorListener(errorMsgListener);

    try {
      // Simulate recent callback
      sessionStorage.setItem('nyxid:last_callback_at', String(Date.now()));

      handleUnauthorized();

      expect(clearSessionSpy).toHaveBeenCalledTimes(1);
      expect(loginWithRedirectSpy).not.toHaveBeenCalled();
      expect(sessionStorage.getItem('nyxid:auth_error')).toBe('Session rejected — please sign in again');
      expect(errorMsgListener).toHaveBeenCalledWith('Session rejected — please sign in again');
    } finally {
      cleanup();
    }
  });

  it('prevents redirects and triggers auth-error state if auto-login attempts >= 3', () => {
    const errorMsgListener = vi.fn();
    const cleanup = registerAuthErrorListener(errorMsgListener);

    try {
      // Set attempts to 3
      sessionStorage.setItem('nyxid:auto_login_attempts', '3');

      handleUnauthorized();

      expect(clearSessionSpy).toHaveBeenCalledTimes(1);
      expect(loginWithRedirectSpy).not.toHaveBeenCalled();
      expect(sessionStorage.getItem('nyxid:auth_error')).toBe('Session rejected — please sign in again');
      expect(errorMsgListener).toHaveBeenCalledWith('Session rejected — please sign in again');
    } finally {
      cleanup();
    }
  });

  it('guards concurrent 401s so that only the first triggers clearSession/loginWithRedirect', async () => {
    // Keep login redirect pending to simulate in-flight redirect
    let resolveLogin: (() => void) | undefined = undefined;
    const loginPendingPromise = new Promise<void>((resolve) => {
      resolveLogin = resolve;
    });
    loginWithRedirectSpy.mockReturnValue(loginPendingPromise);

    // Call concurrently
    handleUnauthorized();
    handleUnauthorized();
    handleUnauthorized();

    expect(clearSessionSpy).toHaveBeenCalledTimes(1);
    expect(loginWithRedirectSpy).toHaveBeenCalledTimes(1);

    if (resolveLogin) {
      (resolveLogin as () => void)();
    }
  });

  it('does nothing if authRequired() is false', () => {
    (import.meta.env as unknown as Record<string, string>).VITE_AUTH_REQUIRED = 'false';

    handleUnauthorized();

    expect(clearSessionSpy).not.toHaveBeenCalled();
    expect(loginWithRedirectSpy).not.toHaveBeenCalled();
  });

  it('integration: a 401 response from the API client actually calls clearSession() and loginWithRedirect()', async () => {
    server.use(
      http.get('*/api/v1/health', () => {
        return HttpResponse.json({ error: 'unauthorized' }, { status: 401 });
      })
    );

    await expect(getHealth()).rejects.toThrow();

    expect(clearSessionSpy).toHaveBeenCalledTimes(1);
    expect(loginWithRedirectSpy).toHaveBeenCalledTimes(1);
  });

  it('integration: a 401 response from the API client prevents second redirect on post-callback / repeated 401', async () => {
    server.use(
      http.get('*/api/v1/health', () => {
        return HttpResponse.json({ error: 'unauthorized' }, { status: 401 });
      })
    );

    // Simulate recent callback
    sessionStorage.setItem('nyxid:last_callback_at', String(Date.now()));

    await expect(getHealth()).rejects.toThrow();

    expect(clearSessionSpy).toHaveBeenCalledTimes(1);
    expect(loginWithRedirectSpy).not.toHaveBeenCalled();
    expect(sessionStorage.getItem('nyxid:auth_error')).toBe('Session rejected — please sign in again');
  });
});
