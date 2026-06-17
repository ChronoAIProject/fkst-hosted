import React, { createContext, useContext, useState, useEffect, useMemo, useCallback } from 'react';
import { NyxIDClient, LoginRedirectOptions, OAuthUserInfo, NyxIDTokenSet } from './nyxid-client';
import { authRequired, registerAuthErrorListener } from './token';
import { clearPersistedCache } from '../persist/persister';

export interface AuthSession {
  isAuthenticated: boolean;
  accessToken: string | null;
  login: (options?: LoginRedirectOptions) => Promise<void>;
  logout: () => void;
  handleRedirectCallback: (url?: string) => Promise<NyxIDTokenSet>;
  getUserInfo: () => Promise<OAuthUserInfo>;
}

const AuthContext = createContext<AuthSession | null>(null);

export interface NyxIDProviderProps {
  children: React.ReactNode;
  baseUrl: string;
  clientId: string;
  redirectUri: string;
  scope?: string;
}

export function NyxIDProvider({
  children,
  baseUrl,
  clientId,
  redirectUri,
  scope,
}: NyxIDProviderProps) {
  const client = useMemo(() => {
    return new NyxIDClient({ baseUrl, clientId, redirectUri, scope });
  }, [baseUrl, clientId, redirectUri, scope]);

  const [accessToken, setAccessToken] = useState<string | null>(() => {
    if (!authRequired()) return null;
    return client.getStoredTokens()?.accessToken ?? null;
  });

  const [authError, setAuthError] = useState<string | null>(() => {
    if (typeof window !== 'undefined') {
      return sessionStorage.getItem('nyxid:auth_error');
    }
    return null;
  });

  useEffect(() => {
    return registerAuthErrorListener((err) => {
      setAuthError(err);
      if (err) {
        setAccessToken(null);
      }
    });
  }, []);

  const isAuthenticated = useMemo(() => {
    if (!authRequired()) return false;
    return accessToken !== null;
  }, [accessToken]);

  // Synchronize stored tokens on mount/client update
  useEffect(() => {
    if (authRequired()) {
      const tokens = client.getStoredTokens();
      if (tokens?.accessToken !== accessToken) {
        setAccessToken(tokens?.accessToken ?? null);
      }
    }
  }, [client, accessToken]);

  const login = useCallback(async (options?: LoginRedirectOptions) => {
    if (typeof window !== 'undefined') {
      sessionStorage.removeItem('nyxid:auth_error');
      sessionStorage.setItem('nyxid:auto_login_attempts', '0');
      sessionStorage.removeItem('nyxid:last_callback_at');
    }
    setAuthError(null);
    await client.loginWithRedirect(options);
  }, [client]);

  const logout = useCallback(() => {
    if (typeof window !== 'undefined') {
      sessionStorage.removeItem('nyxid:auth_error');
      sessionStorage.setItem('nyxid:auto_login_attempts', '0');
      sessionStorage.removeItem('nyxid:last_callback_at');
    }
    setAuthError(null);
    client.clearSession();
    setAccessToken(null);
    // Wipe the persisted query cache so one identity's cached goal/GitHub
    // reads never bleed into the next session (ARCHITECTURE.md §8).
    void clearPersistedCache();
  }, [client]);

  const handleRedirectCallback = useCallback(async (url?: string) => {
    const tokens = await client.handleRedirectCallback(url);
    setAccessToken(tokens.accessToken);
    if (typeof window !== 'undefined') {
      sessionStorage.setItem('nyxid:last_callback_at', String(Date.now()));
      sessionStorage.setItem('nyxid:auto_login_attempts', '0');
      sessionStorage.removeItem('nyxid:auth_error');
    }
    setAuthError(null);
    return tokens;
  }, [client]);

  const getUserInfo = useCallback(async () => {
    return await client.getUserInfo(accessToken ?? undefined);
  }, [client, accessToken]);

  const value = useMemo<AuthSession>(() => ({
    isAuthenticated,
    accessToken,
    login,
    logout,
    handleRedirectCallback,
    getUserInfo,
  }), [isAuthenticated, accessToken, login, logout, handleRedirectCallback, getUserInfo]);

  if (authError) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-bg text-fg font-ui px-4 select-none">
        <div className="max-w-md w-full bg-raise border border-red/20 rounded-xl p-8 shadow-modal-seat relative overflow-hidden backdrop-blur-md">
          {/* Subtle decorative line at the top */}
          <div className="absolute top-0 left-0 right-0 h-1 bg-red" />

          <div className="flex items-center gap-3 mb-6">
            <div className="w-10 h-10 rounded-lg bg-red/10 border border-red/30 flex items-center justify-center flex-shrink-0 text-red font-mono text-xl font-bold">
              !
            </div>
            <div>
              <h1 className="font-display font-bold text-lg tracking-[0.01em] text-fg">
                Authentication Error
              </h1>
              <p className="text-xs text-ghost">
                ChronoAI fkst-hosted Auth
              </p>
            </div>
          </div>

          <div className="space-y-6">
            <p className="text-sm text-dim leading-relaxed">
              {authError}
            </p>

            <button
              onClick={() => login()}
              className="w-full py-2.5 bg-amber hover:brightness-[1.06] text-amber-ink font-semibold rounded-control transition-all text-sm cursor-pointer"
            >
              Sign-in
            </button>
          </div>
        </div>
      </div>
    );
  }

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuthSession(): AuthSession {
  const context = useContext(AuthContext);
  if (!context) {
    throw new Error('useAuthSession must be used within a NyxIDProvider');
  }
  return context;
}
