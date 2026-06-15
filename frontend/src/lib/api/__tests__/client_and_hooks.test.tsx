import React from 'react';
import { describe, it, expect, beforeEach, afterEach, vi, beforeAll, afterAll } from 'vitest';
import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import { renderHook, waitFor, act } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

import { countOrUnknown, isTerminal, STAGE_BY_STATE, GoalState, isSessionTerminal } from '../truth';
import { SessionRegistryProvider, useSessionRegistry } from '../../hooks/session-registry';
import { useHealth } from '../../hooks/useHealth';
import { useSession, useCreateSession } from '../../hooks/useSessions';
import { useGitHubAccounts } from '../../hooks/useGitHubAccounts';
import { ApiError } from '../client';
import { isApiErrorBody } from '../types';

// MSW test server setup
const healthMockHandler = http.get('*/api/v1/health', () => {
  return HttpResponse.json({
    status: 'ok',
    mongo: 'up',
    version: '1.0.0',
  });
});

let sessionFetchCount = 0;
let currentSessionStatus = 'running';
const sessionGetMockHandler = http.get('*/api/v1/sessions/:id', () => {
  sessionFetchCount++;
  return HttpResponse.json({
    id: 'session-123',
    package_name: 'test-package',
    status: currentSessionStatus,
    pod_id: 'pod-1',
    fencing_token: 12345,
    pid: 999,
    runtime_dir: '/tmp',
    error: null,
    created_at: '2026-06-13T00:00:00Z',
    started_at: '2026-06-13T00:00:01Z',
    stopped_at: null,
  });
});

const sessionPostMockHandler = http.post('*/api/v1/sessions', () => {
  return HttpResponse.json({
    id: 'session-123',
    status: 'pending',
  }, { status: 201 });
});

const server = setupServer();

beforeAll(() => server.listen({ onUnhandledRequest: 'bypass' }));
afterEach(() => {
  server.resetHandlers();
  sessionFetchCount = 0;
  currentSessionStatus = 'running';
});
afterAll(() => server.close());

// Wrapper combining Registry and React Query Client
function createTestWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        refetchOnWindowFocus: false,
        refetchOnReconnect: false,
        refetchOnMount: false,
      },
    },
  });
  return ({ children }: { children: React.ReactNode }) => (
    <SessionRegistryProvider>
      <QueryClientProvider client={queryClient}>
        {children}
      </QueryClientProvider>
    </SessionRegistryProvider>
  );
}

