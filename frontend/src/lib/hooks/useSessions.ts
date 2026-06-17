import { useQuery, useMutation } from '@tanstack/react-query';
import { createSession, getSession, stopSession } from '../api/client';
import { useSessionRegistry } from './session-registry';
import { isSessionTerminal } from '../api/truth';

/**
 * Hook to create a new session for a package.
 * On 201 success, registers the session ID in the in-memory tab registry.
 * On 409 conflict, surfaces the typed error.
 */
export function useCreateSession() {
  const { registerSession } = useSessionRegistry();

  return useMutation({
    mutationFn: (packageName: string) => createSession(packageName),
    onSuccess: (data, packageName) => {
      if (data && data.id) {
        registerSession(packageName, data.id);
      }
    },
  });
}

/**
 * Hook to fetch the status of an active session.
 *
 * Choices and Rationales:
 * - staleTime (0ms): Sessions are rapidly evolving processes. Caching is disabled to ensure fresh data.
 * - retry (false): Fail fast. Do not block UI or mask transient errors with query client retries.
 * - enabled: Only executes if the session ID is defined.
 * - refetchInterval (2000ms): Polls every 2 seconds while the session is active.
 *   Stops polling (returns false) if the session reaches terminal status.
 */
export function useSession(id: string | undefined) {
  return useQuery({
    queryKey: ['sessions', id],
    queryFn: () => getSession(id!),
    staleTime: 0,
    retry: false,
    enabled: !!id,
    refetchInterval: (query) => {
      const status = query.state.data?.status;
      if (status && isSessionTerminal(status)) {
        return false;
      }
      return 2000;
    },
  });
}

/**
 * Hook to stop an active session.
 *
 * NOTE: The backend returns a 202 Accepted, which is an ACK only.
 * The truth of whether the session is stopped must be determined by
 * subsequent GET polling (via useSession).
 */
export function useStopSession() {
  return useMutation({
    mutationFn: (id: string) => stopSession(id),
  });
}
