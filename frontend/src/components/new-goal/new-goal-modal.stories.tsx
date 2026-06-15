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
      new Response(
        JSON.stringify({
          name: pkgName,
          files: [],
          composed_deps: deps,
          created_at: '2026-06-13T00:00:00Z',
          updated_at: '2026-06-13T00:00:00Z',
        }),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }
      )
    );
  }
  return Promise.reject(new Error('Unknown URL: ' + url));
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
  return Promise.reject(new Error('Unknown URL: ' + url));
};

// --- Custom Failure Mocks ---

const mockConflictFetch = (url: string, init?: RequestInit) => {
  if (url.includes('/api/v1/packages')) {
    return mockSuccessFetch(url);
  }
  if (url.endsWith('/api/v1/goals') && init?.method === 'POST') {
    return Promise.resolve(
      new Response(
        JSON.stringify({
          error: 'Conflict',
          message: 'Goal with this title already exists',
        }),
        {
          status: 409,
          headers: { 'Content-Type': 'application/json' },
        }
      )
    );
  }
  return Promise.reject(new Error('Unknown URL: ' + url));
};

const mockTriggerErrorFetch = (url: string, init?: RequestInit) => {
  if (url.includes('/api/v1/packages')) {
    return mockSuccessFetch(url);
  }
  if (url.endsWith('/api/v1/goals') && init?.method === 'POST') {
    return Promise.resolve(
      new Response(
        JSON.stringify({
          id: 'mock-goal-123',
          title: 'Trigger Failure Goal',
          description: 'New Goal Description',
          package_names: ['github-devloop'],
          repo: { owner: 'ChronoAIProject', name: 'fkst-substrate' },
          status: 'not_started',
          owner_user_id: 'user-1',
          org_id: null,
          active_session_id: null,
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        }),
        {
          status: 201,
          headers: { 'Content-Type': 'application/json' },
        }
      )
    );
  }
  if (url.includes('/trigger') && init?.method === 'POST') {
    return Promise.resolve(
      new Response(
        JSON.stringify({
          error: 'Unprocessable',
          message: 'GitHub App not installed on ChronoAIProject/fkst-substrate',
        }),
        {
          status: 422,
          headers: { 'Content-Type': 'application/json' },
        }
      )
    );
  }
  return Promise.reject(new Error('Unknown URL: ' + url));
};

// --- Decorator Creator ---

const createQueryDecorator = (fetchMock: (url: string, init?: RequestInit) => Promise<Response>) => {
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

// 6. Form pre-filled (created state view)
export const FormFilled: Story = {
  args: {
    open: true,
  },
  decorators: [createQueryDecorator(mockSuccessFetch)],
  play: async ({ canvasElement }) => {
    // Fill title
    const titleInput = canvasElement.querySelector('[data-testid="title-input"]') as HTMLInputElement;
    if (titleInput) {
      titleInput.value = 'Database Caching Integration';
      titleInput.dispatchEvent(new Event('input', { bubbles: true }));
    }
    // Fill description
    const descInput = canvasElement.querySelector('[data-testid="description-textarea"]') as HTMLTextAreaElement;
    if (descInput) {
      descInput.value = 'Add Redis caching layer to speed up user profile queries by 200%.';
      descInput.dispatchEvent(new Event('input', { bubbles: true }));
    }
    // Fill repository
    const repoInput = canvasElement.querySelector('[data-testid="repo-input"]') as HTMLInputElement;
    if (repoInput) {
      repoInput.value = 'ChronoAIProject/fkst-substrate';
      repoInput.dispatchEvent(new Event('input', { bubbles: true }));
    }
    // Toggle first package
    const checkbox = canvasElement.querySelector('[data-testid="package-checkbox-github-devloop"]') as HTMLInputElement;
    if (checkbox) {
      checkbox.click();
    }
    // Enable triggerOnCreate
    const switchEl = canvasElement.querySelector('[data-testid="trigger-on-create-switch"]') as HTMLButtonElement;
    if (switchEl) {
      switchEl.click();
    }
  },
};

// 7. Validation errors triggered by submitting empty fields
export const ValidationErrors: Story = {
  args: {
    open: true,
  },
  decorators: [createQueryDecorator(mockSuccessFetch)],
  play: async ({ canvasElement }) => {
    const submitButton = canvasElement.querySelector('[data-testid="submit-button"]') as HTMLButtonElement;
    if (submitButton) {
      submitButton.click();
    }
  },
};

// 8. API conflict error on creation (e.g., duplicate title)
export const ApiConflictError: Story = {
  args: {
    open: true,
  },
  decorators: [createQueryDecorator(mockConflictFetch)],
  play: async ({ canvasElement }) => {
    const titleInput = canvasElement.querySelector('[data-testid="title-input"]') as HTMLInputElement;
    if (titleInput) {
      titleInput.value = 'Duplicate Goal Title';
      titleInput.dispatchEvent(new Event('input', { bubbles: true }));
    }
    const descInput = canvasElement.querySelector('[data-testid="description-textarea"]') as HTMLTextAreaElement;
    if (descInput) {
      descInput.value = 'This is a description for a duplicate goal.';
      descInput.dispatchEvent(new Event('input', { bubbles: true }));
    }
    const checkbox = canvasElement.querySelector('[data-testid="package-checkbox-github-devloop"]') as HTMLInputElement;
    if (checkbox) {
      checkbox.click();
    }
    const submitButton = canvasElement.querySelector('[data-testid="submit-button"]') as HTMLButtonElement;
    if (submitButton) {
      submitButton.click();
    }
  },
};

// 9. API trigger error after successful creation
export const ApiTriggerError: Story = {
  args: {
    open: true,
  },
  decorators: [createQueryDecorator(mockTriggerErrorFetch)],
  play: async ({ canvasElement }) => {
    const titleInput = canvasElement.querySelector('[data-testid="title-input"]') as HTMLInputElement;
    if (titleInput) {
      titleInput.value = 'Trigger Failure Goal';
      titleInput.dispatchEvent(new Event('input', { bubbles: true }));
    }
    const descInput = canvasElement.querySelector('[data-testid="description-textarea"]') as HTMLTextAreaElement;
    if (descInput) {
      descInput.value = 'This goal is created successfully but trigger fails.';
      descInput.dispatchEvent(new Event('input', { bubbles: true }));
    }
    const repoInput = canvasElement.querySelector('[data-testid="repo-input"]') as HTMLInputElement;
    if (repoInput) {
      repoInput.value = 'ChronoAIProject/fkst-substrate';
      repoInput.dispatchEvent(new Event('input', { bubbles: true }));
    }
    const checkbox = canvasElement.querySelector('[data-testid="package-checkbox-github-devloop"]') as HTMLInputElement;
    if (checkbox) {
      checkbox.click();
    }
    const switchEl = canvasElement.querySelector('[data-testid="trigger-on-create-switch"]') as HTMLButtonElement;
    if (switchEl) {
      switchEl.click();
    }
    const submitButton = canvasElement.querySelector('[data-testid="submit-button"]') as HTMLButtonElement;
    if (submitButton) {
      submitButton.click();
    }
  },
};
