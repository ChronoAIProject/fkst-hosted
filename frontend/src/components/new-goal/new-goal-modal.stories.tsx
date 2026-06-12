import type { Meta, StoryObj } from '@storybook/react-vite';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { useEffect, ComponentType } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { NewGoalModal } from './new-goal-modal';

const meta: Meta<typeof NewGoalModal> = {
  title: 'Components/NewGoalModal',
  component: NewGoalModal,
};

export default meta;
type Story = StoryObj<typeof NewGoalModal>;

// --- Fetch Mocking Implementations ---

const mockSuccessFetch = (url: string) => {
  if (url.includes('/api/v1/packages')) {
    if (url.endsWith('/api/v1/packages')) {
      return Promise.resolve(
        new Response(JSON.stringify(['github-devloop', 'github-proxy', 'consensus']), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      );
    }
    const parts = url.split('/');
    const pkgName = decodeURIComponent(parts[parts.length - 1] || '');
    let deps: string[] = [];
    if (pkgName === 'github-devloop') {
      deps = ['github-proxy', 'consensus'];
    }
    return Promise.resolve(
      new Response(JSON.stringify({
        name: pkgName,
        files: [],
        composed_deps: deps,
        created_at: '2026-06-13T00:00:00Z',
        updated_at: '2026-06-13T00:00:00Z',
      }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    );
  }
  return Promise.reject(new Error('Unknown URL'));
};

const mockLoadingFetch = () => {
  return new Promise<Response>(() => {}); // Never resolves
};

const mockUnreachableFetch = () => {
  return Promise.reject(new TypeError('Failed to fetch'));
};

const mockEmptyFetch = (url: string) => {
  if (url.endsWith('/api/v1/packages')) {
    return Promise.resolve(
      new Response(JSON.stringify([]), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    );
  }
  return Promise.reject(new Error('Unknown URL'));
};

// --- Decorator Creator ---

const createQueryDecorator = (fetchMock: (url: string) => Promise<Response>) => {
  return (Story: ComponentType) => {
    useEffect(() => {
      const originalFetch = globalThis.fetch;
      globalThis.fetch = fetchMock as typeof globalThis.fetch;
      return () => {
        globalThis.fetch = originalFetch;
      };
    }, []);

    const queryClient = new QueryClient({
      defaultOptions: {
        queries: {
          retry: false,
          staleTime: 0,
          gcTime: 0,
        },
      },
    });

    return (
      <MemoryRouter>
        <QueryClientProvider client={queryClient}>
          <Story />
        </QueryClientProvider>
      </MemoryRouter>
    );
  };
};

// --- Stories ---

export const OpenWithMockData: Story = {
  args: {
    open: true,
  },
  decorators: [createQueryDecorator(mockSuccessFetch)],
};

export const GraphLoading: Story = {
  args: {
    open: true,
  },
  decorators: [createQueryDecorator(mockLoadingFetch)],
};

export const GraphUnreachable: Story = {
  args: {
    open: true,
  },
  decorators: [createQueryDecorator(mockUnreachableFetch)],
};

export const GraphEmpty: Story = {
  args: {
    open: true,
  },
  decorators: [createQueryDecorator(mockEmptyFetch)],
};

export const ClosedWithTrigger: Story = {
  args: {
    open: undefined,
  },
  decorators: [createQueryDecorator(mockSuccessFetch)],
  render: (args) => (
    <div className="p-8">
      <NewGoalModal
        {...args}
        trigger={
          <button
            type="button"
            className="bg-amber text-amber-ink hover:brightness-[1.06] rounded-control px-4 py-2 font-semibold transition-colors focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2"
          >
            + New goal
          </button>
        }
      />
    </div>
  ),
};
