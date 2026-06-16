import { useQuery } from '@tanstack/react-query';
import { getGitHubAccounts } from '../api/client';

/**
 * Hook to retrieve the list of linked GitHub accounts.
 *
 * Choices and Rationales:
 * - staleTime (30000ms): Connected accounts change very infrequently. Cache for 30s to reduce unnecessary API load.
 * - retry (false): Fail fast. If the credential proxy is down (503) or another error occurs, surface it immediately.
 */
export function useGitHubAccounts(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: ['github-accounts'],
    queryFn: getGitHubAccounts,
    staleTime: 30000,
    retry: false,
    enabled: options?.enabled,
  });
}
