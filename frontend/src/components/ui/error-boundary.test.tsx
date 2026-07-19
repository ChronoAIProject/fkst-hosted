import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createMemoryRouter, RouterProvider } from 'react-router-dom';
import { ErrorBoundary, ErrorFallbackView, RouteErrorElement } from './error-boundary';

// A child that always throws during render — the input to both boundaries.
function Boom(): never {
  throw new Error('kaboom');
}

describe('ErrorBoundary (class)', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders the friendly fallback instead of unmounting when a child throws', () => {
    // React logs the caught error to console.error; silence it for a clean run.
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {});

    render(
      <ErrorBoundary>
        <Boom />
      </ErrorBoundary>
    );

    expect(screen.getByRole('alert')).toBeInTheDocument();
    expect(screen.getByText('Something went wrong')).toBeInTheDocument();
    // The thrown message surfaces in the collapsible detail.
    expect(screen.getByText('kaboom')).toBeInTheDocument();
    spy.mockRestore();
  });

  it('renders children untouched on the success path', () => {
    render(
      <ErrorBoundary>
        <p>all good</p>
      </ErrorBoundary>
    );

    expect(screen.getByText('all good')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });
});

describe('RouteErrorElement', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders the fallback in a route whose element threw', () => {
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {});

    const router = createMemoryRouter([
      { path: '/', element: <Boom />, errorElement: <RouteErrorElement /> },
    ]);
    render(<RouterProvider router={router} />);

    expect(screen.getByRole('alert')).toBeInTheDocument();
    expect(screen.getByText('Something went wrong')).toBeInTheDocument();
    expect(screen.getByText('kaboom')).toBeInTheDocument();
    spy.mockRestore();
  });
});

describe('ErrorFallbackView reload', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('hard-reloads the page when the reload control is clicked', async () => {
    const reload = vi.fn();
    // location.reload is read-only in jsdom; stub the whole location object,
    // preserving the fields other code may read.
    vi.stubGlobal('location', { ...window.location, reload });
    const user = userEvent.setup();

    render(<ErrorFallbackView />);
    await user.click(screen.getByRole('button', { name: 'Reload the page' }));

    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('omits the detail block when no detail is supplied', () => {
    render(<ErrorFallbackView />);
    // No <details> disclosure without a detail string.
    expect(screen.queryByText('Error details')).not.toBeInTheDocument();
  });
});
