import type { Meta, StoryObj } from '@storybook/react-vite';
import React from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { PackagesView, default as PackagesScreen } from './packages-screen';
import { SessionRegistryProvider, useSessionRegistry } from '../../lib/hooks/session-registry';

const meta: Meta<typeof PackagesView> = {
  title: 'Screens/PackagesScreen',
  component: PackagesView,
};

export default meta;
type Story = StoryObj<typeof PackagesView>;

export const Populated: Story = {
  args: {
    isLoadingList: false,
    listError: null,
    selectedPkgName: 'github-devloop',
    packageNames: ['github-proxy', 'consensus', 'autochrono', 'github-devloop'],
    packagesData: {
      'github-proxy': {
        isLoading: false,
        pkg: {
          name: 'github-proxy',
          files: [],
          composed_deps: [],
          created_at: '2026-06-13T00:00:00Z',
          updated_at: '2026-06-13T00:00:00Z',
        },
      },
      'consensus': {
        isLoading: false,
        pkg: {
          name: 'consensus',
          files: [],
          composed_deps: [],
          created_at: '2026-06-13T00:00:00Z',
          updated_at: '2026-06-13T00:00:00Z',
        },
      },
      'autochrono': {
        isLoading: false,
        pkg: {
          name: 'autochrono',
          files: [],
          composed_deps: ['consensus'],
          created_at: '2026-06-13T00:00:00Z',
          updated_at: '2026-06-13T00:00:00Z',
        },
      },
      'github-devloop': {
        isLoading: false,
        pkg: {
          name: 'github-devloop',
          files: [
            { path: 'departments/intake_scan/main.lua', content: '' },
            { path: 'departments/intake_judge/main.lua', content: '' },
            { path: 'departments/implement/main.lua', content: '' },
            { path: 'raisers/github_poll.lua', content: '' },
            { path: 'raisers/intake_poll.lua', content: '' },
          ],
          composed_deps: ['github-proxy', 'consensus'],
          created_at: '2026-06-13T00:00:00Z',
          updated_at: '2026-06-13T00:00:00Z',
        },
      },
    },
  },
};

export const LoadingList: Story = {
  args: {
    isLoadingList: true,
    listError: null,
    packageNames: [],
  },
};

export const LoadingDetails: Story = {
  args: {
    isLoadingList: false,
    listError: null,
    packageNames: ['github-proxy', 'consensus'],
    packagesData: {
      'github-proxy': {
        isLoading: true,
      },
      'consensus': {
        isLoading: true,
      },
    },
  },
};

export const ListError: Story = {
  args: {
    isLoadingList: false,
    listError: 'package store unreachable — unknown',
    packageNames: [],
  },
};

export const EmptyList: Story = {
  args: {
    isLoadingList: false,
    listError: null,
    packageNames: [],
  },
};

export const UnknownStates: Story = {
  args: {
    isLoadingList: false,
    listError: null,
    packageNames: ['unknown-api-package'],
    packagesData: {
      'unknown-api-package': {
        isLoading: false,
        pkg: {
          name: 'unknown-api-package',
          files: [],
          composed_deps: [],
          created_at: '2026-06-13T00:00:00Z',
          updated_at: '2026-06-13T00:00:00Z',
        },
      },
    },
  },
};

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
    } else if (key.startsWith('packages/')) {
      const name = key.split('/')[1];
      queryClient.setQueryData(['packages', name], value);
    } else {
      queryClient.setQueryData([key], value);
    }
  });

  return queryClient;
}

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

export const PropsKnownSessionHappyPath: Story = {
  args: {
    ...Populated.args,
    selectedPkgName: 'github-devloop',
    isApplyDisabled: false,
    sessionStatusCopy: null,
  },
};

export const PropsNoKnownSessionGap: Story = {
  args: {
    ...Populated.args,
    selectedPkgName: 'github-devloop',
    isApplyDisabled: true,
    sessionStatusCopy: (
      <span className="text-gold font-mono text-[11px] leading-tight">
        github-devloop · current session id not exposed by the v1 API — this console can only manage sessions it started this tab.
      </span>
    ),
  },
};

export const PropsPollingStatus: Story = {
  args: {
    ...Populated.args,
    selectedPkgName: 'github-devloop',
    isApplyDisabled: true,
    sessionStatusCopy: <span className="text-ghost">github-devloop · waiting for stopped →</span>,
  },
};

export const PropsConflictError: Story = {
  args: {
    ...Populated.args,
    selectedPkgName: 'github-devloop',
    isApplyDisabled: true,
    sessionStatusCopy: (
      <span className="text-red font-mono text-[11px] leading-tight">
        github-devloop · session stopped, but restart failed — package already has a live session; its id isn't exposed by the v1 API, so it can't be stopped from here.
      </span>
    ),
  },
};

export const ContainerKnownSession: Story = {
  render: () => {
    const queryClient = createMockQueryClient({
      packages: ['github-proxy', 'consensus', 'autochrono', 'github-devloop'],
      'packages/github-proxy': Populated.args?.packagesData?.['github-proxy']?.pkg,
      'packages/consensus': Populated.args?.packagesData?.['consensus']?.pkg,
      'packages/autochrono': Populated.args?.packagesData?.['autochrono']?.pkg,
      'packages/github-devloop': Populated.args?.packagesData?.['github-devloop']?.pkg,
      'sessions/session-happy-456': {
        id: 'session-happy-456',
        package_name: 'github-devloop',
        status: 'running',
        created_at: '2026-06-13T02:00:00Z',
        started_at: '2026-06-13T02:00:02Z',
        stopped_at: null,
      },
    });

    return (
      <SessionRegistryProvider>
        <QueryClientProvider client={queryClient}>
          <RegistryInit packageName="github-devloop" sessionId="session-happy-456">
            <PackagesScreen />
          </RegistryInit>
        </QueryClientProvider>
      </SessionRegistryProvider>
    );
  },
};

export const ContainerNoKnownSession: Story = {
  render: () => {
    const queryClient = createMockQueryClient({
      packages: ['github-proxy', 'consensus', 'autochrono', 'github-devloop'],
      'packages/github-proxy': Populated.args?.packagesData?.['github-proxy']?.pkg,
      'packages/consensus': Populated.args?.packagesData?.['consensus']?.pkg,
      'packages/autochrono': Populated.args?.packagesData?.['autochrono']?.pkg,
      'packages/github-devloop': Populated.args?.packagesData?.['github-devloop']?.pkg,
    });

    return (
      <SessionRegistryProvider>
        <QueryClientProvider client={queryClient}>
          <PackagesScreen />
        </QueryClientProvider>
      </SessionRegistryProvider>
    );
  },
};
