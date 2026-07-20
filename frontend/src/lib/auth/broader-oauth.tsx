import { createContext, useCallback, useContext, useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { API_BASE } from '@/lib/env';

// The BROADER (classic-OAuth) credential is an OPTIONAL, secondary token that
// unlocks full repo/org visibility — repositories and organizations where the
// fkst App is NOT installed. It is kept deliberately separate from the primary
// login session (github-auth.tsx):
//   - it lives in sessionStorage, not localStorage: a "show me everything"
//     grant is per-tab and never outlives the browser session, matching its
//     transient intent and keeping a broad-scope token off long-term storage;
//   - it never refreshes. It is only ever sent as a read HINT on the overview
//     call (`X-Github-Broader-Token`); a stale/rejected token simply falls back
//     to the installed-only view server-side, so there is nothing to recover.
// The backend delivers it in the return-redirect URL FRAGMENT (#broader_token),
// the SAME fragment-delivery discipline the primary login uses, so the token
// never reaches server logs, browser history, or a shared URL.

/** sessionStorage key — distinct from the primary login's `fkst-gh-*` keys. */
const BROADER_KEY = 'fkst-gh-broader';

const ss = {
  get(): string | null {
    try {
      return window.sessionStorage.getItem(BROADER_KEY);
    } catch {
      return null;
    }
  },
  set(v: string) {
    try {
      window.sessionStorage.setItem(BROADER_KEY, v);
    } catch {
      /* private mode — ignore */
    }
  },
  del() {
    try {
      window.sessionStorage.removeItem(BROADER_KEY);
    } catch {
      /* private mode — ignore */
    }
  },
};

export interface BroaderOAuthContextValue {
  /** A broader-visibility token is stored (the overview call will send it). */
  connected: boolean;
  /** The stored broader token, or null. Threaded onto the overview call only. */
  token: string | null;
  /** Begin the broader OAuth: full-page navigate to the backend authorize URL
   *  (same API base the primary login redirect uses). */
  connectBroader: () => void;
  /** Forget the broader token — returns the dashboard to the installed-only
   *  view (the next overview fetch omits the header). */
  disconnectBroader: () => void;
}

const BroaderOAuthContext = createContext<BroaderOAuthContextValue | null>(null);

export function BroaderOAuthProvider({ children }: { children: ReactNode }) {
  const [token, setToken] = useState<string | null>(() => ss.get());

  // Capture the broader token the connect callback delivered in the fragment.
  // Mirrors the primary login's fragment capture: parse the hash, store it,
  // then strip it so the token never lingers in history or a shared URL. Only
  // the `broader_token` param is removed, so a co-arriving primary-login
  // fragment (a different redirect in practice, but be defensive) survives for
  // AuthProvider's own handler.
  useEffect(() => {
    const raw = window.location.hash.startsWith('#') ? window.location.hash.slice(1) : '';
    if (!raw) return;
    const params = new URLSearchParams(raw);
    const captured = params.get('broader_token');
    if (!captured) return;
    ss.set(captured);
    setToken(captured);
    params.delete('broader_token');
    const rest = params.toString();
    window.history.replaceState(
      null,
      '',
      window.location.pathname + window.location.search + (rest ? `#${rest}` : '')
    );
  }, []);

  const connectBroader = useCallback(() => {
    window.location.href = `${API_BASE}/api/v1/auth/github/broader`;
  }, []);

  const disconnectBroader = useCallback(() => {
    ss.del();
    setToken(null);
  }, []);

  const value: BroaderOAuthContextValue = {
    connected: token != null,
    token,
    connectBroader,
    disconnectBroader,
  };

  return <BroaderOAuthContext.Provider value={value}>{children}</BroaderOAuthContext.Provider>;
}

export function useBroaderOAuth(): BroaderOAuthContextValue {
  const ctx = useContext(BroaderOAuthContext);
  if (!ctx) {
    throw new Error('useBroaderOAuth must be used within a <BroaderOAuthProvider>');
  }
  return ctx;
}
