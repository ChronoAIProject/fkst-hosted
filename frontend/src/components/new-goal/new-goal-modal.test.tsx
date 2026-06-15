import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { MemoryRouter } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { NewGoalModal } from './new-goal-modal';

class ResizeObserverMock {
  observe() {}
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver = ResizeObserverMock;

const mockSuccessFetch = (url: string, options?: RequestInit) => {
  const method = options?.method?.toUpperCase() || 'GET';

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

  if (url.endsWith('/api/v1/goals') && method === 'POST') {
    const body = JSON.parse(options?.body as string || '{}');
    return Promise.resolve(
      new Response(JSON.stringify({
        id: 'mock-goal-123',
        title: body.title,
        description: body.description,
        package_names: body.package_names,
        repo: body.repo || null,
        status: 'not_started',
        owner_user_id: 'user-123',
        org_id: null,
        active_session_id: null,
        created_at: '2026-06-15T00:00:00Z',
        updated_at: '2026-06-15T00:00:00Z',
      }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    );
  }

  if (url.includes('/api/v1/goals/mock-goal-123/trigger') && method === 'POST') {
    return Promise.resolve(
      new Response(JSON.stringify({
        goal_id: 'mock-goal-123',
        session_id: 'mock-session-123',
        goal_status: 'triggered',
        session_status: 'pending',
      }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    );
  }

  return Promise.reject(new Error('Unknown URL: ' + url));
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

describe('NewGoalModal Unit Tests', () => {
  let originalFetch: typeof globalThis.fetch;

  beforeEach(() => {
    originalFetch = globalThis.fetch;
    if (typeof window !== 'undefined') {
      if (!window.Element.prototype.hasPointerCapture) {
        window.Element.prototype.hasPointerCapture = () => false;
      }
      if (!window.Element.prototype.setPointerCapture) {
        window.Element.prototype.setPointerCapture = () => {};
      }
      if (!window.Element.prototype.releasePointerCapture) {
        window.Element.prototype.releasePointerCapture = () => {};
      }
      window.HTMLElement.prototype.scrollIntoView = vi.fn();
    }
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  const createQueryClient = () => {
    return new QueryClient({
      defaultOptions: {
        queries: {
          retry: false,
          staleTime: 0,
          gcTime: 0,
        },
      },
    });
  };

  it('renders fields and handles validation error when submitting empty fields', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(mockSuccessFetch as typeof globalThis.fetch);

    render(
      <MemoryRouter>
        <QueryClientProvider client={createQueryClient()}>
          <NewGoalModal open={true} />
        </QueryClientProvider>
      </MemoryRouter>
    );

    // Verify Repository input, title, description fields are present
    const repoInput = screen.getByTestId('repo-input');
    const titleInput = screen.getByTestId('title-input');
    const descTextarea = screen.getByTestId('description-textarea');

    expect(repoInput).toBeInTheDocument();
    expect(titleInput).toBeInTheDocument();
    expect(descTextarea).toBeInTheDocument();

    const submitBtn = screen.getByTestId('submit-button');
    expect(submitBtn).toBeInTheDocument();
    expect(submitBtn).not.toBeDisabled();

    // Verify notes are rendered honestly
    expect(screen.getByTestId('submit-note')).toHaveTextContent('requires NyxID sign-in');
    expect(screen.getByTestId('engine-pickup-footnote')).toHaveTextContent('the engine\'s next ~5-min poll picks it up → development cycle');

    // Click submit immediately
    const user = userEvent.setup();
    await user.click(submitBtn);

    // Verify validation errors show up
    expect(await screen.findByTestId('title-validation-error')).toHaveTextContent('Title is required');
    expect(await screen.findByTestId('description-validation-error')).toHaveTextContent('Description is required');
    expect(await screen.findByTestId('package-selection-error')).toHaveTextContent('At least one package must be selected');
  });

  it('toggles package selection and submits successfully without repository', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockImplementation(mockSuccessFetch as typeof globalThis.fetch);
    const onOpenChange = vi.fn();

    render(
      <MemoryRouter>
        <QueryClientProvider client={createQueryClient()}>
          <NewGoalModal open={true} onOpenChange={onOpenChange} />
        </QueryClientProvider>
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByTestId('package-graph-list')).toBeInTheDocument();
    });

    const user = userEvent.setup();

    // Fill title and description
    await user.type(screen.getByTestId('title-input'), 'Test Title');
    await user.type(screen.getByTestId('description-textarea'), 'Test Description');

    // Toggle package selection
    const firstCheckbox = screen.getByTestId('package-checkbox-github-devloop');
    expect(firstCheckbox).not.toBeChecked();
    await user.click(firstCheckbox);
    expect(firstCheckbox).toBeChecked();

    // Submit form
    await user.click(screen.getByTestId('submit-button'));

    // Wait for mockSuccessFetch to be called and modal closed
    await waitFor(() => {
      expect(onOpenChange).toHaveBeenCalledWith(false);
    });

    // Verify correct body was sent to /api/v1/goals
    const createCall = fetchSpy.mock.calls.find((call) => call[0].toString().endsWith('/api/v1/goals'));
    expect(createCall).toBeDefined();
    const createReqBody = JSON.parse(createCall![1]?.body as string);
    expect(createReqBody).toEqual({
      title: 'Test Title',
      description: 'Test Description',
      package_names: ['github-devloop'],
      repo: null,
    });
  });

  it('submits successfully with repository and triggers immediately if toggled', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockImplementation(mockSuccessFetch as typeof globalThis.fetch);
    const onOpenChange = vi.fn();

    render(
      <MemoryRouter>
        <QueryClientProvider client={createQueryClient()}>
          <NewGoalModal open={true} onOpenChange={onOpenChange} />
        </QueryClientProvider>
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByTestId('package-graph-list')).toBeInTheDocument();
    });

    const user = userEvent.setup();

    // Fill details
    await user.type(screen.getByTestId('repo-input'), 'owner/repo-name');
    await user.type(screen.getByTestId('title-input'), 'Trigger test goal');
    await user.type(screen.getByTestId('description-textarea'), 'Description');

    // Toggle package selection
    await user.click(screen.getByTestId('package-checkbox-github-devloop'));

    // Toggle triggerOnCreate switch
    const triggerSwitch = screen.getByTestId('trigger-on-create-switch');
    await user.click(triggerSwitch);

    // Submit form
    await user.click(screen.getByTestId('submit-button'));

    await waitFor(() => {
      expect(onOpenChange).toHaveBeenCalledWith(false);
    });

    // Verify createGoal call
    const createCall = fetchSpy.mock.calls.find((call) => call[0].toString().endsWith('/api/v1/goals'));
    expect(createCall).toBeDefined();
    const createReqBody = JSON.parse(createCall![1]?.body as string);
    expect(createReqBody.repo).toEqual({
      owner: 'owner',
      name: 'repo-name',
    });

    // Verify triggerGoal call
    const triggerCall = fetchSpy.mock.calls.find((call) => call[0].toString().endsWith('/trigger'));
    expect(triggerCall).toBeDefined();
    const triggerReqBody = JSON.parse(triggerCall![1]?.body as string);
    expect(triggerReqBody).toEqual({
      repo: {
        owner: 'owner',
        name: 'repo-name',
      },
      repo_mode: 'existing',
    });
  });

  it('validates repository input format', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(mockSuccessFetch as typeof globalThis.fetch);

    render(
      <MemoryRouter>
        <QueryClientProvider client={createQueryClient()}>
          <NewGoalModal open={true} />
        </QueryClientProvider>
      </MemoryRouter>
    );

    const user = userEvent.setup();

    // Type invalid repo format
    await user.type(screen.getByTestId('repo-input'), 'invalid-format');
    await user.click(screen.getByTestId('submit-button'));

    // Verify repo validation error
    expect(await screen.findByTestId('repo-validation-error')).toHaveTextContent(
      "Repository must be in the format 'owner/repo'"
    );
  });

  it('validates whitespace-only title and description', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(mockSuccessFetch as typeof globalThis.fetch);

    render(
      <MemoryRouter>
        <QueryClientProvider client={createQueryClient()}>
          <NewGoalModal open={true} />
        </QueryClientProvider>
      </MemoryRouter>
    );

    const user = userEvent.setup();

    // Type spaces in title and description
    await user.type(screen.getByTestId('title-input'), '   ');
    await user.type(screen.getByTestId('description-textarea'), '   ');
    await user.click(screen.getByTestId('submit-button'));

    // Verify validation errors show up
    expect(await screen.findByTestId('title-validation-error')).toHaveTextContent('Title is required');
    expect(await screen.findByTestId('description-validation-error')).toHaveTextContent('Description is required');
  });