describe('W1.E Backend Client & Hooks Tests', () => {
  
  describe('Truth Helpers', () => {
    it('countOrUnknown matrix', () => {
      expect(countOrUnknown(undefined, false)).toBe('unknown');
      expect(countOrUnknown(10, false)).toBe('unknown');
      expect(countOrUnknown(undefined, true)).toBe('unknown');
      expect(countOrUnknown(10, true)).toBe(10);
      expect(countOrUnknown(0, true)).toBe(0);
    });

    it('STAGE_BY_STATE covers all 12 states', () => {
      const states: GoalState[] = [
        'thinking', 'ready', 'implementing', 'pr-open', 'reviewing',
        'merge-ready', 'merging', 'fixing', 'review-meta',
        'impl-failed', 'blocked', 'merged'
      ];
      
      expect(states.length).toBe(12);
      states.forEach(state => {
        expect(STAGE_BY_STATE[state]).toBeDefined();
      });

      expect(STAGE_BY_STATE['thinking']).toBe('Design');
      expect(STAGE_BY_STATE['ready']).toBe('Design');
      expect(STAGE_BY_STATE['implementing']).toBe('Build');
      expect(STAGE_BY_STATE['pr-open']).toBe('Build');
      expect(STAGE_BY_STATE['reviewing']).toBe('Review');
      expect(STAGE_BY_STATE['fixing']).toBe('Review');
      expect(STAGE_BY_STATE['review-meta']).toBe('Review');
      expect(STAGE_BY_STATE['merge-ready']).toBe('Ship');
      expect(STAGE_BY_STATE['merging']).toBe('Ship');
      expect(STAGE_BY_STATE['impl-failed']).toBe('Blocked');
      expect(STAGE_BY_STATE['blocked']).toBe('Blocked');
      expect(STAGE_BY_STATE['merged']).toBe('Merged');
    });

    it('isTerminal correctness', () => {
      expect(isTerminal('impl-failed')).toBe(true);
      expect(isTerminal('blocked')).toBe(true);
      expect(isTerminal('merged')).toBe(true);

      expect(isTerminal('thinking')).toBe(false);
      expect(isTerminal('ready')).toBe(false);
      expect(isTerminal('implementing')).toBe(false);
      expect(isTerminal('pr-open')).toBe(false);
      expect(isTerminal('reviewing')).toBe(false);
      expect(isTerminal('merge-ready')).toBe(false);
      expect(isTerminal('merging')).toBe(false);
      expect(isTerminal('fixing')).toBe(false);
      expect(isTerminal('review-meta')).toBe(false);
    });

    it('isSessionTerminal correctness', () => {
      expect(isSessionTerminal('stopped')).toBe(true);
      expect(isSessionTerminal('failed')).toBe(true);
      
      expect(isSessionTerminal('pending')).toBe(false);
      expect(isSessionTerminal('validating')).toBe(false);
      expect(isSessionTerminal('running')).toBe(false);
      expect(isSessionTerminal('stopping')).toBe(false);
    });
  });

  describe('Session Registry Provider & Hook', () => {
    it('supports register, lookup, and clear operations', () => {
      const { result } = renderHook(() => useSessionRegistry(), {
        wrapper: ({ children }) => <SessionRegistryProvider>{children}</SessionRegistryProvider>,
      });

      expect(result.current.getSessionId('package-a')).toBeUndefined();

      act(() => {
        result.current.registerSession('package-a', 'session-1');
      });
      expect(result.current.getSessionId('package-a')).toBe('session-1');

      act(() => {
        result.current.registerSession('package-b', 'session-2');
      });
      expect(result.current.getSessionId('package-b')).toBe('session-2');

      act(() => {
        result.current.clearSession('package-a');
      });
      expect(result.current.getSessionId('package-a')).toBeUndefined();
      expect(result.current.getSessionId('package-b')).toBe('session-2');

      act(() => {
        result.current.clearAllSessions();
      });
      expect(result.current.getSessionId('package-b')).toBeUndefined();
    });
  });

  describe('useHealth Hook', () => {
    it('surfaces ok for 200 response with ok status', async () => {
      server.use(healthMockHandler);
      
      const { result } = renderHook(() => useHealth(), { wrapper: createTestWrapper() });
      
      await waitFor(() => expect(result.current.isSuccess).toBe(true));
      expect(result.current.healthStatus).toBe('ok');
      expect(result.current.mongo).toBe('up');
      expect(result.current.version).toBe('1.0.0');
    });

    it('surfaces degraded for 503 response with degraded status', async () => {
      server.use(
        http.get('*/api/v1/health', () => {
          return HttpResponse.json({
            status: 'degraded',
            mongo: 'down',
            version: '1.0.0',
          }, { status: 503 });
        })
      );

      const { result } = renderHook(() => useHealth(), { wrapper: createTestWrapper() });
      
      await waitFor(() => expect(result.current.healthStatus).toBe('degraded'));
      expect(result.current.mongo).toBe('down');
      expect(result.current.version).toBe('1.0.0');
    });

    it('surfaces unknown for network failure', async () => {
      server.use(
        http.get('*/api/v1/health', () => {
          return HttpResponse.error();
        })
      );

      const { result } = renderHook(() => useHealth(), { wrapper: createTestWrapper() });
      
      await waitFor(() => expect(result.current.healthStatus).toBe('unknown'));
      expect(result.current.mongo).toBeUndefined();
      expect(result.current.version).toBeUndefined();
    });
  });

  describe('useGitHubAccounts Hook', () => {
    it('surfaces linked accounts array for 200 response', async () => {
      const mockAccounts = [
        { connection_id: 'c1', login: 'octocat', primary: true },
        { connection_id: 'c2', login: 'octocat-dev', primary: false },
      ];
      server.use(
        http.get('*/api/v1/github/accounts', () => {
          return HttpResponse.json(mockAccounts);
        })
      );

      const { result } = renderHook(() => useGitHubAccounts(), { wrapper: createTestWrapper() });

      await waitFor(() => expect(result.current.isSuccess).toBe(true));
      expect(result.current.data).toEqual(mockAccounts);
    });

    it('surfaces empty array for 200 response with zero accounts', async () => {
      server.use(
        http.get('*/api/v1/github/accounts', () => {
          return HttpResponse.json([]);
        })
      );

      const { result } = renderHook(() => useGitHubAccounts(), { wrapper: createTestWrapper() });

      await waitFor(() => expect(result.current.isSuccess).toBe(true));
      expect(result.current.data).toEqual([]);
    });

    it('surfaces error for 503 response (credential proxy down)', async () => {
      server.use(
        http.get('*/api/v1/github/accounts', () => {
          return new HttpResponse(null, { status: 503 });
        })
      );

      const { result } = renderHook(() => useGitHubAccounts(), { wrapper: createTestWrapper() });

      await waitFor(() => expect(result.current.isError).toBe(true));
      expect(result.current.error).toBeDefined();
    });
  });

  describe('useCreateSession and Registry Integration', () => {
    it('registers sessionId on 201 success', async () => {
      server.use(sessionPostMockHandler);

      const { result } = renderHook(() => {
        const createSessionMut = useCreateSession();
        const registry = useSessionRegistry();
        return { createSessionMut, registry };
      }, { wrapper: createTestWrapper() });

      act(() => {
        result.current.createSessionMut.mutate('test-pkg');
      });

      await waitFor(() => expect(result.current.createSessionMut.isSuccess).toBe(true));
      expect(result.current.registry.getSessionId('test-pkg')).toBe('session-123');
    });

    it('surfaces typed conflict on 409 and does not pollute registry', async () => {
      server.use(
        http.post('*/api/v1/sessions', () => {
          return HttpResponse.json({
            error: 'conflict',
            message: 'package already has a live session; id not exposed by v1 API',
          }, { status: 409 });
        })
      );

      const { result } = renderHook(() => {
        const createSessionMut = useCreateSession();
        const registry = useSessionRegistry();
        return { createSessionMut, registry };
      }, { wrapper: createTestWrapper() });

      act(() => {
        result.current.createSessionMut.mutate('test-pkg-conflict');
      });

      await waitFor(() => expect(result.current.createSessionMut.isError).toBe(true));
      
      const error = result.current.createSessionMut.error;
      expect(error).toBeInstanceOf(ApiError);
      
      const apiErr = error as ApiError;
      expect(apiErr.status).toBe(409);
      if (isApiErrorBody(apiErr.body)) {
        expect(apiErr.body.error).toBe('conflict');
      } else {
        throw new Error('Expected body to be ApiErrorBody');
      }

      // Registry must not be polluted with the failed request
      expect(result.current.registry.getSessionId('test-pkg-conflict')).toBeUndefined();
    });
  });

  describe('useSession Hook Polling', () => {
    beforeEach(() => {
      vi.useFakeTimers();
    });

    afterEach(() => {
      vi.useRealTimers();
    });

    it('polls while status is non-terminal, and stops polling on stopped/failed', async () => {
      server.use(sessionGetMockHandler);

      const { result } = renderHook(() => useSession('session-123'), {
        wrapper: createTestWrapper(),
      });

      // Let the initial request start and resolve
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      // Flush microtasks and pending 0ms timers
      for (let i = 0; i < 10; i++) {
        await act(async () => {
          await Promise.resolve();
          await vi.advanceTimersByTimeAsync(0);
        });
      }

      expect(result.current.data?.status).toBe('running');
      expect(sessionFetchCount).toBe(1);

      // Advance 2s to trigger next poll
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      // Advance by another 50ms and flush microtasks to let the fetch request resolve
      for (let i = 0; i < 10; i++) {
        await act(async () => {
          await Promise.resolve();
          await vi.advanceTimersByTimeAsync(50);
        });
      }

      expect(sessionFetchCount).toBe(2);

      // Change status to terminal 'stopped'
      currentSessionStatus = 'stopped';

      // Advance 2s to trigger next poll
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      // Advance by another 50ms and flush microtasks to let the fetch request resolve
      for (let i = 0; i < 10; i++) {
        await act(async () => {
          await Promise.resolve();
          await vi.advanceTimersByTimeAsync(50);
        });
      }

      expect(result.current.data?.status).toBe('stopped');
      expect(sessionFetchCount).toBe(3);

      // Advance 10s: should NOT fetch anymore
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10000);
      });
      // Flush microtasks and timers
      for (let i = 0; i < 10; i++) {
        await act(async () => {
          await Promise.resolve();
          await vi.advanceTimersByTimeAsync(50);
        });
      }

      expect(sessionFetchCount).toBe(3);
    });

    it('stops polling on unmount', async () => {
      server.use(sessionGetMockHandler);

      const { result, unmount } = renderHook(() => useSession('session-123'), {
        wrapper: createTestWrapper(),
      });

      // Let initial fetch resolve
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      for (let i = 0; i < 10; i++) {
        await act(async () => {
          await Promise.resolve();
          await vi.advanceTimersByTimeAsync(0);
        });
      }

      expect(result.current.data?.status).toBe('running');
      expect(sessionFetchCount).toBe(1);

      // Unmount the component
      unmount();

      // Advance timers by 10s: should NOT fetch anymore
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10000);
      });

      expect(sessionFetchCount).toBe(1); // remains 1, no more fetches occurred
    });
  });
});
