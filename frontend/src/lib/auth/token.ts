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
  const raw = localStorage.getItem(`nyxid:tokens:${clientId}`);
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

export function handleUnauthorized(): void {
  if (!authRequired()) {
    return;
  }
  const baseUrl = import.meta.env.VITE_NYXID_BASE_URL || '';
  const clientId = import.meta.env.VITE_NYXID_CLIENT_ID || '';
  const origin = typeof window !== 'undefined' ? window.location.origin : 'http://localhost';
  const redirectUri = import.meta.env.VITE_NYXID_REDIRECT_URI || `${origin}/auth/callback`;
  const scope = 'openid profile email';

  const client = new NyxIDClient({ baseUrl, clientId, redirectUri, scope });
  client.clearSession();

  if (typeof window !== 'undefined') {
    client.loginWithRedirect().catch((err) => {
      console.error('Failed to redirect to login after 401:', err);
    });
  }
}

