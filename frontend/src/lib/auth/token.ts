export function authRequired(): boolean {
  const val = import.meta.env.VITE_AUTH_REQUIRED;
  if (val === undefined) {
    // For tests, if it's unset, default to false. Otherwise default to true.
    const isTest = typeof process !== 'undefined' && (process.env.NODE_ENV === 'test' || import.meta.env.MODE === 'test');
    return !isTest;
  }
  return val === 'true' || val === true;
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
