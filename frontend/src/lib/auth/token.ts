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
