import type { Meta, StoryObj } from '@storybook/react-vite';
import React from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { BrowserRouter } from 'react-router-dom';
import { SettingsScreen } from './settings-screen';
import { SessionRegistryProvider, useSessionRegistry } from '@/lib/hooks/session-registry';

const meta: Meta<typeof SettingsScreen> = {
  title: 'Screens/SettingsScreen',
  component: SettingsScreen,
  decorators: [
    (Story) => (
      <BrowserRouter>
        <Story />
      </BrowserRouter>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof SettingsScreen>;

function createMockQueryClient(queries: Record<string, unknown>) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: Infinity,
        gcTime: Infinity,
        retry: false,
      },
    },
  });

  Object.entries(queries).forEach(([key, value]) => {
    if (key.startsWith('sessions/')) {
      const sessionId = key.split('/')[1];
      queryClient.setQueryData(['sessions', sessionId], value);
    } else {
      queryClient.setQueryData([key], value);
    }
  });

  return queryClient;
}

// Helper component to initialize the session registry in Storybook
function RegistryInit({
  packageName,
  sessionId,
  children,
}: {
  packageName: string;
  sessionId: string;
  children: React.ReactNode;
}) {
  const { registerSession } = useSessionRegistry();
  React.useEffect(() => {
    registerSession(packageName, sessionId);
  }, [packageName, sessionId, registerSession]);
  return <>{children}</>;
}

export const EngineHealthyWithKnownSession: Story = {
  render: () => {
    const queryClient = createMockQueryClient({
      health: {
        status: 'ok',
        mongo: 'up',
        version: '1.4.2-build.998',
      },
      packages: ['fkst-substrate'],
      'sessions/session-healthy-123': {
        id: 'session-healthy-123',
        package_name: 'fkst-substrate',
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
    });

    return (
      <SessionRegistryProvider>
        <QueryClientProvider client={queryClient}>
          <RegistryInit packageName="fkst-substrate" sessionId="session-healthy-123">
            <SettingsScreen />
          </RegistryInit>
        </QueryClientProvider>
      </SessionRegistryProvider>
    );
  },
};

export const EngineUnknown: Story = {
  render: () => {
    const queryClient = createMockQueryClient({
      health: null,
      packages: [],
    });

    return (
      <SessionRegistryProvider>
        <QueryClientProvider client={queryClient}>
          <SettingsScreen />
        </QueryClientProvider>
      </SessionRegistryProvider>
    );
  },
};

export const NoKnownSession: Story = {
  render: () => {
    const queryClient = createMockQueryClient({
      health: {
        status: 'ok',
        mongo: 'up',
        version: '1.4.2-build.998',
      },
      packages: ['fkst-substrate'],
    });

    return (
      <SessionRegistryProvider>
        <QueryClientProvider client={queryClient}>
          <SettingsScreen />
        </QueryClientProvider>
      </SessionRegistryProvider>
    );
  },
};

export const Degraded: Story = {
  render: () => {
    const queryClient = createMockQueryClient({
      health: {
        status: 'degraded',
        mongo: 'down',
        version: '1.4.2-build.998',
      },
      packages: ['fkst-substrate'],
    });

    return (
      <SessionRegistryProvider>
        <QueryClientProvider client={queryClient}>
          <SettingsScreen />
        </QueryClientProvider>
      </SessionRegistryProvider>
    );
  },
};
