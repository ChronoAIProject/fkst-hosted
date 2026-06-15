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

  // E2E-only seam (flag-gated; never present in a prod build): lets a test seed a
  // tab-known session so the §1.1 session controls (Apply-changes / Settings Stop)
  // become drivable through the UI against a live backend. `__fkstGetSession` is a
  // read-only reader of the registry so a test can assert the registry advanced to
  // a NEW session id after an Apply-changes restart (stop → poll → create).
  React.useEffect(() => {
    if ((import.meta.env as Record<string, string | undefined>).VITE_E2E === '1') {
      const w = window as unknown as Record<string, unknown>;
      w.__fkstSeedSession = registerSession;
      w.__fkstClearSessions = clearAllSessions;
      w.__fkstGetSession = getSessionId;
    }
  }, [registerSession, clearAllSessions, getSessionId]);

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
