import { PackageResponse, SessionView, HealthResponse } from '../lib/api/types';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import React from 'react';

// 1. Fixture data objects
export const mockPackages: Record<string, PackageResponse> = {
  'github-proxy': {
    name: 'github-proxy',
    files: [
      { path: 'departments/intake/main.lua', content: '-- intake proxy' }
    ],
    composed_deps: [],
    created_at: '2026-06-13T00:00:00Z',
    updated_at: '2026-06-13T00:00:00Z',
  },
  'consensus': {
    name: 'consensus',
    files: [
      { path: 'departments/consensus/main.lua', content: '-- consensus engine' }
    ],
    composed_deps: [],
    created_at: '2026-06-13T00:00:00Z',
    updated_at: '2026-06-13T00:00:00Z',
  },
  'autochrono': {
    name: 'autochrono',
    files: [
      { path: 'departments/autochrono/main.lua', content: '-- auto scheduler' }
    ],
    composed_deps: ['consensus'],
    created_at: '2026-06-13T00:00:00Z',
    updated_at: '2026-06-13T00:00:00Z',
  },
  'github-devloop': {
    name: 'github-devloop',
    files: [
      { path: 'departments/intake_scan/main.lua', content: '-- scan files' },
      { path: 'departments/intake_judge/main.lua', content: '-- judge files' },
      { path: 'departments/implement/main.lua', content: '-- implement files' },
      { path: 'raisers/github_poll.lua', content: '-- raiser poll' },
      { path: 'raisers/intake_poll.lua', content: '-- raiser intake' },
    ],
    composed_deps: ['github-proxy', 'consensus'],
    created_at: '2026-06-13T00:00:00Z',
    updated_at: '2026-06-13T00:00:00Z',
  },
};

export const mockSessions: Record<string, SessionView> = {
  'session-happy-456': {
    id: 'session-happy-456',
    package_name: 'github-devloop',
    status: 'running',
    pod_id: 'pod-abc',
    fencing_token: 42,
    pid: 1045,
    runtime_dir: '/var/run/fkst',
    error: null,
    created_at: '2026-06-13T02:00:00Z',
    started_at: '2026-06-13T02:00:02Z',
    stopped_at: null,
  },
  'session-healthy-123': {
    id: 'session-healthy-123',
    package_name: 'github-devloop',
    status: 'running',
    pod_id: 'pod-abc',
    fencing_token: 42,
    pid: 1045,
    runtime_dir: '/var/run/fkst',
    error: null,
    created_at: '2026-06-13T02:00:00Z',
    started_at: '2026-06-13T02:00:02Z',
    stopped_at: null,
  },
};

export const mockHealthHealthy: HealthResponse = {
  status: 'ok',
  mongo: 'up',
  version: '1.4.2-build.998',
};

export const mockHealthDegraded: HealthResponse = {
  status: 'degraded',
  mongo: 'down',
  version: '1.4.2-build.998',
};

// 2. Fetch stub creators and decorators
export const mockSuccessFetch = (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
  const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;

  if (url.includes('/api/v1/health')) {
    return Promise.resolve(
      new Response(JSON.stringify(mockHealthHealthy), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    );
  }
  if (url.includes('/api/v1/packages')) {
    if (url.endsWith('/api/v1/packages')) {
      if (init?.method === 'POST') {
        const body = typeof init.body === 'string' ? JSON.parse(init.body) : {};
        return Promise.resolve(
          new Response(JSON.stringify({ name: body.name }), {
            status: 201,
            headers: { 'Content-Type': 'application/json' },
          })
        );
      }
      return Promise.resolve(
        new Response(JSON.stringify(Object.keys(mockPackages)), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      );
    }
    const parts = url.split('/');
    const name = decodeURIComponent(parts[parts.length - 1] || '');
    const pkg = mockPackages[name];
    if (pkg) {
      return Promise.resolve(
        new Response(JSON.stringify(pkg), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      );
    }
    return Promise.resolve(
      new Response(JSON.stringify({ error: 'not_found', message: 'Package not found' }), {
        status: 404,
        headers: { 'Content-Type': 'application/json' },
      })
    );
  }
  if (url.includes('/api/v1/sessions')) {
    if (url.endsWith('/api/v1/sessions')) {
      return Promise.resolve(
        new Response(JSON.stringify({ id: 'session-happy-456', status: 'pending' }), {
          status: 201,
          headers: { 'Content-Type': 'application/json' },
        })
      );
    }
    const parts = url.split('/');
    if (url.endsWith('/stop')) {
      return Promise.resolve(
        new Response(JSON.stringify({ status: 'stopping' }), {
          status: 202,
          headers: { 'Content-Type': 'application/json' },
        })
      );
    }
    const id = decodeURIComponent(parts[parts.length - 1] || '');
    const session = mockSessions[id] || mockSessions['session-happy-456'];
    return Promise.resolve(
      new Response(JSON.stringify(session), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    );
  }
  return Promise.reject(new Error(`Unknown success fetch endpoint: ${url}`));
};

export const mockLoadingFetch = (): Promise<Response> => {
  return new Promise<Response>(() => {}); // Never resolves
};

export const mockUnreachableFetch = (): Promise<Response> => {
  return Promise.reject(new TypeError('Failed to fetch'));
};

export const mockEmptyFetch = (input: RequestInfo | URL): Promise<Response> => {
  const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;

  if (url.includes('/api/v1/health')) {
    return Promise.resolve(
      new Response(JSON.stringify(mockHealthHealthy), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    );
  }
  if (url.includes('/api/v1/packages')) {
    if (url.endsWith('/api/v1/packages')) {
      return Promise.resolve(
        new Response(JSON.stringify([]), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      );
    }
  }
  if (url.includes('/api/v1/sessions')) {
    if (url.endsWith('/api/v1/sessions')) {
      return Promise.resolve(
        new Response(JSON.stringify([]), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      );
    }
  }
  return Promise.reject(new Error(`Unknown empty fetch endpoint: ${url}`));
};

export const mockCreateConflictFetch = (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
  const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;

  if (url.includes('/api/v1/packages') && init?.method === 'POST') {
    return Promise.resolve(
      new Response(
        JSON.stringify({
          error: 'conflict',
          message: 'name already exists (a revision is a new name)',
        }),
        {
          status: 409,
          headers: { 'Content-Type': 'application/json' },
        }
      )
    );
  }
  return mockSuccessFetch(input, init);
};

export const mockDegradedFetch = (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
  const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;

  if (url.includes('/api/v1/health')) {
    return Promise.resolve(
      new Response(
        JSON.stringify(mockHealthDegraded),
        {
          status: 503,
          headers: { 'Content-Type': 'application/json' },
        }
      )
    );
  }
  return mockSuccessFetch(input, init);
};

// Decorator to apply fetch mock synchronously
export const createQueryDecorator = (fetchMock: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>) => {
  return (Story: React.ComponentType) => {
    const originalFetchRef = React.useRef(globalThis.fetch);

    const [queryClient] = React.useState(() => {
      globalThis.fetch = fetchMock as typeof globalThis.fetch;
      return new QueryClient({
        defaultOptions: {
          queries: {
            retry: false,
            staleTime: 0,
            gcTime: 0,
          },
        },
      });
    });

    React.useEffect(() => {
      const originalFetch = originalFetchRef.current;
      globalThis.fetch = fetchMock as typeof globalThis.fetch;
      return () => {
        globalThis.fetch = originalFetch;
      };
    }, []);

    return React.createElement(
      QueryClientProvider,
      { client: queryClient },
      React.createElement(Story)
    );
  };
};
