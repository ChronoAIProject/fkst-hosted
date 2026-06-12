import React, { createContext, useContext, useState, useCallback, useMemo } from 'react';

export interface SessionRegistryContextType {
  registerSession: (packageName: string, sessionId: string) => void;
  getSessionId: (packageName: string) => string | undefined;
  clearSession: (packageName: string) => void;
  clearAllSessions: () => void;
}

const SessionRegistryContext = createContext<SessionRegistryContextType | undefined>(undefined);

export const SessionRegistryProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [registry, setRegistry] = useState<Map<string, string>>(new Map());

  const registerSession = useCallback((packageName: string, sessionId: string) => {
    setRegistry((prev) => {
      const next = new Map(prev);
      next.set(packageName, sessionId);
      return next;
    });
  }, []);

  const getSessionId = useCallback((packageName: string) => {
    return registry.get(packageName);
  }, [registry]);

  const clearSession = useCallback((packageName: string) => {
    setRegistry((prev) => {
      const next = new Map(prev);
      next.delete(packageName);
      return next;
    });
  }, []);

  const clearAllSessions = useCallback(() => {
    setRegistry(new Map());
  }, []);

  const contextValue = useMemo(() => ({
    registerSession,
    getSessionId,
    clearSession,
    clearAllSessions,
  }), [registerSession, getSessionId, clearSession, clearAllSessions]);

  return (
    <SessionRegistryContext.Provider value={contextValue}>
      {children}
    </SessionRegistryContext.Provider>
  );
};

export function useSessionRegistry(): SessionRegistryContextType {
  const context = useContext(SessionRegistryContext);
  if (!context) {
    throw new Error('useSessionRegistry must be used within a SessionRegistryProvider');
  }
  return context;
}
