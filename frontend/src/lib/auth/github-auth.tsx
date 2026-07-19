import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from 'react';
import type { ReactNode } from 'react';
import { API_BASE, API_CONFIGURED } from '@/lib/env';

// The frontend holds the GitHub user token set the backend login callback hands it
// (in the redirect fragment) and refreshes it transparently, so the user stays
// signed in past the 8h access-token lifetime without interruption. GitHub's token
// endpoint needs the client secret + has no CORS, so the refresh EXCHANGE is a
// backend call (`POST /api/v1/auth/github/refresh`); the SPA only holds the tokens.

const ACCESS_KEY = 'fkst-gh-access';
const REFRESH_KEY = 'fkst-gh-refresh';
const EXPIRES_KEY = 'fkst-gh-expires'; // absolute ms; absent = non-expiring token
/** Refresh this many ms BEFORE the access token actually expires. */
const EXPIRY_SKEW_MS = 60_000;

const ls = {
  get(k: string): string | null {
    try {
      return window.localStorage.getItem(k);
    } catch {
      return null;
    }
  },
  set(k: string, v: string) {
    try {
      window.localStorage.setItem(k, v);
    } catch {
      /* private mode — ignore */
    }
  },
  del(k: string) {
    try {
      window.localStorage.removeItem(k);
    } catch {
      /* private mode — ignore */
    }
  },
};

function storeTokens(access: string, refresh: string | null, expiresInSecs: number | null) {
  ls.set(ACCESS_KEY, access);
  if (refresh) ls.set(REFRESH_KEY, refresh);
  if (expiresInSecs && expiresInSecs > 0) {
    ls.set(EXPIRES_KEY, String(Date.now() + expiresInSecs * 1000));
  } else {
    ls.del(EXPIRES_KEY); // non-expiring token: refresh only reactively (on 401)
  }
}

function clearTokens() {
  ls.del(ACCESS_KEY);
  ls.del(REFRESH_KEY);
  ls.del(EXPIRES_KEY);
}

interface RefreshBody {
  access_token: string;
  refresh_token?: string;
  expires_in?: number;
}

export interface AuthContextValue {
  /** Backend base URL is configured, so login can be attempted at all. */
  configured: boolean;
  /** A token set is present (may need a refresh before use). */
  isAuthenticated: boolean;
  /** OAuth error slug from the callback (e.g. `access_denied`), else null. */
  error: string | null;
  /**
   * The session was lost involuntarily (a 401 whose refresh could not recover
   * it), as opposed to an explicit sign-out. Stays true until the next fresh
   * sign-in so the dashboard can show a context-preserving "re-authenticate"
   * prompt rather than the cold sign-in card.
   */
  sessionExpired: boolean;
  /** Begin login: navigate the browser to the backend authorize endpoint. */
  signIn: () => void;
  /** Forget the local token set. */
  signOut: () => void;
  /** A valid access token, refreshing transparently if expired; null if signed out. */
  getToken: () => Promise<string | null>;
  /** fetch() against the API with a fresh Bearer token + one reactive-refresh retry on 401. */
  apiFetch: (path: string, init?: RequestInit) => Promise<Response>;
}

const AuthContext = createContext<AuthContextValue | null>(null);

