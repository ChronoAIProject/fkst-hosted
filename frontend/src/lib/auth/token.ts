import { NyxIDClient } from './nyxid-client';

export function authRequired(): boolean {
  const val = import.meta.env.VITE_AUTH_REQUIRED;
  return val === 'true' || val === '1' || val === true;
}

export function getAccessToken(): string | null {
  if (!authRequired()) {
    return null;
  }
  const clientId = import.meta.env.VITE_NYXID_CLIENT_ID;
  if (!clientId) {
    return null;
  }
  if (typeof window === 'undefined' || !window.localStorage) {
    return null;
  }
  const raw = window.localStorage.getItem(`nyxid:tokens:${clientId}`);
  if (!raw) {
    return null;
  }
  try {
    const tokens = JSON.parse(raw);
    return tokens?.accessToken ?? null;
  } catch {
    return null;
  }
}

export type AuthErrorListener = (error: string | null) => void;
const listeners = new Set<AuthErrorListener>();

export function registerAuthErrorListener(listener: AuthErrorListener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function notifyAuthError(error: string | null): void {
  listeners.forEach((l) => l(error));
}

let redirectInFlight = false;

export function resetRedirectInFlight(): void {
  redirectInFlight = false;
}

export function handleUnauthorized(): void {
  if (!authRequired()) {
    return;
  }

  if (redirectInFlight) {
    return;
  }
  redirectInFlight = true;

  const baseUrl = import.meta.env.VITE_NYXID_BASE_URL || '';
  const clientId = import.meta.env.VITE_NYXID_CLIENT_ID || '';
  const origin = typeof window !== 'undefined' ? window.location.origin : 'http://localhost';
  const redirectUri = import.meta.env.VITE_NYXID_REDIRECT_URI || `${origin}/auth/callback`;
  const scope = 'openid profile email';

  const client = new NyxIDClient({ baseUrl, clientId, redirectUri, scope });
  client.clearSession();

  if (typeof window !== 'undefined') {
    const lastCallback = sessionStorage.getItem('nyxid:last_callback_at');
    const isRecentCallback = lastCallback && (Date.now() - parseInt(lastCallback, 10) < 10000); // 10 seconds
    const attempts = parseInt(sessionStorage.getItem('nyxid:auto_login_attempts') || '0', 10);

    if (isRecentCallback || attempts >= 3) {
      sessionStorage.setItem('nyxid:auth_error', 'Session rejected — please sign in again');
      notifyAuthError('Session rejected — please sign in again');
      redirectInFlight = false;
      return;
    }

    sessionStorage.setItem('nyxid:auto_login_attempts', String(attempts + 1));
    client.loginWithRedirect().catch((err) => {
      console.error('Failed to redirect to login after 401:', err);
      redirectInFlight = false;
    });
  }
}


