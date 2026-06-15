import React, { createContext, useContext, useState, useEffect, useMemo, useCallback } from 'react';
import { NyxIDClient, LoginRedirectOptions, OAuthUserInfo, NyxIDTokenSet } from './nyxid-client';
import { authRequired } from './token';

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
    await client.loginWithRedirect(options);
  }, [client]);

  const logout = useCallback(() => {
    client.clearSession();
    setAccessToken(null);
  }, [client]);

  const handleRedirectCallback = useCallback(async (url?: string) => {
    const tokens = await client.handleRedirectCallback(url);
    setAccessToken(tokens.accessToken);
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

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuthSession(): AuthSession {
  const context = useContext(AuthContext);
  if (!context) {
    throw new Error('useAuthSession must be used within a NyxIDProvider');
  }
  return context;
}
