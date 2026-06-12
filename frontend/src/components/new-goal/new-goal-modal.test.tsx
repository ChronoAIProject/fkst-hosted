import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { MemoryRouter } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { NewGoalModal } from './new-goal-modal';

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

  it('renders fields and disabled submit button with the NyxID note, even after typing', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(mockSuccessFetch as typeof globalThis.fetch);

    render(
      <MemoryRouter>
        <QueryClientProvider client={createQueryClient()}>
          <NewGoalModal open={true} />
        </QueryClientProvider>
      </MemoryRouter>
    );

    // Verify Repository select, title, description fields are present
    const repoTrigger = screen.getByTestId('repo-select-trigger');
    const titleInput = screen.getByTestId('title-input');
    const descTextarea = screen.getByTestId('description-textarea');

    expect(repoTrigger).toBeInTheDocument();
    expect(titleInput).toBeInTheDocument();
    expect(descTextarea).toBeInTheDocument();

    // Verify submit button is disabled initially
    const submitBtn = screen.getByTestId('submit-button');
    expect(submitBtn).toBeInTheDocument();
    expect(submitBtn).toBeDisabled();

    // Verify notes are rendered
    expect(screen.getByTestId('submit-note')).toHaveTextContent('requires NyxID sign-in');
    expect(screen.getByTestId('engine-pickup-footnote')).toHaveTextContent('the engine\'s next ~5-min poll picks it up → Design stage');

    // Type into Title and Description
    const user = userEvent.setup();
    await user.type(titleInput, 'Add user settings panel');
    await user.type(descTextarea, 'Neutral product-work description details');

    // Select a repository via keyboard to trigger value change in JSDOM safely
    repoTrigger.focus();
    await user.keyboard(' '); // opens select listbox
    await user.keyboard('{ArrowDown}'); // highlight first option
    await user.keyboard('{Enter}'); // select it

    // Verify submit button remains disabled (the literal contract)
    expect(submitBtn).toBeDisabled();
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
