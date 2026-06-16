import { useQuery } from '@tanstack/react-query';
import { getHealth, ApiError } from '../api/client';
import { isHealthResponse } from '../api/types';

export type UseHealthResult =
  | {
      healthStatus: 'ok' | 'degraded';
      mongo: 'up' | 'down';
      version: string;
      isSuccess: boolean;
      isError: boolean;
      isLoading: boolean;
      error: unknown;
      refetch: () => Promise<void>;
    }
  | {
      healthStatus: 'unknown';
      mongo: undefined;
      version: undefined;
      isSuccess: boolean;
      isError: boolean;
      isLoading: boolean;
      error: unknown;
      refetch: () => Promise<void>;
    };

/**
 * Hook to query and monitor the backend API and database health status.
 *
 * Choices and Rationales:
 * - staleTime (10000ms): Health status changes when dependencies go down/up. Cache briefly to prevent duplicate queries on multiple mounts.
 * - refetchInterval (30000ms): Periodically poll to track service availability without overloading the backend.
 * - retry (false): Fail fast. If the backend is degraded or unreachable, we want to know immediately.
 */
export function useHealth(): UseHealthResult {
  const query = useQuery({
    queryKey: ['health'],
    queryFn: getHealth,
    staleTime: 10000,
    refetchInterval: 30000,
    retry: false,
  });

  const refetch = () => query.refetch().then(() => {});

  if (query.isSuccess && query.data) {
    return {
      healthStatus: query.data.status,
      mongo: query.data.mongo,
      version: query.data.version,
      isSuccess: query.isSuccess,
      isError: query.isError,
      isLoading: query.isLoading,
      error: query.error,
      refetch,
    };
  }

  if (query.isError && query.error instanceof ApiError) {
    const error = query.error;
    if (error.status === 503 && isHealthResponse(error.body) && error.body.status === 'degraded') {
      return {
        healthStatus: 'degraded',
        mongo: error.body.mongo,
        version: error.body.version,
        isSuccess: query.isSuccess,
        isError: query.isError,
        isLoading: query.isLoading,
        error: query.error,
        refetch,
      };
    }
  }

  return {
    healthStatus: 'unknown',
    mongo: undefined,
    version: undefined,
    isSuccess: query.isSuccess,
    isError: query.isError,
    isLoading: query.isLoading,
    error: query.error,
    refetch,
  };
}
