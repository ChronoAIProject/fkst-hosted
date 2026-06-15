import type { Meta, StoryObj } from '@storybook/react-vite';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { useEffect, ComponentType } from 'react';
import { userEvent, within, expect } from 'storybook/test';

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
    const body = within(canvasElement.ownerDocument.body);

    const titleInput = await body.findByTestId('title-input');
    const descInput = await body.findByTestId('description-textarea');
    const repoInput = await body.findByTestId('repo-input');
    const checkbox = await body.findByTestId('package-checkbox-github-devloop');
    const switchEl = await body.findByTestId('trigger-on-create-switch');

    await userEvent.type(titleInput, 'Database Caching Integration');
    await userEvent.type(descInput, 'Add Redis caching layer to speed up user profile queries by 200%.');
    await userEvent.type(repoInput, 'ChronoAIProject/fkst-substrate');
    await userEvent.click(checkbox);
    await userEvent.click(switchEl);

    expect(titleInput).toHaveValue('Database Caching Integration');
    expect(descInput).toHaveValue('Add Redis caching layer to speed up user profile queries by 200%.');
    expect(repoInput).toHaveValue('ChronoAIProject/fkst-substrate');
    expect(checkbox).toBeChecked();
    expect(switchEl).toHaveAttribute('aria-checked', 'true');
  },
};

// 7. Validation errors triggered by submitting empty fields
export const ValidationErrors: Story = {
  args: {
    open: true,
  },
  decorators: [createQueryDecorator(mockSuccessFetch)],
  play: async ({ canvasElement }) => {
    const body = within(canvasElement.ownerDocument.body);
    const submitButton = await body.findByTestId('submit-button');
    await userEvent.click(submitButton);

    const titleError = await body.findByTestId('title-validation-error');
    const descError = await body.findByTestId('description-validation-error');
    const packageError = await body.findByTestId('package-selection-error');

    expect(titleError).toHaveTextContent('Title is required');
    expect(descError).toHaveTextContent('Description is required');
    expect(packageError).toHaveTextContent('At least one package must be selected');
  },
};

// 8. API conflict error on creation (e.g., duplicate title)
export const ApiConflictError: Story = {
  args: {
    open: true,
  },
  decorators: [createQueryDecorator(mockConflictFetch)],
  play: async ({ canvasElement }) => {
    const body = within(canvasElement.ownerDocument.body);

    const titleInput = await body.findByTestId('title-input');
    const descInput = await body.findByTestId('description-textarea');
    const checkbox = await body.findByTestId('package-checkbox-github-devloop');
    const submitButton = await body.findByTestId('submit-button');

    await userEvent.type(titleInput, 'Duplicate Goal Title');
    await userEvent.type(descInput, 'This is a description for a duplicate goal.');
    await userEvent.click(checkbox);
    await userEvent.click(submitButton);

    const submitError = await body.findByTestId('submit-error');
    expect(submitError).toHaveTextContent('Goal with this title already exists');
  },
};

// 9. API trigger error after successful creation
export const ApiTriggerError: Story = {
  args: {
    open: true,
  },
  decorators: [createQueryDecorator(mockTriggerErrorFetch)],
  play: async ({ canvasElement }) => {
    const body = within(canvasElement.ownerDocument.body);

    const titleInput = await body.findByTestId('title-input');
    const descInput = await body.findByTestId('description-textarea');
    const repoInput = await body.findByTestId('repo-input');
    const checkbox = await body.findByTestId('package-checkbox-github-devloop');
    const switchEl = await body.findByTestId('trigger-on-create-switch');
    const submitButton = await body.findByTestId('submit-button');

    await userEvent.type(titleInput, 'Trigger Failure Goal');
    await userEvent.type(descInput, 'This goal is created successfully but trigger fails.');
    await userEvent.type(repoInput, 'ChronoAIProject/fkst-substrate');
    await userEvent.click(checkbox);
    await userEvent.click(switchEl);
    await userEvent.click(submitButton);

    const submitError = await body.findByTestId('submit-error');
    expect(submitError).toHaveTextContent('GitHub App not installed on ChronoAIProject/fkst-substrate');
  },
};