export function AuthProvider({ children }: { children: ReactNode }) {
  const [isAuthenticated, setIsAuthenticated] = useState<boolean>(() => !!ls.get(ACCESS_KEY));
  const [error, setError] = useState<string | null>(null);
  const [sessionExpired, setSessionExpired] = useState<boolean>(false);
  // Coalesce concurrent refreshes into a single in-flight request.
  const inflight = useRef<Promise<string | null> | null>(null);

  const signOut = useCallback(() => {
    clearTokens();
    setIsAuthenticated(false);
    // An explicit sign-out is a deliberate clean exit, not an expiry — leave
    // sessionExpired alone so a real expiry flag can't be masked by it.
  }, []);

  // Involuntary loss of an authenticated session: a 401 whose refresh could not
  // recover it (refresh token missing or rejected). Flags sessionExpired so the
  // dashboard can prompt to re-authenticate while keeping the user's context.
  const expireSession = useCallback(() => {
    clearTokens();
    setIsAuthenticated(false);
    setSessionExpired(true);
  }, []);

  // Capture the token set (or error) the login callback delivered in the fragment.
  useEffect(() => {
    const raw = window.location.hash.startsWith('#') ? window.location.hash.slice(1) : '';
    if (!raw) return;
    const params = new URLSearchParams(raw);
    const token = params.get('gh_token');
    const errSlug = params.get('gh_error');
    if (token) {
      storeTokens(token, params.get('gh_refresh'), Number(params.get('gh_expires_in')) || null);
      setIsAuthenticated(true);
      // A fresh sign-in clears any prior expiry/error so the dashboard drops the
      // re-authenticate prompt and returns to the normal signed-in view.
      setError(null);
      setSessionExpired(false);
    } else if (errSlug) {
      // Surface the callback's real OAuth error slug (e.g. `access_denied`) so
      // the dashboard can map it to a specific message + retry/dismiss.
      setError(errSlug);
    }
    if (token || errSlug) {
      // Strip the fragment so the token never lingers in history / a shared URL.
      window.history.replaceState(
        null,
        '',
        window.location.pathname + window.location.search
      );
    }
  }, []);

  const refresh = useCallback((): Promise<string | null> => {
    if (inflight.current) return inflight.current;
    const rt = ls.get(REFRESH_KEY);
    if (!rt) {
      // No refresh token to recover with: the access token cannot be renewed,
      // so the session is over involuntarily.
      expireSession();
      return Promise.resolve(null);
    }
    const run = (async (): Promise<string | null> => {
      try {
        const res = await fetch(`${API_BASE}/api/v1/auth/github/refresh`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ refresh_token: rt }),
        });
        if (res.status === 401) {
          // The refresh token itself was rejected → the session expired and
          // must be re-established via a fresh login.
          expireSession();
          return null;
        }
        if (!res.ok) return null; // transient (5xx/network) — keep the session
        const data = (await res.json()) as RefreshBody;
        storeTokens(data.access_token, data.refresh_token ?? rt, data.expires_in ?? null);
        setIsAuthenticated(true);
        return data.access_token;
      } catch {
        return null; // network blip — keep the session, caller may retry
      } finally {
        inflight.current = null;
      }
    })();
    inflight.current = run;
    return run;
  }, [expireSession]);

  const getToken = useCallback(async (): Promise<string | null> => {
    const access = ls.get(ACCESS_KEY);
    if (!access) return null;
    const expiresAt = Number(ls.get(EXPIRES_KEY) ?? 0);
    // No expiry recorded → treat as non-expiring (refresh only reactively on 401).
    if (!expiresAt || Date.now() < expiresAt - EXPIRY_SKEW_MS) return access;
    return refresh();
  }, [refresh]);

  const apiFetch = useCallback(
    async (path: string, init: RequestInit = {}): Promise<Response> => {
      const url = path.startsWith('http') ? path : `${API_BASE}${path}`;
      const call = (token: string | null) =>
        fetch(url, {
          ...init,
          headers: {
            ...(init.headers ?? {}),
            ...(token ? { Authorization: `Bearer ${token}` } : {}),
          },
        });
      let res = await call(await getToken());
      if (res.status === 401) {
        const next = await refresh();
        if (next) res = await call(next);
      }
      return res;
    },
    [getToken, refresh]
  );

  const signIn = useCallback(() => {
    window.location.href = `${API_BASE}/api/v1/auth/github/login`;
  }, []);

  const value: AuthContextValue = {
    configured: API_CONFIGURED,
    isAuthenticated,
    error,
    sessionExpired,
    signIn,
    signOut,
    getToken,
    apiFetch,
  };

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthContextValue {
  const ctx = useContext(AuthContext);
  if (!ctx) {
    throw new Error('useAuth must be used within an <AuthProvider>');
  }
  return ctx;
}