  it('surfaces honest API error on createGoal failure', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation((url, options) => {
      const urlStr = typeof url === 'string' ? url : url instanceof Request ? url.url : String(url);
      const method = options?.method?.toUpperCase() || 'GET';
      if (urlStr.endsWith('/api/v1/goals') && method === 'POST') {
        return Promise.resolve(
          new Response(JSON.stringify({ error: 'Conflict', message: 'Goal with this title already exists' }), {
            status: 409,
            headers: { 'Content-Type': 'application/json' },
          })
        );
      }
      return mockSuccessFetch(urlStr, options);
    });

    render(
      <MemoryRouter>
        <QueryClientProvider client={createQueryClient()}>
          <NewGoalModal open={true} />
        </QueryClientProvider>
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByTestId('package-graph-list')).toBeInTheDocument();
    });

    const user = userEvent.setup();

    await user.type(screen.getByTestId('title-input'), 'Title');
    await user.type(screen.getByTestId('description-textarea'), 'Description');
    await user.click(screen.getByTestId('package-checkbox-github-devloop'));

    // Submit form
    await user.click(screen.getByTestId('submit-button'));

    // Expect submit error to be surfaced honestly without repo target mapping
    const submitError = await screen.findByTestId('submit-error');
    expect(submitError).toHaveTextContent('Goal with this title already exists');
  });

  it('surfaces mapped repo target error on triggerGoal failure', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation((url, options) => {
      const urlStr = typeof url === 'string' ? url : url instanceof Request ? url.url : String(url);
      const method = options?.method?.toUpperCase() || 'GET';
      if (urlStr.includes('/trigger') && method === 'POST') {
        return Promise.resolve(
          new Response(JSON.stringify({ error: 'Unprocessable', message: 'Some other error' }), {
            status: 422,
            headers: { 'Content-Type': 'application/json' },
          })
        );
      }
      return mockSuccessFetch(urlStr, options);
    });

    render(
      <MemoryRouter>
        <QueryClientProvider client={createQueryClient()}>
          <NewGoalModal open={true} />
        </QueryClientProvider>
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByTestId('package-graph-list')).toBeInTheDocument();
    });

    const user = userEvent.setup();

    await user.type(screen.getByTestId('repo-input'), 'owner/repo-name');
    await user.type(screen.getByTestId('title-input'), 'Title');
    await user.type(screen.getByTestId('description-textarea'), 'Description');
    await user.click(screen.getByTestId('package-checkbox-github-devloop'));

    // Toggle triggerOnCreate switch
    const triggerSwitch = screen.getByTestId('trigger-on-create-switch');
    await user.click(triggerSwitch);

    // Submit form
    await user.click(screen.getByTestId('submit-button'));

    // Expect trigger error to be mapped honestly to GitHub App error
    const submitError = await screen.findByTestId('submit-error');
    expect(submitError).toHaveTextContent('GitHub App not installed on owner/repo');
  });

  it('renders the package graph from mock data with correct dep chip styling', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(mockSuccessFetch as typeof globalThis.fetch);

    render(
      <MemoryRouter>
        <QueryClientProvider client={createQueryClient()}>
          <NewGoalModal open={true} />
        </QueryClientProvider>
      </MemoryRouter>
    );

    // Wait for the packages list to render
    await waitFor(() => {
      expect(screen.getByTestId('package-graph-list')).toBeInTheDocument();
    });

    expect(screen.getByText('github-devloop')).toBeInTheDocument();
    expect(screen.getByText('github-proxy')).toBeInTheDocument();
    expect(screen.getByText('consensus')).toBeInTheDocument();

    // Wait for and verify composed deps details sub-queries to resolve and render as chips
    const dep1 = await screen.findByText('dep · github-proxy');
    const dep2 = await screen.findByText('dep · consensus');

    expect(dep1).toBeInTheDocument();
    expect(dep2).toBeInTheDocument();

    // Verify dep chips have bg-raise styling (not bg-raise-2)
    expect(dep1).toHaveClass('bg-raise');
    expect(dep1).not.toHaveClass('bg-raise-2');
  });

  it('renders unknown when package graph is unreachable', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(mockUnreachableFetch as typeof globalThis.fetch);

    render(
      <MemoryRouter>
        <QueryClientProvider client={createQueryClient()}>
          <NewGoalModal open={true} />
        </QueryClientProvider>
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByTestId('package-graph-unknown')).toBeInTheDocument();
    });

    expect(screen.getByText('unknown (unreachable)')).toBeInTheDocument();
  });

  it('renders honest empty state when package list is empty', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(mockEmptyFetch as typeof globalThis.fetch);

    render(
      <MemoryRouter>
        <QueryClientProvider client={createQueryClient()}>
          <NewGoalModal open={true} />
        </QueryClientProvider>
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByTestId('package-graph-empty')).toBeInTheDocument();
    });

    expect(screen.getByText('no packages in the hosted store')).toBeInTheDocument();
    expect(screen.queryByText('unknown (unreachable)')).not.toBeInTheDocument();
  });

  it('closes the modal on Esc keypress, backdrop click, and close button click', async () => {
    const onOpenChange = vi.fn();
    const user = userEvent.setup();

    const { rerender } = render(
      <MemoryRouter>
        <QueryClientProvider client={createQueryClient()}>
          <NewGoalModal open={true} onOpenChange={onOpenChange} />
        </QueryClientProvider>
      </MemoryRouter>
    );

    // Verify modal content is rendered
    expect(screen.getByTestId('new-goal-modal-content')).toBeInTheDocument();

    // 1. Close via close button (×)
    const closeBtn = screen.getByRole('button', { name: /close/i });
    await user.click(closeBtn);
    expect(onOpenChange).toHaveBeenCalledWith(false);

    // Reset mock
    onOpenChange.mockClear();

    // Re-render open modal
    rerender(
      <MemoryRouter>
        <QueryClientProvider client={createQueryClient()}>
          <NewGoalModal open={true} onOpenChange={onOpenChange} />
        </QueryClientProvider>
      </MemoryRouter>
    );

    // 2. Close via Esc key
    await user.keyboard('{Escape}');
    expect(onOpenChange).toHaveBeenCalledWith(false);

    // Reset mock
    onOpenChange.mockClear();

    // Re-render open modal
    rerender(
      <MemoryRouter>
        <QueryClientProvider client={createQueryClient()}>
          <NewGoalModal open={true} onOpenChange={onOpenChange} />
        </QueryClientProvider>
      </MemoryRouter>
    );

    // 3. Close via backdrop click
    const backdrop = screen.getByTestId('dialog-backdrop');
    await user.click(backdrop);
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});
